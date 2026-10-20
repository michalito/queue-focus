import AppKit
import SwiftUI

extension View {
    /// Do something with the AppKit window this view is in, once it is in one.
    func withWindow(_ configure: @escaping @MainActor (NSWindow) -> Void) -> some View {
        background(WindowAccessor(configure: configure))
    }
}

private struct WindowAccessor: NSViewRepresentable {
    let configure: @MainActor (NSWindow) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = WindowReader()
        view.configure = configure
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {}
}

private final class WindowReader: NSView {
    var configure: (@MainActor (NSWindow) -> Void)?

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let window {
            configure?(window)
        }
    }
}
