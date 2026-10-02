import QuartzCore
import Testing
@testable import QueueFocus

/// A surface that records what it was asked to do.
@MainActor
private final class Surface: FlashSurface {
    var presented: [(stage: CALayer, frame: CGRect, label: String)] = []
    var dismissed = 0

    func present(_ stage: CALayer, frame: CGRect, label: String) {
        presented.append((stage, frame, label))
    }

    func dismiss() {
        dismissed += 1
    }
}

/// A flash's time, passed by hand.
@MainActor
private final class Clock {
    var plays: [@MainActor () -> Void] = []
    var holds: [(seconds: Double, done: @MainActor () -> Void)] = []
    var cancelled: Set<Int> = []

    var timing: FlashTiming {
        FlashTiming(
            play: { _, _, done in self.plays.append(done) },
            wait: { seconds, done in
                let index = self.holds.count
                self.holds.append((seconds, done))
                return { self.cancelled.insert(index) }
            }
        )
    }
}

@MainActor
@Suite struct FlashControllerTests {
    private let screen = FlashScreen(frame: CGRect(x: 0, y: 0, width: 800, height: 600), menuBar: 30, scale: 2)
    private let clock = Clock()

    private func controller(still: Bool = false, screen: FlashScreen?? = nil) -> (FlashController, () -> [Surface]) {
        var made: [Surface] = []
        let shown = screen ?? self.screen
        let controller = FlashController(screen: { shown }, still: { still }, makeSurface: {
            let surface = Surface()
            made.append(surface)
            return surface
        }, timing: clock.timing)
        return (controller, { made })
    }

    @Test func aFlashIsDrawnOverTheScreenAndTakenDownOnceItHasPlayed() throws {
        let (flash, surfaces) = controller()
        flash.show(flashEvent(.edges))
        let surface = try #require(surfaces().first)
        #expect(surface.presented.count == 1)
        #expect(surface.presented.first?.frame == screen.frame)
        #expect(surface.presented.first?.label == "Queue Focus flash: NOW, ship v0.1, 23m")
        #expect(flash.isShowing)
        #expect(clock.holds.isEmpty, "it plays rather than holds")
        clock.plays[0]()
        #expect(surface.dismissed == 1)
        #expect(!flash.isShowing, "and the surface is let go")
    }

    @Test func aNewFlashReplacesOneStillRunning() throws {
        let (flash, surfaces) = controller()
        flash.show(flashEvent(.wash))
        flash.show(flashEvent(.edges))
        #expect(surfaces().count == 2, "a surface per flash")
        #expect(surfaces()[0].dismissed == 1, "the one it interrupted is gone")
        // The first flash's run ends as its layers go; that ends nothing now.
        clock.plays[0]()
        #expect(surfaces()[1].dismissed == 0)
        #expect(flash.isShowing)
        clock.plays[1]()
        #expect(surfaces()[1].dismissed == 1)
    }

    @Test func withReduceMotionTheFlashHoldsStillAtItsPeakThenGoes() throws {
        let (flash, surfaces) = controller(still: true)
        flash.show(flashEvent(.topbarBeam))
        let stage = try #require(surfaces().first?.presented.first?.stage)
        #expect(stage.sublayers?.flatMap { $0.sublayers ?? [] }.allSatisfy { $0.opacity == 1 } == true)
        #expect(clock.plays.isEmpty)
        #expect(clock.holds.map(\.seconds) == [1.5])
        clock.holds[0].done()
        #expect(surfaces()[0].dismissed == 1)
    }

    @Test func aReplacedHoldIsCancelledAndCannotEndItsSuccessor() {
        let (flash, surfaces) = controller(still: true)
        flash.show(flashEvent(.wash))
        flash.show(flashEvent(.wash2))
        #expect(clock.cancelled == [0])
        clock.holds[0].done()
        #expect(surfaces()[1].dismissed == 0)
    }

    @Test func withNoScreenThereIsNoFlash() {
        let (flash, surfaces) = controller(screen: .some(nil))
        flash.show(flashEvent(.wash))
        #expect(surfaces().isEmpty)
        #expect(!flash.isShowing)
    }

    @Test func clearingTwiceOrBeforeAnyFlashIsHarmless() {
        let (flash, surfaces) = controller()
        flash.clear()
        flash.show(flashEvent(.wash))
        flash.clear()
        flash.clear()
        #expect(surfaces()[0].dismissed == 1)
        clock.plays[0]()
        #expect(surfaces()[0].dismissed == 1, "a run that ends after a clear ends nothing")
    }
}
