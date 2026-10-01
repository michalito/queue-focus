import SwiftUI

/// The windows the popover opens.
enum WindowID {
    static let queue = "queue"
    static let board = "board"
}

/// Queue Focus lives in the menu bar: no Dock icon, no window at launch. The
/// status item and its popover are AppKit's (see `StatusItemController`); the
/// windows are SwiftUI scenes.
@main
struct QueueFocusApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        // SwiftUI opens the first scene at launch, even Settings. A menu bar
        // app opens nothing until asked, so the first scene is a menu bar
        // extra that is never shown: the status item is AppKit's own.
        MenuBarExtra("Queue Focus", isInserted: .constant(false)) {
            EmptyView()
        }
        Settings {
            WithModel(delegate: delegate) { SettingsView() }
        }
        Window("Queue", id: WindowID.queue) {
            WithModel(delegate: delegate) { QueueWindow() }
        }
        .defaultSize(width: 400, height: 640)
        Window("Board", id: WindowID.board) {
            WithModel(delegate: delegate) { BoardWindow() }
        }
        .defaultSize(width: 1040, height: 640)
    }
}

/// A window's content with the app's model in its environment. The model
/// exists from launch on; a window opened before then shows nothing.
private struct WithModel<Content: View>: View {
    let delegate: AppDelegate
    @ViewBuilder let content: () -> Content

    var body: some View {
        if let model = delegate.model {
            content().environment(model)
        }
    }
}
