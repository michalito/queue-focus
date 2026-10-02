import Foundation
import Testing
@testable import QueueFocus

/// Notification Center, stood in for: what was posted, asked and withdrawn.
/// With `holds`, each answer about permission waits until it is released,
/// as the real one takes a moment.
@MainActor
private final class Bell: Notifier {
    var answer: NotePermission
    var accepts = true
    var holds = false
    var posted: [Note] = []
    var asks = 0
    var withdrawn: [String] = []
    private var held: [CheckedContinuation<Void, Never>?] = []

    init(_ answer: NotePermission) {
        self.answer = answer
    }

    /// How many answers have been asked for and are waiting.
    var waiting: Int { held.count }

    func release(_ index: Int) {
        held[index]?.resume()
        held[index] = nil
    }

    func permission() async -> NotePermission {
        if holds {
            await withCheckedContinuation { held.append($0) }
        }
        return answer
    }

    func ask() { asks += 1 }

    func post(_ note: Note) async -> Bool {
        if accepts { posted.append(note) }
        return accepts
    }

    func withdraw(_ id: String) { withdrawn.append(id) }
}

/// Notices over an engine in a directory of its own, removed afterwards,
/// with a popover that only counts its openings.
@MainActor
private final class Setup {
    let dir: URL
    let model: QueueModel
    let bell: Bell
    var popoverShown = false
    var popoverOpenings = 0
    private(set) var notices: Notices!

    init(_ answer: NotePermission, current: String? = "ship v0.1", next: [String] = ["write notes"]) throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("qf-notices-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        model = QueueModel(engine: try QueueEngine(dir: dir.path))
        bell = Bell(answer)
        notices = Notices(model: model, notifier: bell, popoverIsShown: { [unowned self] in popoverShown },
                          showPopover: { [unowned self] in popoverOpenings += 1 })
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

    /// Run `work` until it waits on the stand-in for an answer.
    func start(_ work: @escaping @MainActor () async -> Void) async -> Task<Void, Never> {
        let count = bell.waiting
        let task = Task { await work() }
        while bell.waiting == count {
            await Task.yield()
        }
        return task
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
        await s.notices.completeCurrent()
        #expect(s.bell.posted == [Note(id: "done", title: "Done", body: "ship v0.1", undo: task.id)])
        #expect(s.model.undoOffer?.id == task.id, "the popover offers it too")
        #expect(s.popoverOpenings == 0)
        #expect(s.model.snapshot.current?.title == "write notes")
    }

    @Test func anOpenPopoverOffersItInItsOwnRow() async throws {
        let s = try Setup(.allowed)
        s.popoverShown = true
        await s.notices.completeCurrent()
        #expect(s.bell.posted.isEmpty)
        #expect(s.model.undoOffer != nil)
    }

    @Test func theFirstTimeItAsksAndThePopoverOffersItMeanwhile() async throws {
        let s = try Setup(.notAsked, next: ["one", "two"])
        await s.notices.completeCurrent()
        #expect(s.bell.asks == 1)
        #expect(s.popoverOpenings == 1, "nothing waits on the answer")
        #expect(s.bell.posted.isEmpty)
        await s.notices.completeCurrent()
        #expect(s.bell.asks == 1, "the system asks the user once")
        #expect(s.popoverOpenings == 2)
    }

    @Test func withNotificationsOffThePopoverOffersIt() async throws {
        let s = try Setup(.denied)
        await s.notices.completeCurrent()
        #expect(s.bell.asks == 0 && s.bell.posted.isEmpty)
        #expect(s.popoverOpenings == 1)
        #expect(s.model.undoOffer != nil)
    }

    @Test func anEmptyNowSaysSo() async throws {
        let s = try Setup(.allowed, current: nil)
        await s.notices.completeCurrent()
        #expect(s.bell.posted.map(\.title) == ["Nothing in Now"])
        #expect(s.model.snapshot.next.map(\.title) == ["write notes"], "nothing was completed")
        let off = try Setup(.denied, current: nil)
        await off.notices.completeCurrent()
        #expect(off.popoverOpenings == 1, "the popover shows Now is empty")
    }

    @Test func aCompletionThatCannotBeSavedSaysWhy() async throws {
        let s = try Setup(.allowed)
        try s.breakSaving()
        await s.notices.completeCurrent()
        let note = try #require(s.bell.posted.first)
        #expect(note.title == "Could not complete the task")
        #expect(note.body?.contains("could not save") == true, "\(note.body ?? "")")
        #expect(s.model.snapshot.current?.title == "ship v0.1")
        #expect(s.model.undoOffer == nil)
    }

    @Test func undoFromTheNotePutsTheTaskBack() async throws {
        let s = try Setup(.allowed)
        let task = try #require(s.model.snapshot.current)
        await s.notices.completeCurrent()
        await s.notices.undo(id: task.id)
        #expect(s.model.snapshot.current?.title == "ship v0.1")
        #expect(s.model.undoOffer == nil, "the popover's row goes too")
        #expect(s.bell.withdrawn == ["done"])
        #expect(s.bell.posted.count == 1, "nothing more to say")
    }

    @Test func aStaleUndoSaysTheQueueChanged() async throws {
        let s = try Setup(.allowed)
        let task = try #require(s.model.snapshot.current)
        await s.notices.completeCurrent()
        s.model.add("something else")
        await s.notices.undo(id: task.id)
        let note = try #require(s.bell.posted.last)
        #expect(note.title == "Nothing to undo" && note.body == "The queue changed since.")
        #expect(s.model.snapshot.current?.title == "write notes")
    }

    @Test func theDoneNoteGoesOnceItsUndoCannotWork() async throws {
        let s = try Setup(.allowed)
        await s.notices.completeCurrent()
        s.notices.modelDidChange()
        #expect(s.bell.withdrawn.isEmpty, "nothing changed yet")
        s.model.add("something else")
        s.notices.modelDidChange()
        #expect(s.bell.withdrawn == ["done"])
        s.notices.modelDidChange()
        #expect(s.bell.withdrawn == ["done"], "once")
    }

    @Test func eachProblemIsOneNote() async throws {
        let s = try Setup(.allowed)
        await s.notices.report(["could not save a", "could not save b"])
        #expect(s.bell.posted.map(\.body) == ["could not save a", "could not save b"])
        let off = try Setup(.denied)
        await off.notices.report(["could not save a"])
        #expect(off.bell.posted.isEmpty && off.popoverOpenings == 0, "the message line has it")
        let first = try Setup(.notAsked)
        await first.notices.report(["a", "b", "c"])
        #expect(first.bell.asks == 1)
    }

    @Test func aChangeWhileAskingLeavesNoNoteBehind() async throws {
        let s = try Setup(.allowed)
        s.bell.holds = true
        let completing = await s.start { await s.notices.completeCurrent() }
        s.model.add("something else")
        s.bell.release(0)
        await completing.value
        #expect(s.bell.posted.isEmpty, "its undo could no longer work")
        #expect(s.popoverOpenings == 0)
    }

    @Test func completionsAnsweredOutOfOrderLeaveTheLatestNote() async throws {
        let s = try Setup(.allowed, next: ["one", "two"])
        s.bell.holds = true
        let first = await s.start { await s.notices.completeCurrent() }
        let second = await s.start { await s.notices.completeCurrent() }
        s.bell.release(1)
        await second.value
        s.bell.release(0)
        await first.value
        #expect(s.bell.posted.map(\.body) == ["one"], "the first completion's undo had gone")
    }

    @Test func aNoteNotTakenFallsBackToThePopover() async throws {
        let s = try Setup(.allowed)
        s.bell.accepts = false
        await s.notices.completeCurrent()
        #expect(s.popoverOpenings == 1)
    }

    @Test func aFailedUndoOffersToTryAgain() async throws {
        let s = try Setup(.allowed)
        let task = try #require(s.model.snapshot.current)
        await s.notices.completeCurrent()
        try s.breakSaving()
        await s.notices.undo(id: task.id)
        let retry = try #require(s.bell.posted.last)
        #expect(retry.id == Note.doneID && retry.title == "Could not undo" && retry.undo == task.id)
        #expect(retry.body?.contains("could not save") == true)
        #expect(s.bell.withdrawn.isEmpty, "the offer stays")
        try s.repairSaving()
        await s.notices.undo(id: task.id)
        #expect(s.model.snapshot.current?.title == "ship v0.1")
        #expect(s.bell.withdrawn == ["done"])
    }

    @Test func aFailureFromOutsideWithoutNotesShowsInThePopover() async throws {
        let s = try Setup(.denied)
        await s.notices.failed("Could not add the task", "empty title")
        #expect(s.popoverOpenings == 1)
        #expect(s.model.actionError == "empty title", "on its message line")
    }
}

