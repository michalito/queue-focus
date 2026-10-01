import AppKit
import Foundation
import Testing
@testable import QueueFocus

/// Typing in the quick add panel, which XCTest's UI tests cannot do: they
/// find the field but never see it take the keyboard, since the panel never
/// makes the app active. Here the keys go to the panel itself, as AppKit
/// would deliver them.
@MainActor
@Suite struct QuickAddTests {
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

    /// Five times over, each on a new panel, as the first time it opens is
    /// when SwiftUI builds the field.
    @Test func typingAndReturnAddsTheTaskAndClosesThePanel() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("qf-quick-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let model = QueueModel(engine: try QueueEngine(dir: dir.path))
        for round in 1...5 {
            let quickAdd = QuickAddController(model: model)
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
        #expect(model.snapshot.next.map(\.title) == (1...5).map { "task \($0)" })
    }
}
