import Foundation
import Testing
@testable import QueueFocus

/// Notification Center, stood in for: what was posted, asked and withdrawn.
@MainActor
private final class Bell: Notifier {
    var answer: NotePermission
    var posted: [Note] = []
    var asks = 0
    var withdrawn: [String] = []

    init(_ answer: NotePermission) {
        self.answer = answer
    }

    func permission() async -> NotePermission { answer }
    func ask() { asks += 1 }
    func post(_ note: Note) { posted.append(note) }
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
}
