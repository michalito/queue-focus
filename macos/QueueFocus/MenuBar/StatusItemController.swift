import AppKit
import SwiftUI

/// The status item: the current task in the menu bar. A click opens the
/// popover; a right click (or Control-click) pauses or resumes the timer.
/// Where Settings asks for the task in the centre of the menu bar, a label
/// there carries the title and the item keeps the dot.
@MainActor
final class StatusItemController: NSObject, NSPopoverDelegate {
    static let identifier = "queue-focus-status-item"
    static let popoverRootIdentifier = "popover-root"

    /// What is drawn, to draw again only when it changes.
    private struct Shown: Equatable {
        var title: StatusTitle
        var options: DisplayOptions
        /// Where the label in the centre is; `nil` with none.
        var label: CGRect?
        /// The item is down to its dot, the label carrying the title.
        var asDot = false
        /// The menu bar's light or dark, which the label takes.
        var appearance: NSAppearance.Name?
    }

    // The item is the controller's to drive; the tests read what it says.
    let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let popover = NSPopover()
    /// How wide the popover's columns have grown while it is open.
    private let popoverWidths = PopoverWidths()
    /// What the popover points at when it opens from the item, made when
    /// it first does.
    private var itemAnchor: PopoverAnchorPanel?
    private let model: QueueModel
    private let display: Display
    private let defaults: UserDefaults
    /// The screen with the menu bar, where the label in the centre goes.
    private let screen: @MainActor () -> CentreTitle.Screen?
    private var shown: Shown?
    /// A redraw is under way, and whether it was asked for again meanwhile.
    private var redrawing = false
    private var redrawAgain = false
    /// The label in the centre of the menu bar, made when first asked for.
    private var centre: CentreTitlePanel?
    /// How the popover last closed.
    private var closing: Closing?
    /// The app that was in front when the popover opened, to hand the
    /// keyboard back to when it closes.
    private var previousApp: NSRunningApplication?

    init(model: QueueModel, loginItem: LoginItem, windows: AppWindows, display: Display, defaults: UserDefaults,
         screen: @escaping @MainActor () -> CentreTitle.Screen? = { NSScreen.screens.first.map { CentreTitle.Screen($0) } },
         quit: @escaping () -> Void) {
        self.model = model
        self.display = display
        self.defaults = defaults
        self.screen = screen
        super.init()

        if let button = item.button {
            button.target = self
            button.action = #selector(clicked)
            // On the way down, as the menu bar's own items answer, and so
            // that the click is the event in hand when the popover it
            // closed says so.
            button.sendAction(on: [.leftMouseDown, .rightMouseDown])
            button.setAccessibilityIdentifier(Self.identifier)
            // Always in a window, so SwiftUI's window actions reach AppKit.
            button.addSubview(NSHostingView(rootView: WindowActionsBridge(windows: windows)))
            // The one place that says whether the menu bar's words are light
            // or dark, which the label in the centre must match.
            let watcher = AppearanceWatcher()
            watcher.didChange = { [weak self] in self?.update() }
            button.addSubview(watcher)
        }
        item.autosaveName = "QueueFocusStatusItem"
        // What moves the label or changes its words without the queue
        // changing: the screens, and the settings kept in the defaults.
        NotificationCenter.default.addObserver(self, selector: #selector(layoutDidChange),
                                               name: NSApplication.didChangeScreenParametersNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(defaultsDidChange),
                                               name: UserDefaults.didChangeNotification, object: defaults)

        let actions = PopoverActions(
            close: { [weak self] in self?.closePopover() },
            quit: quit
        )
        let widths = popoverWidths
        let host = NSHostingController(rootView: FollowingDisplay(display: display) {
            PopoverView(actions: actions)
                .environment(model)
                .environment(loginItem)
                .environment(widths)
        })
        host.sizingOptions = [.preferredContentSize]
        // SwiftUI's root is a group with no name, and nothing to name: the
        // accessibility audit is told which one it is.
        host.view.setAccessibilityIdentifier(Self.popoverRootIdentifier)
        popover.contentViewController = host
        // Dark whatever the Mac's appearance, as GNOME Shell's menus are.
        popover.appearance = NSAppearance(named: .darkAqua)
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
        update()
    }

    /// Redraw the title if anything in it changed. Called after every change
    /// and every tick, so it must be cheap when nothing did. Showing or
    /// moving the label can tell of a change itself, while this is still
    /// drawing: that asks for another pass, after this one.
    func update() {
        guard !redrawing else {
            redrawAgain = true
            return
        }
        redrawing = true
        defer { redrawing = false }
        repeat {
            redrawAgain = false
            redraw()
        } while redrawAgain
    }

    private func redraw() {
        guard let button = item.button else { return }
        let font = NSFont.menuBarFont(ofSize: 0)
        let title = StatusTitle.make(
            current: model.snapshot.current,
            now: model.unixNow,
            showTimer: model.settings.showTimer,
            maxWidth: Preferences.menuBarTitleWidth(in: defaults),
            measure: { ($0 as NSString).size(withAttributes: [.font: font]).width }
        )
        var next = Shown(title: title, options: display.options)
        let words = Preferences.menuBarTitleCentred(in: defaults)
            ? Self.render(title, font: font, options: next.options, color: .labelColor) : nil
        if let words {
            next.label = labelFrame(for: words, dot: Self.renderDot(title, font: font, options: next.options), button: button)
            next.appearance = CentreTitle.appearance(matching: button.effectiveAppearance)
        }
        // The label first: the item gives its title up only to one in view.
        if let words, let frame = next.label {
            let panel = centre ?? makeCentre()
            if shown?.label != frame || shown?.title != title || shown?.options != next.options
                || shown?.appearance != next.appearance {
                panel.appearance = next.appearance.flatMap { NSAppearance(named: $0) }
                panel.label.title = words
                panel.label.spokenTitle = "Queue Focus: \(title.accessibilityLabel)"
                panel.label.toolTip = title.fullTitle
                panel.setFrame(frame, display: true)
                if popover.isShown, popoverIsOnLabel {
                    popover.positioningRect = panel.label.bounds
                }
            }
            if !panel.isVisible {
                panel.orderFrontRegardless()
            }
            next.asDot = panel.isShowing
        } else if let panel = centre, panel.isVisible {
            if popoverIsOnLabel {
                closePopover()
            }
            panel.orderOut(nil)
        }
        guard shown != next else { return }
        if shown?.title != title || shown?.options != next.options || shown?.asDot != next.asDot {
            button.attributedTitle = next.asDot
                ? Self.renderDot(title, font: font, options: next.options)
                : Self.render(title, font: font, options: next.options)
            button.toolTip = title.fullTitle
            button.setAccessibilityLabel("Queue Focus: \(title.accessibilityLabel)")
        }
        shown = next
    }

    /// Where the label goes, on the screen with the menu bar; `nil` where it
    /// cannot, and the item keeps the title. Judged against the item as a
    /// dot, whichever it is now, so the answer does not turn on itself.
    private func labelFrame(for words: NSAttributedString, dot: NSAttributedString, button: NSStatusBarButton) -> CGRect? {
        guard let screen = screen() else { return nil }
        var dotMinX: CGFloat?
        if let item = button.window?.frame, item.width > 0, item.intersects(screen.frame) {
            dotMinX = item.maxX - (dot.size().width + Self.itemPadding)
        }
        return CentreTitle.frame(on: screen, words: words.size().width, dotMinX: dotMinX)
    }

    /// What a status item keeps around its words: a little over the 20
    /// points macOS gives, to judge the dot's place on the safe side.
    static let itemPadding: CGFloat = 24

    private func makeCentre() -> CentreTitlePanel {
        let panel = CentreTitlePanel()
        panel.label.onClick = { [weak self] in self?.labelClicked($0) }
        panel.label.onSecondaryClick = { [weak self] in self?.secondaryClick() }
        // In a full-screen space or Mission Control the label is out of
        // view, and the item takes the title back.
        NotificationCenter.default.addObserver(self, selector: #selector(layoutDidChange),
                                               name: NSWindow.didChangeOcclusionStateNotification, object: panel)
        centre = panel
        return panel
    }

    @objc private func layoutDidChange(_ notification: Notification) {
        update()
    }

    /// The defaults say so on whichever thread changed them.
    @objc private nonisolated func defaultsDidChange(_ notification: Notification) {
        DispatchQueue.main.async { [weak self] in
            self?.update()
        }
    }

    /// A paused task's words, dimmed as GNOME dims its label: a sign of the
    /// pause even with the clock hidden. The menu bar's own text colour at
    /// 70%, worked out each time it is drawn, for the appearance it is drawn
    /// in: `withAlphaComponent` on `labelColor` would fix it at once, black
    /// on a dark menu bar.
    static let pausedTitleColor = NSColor(name: "pausedTitle") { appearance in
        var label = NSColor.black
        appearance.performAsCurrentDrawingAppearance {
            label = NSColor.labelColor.usingColorSpace(.sRGB) ?? .black
        }
        return label.withAlphaComponent(label.alphaComponent * 0.7)
    }

    /// Only the dot is coloured, so the button's pressed state can still
    /// invert the rest; a paused task's words are dimmed. Where colours
    /// should not be the only difference, the tag's letter follows the dot.
    /// The label in the centre is no button, and is told its `color`.
    static func render(_ title: StatusTitle, font: NSFont, options: DisplayOptions,
                       color: NSColor? = nil) -> NSAttributedString {
        let text = NSMutableAttributedString(string: "● ", attributes: dotAttributes(title, font: font))
        var words: [NSAttributedString.Key: Any] = [.font: font]
        var clock: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: font.pointSize, weight: .regular),
        ]
        if let color {
            words[.foregroundColor] = color
            clock[.foregroundColor] = color
        }
        if title.paused {
            words[.foregroundColor] = pausedTitleColor
        }
        if options.differentiateWithoutColor, let tag = title.tag {
            text.append(NSAttributedString(string: "\(TagStyle.letter(tag)) ", attributes: words))
        }
        text.append(NSAttributedString(string: title.title, attributes: words))
        if let timer = title.timer {
            text.append(NSAttributedString(string: "  \(timer)", attributes: clock))
        }
        return text
    }

    /// What the item keeps while the label in the centre carries the title:
    /// the dot, with the tag's letter where it would follow it.
    static func renderDot(_ title: StatusTitle, font: NSFont, options: DisplayOptions) -> NSAttributedString {
        let text = NSMutableAttributedString(string: "●", attributes: dotAttributes(title, font: font))
        if options.differentiateWithoutColor, let tag = title.tag {
            text.append(NSAttributedString(string: " \(TagStyle.letter(tag))", attributes: [.font: font]))
        }
        return text
    }

    private static func dotAttributes(_ title: StatusTitle, font: NSFont) -> [NSAttributedString.Key: Any] {
        [
            .font: NSFont.menuBarFont(ofSize: font.pointSize * 0.7),
            .foregroundColor: TagStyle.dot(title.tag),
            .baselineOffset: 1,
        ]
    }

    // MARK: Clicks

    @objc private func clicked(_ sender: NSStatusBarButton) {
        let event = NSApp.currentEvent
        let secondary = event?.type == .rightMouseDown || event?.modifierFlags.contains(.control) == true
        if secondary {
            secondaryClick()
        } else if closedThePopover(event, on: .item) {
            return
        } else if popover.isShown {
            closePopover()
        } else {
            showPopover(from: sender)
        }
    }

    /// The two things in the menu bar a click opens the popover from.
    enum Opener {
        case item, label
    }

    /// How the popover closed, kept to tell the click that closed it from a
    /// click to open it. The popover is transient: a click outside it
    /// closes it, and the item and the label are outside it. That click
    /// then comes on to the item or the label, and must not open the
    /// popover again.
    struct Closing: Equatable {
        /// When it closed, on the clock that times events.
        var at: TimeInterval
        /// The event in hand as it closed: its kind, and when it was made.
        var by: NSEvent.EventType?
        var eventTime: TimeInterval = 0
        /// What the pointer was on.
        var over: Opener?

        /// The longest the menu bar is taken to need to hand a click on.
        static let handingOn: TimeInterval = 0.5

        /// Whether a left click made at `time` on `opener` is the one that
        /// closed the popover.
        ///
        /// Where a click comes straight to the app, the popover closes on
        /// that very event, and the click has its time. But macOS keeps the
        /// menu bar to itself (from 26 on): a click on the item first has
        /// AppKit close the popover, on an event of its own, and is handed
        /// on after, timed when it was handed on (26 and 77 thousandths of
        /// a second after the close, measured on macOS 27). So a popover
        /// closed by AppKit with the pointer on the opener, and a click on
        /// that opener straight after, are taken for one click.
        ///
        /// Escape, Return, a button in the popover, a click elsewhere: the
        /// next click is none of theirs, and opens the popover.
        func wasMade(by time: TimeInterval, on opener: Opener) -> Bool {
            switch by {
            case .leftMouseDown: time == eventTime
            case .appKitDefined: over == opener && time <= at + Self.handingOn
            default: false
            }
        }

        /// Whether the click is the one that closed the popover, as
        /// `closing` tells it; if so `closing` is used up, one close having
        /// one click: the next click, however soon, is a click to open.
        static func claims(_ closing: inout Closing?, clickAt time: TimeInterval, on opener: Opener) -> Bool {
            guard closing?.wasMade(by: time, on: opener) == true else { return false }
            closing = nil
            return true
        }
    }

    /// Whether `event` is the click that closed the popover. (A press from
    /// VoiceOver is no click, and always acts.)
    private func closedThePopover(_ event: NSEvent?, on opener: Opener) -> Bool {
        guard let event, event.type == .leftMouseDown else { return false }
        return Closing.claims(&closing, clickAt: event.timestamp, on: opener)
    }

    /// Which of the two the pointer is on.
    private var pointerIsOver: Opener? {
        let pointer = NSEvent.mouseLocation
        if let centre, centre.isVisible, centre.frame.contains(pointer) {
            return .label
        }
        if let button = item.button, let window = button.window,
           window.convertToScreen(button.convert(button.bounds, to: nil)).contains(pointer) {
            return .item
        }
        return nil
    }

    private func secondaryClick() {
        closePopover()
        model.togglePause()
    }

    /// A click on the label, or VoiceOver's press, which is no event.
    private func labelClicked(_ event: NSEvent?) {
        if closedThePopover(event, on: .label) { return }
        if popover.isShown {
            closePopover()
        } else if let label = centre?.label {
            showPopover(from: label)
        }
    }

    var isPopoverShown: Bool { popover.isShown }
    /// The popover is open from the label in the centre, not the item.
    private var popoverIsOnLabel = false

    /// How far in from the item's right edge the popover's arrow points:
    /// the middle of the item at its narrowest, the dot alone.
    static let arrowInset: CGFloat = 16

    /// Where in the item the popover points: its right end. macOS packs the
    /// status items from the right, so when a new title changes the item's
    /// width its left edge moves and its right edge stays; pointing there,
    /// the popover stays on the item without moving under the pointer. (An
    /// item to the right that changes width moves this one whole, and the
    /// popover stays where it opened.)
    static func anchor(in bounds: NSRect) -> NSRect {
        // A sliver about that point, kept inside an item too narrow for it.
        let width = min(2, bounds.width)
        let x = min(max(bounds.maxX - arrowInset - width / 2, bounds.minX), bounds.maxX - width)
        return NSRect(x: x, y: bounds.minY, width: width, height: bounds.height)
    }

    /// Open the popover where the title is: under the label in the centre
    /// when it is in view, else from the item.
    func showPopover() {
        if let centre, centre.isShowing {
            showPopover(from: centre.label)
        } else if let button = item.button {
            showPopover(from: button)
        }
    }

    private func showPopover(from view: NSView) {
        // A popover cannot open from a view in no window, and says so with
        // an exception.
        guard let window = view.window else { return }
        closing = nil
        // An accessory app must be active for the add field to take the keys,
        // and a global shortcut, unlike a click, is not reason enough for the
        // system to let it be.
        previousApp = NSWorkspace.shared.frontmostApplication
        if previousApp == .current { previousApp = nil }
        Activation.takeFront()
        // The label's middle stays where it is whatever its words. The
        // item's does not, and its right end is what stays: that place on
        // the screen is marked, and the popover points at the mark.
        popoverIsOnLabel = view !== item.button
        var pointsAt = view
        if !popoverIsOnLabel {
            let anchor = itemAnchor ?? PopoverAnchorPanel()
            itemAnchor = anchor
            anchor.setFrame(window.convertToScreen(view.convert(Self.anchor(in: view.bounds), to: nil)), display: false)
            anchor.orderFrontRegardless()
            pointsAt = anchor.mark
        }
        popoverWidths.cap = ((window.screen ?? NSScreen.main)?.visibleFrame.width ?? 1440) - 24
        popover.show(relativeTo: pointsAt.bounds, of: pointsAt, preferredEdge: .minY)
        let shown = popover.contentViewController?.view.window
        // Hung on a window of the app's own, the popover has no name of its
        // own for VoiceOver: the status item gave it one.
        shown?.setAccessibilityLabel("Queue Focus")
        shown?.makeKey()
    }

    func closePopover() {
        popover.performClose(nil)
    }

    nonisolated func popoverDidClose(_ notification: Notification) {
        MainActor.assumeIsolated {
            popoverIsOnLabel = false
            popoverWidths.reset()
            itemAnchor?.orderOut(nil)
            let event = NSApp.currentEvent
            closing = Closing(at: ProcessInfo.processInfo.systemUptime, by: event?.type, eventTime: event?.timestamp ?? 0,
                              over: pointerIsOver)
            model.actionError = nil
            // Escape or a button closed it while we were still in front: give
            // the keyboard back, unless one of our own windows is where the
            // user went. A click in another app has already moved it there.
            let ownWindowInFront = NSApp.windows.contains { $0.isVisible && $0.isKeyWindow }
            if NSApp.isActive, !ownWindowInFront {
                previousApp?.activate()
            }
            previousApp = nil
        }
    }
}

/// Tells when the menu bar turns light or dark: it sits in the status item's
/// button, the one view that takes the menu bar's appearance.
private final class AppearanceWatcher: NSView {
    var didChange: @MainActor () -> Void = {}

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        didChange()
    }
}
