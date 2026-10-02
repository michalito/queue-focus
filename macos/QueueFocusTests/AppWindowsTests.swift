import AppKit
import SwiftUI
import Testing
@testable import QueueFocus

/// The global shortcuts and links open windows with actions that a SwiftUI
/// view in the status item lends. macOS hides the item when the menu bar is
/// full, or when the user turns it off, and the view must lend them then too.
@MainActor
@Suite struct AppWindowsTests {
    @Test func aHiddenStatusItemStillLendsTheWindowActions() async throws {
        let windows = AppWindows()
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        defer { NSStatusBar.system.removeStatusItem(item) }
        item.isVisible = false
        let button = try #require(item.button)
        button.addSubview(NSHostingView(rootView: WindowActionsBridge(windows: windows)))
        for _ in 0..<40 where !windows.isBridged {
            try await Task.sleep(for: .milliseconds(50))
        }
        #expect(windows.isBridged, "the bridge appeared in the hidden item's button")
    }
}
