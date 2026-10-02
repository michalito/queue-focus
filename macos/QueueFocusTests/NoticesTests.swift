import Foundation
import Testing
@testable import QueueFocus

/// Notification Center, stood in for: what was asked, posted and withdrawn,
/// and what is on screen. With `holdsAnswers` or `holdsPosts`, each answer
/// waits until it is released, as the real one takes a moment.
@MainActor
private final class Bell: Notifier {
    var answer: NotePermission
    var accepts = true
    var holdsAnswers = false
    var holdsPosts = false
    var posted: [Note] = []
    var asks = 0
    var withdrawn: [String] = []
    /// The notes on screen, by id: a note replaces one with its id.
    var shown: [String: Note] = [:]
    private var held: [CheckedContinuation<Void, Never>?] = []

    init(_ answer: NotePermission) {
        self.answer = answer
    }

    /// How many answers have been held back so far.
    var waiting: Int { held.count }

    func release(_ index: Int) {
        held[index]?.resume()
        held[index] = nil
    }

    private func hold() async {
        await withCheckedContinuation { held.append($0) }
    }

    func permission() async -> NotePermission {
        if holdsAnswers { await hold() }
        return answer
    }

    func ask() { asks += 1 }

    func post(_ note: Note) async -> Bool {
        if holdsPosts { await hold() }
        guard accepts else { return false }
        posted.append(note)
        shown[note.id] = note
        return true
    }

    func withdraw(_ id: String) {
        withdrawn.append(id)
        shown[id] = nil
    }
}

/// A clock moved by hand, for the popover's eight seconds.
private final class Clock: @unchecked Sendable {
    var date = Date()
}

/// Notices over an engine in a directory of its own, removed afterwards,
/// told of every change as the app tells them, with a popover that only
/// counts its openings.
@MainActor
private final class Setup {
    let dir: URL
    let clock = Clock()
    let model: QueueModel
    let bell: Bell
    var popoverShown = false
    var popoverOpenings = 0
    private(set) var notices: Notices!

    init(_ answer: NotePermission, current: String? = "ship v0.1", next: [String] = ["write notes"]) throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("qf-notices-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let clock = clock
        model = QueueModel(engine: try QueueEngine(dir: dir.path), clock: { clock.date })
        bell = Bell(answer)
        notices = Notices(model: model, notifier: bell, popoverIsShown: { [unowned self] in popoverShown },
                          showPopover: { [unowned self] in popoverOpenings += 1 })
        model.didChange = { [unowned self] in notices.modelDidChange() }
        for title in next {
            model.add(title)
        }
        if let current {
            model.add(current, asCurrent: true)
        }
    }

    /// Saving fails from now on: a folder stands where the task file goes.
    func breakSaving() throws {
        let file = dir.appendingPathComponent("tasks.json")
        try? FileManager.default.removeItem(at: file)
        try FileManager.default.createDirectory(at: file.appendingPathComponent("in-the-way"), withIntermediateDirectories: true)
    }

    func repairSaving() throws {
        try FileManager.default.removeItem(at: dir.appendingPathComponent("tasks.json"))
    }

    /// Let things run until `count` answers have been held back.
    func untilWaiting(_ count: Int) async {
        while bell.waiting < count {
            await Task.yield()
        }
    }

    deinit {
        try? FileManager.default.removeItem(at: dir)
    }
}

@MainActor
@Suite struct NoticesTests {
    @Test func aCompletionIsAnnouncedWithUndo() async throws {
        let s = try Setup(.allowed)
        let task = try #require(s.model.snapshot.current)
        await s.notices.completeCurrent().value
        #expect(s.bell.posted == [Note(id: "done", title: "Done", body: "ship v0.1", undo: task.id)])
        #expect(s.model.undoOffer?.id == task.id, "the popover offers it too")
        #expect(s.popoverOpenings == 0)
        #expect(s.model.snapshot.current?.title == "write notes")
    }

    @Test func anOpenPopoverOffersItInItsOwnRow() async throws {
        let s = try Setup(.allowed)
        s.popoverShown = true
        await s.notices.completeCurrent().value
        #expect(s.bell.posted.isEmpty)
        #expect(s.model.undoOffer != nil)
    }

    @Test func theFirstTimeItAsksAndThePopoverOffersItMeanwhile() async throws {
        let s = try Setup(.notAsked, next: ["one", "two"])
        await s.notices.completeCurrent().value
        #expect(s.bell.asks == 1)
        #expect(s.popoverOpenings == 1, "nothing waits on the answer")
        #expect(s.bell.posted.isEmpty)
        await s.notices.completeCurrent().value
        #expect(s.bell.asks == 1, "the system asks the user once")
        #expect(s.popoverOpenings == 2)
    }

    @Test func withNotificationsOffThePopoverOffersIt() async throws {
        let s = try Setup(.denied)
        await s.notices.completeCurrent().value
        #expect(s.bell.asks == 0 && s.bell.posted.isEmpty)
        #expect(s.popoverOpenings == 1)
        #expect(s.model.undoOffer != nil)
    }

    @Test func aNoteNotTakenFallsBackToThePopover() async throws {
        let s = try Setup(.allowed)
        s.bell.accepts = false
        await s.notices.completeCurrent().value
        #expect(s.popoverOpenings == 1)
    }

    @Test func anEmptyNowSaysSo() async throws {
        let s = try Setup(.allowed, current: nil)
        await s.notices.completeCurrent().value
        #expect(s.bell.posted.map(\.title) == ["Nothing in Now"])
        #expect(s.model.snapshot.next.map(\.title) == ["write notes"], "nothing was completed")
        let off = try Setup(.denied, current: nil)
        await off.notices.completeCurrent().value
        #expect(off.popoverOpenings == 1, "the popover shows Now is empty")
    }

    @Test func aCompletionThatCannotBeSavedSaysWhy() async throws {
        let s = try Setup(.allowed)
        try s.breakSaving()
        await s.notices.completeCurrent().value
        let note = try #require(s.bell.posted.first)
        #expect(note.title == "Could not complete the task")
        #expect(note.body?.contains("could not save") == true, "\(note.body ?? "")")
        #expect(s.model.snapshot.current?.title == "ship v0.1")
        #expect(s.model.undoOffer == nil)
    }

    @Test func aFailedCompletionKeepsItsReasonThroughAnotherRequest() async throws {
        let s = try Setup(.denied)
        s.bell.holdsAnswers = true
        try s.breakSaving()
        let completing = s.notices.completeCurrent()
        await s.untilWaiting(1)
        // Something else goes through meanwhile, and clears the message line.
        try s.repairSaving()
        s.model.add("from a window")
        #expect(s.model.actionError == nil)
        s.bell.release(0)
        await completing.value
        #expect(s.popoverOpenings == 1)
        #expect(s.model.actionError?.contains("could not save") == true, "the popover says why")
    }

    @Test func undoFromTheNotePutsTheTaskBack() async throws {
        let s = try Setup(.allowed)
        let task = try #require(s.model.snapshot.current)
        await s.notices.completeCurrent().value
        await s.notices.undo(id: task.id).value
        #expect(s.model.snapshot.current?.title == "ship v0.1")
        #expect(s.model.undoOffer == nil, "the popover's row goes too")
        #expect(s.bell.shown.isEmpty)
        #expect(s.bell.posted.count == 1, "nothing more to say")
    }

    @Test func aStaleUndoSaysTheQueueChanged() async throws {
        let s = try Setup(.allowed)
        let task = try #require(s.model.snapshot.current)
        await s.notices.completeCurrent().value
        s.model.add("something else")
        #expect(s.bell.shown.isEmpty, "the note went with the change")
        await s.notices.undo(id: task.id).value
        let note = try #require(s.bell.posted.last)
        #expect(note.title == "Nothing to undo" && note.body == "The queue changed since.")
        #expect(s.model.snapshot.current?.title == "write notes")
    }

    @Test func aStaleUndoLeavesTheNewerNoteAlone() async throws {
        let s = try Setup(.allowed, next: ["one", "two"])
        let first = try #require(s.model.snapshot.current)
        await s.notices.completeCurrent().value
        await s.notices.completeCurrent().value
        #expect(s.bell.shown[Note.doneID]?.body == "one")
        // The first note's Undo, pressed late.
        await s.notices.undo(id: first.id).value
        #expect(s.bell.shown[Note.doneID]?.body == "one", "the latest completion is still offered")
    }

    @Test func theDoneNoteGoesOnceItsUndoCannotWork() async throws {
        let s = try Setup(.allowed)
        await s.notices.completeCurrent().value
        #expect(s.bell.shown[Note.doneID] != nil)
        s.model.add("something else")
        #expect(s.bell.shown.isEmpty)
        #expect(s.bell.withdrawn == ["done"], "once")
        s.notices.modelDidChange()
        #expect(s.bell.withdrawn == ["done"])
    }

    @Test func aChangeWhileAskingLeavesNoNoteBehind() async throws {
        let s = try Setup(.allowed)
        s.bell.holdsAnswers = true
        let completing = s.notices.completeCurrent()
        await s.untilWaiting(1)
        s.model.add("something else")
        s.bell.release(0)
        await completing.value
        #expect(s.bell.posted.isEmpty, "its undo could no longer work")
        #expect(s.popoverOpenings == 0)
    }

    /// The second completion waits for the first to be told, so the note on
    /// screen at the end is the second's, whatever order the system answers.
    @Test func completionsAreToldInTurnAndTheLatestNoteStays() async throws {
        let s = try Setup(.allowed, next: ["one", "two"])
        s.bell.holdsPosts = true
        let first = s.notices.completeCurrent()
        await s.untilWaiting(1)
        let second = s.notices.completeCurrent()
        for _ in 0..<50 {
            await Task.yield()
        }
        #expect(s.model.snapshot.current?.title == "one", "the second waits its turn")
        if s.bell.waiting == 2 {
            // Both are being posted: the system answers the second first.
            s.bell.release(1)
            s.bell.release(0)
        } else {
            s.bell.release(0)
            await s.untilWaiting(2)
            s.bell.release(1)
        }
        await first.value
        await second.value
        #expect(s.bell.shown[Note.doneID]?.body == "one", "the latest completion is offered")
        #expect(s.model.snapshot.current?.title == "two")
    }

    @Test func aFailedUndoOffersToTryAgain() async throws {
        let s = try Setup(.allowed)
        let task = try #require(s.model.snapshot.current)
        await s.notices.completeCurrent().value
        try s.breakSaving()
        await s.notices.undo(id: task.id).value
        let retry = try #require(s.bell.shown[Note.doneID])
        #expect(retry.title == "Could not undo" && retry.undo == task.id)
        #expect(retry.body?.contains("could not save") == true)
        try s.repairSaving()
        await s.notices.undo(id: task.id).value
        #expect(s.model.snapshot.current?.title == "ship v0.1")
        #expect(s.bell.shown.isEmpty)
    }

    @Test func aRetryIsNotOfferedOnceTheQueueHasMovedOn() async throws {
        let s = try Setup(.allowed)
        let task = try #require(s.model.snapshot.current)
        await s.notices.completeCurrent().value
        try s.breakSaving()
        s.bell.holdsAnswers = true
        let undoing = s.notices.undo(id: task.id)
        await s.untilWaiting(1)
        try s.repairSaving()
        s.model.add("something else")
        s.bell.release(0)
        await undoing.value
        #expect(s.bell.shown.isEmpty, "the undo can no longer work, so nothing offers it")
        #expect(!s.bell.posted.contains { $0.title == "Could not undo" }, "not even for a moment")
    }

    @Test func aFailedUndoWithoutNotesIsOfferedAgainInThePopover() async throws {
        let s = try Setup(.allowed)
        let task = try #require(s.model.snapshot.current)
        await s.notices.completeCurrent().value
        // The popover's own eight seconds are up.
        s.clock.date += 9
        s.model.tick()
        #expect(s.model.liveUndoOffer == nil)
        s.bell.answer = .denied
        try s.breakSaving()
        await s.notices.undo(id: task.id).value
        #expect(s.popoverOpenings == 1)
        #expect(s.model.liveUndoOffer?.id == task.id, "with its Undo, to try again")
        #expect(s.model.actionError?.contains("could not save") == true)
    }

    @Test func eachProblemIsOneNote() async throws {
        let s = try Setup(.allowed)
        await s.notices.report(["could not save a", "could not save b"]).value
        #expect(s.bell.posted.map(\.body) == ["could not save a", "could not save b"])
        let off = try Setup(.denied)
        await off.notices.report(["could not save a"]).value
        #expect(off.bell.posted.isEmpty && off.popoverOpenings == 0, "the message line has it")
        let first = try Setup(.notAsked)
        await first.notices.report(["a", "b", "c"]).value
        #expect(first.bell.asks == 1)
    }

    @Test func aFailureFromOutsideWithoutNotesShowsInThePopover() async throws {
        let s = try Setup(.denied)
        await s.notices.failed("Could not add the task", "empty title").value
        #expect(s.popoverOpenings == 1)
        #expect(s.model.actionError == "empty title", "on its message line")
    }

    @Test func failuresFromOutsideShowInTheOrderTheyCame() async throws {
        let s = try Setup(.denied)
        s.bell.holdsAnswers = true
        let older = s.notices.failed("Could not add the task", "the older one")
        let newer = s.notices.failed("Could not add the task", "the newer one")
        await s.untilWaiting(1)
        s.bell.release(0)
        await s.untilWaiting(2)
        s.bell.release(1)
        await older.value
        await newer.value
        #expect(s.model.actionError == "the newer one")
    }
}
