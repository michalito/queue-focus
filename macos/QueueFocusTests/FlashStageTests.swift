import AppKit
import QuartzCore
import Testing
@testable import QueueFocus

private let size = CGSize(width: 400, height: 300)

private func stage(_ style: FlashStyle, card: CGImage? = nil, still: Bool) -> (FlashPlan, CALayer) {
    let plan = FlashPlan.make(flashEvent(style), screen: size, menuBar: 30)
    return (plan, FlashStage.build(plan, card: card, scale: 1, still: still))
}

/// A stage drawn off screen, read by pixel from the top left.
private struct Picture {
    let width: Int
    let height: Int
    let bytes: [UInt8]

    /// Drawn as the panel hosts it, one layer down: a layer's flip is how
    /// its superlayer places what is in it.
    init(_ stage: CALayer) {
        let layer = CALayer()
        layer.frame = stage.frame
        layer.addSublayer(stage)
        let width = Int(layer.bounds.width)
        let height = Int(layer.bounds.height)
        self.width = width
        self.height = height
        var bytes = [UInt8](repeating: 0, count: width * height * 4)
        bytes.withUnsafeMutableBytes { buffer in
            let context = CGContext(data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
                                    bytesPerRow: width * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
            layer.render(in: context)
        }
        self.bytes = bytes
    }

    /// Red, green, blue and alpha, premultiplied, at `x` and `y` from the top left.
    func at(_ x: Int, _ y: Int) -> (r: Int, g: Int, b: Int, a: Int) {
        let i = (y * width + x) * 4
        return (Int(bytes[i]), Int(bytes[i + 1]), Int(bytes[i + 2]), Int(bytes[i + 3]))
    }
}

/// Clutter's ease-in-out-quad.
private func quad(_ t: Double) -> Double {
    t < 0.5 ? 2 * t * t : 1 - 2 * (1 - t) * (1 - t)
}

/// How far along a cubic timing curve is at `x`, as Core Animation solves it.
private func progress(_ function: CAMediaTimingFunction, at x: Double) -> Double {
    var point = [Float](repeating: 0, count: 2)
    function.getControlPoint(at: 1, values: &point)
    let (x1, y1) = (Double(point[0]), Double(point[1]))
    function.getControlPoint(at: 2, values: &point)
    let (x2, y2) = (Double(point[0]), Double(point[1]))
    let bezier = { (s: Double, a: Double, b: Double) in 3 * (1 - s) * (1 - s) * s * a + 3 * (1 - s) * s * s * b + s * s * s }
    var low = 0.0, high = 1.0
    for _ in 0..<60 {
        let middle = (low + high) / 2
        if bezier(middle, x1, x2) < x { low = middle } else { high = middle }
    }
    return bezier((low + high) / 2, y1, y2)
}

/// A keyframe animation's value at fraction `t` of its run.
private func value(of animation: CAKeyframeAnimation, at t: Double) -> Double {
    let times = animation.keyTimes?.map(\.doubleValue) ?? []
    let values = animation.values as? [Double] ?? []
    let i = min(times.lastIndex { $0 <= t } ?? 0, times.count - 2)
    guard times[i + 1] > times[i] else { return values[i + 1] }
    let u = (t - times[i]) / (times[i + 1] - times[i])
    return values[i] + (values[i + 1] - values[i]) * progress(animation.timingFunctions![i], at: u)
}

/// A card picture red in its top half and blue in its bottom half.
private func twoToneCard() -> CGImage {
    let context = CGContext(data: nil, width: 100, height: 40, bitsPerComponent: 8, bytesPerRow: 0,
                            space: CGColorSpace(name: CGColorSpace.sRGB)!,
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    context.setFillColor(CGColor(srgbRed: 0, green: 0, blue: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: 100, height: 20))
    context.setFillColor(CGColor(srgbRed: 1, green: 0, blue: 0, alpha: 1))
    context.fill(CGRect(x: 0, y: 20, width: 100, height: 20))
    return context.makeImage()!
}

@MainActor
@Suite struct FlashStageTests {
    @Test func theStageLaysThePlanOutTopDown() {
        let (plan, stage) = stage(.topbarBeam, still: false)
        #expect(stage.isGeometryFlipped)
        #expect(stage.frame == CGRect(origin: .zero, size: size))
        let layers = stage.sublayers ?? []
        #expect(layers.count == plan.layers.count)
        for (layer, spec) in zip(layers, plan.layers) {
            #expect(layer.frame == spec.frame)
            #expect(layer.opacity == 1, "a layer moves as one")
            #expect((layer.sublayers ?? []).count == spec.parts.count)
            #expect((layer.sublayers ?? []).allSatisfy { $0.opacity == 0 },
                    "each part left where its run ends, so nothing jumps when it is over")
        }
        #expect(layers[1].anchorPoint == CGPoint(x: 0.5, y: 0), "the beam grows from its top")
    }

    @Test func aStillFlashIsAtItsPeakWithEverythingInPlace() {
        let (_, stage) = stage(.topbarBeam, still: true)
        let layers = stage.sublayers ?? []
        #expect(layers.flatMap { $0.sublayers ?? [] }.allSatisfy { $0.opacity == 1 })
        #expect(layers[1].value(forKeyPath: "transform.scale.y") as? Double == 1, "the beam drawn, not scaled away")
        #expect(layers[2].value(forKeyPath: "transform.translation.y") as? Double == 0, "the card has landed")
    }

    @Test func eachLayerPlaysItsEnvelopeEasedExactlyAsClutterEasesIt() throws {
        let (plan, _) = stage(.topbarBeam, still: false)
        for spec in plan.layers {
            let run = FlashStage.animations(for: spec)
            let fade = run.fade
            #expect(fade.keyPath == "opacity")
            #expect(fade.duration == spec.envelope.duration)
            // Every step of the envelope, sampled across its run.
            let values = spec.envelope.values
            let times = spec.envelope.keyTimes
            for i in 1..<values.count {
                for t in stride(from: 0.0, through: 1, by: 0.05) {
                    let at = times[i - 1] + t * (times[i] - times[i - 1])
                    let expected = values[i - 1] + (values[i] - values[i - 1]) * quad(t)
                    #expect(abs(value(of: fade, at: at) - expected) < 1e-4, "step \(i) at \(t)")
                }
            }
            let move = run.motion
            #expect((move == nil) == (spec.motion == nil))
            if let move, let motion = spec.motion {
                #expect(move.duration == motion.fraction * spec.envelope.duration)
                for t in stride(from: 0.0, through: 1, by: 0.05) {
                    let expected = motion.from + (motion.to - motion.from) * quad(t)
                    #expect(abs(value(of: move, at: t) - expected) < 1e-4, "motion at \(t)")
                }
            }
        }
    }

    @Test func playingFadesEachPartAndMovesTheLayer() {
        let (plan, stage) = stage(.topbarBeam, still: false)
        FlashStage.play(stage, plan) {}
        for (layer, spec) in zip(stage.sublayers ?? [], plan.layers) {
            #expect(layer.animation(forKey: "fade") == nil, "the layer itself never fades")
            #expect((layer.sublayers ?? []).allSatisfy { $0.animation(forKey: "fade") != nil })
            #expect((layer.animation(forKey: "motion") != nil) == (spec.motion != nil))
        }
    }

    @Test func overlappingGlowAddsUpAsInClutter() throws {
        let (_, stage) = stage(.edgesSoft, still: true)
        // Halfway through a fade. Clutter dims each strip on its own, so a
        // corner where two overlap is 1 − (1 − ½a)², not ½(1 − (1 − a)²).
        for strip in try #require(stage.sublayers?.first?.sublayers) {
            strip.opacity = 0.5
        }
        let corner = Picture(stage).at(1, 1).a
        #expect(corner > 130, "about 142 of 255, not 114: \(corner)")
    }

    @Test func theTopBarIsAtTheTopAndTheBeamDropsFromIt() {
        let (_, stage) = stage(.topbarBeam, still: true)
        let picture = Picture(stage)
        #expect(picture.at(20, 10).b > 100, "the band colours the top")
        #expect(picture.at(20, 290).a == 0, "and leaves the bottom alone")
        #expect(picture.at(200, 60).b > 200, "the beam runs below the band")
        #expect(picture.at(200, 250).a == 0, "and stops at the middle")
    }

    @Test func theGlowIsSolidAtTheEdgeAndClearInside() {
        let (_, stage) = stage(.edgesSoft, still: true)
        let picture = Picture(stage)
        #expect(picture.at(200, 1).a > picture.at(200, 60).a, "the top strip fades downwards")
        #expect(picture.at(200, 298).a > picture.at(200, 240).a, "the bottom strip upwards")
        #expect(picture.at(1, 150).a > picture.at(60, 150).a, "the left strip rightwards")
        #expect(picture.at(398, 150).a > picture.at(340, 150).a, "the right strip leftwards")
        #expect(picture.at(200, 150).a == 0, "the middle is clear")
    }

    @Test func theCardIsTheRightWayUpInTheMiddle() {
        let (_, stage) = stage(.wash, card: twoToneCard(), still: true)
        let picture = Picture(stage)
        // The card is 100 by 40, centred on 200, 150, over a light wash.
        let top = picture.at(200, 140)
        let bottom = picture.at(200, 160)
        #expect(top.r > 200 && top.b < 120, "red on top: \(top)")
        #expect(bottom.b > 200 && bottom.r < 60, "blue below: \(bottom)")
    }

    @Test func theCardSlidesDownIntoPlace() throws {
        let (plan, stage) = stage(.wash, card: twoToneCard(), still: true)
        let topOfCard = { (picture: Picture) in (0..<picture.height).first { picture.at(200, $0).r > 200 } }
        let landed = try #require(topOfCard(Picture(stage)))
        // Where the slide starts, by the plan's own numbers.
        let card = try #require(stage.sublayers?.last)
        let start = try #require(plan.layers.last?.motion?.from)
        card.setValue(start, forKeyPath: "transform.translation.y")
        let starting = try #require(topOfCard(Picture(stage)))
        #expect(landed - starting == 4, "it starts 4 points higher and comes down")
    }

    @Test func aDismissedPanelIsLetGo() {
        weak var gone: FlashPanel?
        autoreleasepool {
            let panel = FlashPanel()
            // Shown for real, as a flash is, but empty and a point wide.
            panel.present(CALayer(), frame: CGRect(x: 0, y: 0, width: 1, height: 1), label: "")
            panel.dismiss()
            gone = panel
        }
        #expect(gone == nil, "an app that runs for days keeps no panel per flash")
    }

    @Test func thePanelKeepsTheStageTheRightWayUp() throws {
        let (plan, stage) = stage(.topbar, still: true)
        let panel = FlashPanel()
        panel.install(stage, size: plan.screen, label: "Queue Focus flash: NOW, ship v0.1, 23m")
        let root = try #require(panel.contentView?.layer)
        #expect(stage.isGeometryFlipped, "AppKit leaves a hosted layer's sublayers alone")
        let band = stage.convert(try #require(stage.sublayers?.first).frame, to: root)
        #expect(band.maxY == 300 && band.minY == 270, "the band is at the top of the window: \(band)")
        #expect(panel.contentView?.accessibilityIdentifier() == "flash-overlay")
        #expect(panel.contentView?.accessibilityLabel() == "Queue Focus flash: NOW, ship v0.1, 23m")
        #expect(panel.ignoresMouseEvents && !panel.canBecomeKey && !panel.canBecomeMain)
        #expect(panel.level == .screenSaver)
        #expect(panel.collectionBehavior.isSuperset(of: [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]))
        #expect(!panel.isVisible, "installed, never shown")
    }
}
