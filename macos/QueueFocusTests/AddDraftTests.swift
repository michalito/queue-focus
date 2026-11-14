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

/// A model that answers adds as told and keeps receipts in a real inbox,
/// so a draft can wait for the server without one.
@MainActor
private final class StandInHost: DraftHost {
    var answer: AddResult = .added
    var inbox = SettlementInbox()
    private(set) var told: [CommandSettlement] = []

    func add(_ text: String, asCurrent: Bool) -> AddResult { answer }
    func awaitSettlement(of command: String, by draft: AddDraft) {
        inbox.wait(for: command, by: draft, latest: nil)
    }
    func takeSettlement(of command: String) -> CommandSettlement? { inbox.take(command) }

    /// A receipt arrives, by either way: the model's `hear`.
    func receive(_ settlement: CommandSettlement) {
        if inbox.hear(settlement) { told.append(settlement) }
    }
}

private func receipt(_ command: String, applied: Bool) -> CommandSettlement {
    CommandSettlement(commandId: command, applied: applied, message: applied ? nil : "Refused.")
}

@MainActor
@Suite struct WaitingDraftTests {
    @Test func aConfirmedTaskClearsTheTextItWasSentWith() {
        let host = StandInHost()
        let draft = AddDraft()
        draft.text = "!ship it"
        host.answer = .pending("c1")
        #expect(!draft.submit(to: host, asCurrent: false), "not added yet")
        #expect(draft.isWaiting && draft.text == "!ship it")
        #expect(!draft.submit(to: host, asCurrent: false), "one add at a time")
        #expect(!draft.settle(with: host), "nothing has come")

        host.receive(receipt("c1", applied: true))
        #expect(draft.settle(with: host))
        #expect(draft.text.isEmpty && !draft.isWaiting)
        #expect(host.inbox.held.isEmpty, "taken, so no longer held")
    }

    @Test func aRefusedTaskKeepsItsTextForAnotherTry() {
        let host = StandInHost()
        let draft = AddDraft()
        draft.text = "!ship it"
        host.answer = .pending("c1")
        _ = draft.submit(to: host, asCurrent: true)
        host.receive(receipt("c1", applied: false))
        #expect(!draft.settle(with: host))
        #expect(draft.text == "!ship it" && !draft.isWaiting)
        host.answer = .added
        #expect(draft.submit(to: host, asCurrent: true), "and it can be sent again")
    }

    @Test func aConfirmationNeverClearsTextThatChanged() {
        let host = StandInHost()
        let draft = AddDraft()
        draft.text = "!first"
        host.answer = .pending("c1")
        _ = draft.submit(to: host, asCurrent: false)
        draft.text = "something else"
        host.receive(receipt("c1", applied: true))
        #expect(!draft.settle(with: host))
        #expect(draft.text == "something else" && !draft.isWaiting)
    }

    /// The status's receipt can come before the field waits for it.
    @Test func aReceiptAlreadyInTheStatusSettlesAtOnce() {
        var inbox = SettlementInbox()
        let field = AddDraft()
        let heard = inbox.hear(receipt("c1", applied: true))
        #expect(heard, "heard, with no one waiting")
        #expect(inbox.held.isEmpty)
        inbox.wait(for: "c1", by: field, latest: receipt("c1", applied: true))
        let first = inbox.take("c1")
        #expect(first?.applied == true)
        inbox.wait(for: "c2", by: field, latest: receipt("c1", applied: true))
        let second = inbox.take("c2")
        #expect(second == nil, "the status's receipt is another command's")
    }

    /// Event and status bring the same receipt: it is told once, whichever
    /// comes first, so a refusal the user put away does not come back.
    @Test func aReceiptHeardTwiceIsToldOnce() {
        let host = StandInHost()
        let refused = receipt("c1", applied: false)
        host.receive(refused)
        host.receive(refused)
        host.receive(refused)
        #expect(host.told == [refused])

        var inbox = SettlementInbox()
        inbox.alreadyTold(refused)
        let again = inbox.hear(refused)
        #expect(!again, "one from before the start is not told again")
    }

    @Test func eachFieldGetsItsOwnReceipt() {
        let host = StandInHost()
        let (one, two) = (AddDraft(), AddDraft())
        one.text = "!one"
        two.text = "!two"
        host.answer = .pending("c1")
        _ = one.submit(to: host, asCurrent: false)
        host.answer = .pending("c2")
        _ = two.submit(to: host, asCurrent: false)
        host.receive(receipt("c2", applied: true))
        #expect(!one.settle(with: host) && one.isWaiting)
        #expect(two.settle(with: host) && two.text.isEmpty)
        host.receive(receipt("c1", applied: false))
        #expect(!one.settle(with: host) && one.text == "!one" && !one.isWaiting)
    }

    /// A field that goes while it waits stops waiting: its receipt is not
    /// held for no one, and the record does not grow.
    @Test func aFieldThatGoesStopsWaiting() {
        let host = StandInHost()
        var draft: AddDraft? = AddDraft()
        draft?.text = "!gone"
        host.answer = .pending("c1")
        _ = draft?.submit(to: host, asCurrent: false)
        #expect(!host.inbox.hasAbandoned)
        draft = nil
        #expect(host.inbox.hasAbandoned)
        host.receive(receipt("c1", applied: true))
        #expect(host.inbox.held.isEmpty, "nobody to hold it for")
        host.inbox.dropAbandoned()
        #expect(!host.inbox.hasAbandoned)

        // Gone after its receipt came: the held receipt goes with it.
        var late: AddDraft? = AddDraft()
        host.answer = .pending("c2")
        late?.text = "!late"
        _ = late?.submit(to: host, asCurrent: false)
        host.receive(receipt("c2", applied: true))
        #expect(host.inbox.held.keys.contains("c2"))
        late = nil
        host.inbox.dropAbandoned()
        #expect(host.inbox.held.isEmpty)
    }

    /// The status keeps a refusal until the next change to Now settles,
    /// while any number of queued edits' receipts pass as events: the
    /// refusal is told once, not again once they have filled the memory.
    @Test func theStatusReceiptIsToldOnceHoweverManyEventsPass() {
        var inbox = SettlementInbox()
        let refused = receipt("focus", applied: false)
        var told = 0
        if inbox.hearInStatus(refused) { told += 1 }
        for index in 0..<(3 * SettlementInbox.remembered) {
            _ = inbox.hear(receipt("queued-\(index)", applied: true))
            if inbox.hearInStatus(refused) { told += 1 }
        }
        #expect(told == 1)
        let next = receipt("next focus", applied: true)
        let replaced = inbox.hearInStatus(next)
        #expect(replaced, "a new receipt in the status is news")

        var launched = SettlementInbox()
        launched.alreadyTold(refused)
        for index in 0..<(2 * SettlementInbox.remembered) {
            _ = launched.hear(receipt("queued-\(index)", applied: true))
        }
        let again = launched.hearInStatus(refused)
        #expect(!again, "nor is the one from before launch")
    }

    @Test func theInboxRemembersABoundedNumberOfReceipts() {
        var inbox = SettlementInbox()
        let fresh = (0..<(SettlementInbox.remembered + 10)).map { index in
            inbox.hear(receipt("c\(index)", applied: true))
        }
        #expect(fresh.allSatisfy { $0 })
        let latest = inbox.hear(receipt("c\(SettlementInbox.remembered + 9)", applied: true))
        #expect(!latest, "the latest are remembered")
        let oldest = inbox.hear(receipt("c0", applied: true))
        #expect(oldest, "the oldest are forgotten")
    }
}
