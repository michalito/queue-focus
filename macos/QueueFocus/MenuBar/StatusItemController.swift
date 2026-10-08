import AppKit
import SwiftUI

/// The status item: the current task in the menu bar. A click opens the
/// popover; a right click (or Control-click) pauses or resumes the timer.
/// Where Settings asks for the task in the centre of the menu bar, a label
/// there carries the title and the item keeps the dot.
@MainActor
final class StatusItemController: NSObject, NSPopoverDelegate {
    static let identifier = "queue-focus-status-item"

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

    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let popover = NSPopover()
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
    /// When the popover last closed, on the clock that times events.
    private var closedAt: TimeInterval?
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
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
            button.setAccessibilityIdentifier(Self.identifier)
            // Always in a window, so SwiftUI's window actions reach AppKit.
            button.addSubview(NSHostingView(rootView: WindowActionsBridge(windows: windows)))
            // The one place that says whether the menu bar's words are light
            // or dark, which the label in the centre must match.
            let watcher = AppearanceWatcher()
            watcher.didChange = { [weak self] in self?.update() }
            button.addSubview(watcher)
            NotificationCenter.default.addObserver(self, selector: #selector(buttonDidResize),
                                                   name: NSView.frameDidChangeNotification, object: button)
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
        let host = NSHostingController(rootView: FollowingDisplay(display: display) {
            PopoverView(actions: actions)
                .environment(model)
                .environment(loginItem)
        })
        host.sizingOptions = [.preferredContentSize]
        popover.contentViewController = host
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
        let secondary = event?.type == .rightMouseUp || event?.modifierFlags.contains(.control) == true
        if secondary {
            secondaryClick()
        } else if popover.isShown {
            closePopover()
        } else {
            showPopover(from: sender)
        }
    }

    private func secondaryClick() {
        closePopover()
        model.togglePause()
    }

    /// A click on the label, or VoiceOver's press, which is no event. The
    /// popover is transient: a click outside it closes it, and the label is
    /// outside it. A click made before the popover last closed is the one
    /// that closed it, and must not open it again.
    private func labelClicked(_ event: NSEvent?) {
        if let event, let closedAt, event.timestamp <= closedAt { return }
        if popover.isShown {
            closePopover()
        } else if let label = centre?.label {
            showPopover(from: label)
        }
    }

    var isPopoverShown: Bool { popover.isShown }
    /// What the item says now: the title, or the dot alone.
    var itemWords: String { item.button?.attributedTitle.string ?? "" }
    /// The popover is open from the label in the centre, not the item.
    private var popoverIsOnLabel = false

    /// How far in from the item's right edge the popover's arrow points:
    /// the middle of the item at its narrowest, the dot alone.
    static let arrowInset: CGFloat = 16

    /// What the popover points at: the item's right end. macOS packs the
    /// status items from the right, so when a new title changes the item's
    /// width its left edge moves and its right edge stays; pointing there,
    /// the popover stays on the item without moving under the pointer.
    /// (An item to the right that changes width moves this one whole, and
    /// the popover, as before, stays where it opened.)
    static func anchor(in bounds: NSRect) -> NSRect {
        NSRect(x: bounds.maxX - arrowInset * 2, y: bounds.minY, width: arrowInset * 2, height: bounds.height)
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
        // An accessory app must be active for the add field to take the keys,
        // and a global shortcut, unlike a click, is not reason enough for the
        // system to let it be.
        previousApp = NSWorkspace.shared.frontmostApplication
        if previousApp == .current { previousApp = nil }
        Activation.takeFront()
        // The label's middle stays where it is whatever its words; the
        // item's does not.
        popoverIsOnLabel = view !== item.button
        popover.show(relativeTo: popoverIsOnLabel ? view.bounds : Self.anchor(in: view.bounds), of: view,
                     preferredEdge: .minY)
        popover.contentViewController?.view.window?.makeKey()
    }

    /// The anchor is in the button's coordinates, which a new width moves:
    /// say again where it is, or the popover's next layout would use the
    /// place it was.
    @objc private func buttonDidResize(_ notification: Notification) {
        guard popover.isShown, !popoverIsOnLabel, let button = item.button else { return }
        popover.positioningRect = Self.anchor(in: button.bounds)
    }

    func closePopover() {
        popover.performClose(nil)
    }

    nonisolated func popoverDidClose(_ notification: Notification) {
        MainActor.assumeIsolated {
            popoverIsOnLabel = false
            closedAt = ProcessInfo.processInfo.systemUptime
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
