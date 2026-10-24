import AppKit
import CoreGraphics
import Foundation
import Testing
@testable import QueueFocus

/// Typing in the quick add panel, which XCTest's UI tests cannot do: they
/// find the field but never see it take the keyboard, since the panel never
/// makes the app active. Here the keys go to the panel itself, as AppKit
/// would deliver them.
@MainActor
@Suite(.serialized) struct QuickAddTests {
    /// Sync already has a status line at launch, while the panel is hidden.
    /// Its hosting view must settle before it is ever opened, and resize
    /// when a message is added or cleared without a constraint feedback loop.
    @Test func aHiddenSyncedPanelLaysOutAndItsMessagesCanChange() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("qf-quick-sync-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let device = UUID().uuidString.lowercased()
        let envelope: [String: Any] = [
            "version": 1, "device": device, "server_time": 0, "wall_time": 0,
            "pending": [], "handles": [:], "next_handle": 1, "recovery": [], "suspension": NSNull(),
            "confirmed": [
                "version": 1, "dataset": UUID().uuidString, "generation": UUID().uuidString,
                "revision": 1, "history": String(repeating: "0", count: 64), "initialized": true,
                "tasks": [], "focus": ["version": UUID().uuidString, "session": NSNull()], "undo": NSNull(),
            ],
        ]
        let config: [String: Any] = [
            "version": 1, "url": "http://127.0.0.1:1", "device": device,
            "token": "localtest", "allow_loopback_http": true,
        ]
        for (name, value) in [("sync-state.json", envelope), ("sync-config.json", config)] {
            let file = dir.appendingPathComponent(name)
            try JSONSerialization.data(withJSONObject: value).write(to: file)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        }
        let model = QueueModel(engine: try QueueEngine(dir: dir.path))
        #expect(model.syncMessage != nil)
        let quickAdd = QuickAddController(model: model, display: Display(defaults: UserDefaults(suiteName: "qf-quick-add-tests")!))
        defer { quickAdd.close() }
        let panel = try #require(NSApp.windows.last { $0.identifier == QuickAddController.identifier })
        try await Task.sleep(for: .milliseconds(150))
        panel.contentView?.layoutSubtreeIfNeeded()
        #expect(!quickAdd.isShown)
        #expect(try await eventually { panel.frame.height > 60 }, "the sync status has been laid out")
        let initialHeight = panel.frame.height
        model.actionError = "The server could not confirm this change. Your draft is still available."
        #expect(try await eventually { panel.frame.height > initialHeight })
        model.actionError = nil
        #expect(try await eventually { abs(panel.frame.height - initialHeight) < 1 },
                "clearing the message restores height \(initialHeight), got \(panel.frame.height)")
        #expect(!quickAdd.isShown)
    }

    private func key(_ characters: String, code: UInt16, in window: NSWindow, modifiers: NSEvent.ModifierFlags = []) -> [NSEvent] {
        [NSEvent.EventType.keyDown, .keyUp].compactMap { type in
            NSEvent.keyEvent(with: type, location: .zero, modifierFlags: modifiers, timestamp: ProcessInfo.processInfo.systemUptime,
                             windowNumber: window.windowNumber, context: nil, characters: characters,
                             charactersIgnoringModifiers: characters, isARepeat: false, keyCode: code)
        }
    }

    private func type(_ text: String, in window: NSWindow) {
        for character in text {
            for event in key(String(character), code: 0, in: window) {
                window.sendEvent(event)
            }
        }
    }

    /// Wait up to two seconds for `condition`.
    private func eventually(_ condition: () -> Bool) async throws -> Bool {
        for _ in 0..<40 {
            if condition() { return true }
            try await Task.sleep(for: .milliseconds(50))
        }
        return condition()
    }

    /// Whether a window can take the keyboard at all: on a locked screen
    /// none becomes key, and the panel's field could not either.
    nonisolated static var screenIsUnlocked: Bool {
        let session = CGSessionCopyCurrentDictionary() as? [String: Any]
        return session?["CGSSessionScreenIsLocked"] as? Bool != true
    }

    /// Five times over, each on a new panel, as the first time it opens is
    /// when SwiftUI builds the field.
    @Test(.enabled(if: screenIsUnlocked, "the screen is locked, so no window can take the keyboard"))
    func typingAndReturnAddsTheTaskAndClosesThePanel() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("qf-quick-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let model = QueueModel(engine: try QueueEngine(dir: dir.path))
        for round in 1...5 {
            let quickAdd = QuickAddController(model: model, display: Display(defaults: UserDefaults(suiteName: "qf-quick-add-tests")!))
            quickAdd.show()
            defer { quickAdd.close() }
            let panel = try #require(NSApp.windows.last { $0.identifier == QuickAddController.identifier && $0.isVisible })
            #expect(try await eventually { panel.firstResponder is NSTextView }, "round \(round): the field has the keyboard")
            type("task \(round)", in: panel)
            for event in key("\r", code: 36, in: panel) {
                panel.sendEvent(event)
            }
            #expect(try await eventually { !quickAdd.isShown }, "round \(round): a task added closes the panel")
        }
        // The first, added while nothing was being done, is the one to do.
        #expect(model.snapshot.current?.title == "task 1")
        #expect(model.snapshot.next.map(\.title) == (2...5).map { "task \($0)" })
    }
}
