import QuartzCore

/// A flash plan as Core Animation layers, played by the render server: the
/// app does no work while a flash runs. Built without a window, so tests can
/// read every layer and animation.
enum FlashStage {
    // flash.js eases every step with Clutter's ease-in-out-quad, which no
    // one cubic curve draws (Core Animation's ease-in-ease-out is another
    // curve). Its two halves are quadratics, which cubics draw exactly, so
    // each step is split at its middle: in to there, out from there.
    static var easeIn: CAMediaTimingFunction {
        CAMediaTimingFunction(controlPoints: 1.0 / 3, 0, 2.0 / 3, 1.0 / 3)
    }

    static var easeOut: CAMediaTimingFunction {
        CAMediaTimingFunction(controlPoints: 1.0 / 3, 2.0 / 3, 2.0 / 3, 1)
    }

    /// A run through `values` at `keyTimes`, each step eased as Clutter eases
    /// it: the keyframes Core Animation needs to draw it exactly.
    static func eased(_ values: [Double], at keyTimes: [Double]) -> CAKeyframeAnimation {
        var points = [values[0]]
        var times = [keyTimes[0]]
        var timing: [CAMediaTimingFunction] = []
        for i in 1..<min(values.count, keyTimes.count) {
            points += [(values[i - 1] + values[i]) / 2, values[i]]
            times += [(keyTimes[i - 1] + keyTimes[i]) / 2, keyTimes[i]]
            timing += [easeIn, easeOut]
        }
        let animation = CAKeyframeAnimation()
        animation.values = points
        animation.keyTimes = times.map { NSNumber(value: $0) }
        animation.timingFunctions = timing
        animation.calculationMode = .linear
        return animation
    }

    /// The plan's layers on one stage the size of the screen, in GNOME's
    /// coordinates: the stage is flipped, so y runs down from the top left.
    /// A layer moves as one, but each of its parts fades on its own, as
    /// Clutter fades an actor's children: where two glow strips overlap,
    /// their light adds up at every step of the fade. Everything is left
    /// where its run ends, so nothing jumps when the animations are taken
    /// off, or at its peak when the flash is `still`. `card` is the card's
    /// picture, `scale` pixels to the point.
    static func build(_ plan: FlashPlan, card: CGImage?, scale: CGFloat, still: Bool) -> CALayer {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }

        let stage = CALayer()
        stage.isGeometryFlipped = true
        stage.frame = CGRect(origin: .zero, size: plan.screen)
        for spec in plan.layers {
            let layer = CALayer()
            layer.anchorPoint = spec.pivot
            layer.frame = spec.frame
            if let motion = spec.motion {
                layer.setValue(motion.to, forKeyPath: keyPath(motion.property))
            }
            for part in spec.parts {
                let drawn = draw(part, card: card, cardScale: plan.card.scale, scale: scale)
                drawn.opacity = Float(still ? spec.envelope.peak : spec.envelope.end)
                layer.addSublayer(drawn)
            }
            stage.addSublayer(layer)
        }
        return stage
    }

    /// Play the stage `build` made from `plan`, and call `done` once its last
    /// layer has played out. The stage must already be on screen: Core
    /// Animation ends the animations of a layer it is not drawing at once.
    static func play(_ stage: CALayer, _ plan: FlashPlan, done: @escaping @MainActor () -> Void) {
        CATransaction.begin()
        CATransaction.setCompletionBlock {
            MainActor.assumeIsolated { done() }
        }
        for (layer, spec) in zip(stage.sublayers ?? [], plan.layers) {
            let run = animations(for: spec)
            for part in layer.sublayers ?? [] {
                part.add(run.fade, forKey: "fade")
            }
            if let motion = run.motion {
                layer.add(motion, forKey: "motion")
            }
        }
        CATransaction.commit()
    }

    /// One layer's run: the fade each of its parts plays, and the movement
    /// the layer makes, if it moves.
    static func animations(for layer: FlashLayer) -> (fade: CAKeyframeAnimation, motion: CAKeyframeAnimation?) {
        let envelope = layer.envelope
        let fade = eased(envelope.values, at: envelope.keyTimes)
        fade.keyPath = "opacity"
        fade.duration = envelope.duration
        guard let motion = layer.motion else { return (fade, nil) }
        let move = eased([motion.from, motion.to], at: [0, 1])
        move.keyPath = keyPath(motion.property)
        move.duration = motion.fraction * envelope.duration
        return (fade, move)
    }

    private static func keyPath(_ property: Motion.Property) -> String {
        switch property {
        case .scaleY: "transform.scale.y"
        case .translationY: "transform.translation.y"
        }
    }

    private static func draw(_ part: FlashPart, card: CGImage?, cardScale: Double, scale: CGFloat) -> CALayer {
        switch part.shape {
        case .fill(let color):
            let layer = CALayer()
            layer.frame = part.frame
            layer.backgroundColor = cgColor(color)
            return layer
        case .border(let width, let color):
            let layer = CALayer()
            layer.frame = part.frame
            layer.borderWidth = width
            layer.borderColor = cgColor(color)
            return layer
        case .gradient(let start, let end, let vertical):
            let layer = CAGradientLayer()
            layer.frame = part.frame
            layer.colors = [cgColor(start), cgColor(end)]
            // Under the flipped stage, y = 0 is the top.
            layer.startPoint = vertical ? CGPoint(x: 0.5, y: 0) : CGPoint(x: 0, y: 0.5)
            layer.endPoint = vertical ? CGPoint(x: 0.5, y: 1) : CGPoint(x: 1, y: 0.5)
            return layer
        case .card:
            let layer = CALayer()
            guard let card else { return layer }
            layer.contents = card
            layer.contentsScale = scale
            layer.bounds = CGRect(x: 0, y: 0, width: CGFloat(card.width) / scale, height: CGFloat(card.height) / scale)
            layer.position = CGPoint(x: part.frame.midX, y: part.frame.midY)
            layer.setAffineTransform(CGAffineTransform(scaleX: cardScale, y: cardScale))
            return layer
        }
    }

    private static func cgColor(_ color: RGBA) -> CGColor {
        CGColor(srgbRed: color.red / 255, green: color.green / 255, blue: color.blue / 255, alpha: color.alpha)
    }
}
