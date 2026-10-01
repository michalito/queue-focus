import Foundation
import Testing
@testable import QueueFocus

/// The intents against a queue of their own, as Shortcuts would run them.
@MainActor
@Suite(.serialized) struct IntentsTests {
    private let dir: URL
    private let model: QueueModel

    init() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("qf-intents-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        model = QueueModel(engine: try QueueEngine(dir: dir.path))
        IntentHost.model = model
    }

    @Test func addingTakesTheMarkersAndCanMakeItCurrent() async throws {
        defer { try? FileManager.default.removeItem(at: dir) }
        var add = AddTaskIntent()
        add.text = "write notes #w"
        add.asCurrent = false
        _ = try await add.perform()
        #expect(model.snapshot.next.map(\.title) == ["write notes"])
        #expect(model.snapshot.next.first?.tag == .work)
        add.text = "ship"
        add.asCurrent = true
        _ = try await add.perform()
        #expect(model.snapshot.current?.title == "ship")
        add.text = "  "
        await #expect(throws: IntentFailure.self) { try await add.perform() }
    }

    @Test func completingOffersUndoAndSaysWhenNowIsEmpty() async throws {
        defer { try? FileManager.default.removeItem(at: dir) }
        model.add("ship", asCurrent: true)
        _ = try await CompleteCurrentTaskIntent().perform()
        #expect(model.snapshot.current == nil)
        #expect(model.undoOffer?.title == "ship", "the popover offers to undo it")
        _ = try await CompleteCurrentTaskIntent().perform()
        #expect(model.undoOffer?.title == "ship", "an empty Now completes nothing")
    }

    @Test func theCurrentTaskReadsAsTheMenuBarDoes() throws {
        defer { try? FileManager.default.removeItem(at: dir) }
        #expect(CurrentTaskIntent.describe(model) == "")
        model.add("ship v0.1", asCurrent: true)
        #expect(CurrentTaskIntent.describe(model) == "ship v0.1 · 0m")
    }

    @Test func withNoQueueOpenTheyFail() async {
        defer { try? FileManager.default.removeItem(at: dir) }
        IntentHost.model = nil
        defer { IntentHost.model = model }
        await #expect(throws: IntentFailure.self) { try await CurrentTaskIntent().perform() }
    }
}
