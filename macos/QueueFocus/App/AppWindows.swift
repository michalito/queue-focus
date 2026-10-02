import AppKit
import SwiftUI

/// The Queue and Board windows, for code outside SwiftUI: the global
/// shortcuts and links. SwiftUI opens its windows only from inside a view,
/// so `WindowActionsBridge`, never seen, lends its actions here.
@MainActor
final class AppWindows {
    fileprivate var open: (String) -> Void = { _ in }
    fileprivate var dismiss: (String) -> Void = { _ in }

    /// Open the window, or bring it forward, with the keyboard in it.
    func show(_ id: String) {
        Activation.takeFront()
        open(id)
    }

    /// Close the window if it is the one in front, else show it, as the GTK
    /// app's shortcut shows and hides its window.
    func toggle(_ id: String) {
        if NSApp.isActive, let key = NSApp.keyWindow, Self.window(key, is: id) {
            dismiss(id)
        } else {
            show(id)
        }
    }

    /// SwiftUI names a scene's window after the scene's id.
    static func window(_ window: NSWindow, is id: String) -> Bool {
        guard let name = window.identifier?.rawValue else { return false }
        return name == id || name.hasPrefix(id + "-")
    }
}

/// A view with nothing to show, kept in the status item, that hands SwiftUI's
/// window actions to `AppWindows` once it is in a window.
struct WindowActionsBridge: View {
    let windows: AppWindows
    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismissWindow) private var dismissWindow

    var body: some View {
        Color.clear
            .frame(width: 0, height: 0)
            .accessibilityHidden(true)
            .onAppear {
                windows.open = { openWindow(id: $0) }
                windows.dismiss = { dismissWindow(id: $0) }
            }
    }
}

enum Activation {
    /// Bring the app in front from a global shortcut. Plain `activate()` only
    /// asks, and the system grants it to an app the user just clicked; a hot
    /// key is not a click, so this insists.
    @MainActor
    static func takeFront() {
        NSApp.activate(ignoringOtherApps: true)
    }
}
