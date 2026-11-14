import Foundation
import Testing
@testable import QueueFocus

/// The add fields' one draft, and what the model tells the views of sync.
@MainActor
@Suite(.serialized) struct AddDraftTests {
    @Test func anAddedTaskClearsTheDraftAndBlankTextAddsNothing() throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("qf-draft-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let model = QueueModel(engine: try QueueEngine(dir: dir.path))
        let draft = AddDraft()

        draft.text = "   "
        #expect(!draft.submit(to: model, asCurrent: false))
        #expect(draft.text == "   " && !draft.isWaiting)

        draft.text = "write the notes #w"
        #expect(draft.submit(to: model, asCurrent: true))
        #expect(draft.text.isEmpty && !draft.isWaiting)
        #expect(model.snapshot.current?.title == "write the notes")
        #expect(model.settledAdds.isEmpty, "nothing was waited for")
    }

    /// Without sync Now can always change, and there is no sync line.
    @Test func standaloneHasNoSyncLineAndNowCanAlwaysChange() throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("qf-draft-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let model = QueueModel(engine: try QueueEngine(dir: dir.path))
        #expect(model.focusAvailable)
        #expect(model.syncMessage == nil)
    }

    /// Offline, the views show the core's line and keep Now's controls off;
    /// an add for Now is refused and its text stays for another try.
    @Test func offlineTheDraftForNowStaysAndTheCoreSaysWhy() throws {
        let dir = try OfflineSyncDirectory()
        defer { dir.remove() }
        let model = QueueModel(engine: try QueueEngine(dir: dir.url.path))
        let sync = try #require(model.snapshot.sync)
        #expect(sync.state == .offline)
        #expect(!model.focusAvailable)
        #expect(model.syncMessage == sync.summary)
        #expect(sync.summary.hasPrefix("Offline — "))

        let draft = AddDraft()
        draft.text = "!take Now"
        #expect(!draft.submit(to: model, asCurrent: false))
        #expect(draft.text == "!take Now" && !draft.isWaiting)
        #expect(model.actionError?.contains("wait for sync") == true)
        #expect(model.snapshot.current == nil)

        // A queued add needs no server: it is saved here and counted.
        draft.text = "offline capture"
        #expect(draft.submit(to: model, asCurrent: false))
        #expect(model.snapshot.next.map(\.title) == ["offline capture"])
        #expect(model.syncMessage == "1 change saved on this device, waiting to sync.")
    }
}
