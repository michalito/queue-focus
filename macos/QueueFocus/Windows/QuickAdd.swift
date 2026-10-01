import AppKit
import SwiftUI

/// The quick add window: a field that floats over whatever is in front,
/// takes the keyboard without bringing the app forward, and goes away on
/// Escape or as soon as something else takes the keyboard.
@MainActor
final class QuickAddController: NSObject, NSWindowDelegate {
    static let identifier = NSUserInterfaceItemIdentifier("quick-add")
    private let panel: QuickAddPanel

    init(model: QueueModel) {
        panel = QuickAddPanel(
            contentRect: NSRect(x: 0, y: 0, width: 520, height: 60),
            styleMask: [.nonactivatingPanel, .titled, .fullSizeContentView],
            backing: .buffered,
            defer: true
        )
        super.init()
        panel.identifier = Self.identifier
        panel.title = "Quick Add"
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = true
        panel.level = .floating
        panel.isFloatingPanel = true
        panel.hidesOnDeactivate = false
        panel.collectionBehavior = [.moveToActiveSpace, .fullScreenAuxiliary]
        panel.isReleasedWhenClosed = false
        for button in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            panel.standardWindowButton(button)?.isHidden = true
        }
        let content = QuickAddView(close: { [weak self] in self?.close() }).environment(model)
        let host = NSHostingController(rootView: content)
        host.sizingOptions = [.preferredContentSize]
        panel.contentViewController = host
        panel.delegate = self
    }

    var isShown: Bool { panel.isVisible }

    /// Over the middle of the main screen, a little above centre, as
    /// Spotlight sits.
    func show() {
        if let screen = NSScreen.main {
            let area = screen.visibleFrame
            let size = panel.frame.size
            panel.setFrameOrigin(NSPoint(x: area.midX - size.width / 2, y: area.midY + area.height / 6))
        }
        panel.makeKeyAndOrderFront(nil)
        // SwiftUI's focus does not reach a field in a panel that never
        // activates the app, so hand it the keyboard the AppKit way.
        DispatchQueue.main.async { [panel] in
            if let field = panel.contentView?.firstDescendant(NSTextField.self), panel.makeFirstResponder(field) {
                // VoiceOver, and anything else that follows the keyboard,
                // hears where it went: a panel that never activates the app
                // does not announce it by itself.
                NSAccessibility.post(element: field, notification: .focusedUIElementChanged)
            }
        }
    }

    func close() {
        panel.orderOut(nil)
    }

    nonisolated func windowDidResignKey(_ notification: Notification) {
        MainActor.assumeIsolated { close() }
    }
}

private extension NSView {
    /// The first view of type `T` in this view's subtree, depth first.
    func firstDescendant<T: NSView>(_ type: T.Type) -> T? {
        for subview in subviews {
            if let match = subview as? T ?? subview.firstDescendant(type) {
                return match
            }
        }
        return nil
    }
}

/// A panel can take the keyboard though it never brings the app forward.
private final class QuickAddPanel: NSPanel {
    override var canBecomeKey: Bool { true }
}

/// The field. Return adds where Settings says; Command-Return adds the task
/// as the current one; Escape closes. A task added closes the panel; one
/// that could not be added stays, with the reason under it.
private struct QuickAddView: View {
    @Environment(QueueModel.self) private var model
    let close: () -> Void
    @State private var draft = ""

    var body: some View {
        VStack(spacing: 0) {
            TextField("Add…  !now  #w #p  @later @side", text: $draft)
                .textFieldStyle(.plain)
                .font(.title2)
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
                .onSubmit {
                    submit(asCurrent: NSApp.currentEvent?.modifierFlags.contains(.command) == true)
                }
                // The app is not in front, so no menu sees Command-Return.
                .onKeyPress(.return, phases: .down) { press in
                    guard press.modifiers.contains(.command) else { return .ignored }
                    submit(asCurrent: true)
                    return .handled
                }
                .onKeyPress(.escape) {
                    close()
                    return .handled
                }
                .onChange(of: draft) { _, text in
                    let limit = Int(maxTitleChars())
                    if text.count > limit { draft = String(text.prefix(limit)) }
                }
                .accessibilityIdentifier("quick-add-field")
            MessageLine()
        }
        .frame(width: 520)
    }

    private func submit(asCurrent: Bool) {
        if model.add(draft, asCurrent: asCurrent) {
            draft = ""
            close()
        }
    }
}
