import AppKit

/// Where the task's title goes when it is shown in the centre of the menu
/// bar. macOS places status items itself, packed from the right, so the
/// centre is reached with a label of the app's own, drawn over the bar.
enum CentreTitle {
    /// The room kept between the label and the app's own status item.
    static let gap: CGFloat = 8
    /// What the label keeps clear to either side of its words.
    static let padding: CGFloat = 8

    /// What decides where the label can go on a screen.
    struct Screen: Equatable {
        /// In global coordinates.
        var frame: CGRect
        /// The screen less the menu bar and the Dock.
        var visibleFrame: CGRect
        /// A camera housing takes the middle of the menu bar.
        var hasNotch: Bool

        @MainActor
        init(_ screen: NSScreen) {
            frame = screen.frame
            visibleFrame = screen.visibleFrame
            hasNotch = screen.safeAreaInsets.top > 0 || screen.auxiliaryTopLeftArea != nil
        }

        init(frame: CGRect, visibleFrame: CGRect, hasNotch: Bool = false) {
            self.frame = frame
            self.visibleFrame = visibleFrame
            self.hasNotch = hasNotch
        }
    }

    /// The label's frame on `screen`: in the menu bar, its middle on the
    /// screen's, as wide as `words` and its padding. `nil` where it cannot
    /// go: on a screen that keeps no menu bar in view, or whose middle a
    /// notch takes, and when it would reach `dotMinX`, the left edge the
    /// app's status item has as a dot. The item is the one thing in the menu
    /// bar the app can see; other apps' menus and items it cannot.
    static func frame(on screen: Screen, words: CGFloat, dotMinX: CGFloat?) -> CGRect? {
        let bar = screen.frame.maxY - screen.visibleFrame.maxY
        guard bar > 0, !screen.hasNotch else { return nil }
        // A whole number of points to either side of the middle, so that
        // the middle is the same whatever the words.
        let half = (words / 2 + padding).rounded(.up)
        let frame = CGRect(x: (screen.frame.midX - half).rounded(), y: screen.visibleFrame.maxY,
                           width: 2 * half, height: bar)
        guard frame.minX >= screen.frame.minX else { return nil }
        if let dotMinX, frame.maxX + gap > dotMinX { return nil }
        return frame
    }
}

/// The label: the task's title as the status item draws it. A click opens
/// the popover under it, a right click or a Control-click pauses, as on the
/// status item.
final class CentreTitleView: NSView {
    static let identifier = "queue-focus-centre-title"

    var title = NSAttributedString() {
        didSet { needsDisplay = true }
    }
    /// What VoiceOver reads; the words drawn are cut short and hold signs.
    var spokenTitle = ""
    var onClick: (NSEvent?) -> Void = { _ in }
    var onSecondaryClick: () -> Void = {}

    override func draw(_ dirtyRect: NSRect) {
        let size = title.size()
        // On whole pixels, or the words blur.
        let origin = backingAlignedRect(
            NSRect(x: (bounds.width - size.width) / 2, y: (bounds.height - size.height) / 2,
                   width: size.width, height: size.height),
            options: .alignAllEdgesNearest
        ).origin
        title.draw(at: origin)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }

    // The panel never has the keyboard, so every click is a first one.
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    // On the way down, as a menu bar item answers.
    override func mouseDown(with event: NSEvent) {
        if event.modifierFlags.contains(.control) {
            onSecondaryClick()
        } else {
            onClick(event)
        }
    }

    override func rightMouseDown(with event: NSEvent) {
        onSecondaryClick()
    }

    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .button }
    override func accessibilityLabel() -> String? { spokenTitle }
    override func accessibilityIdentifier() -> String { Self.identifier }

    override func accessibilityPerformPress() -> Bool {
        onClick(nil)
        return true
    }
}

/// The label's window: a borderless panel over the menu bar, which never
/// takes the keyboard or the app's turn in front. It keeps out of
/// full-screen spaces, where the menu bar is out of view, and of Mission
/// Control.
final class CentreTitlePanel: NSPanel {
    static let identifier = NSUserInterfaceItemIdentifier("centre-title-panel")

    let label = CentreTitleView()

    init() {
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        identifier = Self.identifier
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        level = .statusBar
        collectionBehavior = [.canJoinAllSpaces, .transient, .ignoresCycle]
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        isExcludedFromWindowsMenu = true
        isRestorable = false
        animationBehavior = .none
        contentView = label
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    /// In the menu bar: AppKit would push a window below it.
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect {
        frameRect
    }

    /// Whether the label can be seen: not in a full-screen space, nor in
    /// Mission Control, nor under something else.
    var isShowing: Bool {
        isVisible && occlusionState.contains(.visible)
    }
}
