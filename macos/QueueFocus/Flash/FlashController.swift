import AppKit

/// The screen a flash is drawn on.
struct FlashScreen: Equatable {
    /// In global coordinates.
    let frame: CGRect
    /// How tall the menu bar is, which the top bar styles colour.
    let menuBar: Double
    /// Pixels to the point.
    let scale: CGFloat

    /// The screen with the menu bar, as GNOME flashes its primary monitor;
    /// nil with no screen at all.
    @MainActor
    static func main() -> FlashScreen? {
        guard let screen = NSScreen.screens.first else { return nil }
        // Below the menu bar, or below the notch, which is taller.
        var menuBar = max(screen.frame.maxY - screen.visibleFrame.maxY, screen.safeAreaInsets.top)
        if menuBar <= 0 {
            // A menu bar that hides itself leaves no gap to measure. The main
            // menu knows its height; the status bar's thickness is older
            // than today's taller menu bar, and is the last resort.
            menuBar = NSApp.mainMenu?.menuBarHeight ?? 0
            if menuBar <= 0 { menuBar = NSStatusBar.system.thickness }
        }
        return FlashScreen(frame: screen.frame, menuBar: Double(menuBar), scale: screen.backingScaleFactor)
    }
}

/// Where a flash is drawn.
@MainActor
protocol FlashSurface: AnyObject {
    /// Put `stage` over `frame`, above everything, and say what it is to
    /// anyone listening with VoiceOver.
    func present(_ stage: CALayer, frame: CGRect, label: String)
    func dismiss()
}

/// How a flash's time passes.
struct FlashTiming {
    /// Play a stage on screen; `done` once its last layer has played out.
    var play: @MainActor (CALayer, FlashPlan, @escaping @MainActor () -> Void) -> Void
    /// `done` after this many seconds, unless the returned cancel comes first.
    var wait: @MainActor (Double, @escaping @MainActor () -> Void) -> (() -> Void)

    static let live = FlashTiming(
        play: { stage, plan, done in FlashStage.play(stage, plan, done: done) },
        wait: { seconds, done in
            let work = DispatchWorkItem { MainActor.assumeIsolated { done() } }
            DispatchQueue.main.asyncAfter(deadline: .now() + seconds, execute: work)
            return { work.cancel() }
        }
    )
}

/// Draws the flashes the reminder asks for: one surface per flash, above
/// every window on the screen with the menu bar, never in the pointer's way,
/// and gone again in under two seconds.
@MainActor
final class FlashController {
    private let screen: @MainActor () -> FlashScreen?
    private let still: @MainActor () -> Bool
    private let makeSurface: @MainActor () -> FlashSurface
    private let timing: FlashTiming
    private var surface: FlashSurface?
    /// Bumped for every flash, so a superseded one's end does nothing.
    private var generation = 0
    private var cancelHold: (() -> Void)?

    init(
        screen: @escaping @MainActor () -> FlashScreen? = FlashScreen.main,
        still: @escaping @MainActor () -> Bool = { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion },
        makeSurface: @escaping @MainActor () -> FlashSurface = { FlashPanel() },
        timing: FlashTiming = .live
    ) {
        self.screen = screen
        self.still = still
        self.makeSurface = makeSurface
        self.timing = timing
    }

    /// A flash is on screen.
    var isShowing: Bool { surface != nil }

    /// Draw one flash. One still running is replaced outright.
    func show(_ event: FlashEvent) {
        clear()
        guard let screen = screen() else { return }
        let generation = generation
        let plan = FlashPlan.make(event, screen: screen.frame.size, menuBar: screen.menuBar)
        // With Reduce Motion on, the flash holds still at its peak, then goes.
        let still = still()
        let stage = FlashStage.build(plan, card: FlashCardView.render(plan.card, scale: screen.scale),
                                     scale: screen.scale, still: still)
        let surface = makeSurface()
        self.surface = surface
        surface.present(stage, frame: screen.frame, label: Self.label(plan.card))
        let done: @MainActor () -> Void = { [weak self] in
            guard let self, self.generation == generation else { return }
            self.clear()
        }
        if still {
            cancelHold = timing.wait(FlashPlan.stillDuration, done)
        } else {
            timing.play(stage, plan, done)
        }
    }

    /// Take down whatever is on screen. Safe at any time, twice over.
    func clear() {
        generation += 1
        cancelHold?()
        cancelHold = nil
        surface?.dismiss()
        surface = nil
    }

    nonisolated static func label(_ card: FlashCard) -> String {
        (["Queue Focus flash: NOW", card.title] + [card.timer].compactMap { $0 }).joined(separator: ", ")
    }
}

/// A borderless panel over the whole screen, at the screen saver's level: over
/// the menu bar and full-screen apps, under the pointer. It never takes the
/// keyboard, the pointer, or the app's turn in front.
final class FlashPanel: NSPanel, FlashSurface {
    static let identifier = NSUserInterfaceItemIdentifier("flash-panel")
    /// The flash as VoiceOver, and the UI tests, find it.
    static let accessibilityIdentifier = "flash-overlay"

    init() {
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        identifier = Self.identifier
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        ignoresMouseEvents = true
        level = .screenSaver
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        isExcludedFromWindowsMenu = true
        // AppKit's own fade on ordering in and out would be a second envelope.
        animationBehavior = .none
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    /// Exactly the screen, menu bar included: AppKit would push a window
    /// below the menu bar on older systems.
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect {
        frameRect
    }

    func present(_ stage: CALayer, frame: CGRect, label: String) {
        install(stage, size: frame.size, label: label)
        setFrame(frame, display: false)
        orderFrontRegardless()
    }

    /// Put `stage` in the panel without showing it.
    func install(_ stage: CALayer, size: CGSize, label: String) {
        let view = NSView(frame: CGRect(origin: .zero, size: size))
        // Layer hosting, with the stage one layer down: AppKit sets the
        // flip of a layer a view hosts to match the view, and the stage's
        // own flip is what puts y = 0 at the top of the screen.
        view.layer = CALayer()
        view.wantsLayer = true
        view.layer?.addSublayer(stage)
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.group)
        view.setAccessibilityIdentifier(Self.accessibilityIdentifier)
        view.setAccessibilityLabel(label)
        contentView = view
    }

    /// Off the screen and out of the app's window list.
    func dismiss() {
        contentView = nil
        close()
    }
}
