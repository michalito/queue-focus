import AppKit
import SwiftUI

/// The quick add window: a field that floats over whatever is in front,
/// takes the keyboard without bringing the app forward, and goes away on
/// Escape or as soon as something else takes the keyboard.
@MainActor
final class QuickAddController: NSObject, NSWindowDelegate {
    static let identifier = NSUserInterfaceItemIdentifier("quick-add")
    private let panel: QuickAddPanel

    init(model: QueueModel, display: Display) {
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
        let close: () -> Void = { [weak self] in self?.close() }
        let content = FollowingDisplay(display: display) {
            QuickAddView(close: close).environment(model)
        }
        let host = NSHostingController(rootView: content)
        // Keep the content's minimum height while following its preferred
        // size: a sync message otherwise makes the panel's constraints loop.
        host.sizingOptions = [.minSize, .preferredContentSize]
        // The view's header is the panel's title bar: no room is kept for
        // the bar the panel does not show.
        host.safeAreaRegions = []
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
        // activates the app, so hand it the keyboard the AppKit way. SwiftUI
        // builds the field on the panel's first layout, so lay it out now;
        // and SwiftUI may still move the keyboard as it settles, so hand it
        // over again once it has.
        panel.contentView?.layoutSubtreeIfNeeded()
        focusField()
        DispatchQueue.main.async { [weak self] in self?.focusField() }
    }

    private func focusField() {
        guard panel.isKeyWindow, !(panel.firstResponder is NSText),
              let field = panel.contentView?.firstDescendant(NSTextField.self), panel.makeFirstResponder(field)
        else { return }
        // VoiceOver, and anything else that follows the keyboard, hears
        // where it went: a panel that never activates the app does not
        // announce it by itself.
        NSAccessibility.post(element: field, notification: .focusedUIElementChanged)
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

/// The field. Return adds where Settings says, or to Now while it is empty;
/// Command-Return adds the task as the current one; Escape closes. A task
/// added closes the panel; one that could not be added stays, with the
/// reason under it.
private struct QuickAddView: View {
    @Environment(QueueModel.self) private var model
    let close: () -> Void
    @State private var draft = ""
    @State private var pendingAdd: PendingAdd?

    var body: some View {
        VStack(spacing: 0) {
            // GNOME's quick add is a small window of its own: its title bar,
            // then the entry. This one floats, as a panel, and moves by its
            // background.
            ZStack {
                Text("Add task")
                    .font(Adw.font(1, .bold))
                    .accessibilityAddTraits(.isHeader)
                HStack {
                    Spacer()
                    Button(action: close) {
                        Image(systemName: "xmark")
                            .font(.system(size: 9, weight: .heavy))
                            .frame(width: 22, height: 22)
                            .background(Adw.wash(0.1), in: Circle())
                    }
                    .buttonStyle(.plain)
                    .help("Close (Escape)")
                    .accessibilityLabel("Close")
                }
                .padding(.trailing, 10)
            }
            .frame(height: 38)
            .frame(maxWidth: .infinity)
            .background(Color(nsColor: Adw.headerBarNS))
            TextField("Add…  !now  #w #p  @later @side  (⏎)", text: $draft)
                .disabled(pendingAdd != nil)
                .onChange(of: model.focusResults) { _, _ in reconcilePendingAdd() }
                .onAppear { reconcilePendingAdd() }
                .modifier(AdwEntry(font: Adw.font(1.2), padding: EdgeInsets(top: 7, leading: 14, bottom: 7, trailing: 14),
                                   focused: true))
                .padding(8)
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
                .accessibilityLabel("Add a task")
                .accessibilityIdentifier("quick-add-field")
            MessageLine()
        }
        .frame(width: 520)
        .background(Adw.window)
        .foregroundStyle(Adw.foreground)
    }

    private func submit(asCurrent: Bool) {
        guard pendingAdd == nil else { return }
        if model.add(draft, asCurrent: asCurrent) {
            draft = ""
            close()
        } else if let command = model.lastRequestPending {
            pendingAdd = PendingAdd(command: command, text: draft)
            reconcilePendingAdd()
        }
    }

    private func reconcilePendingAdd() {
        guard let pending = pendingAdd, let result = model.focusResults[pending.command] else { return }
        pendingAdd = nil
        if result.applied && draft == pending.text {
            draft = ""
            close()
        }
    }
}
