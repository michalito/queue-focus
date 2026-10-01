import QuartzCore

/// A flash plan as Core Animation layers, played by the render server: the
/// app does no work while a flash runs. Built without a window, so tests can
/// read every layer and animation.
enum FlashStage {
    /// The ease flash.js gives every step: Clutter's ease-in-out-quad.
    /// Core Animation's own ease-in-ease-out is a different curve.
    static var ease: CAMediaTimingFunction {
        CAMediaTimingFunction(controlPoints: 0.455, 0.03, 0.515, 0.955)
    }

    /// The plan's layers on one stage the size of the screen, in GNOME's
    /// coordinates: the stage is flipped, so y runs down from the top left.
    /// Each layer is left where its run ends, so nothing jumps when the
    /// animations are taken off, or at its peak when the flash is `still`.
    /// `card` is the card's picture, `scale` pixels to the point.
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
            layer.opacity = Float(still ? spec.envelope.peak : spec.envelope.end)
            if let motion = spec.motion {
                layer.setValue(motion.to, forKeyPath: keyPath(motion.property))
            }
            for part in spec.parts {
                layer.addSublayer(draw(part, card: card, cardScale: plan.card.scale, scale: scale))
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
            for (key, animation) in animations(for: spec) {
                layer.add(animation, forKey: key)
            }
        }
        CATransaction.commit()
    }

    /// One layer's run: its fade, then its movement if it has one.
    static func animations(for layer: FlashLayer) -> [(key: String, animation: CAAnimation)] {
        let envelope = layer.envelope
        let fade = CAKeyframeAnimation(keyPath: "opacity")
        fade.values = envelope.values
        fade.keyTimes = envelope.keyTimes.map { NSNumber(value: $0) }
        fade.timingFunctions = Array(repeating: ease, count: envelope.steps.count)
        fade.calculationMode = .linear
        fade.duration = envelope.duration
        var run: [(key: String, animation: CAAnimation)] = [("fade", fade)]
        if let motion = layer.motion {
            let move = CABasicAnimation(keyPath: keyPath(motion.property))
            move.fromValue = motion.from
            move.toValue = motion.to
            move.duration = motion.fraction * envelope.duration
            move.timingFunction = ease
            run.append(("motion", move))
        }
        return run
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
