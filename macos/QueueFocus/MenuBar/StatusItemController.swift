import AppKit
import SwiftUI

/// The status item: the current task in the menu bar. A click opens the
/// popover; a right click (or Control-click) pauses or resumes the timer.
@MainActor
final class StatusItemController: NSObject, NSPopoverDelegate {
    static let identifier = "queue-focus-status-item"

    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let popover = NSPopover()
    private let model: QueueModel
    private let display: Display
    private let defaults: UserDefaults
    private var shown: (title: StatusTitle, options: DisplayOptions)?
    /// The app that was in front when the popover opened, to hand the
    /// keyboard back to when it closes.
    private var previousApp: NSRunningApplication?

    init(model: QueueModel, loginItem: LoginItem, windows: AppWindows, display: Display, defaults: UserDefaults,
         quit: @escaping () -> Void) {
        self.model = model
        self.display = display
        self.defaults = defaults
        super.init()

        if let button = item.button {
            button.target = self
            button.action = #selector(clicked)
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
            button.setAccessibilityIdentifier(Self.identifier)
            // Always in a window, so SwiftUI's window actions reach AppKit.
            button.addSubview(NSHostingView(rootView: WindowActionsBridge(windows: windows)))
        }
        item.autosaveName = "QueueFocusStatusItem"

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
    /// and every tick, so it must be cheap when nothing did.
    func update() {
        let font = NSFont.menuBarFont(ofSize: 0)
        let title = StatusTitle.make(
            current: model.snapshot.current,
            now: model.unixNow,
            showTimer: model.settings.showTimer,
            maxWidth: Preferences.menuBarTitleWidth(in: defaults),
            measure: { ($0 as NSString).size(withAttributes: [.font: font]).width }
        )
        let options = display.options
        guard shown?.title != title || shown?.options != options, let button = item.button else { return }
        shown = (title, options)
        button.attributedTitle = Self.render(title, font: font, options: options)
        button.toolTip = title.fullTitle
        button.setAccessibilityLabel("Queue Focus: \(title.accessibilityLabel)")
    }

    /// A paused task's words, dimmed as GNOME dims its label: a sign of the
    /// pause even with the clock hidden.
    static let pausedTitleColor = NSColor.labelColor.withAlphaComponent(0.7)

    /// Only the dot is coloured, so the button's pressed state can still
    /// invert the rest; a paused task's words are dimmed. Where colours
    /// should not be the only difference, the tag's letter follows the dot.
    static func render(_ title: StatusTitle, font: NSFont, options: DisplayOptions) -> NSAttributedString {
        let text = NSMutableAttributedString()
        text.append(NSAttributedString(string: "● ", attributes: [
            .font: NSFont.menuBarFont(ofSize: font.pointSize * 0.7),
            .foregroundColor: TagStyle.dot(title.tag),
            .baselineOffset: 1,
        ]))
        var words: [NSAttributedString.Key: Any] = [.font: font]
        if title.paused {
            words[.foregroundColor] = pausedTitleColor
        }
        if options.differentiateWithoutColor, let tag = title.tag {
            text.append(NSAttributedString(string: "\(TagStyle.letter(tag)) ", attributes: words))
        }
        text.append(NSAttributedString(string: title.title, attributes: words))
        if let timer = title.timer {
            text.append(NSAttributedString(string: "  \(timer)", attributes: [
                .font: NSFont.monospacedDigitSystemFont(ofSize: font.pointSize, weight: .regular),
            ]))
        }
        return text
    }

    // MARK: Clicks

    @objc private func clicked(_ sender: NSStatusBarButton) {
        let event = NSApp.currentEvent
        let secondary = event?.type == .rightMouseUp || event?.modifierFlags.contains(.control) == true
        if secondary {
            closePopover()
            model.togglePause()
        } else if popover.isShown {
            closePopover()
        } else {
            showPopover()
        }
    }

    var isPopoverShown: Bool { popover.isShown }

    func showPopover() {
        guard let button = item.button else { return }
        // An accessory app must be active for the add field to take the keys,
        // and a global shortcut, unlike a click, is not reason enough for the
        // system to let it be.
        previousApp = NSWorkspace.shared.frontmostApplication
        if previousApp == .current { previousApp = nil }
        Activation.takeFront()
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        popover.contentViewController?.view.window?.makeKey()
    }

    func closePopover() {
        popover.performClose(nil)
    }

    nonisolated func popoverDidClose(_ notification: Notification) {
        MainActor.assumeIsolated {
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
