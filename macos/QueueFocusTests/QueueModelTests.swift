import Foundation
import Testing
@testable import QueueFocus

/// A clock the test moves by hand. It starts at the real time, because the
/// engine times the first wait for a flash from the real clock.
private final class ManualClock: @unchecked Sendable {
    var date = Date()

    func advance(_ seconds: TimeInterval) {
        date = date.addingTimeInterval(seconds)
    }
}

/// A model over an engine in a directory of its own, removed afterwards.
@MainActor
private final class Fixture {
    let dir: URL
    let clock = ManualClock()
    let model: QueueModel
    var changes = 0
    var flashes: [FlashEvent] = []

    init() throws {
        dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("qf-model-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let engine = try QueueEngine(dir: dir.path)
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let clock = clock
        model = QueueModel(engine: engine, clock: { clock.date }, calendar: calendar)
        model.didChange = { [unowned self] in changes += 1 }
        model.presentFlash = { [unowned self] in flashes.append($0) }
    }

    deinit {
        try? FileManager.default.removeItem(at: dir)
    }
}

@MainActor
@Suite struct QueueModelTests {
    @Test func addingGoesWhereSettingsSayOrToNow() throws {
        let f = try Fixture()
        #expect(f.model.add("write notes"))
        #expect(f.model.snapshot.next.map(\.title) == ["write notes"])
        #expect(f.model.add("  ship it #w  ", asCurrent: true))
        #expect(f.model.snapshot.current?.title == "ship it")
        #expect(f.model.snapshot.current?.tag == .work)
        #expect(f.changes >= 2)
    }

    @Test func completingTheCurrentTaskSaysWhatCameOfIt() throws {
        let f = try Fixture()
        #expect(f.model.completeCurrent() == .empty, "Now is empty")
        f.model.add("ship", asCurrent: true)
        let task = try #require(f.model.snapshot.current)
        #expect(f.model.completeCurrent() == .done(task))
        f.model.add("write", asCurrent: true)
        // A folder where the task file goes: the save cannot happen.
        let file = f.dir.appendingPathComponent("tasks.json")
        try FileManager.default.removeItem(at: file)
        try FileManager.default.createDirectory(at: file.appendingPathComponent("in-the-way"), withIntermediateDirectories: true)
        guard case .failed(let message) = f.model.completeCurrent() else {
            Issue.record("a failed save is not an empty Now")
            return
        }
        #expect(message.contains("could not save"))
        #expect(f.model.snapshot.current?.title == "write")
    }

    @Test func anUndoByIdSaysWhatCameOfIt() throws {
        let f = try Fixture()
        f.model.add("ship", asCurrent: true)
        let task = try #require(f.model.completeCurrent().task)
        f.model.offerUndo(for: task)
        f.model.add("other")
        #expect(f.model.undo(id: task.id) == .stale)
        #expect(f.model.undoOffer == nil, "an offer that cannot work goes")
        f.model.add("again", asCurrent: true)
        let again = try #require(f.model.completeCurrent().task)
        f.model.offerUndo(for: again)
        #expect(f.model.undo(id: again.id) == .undone)
        #expect(f.model.undoOffer == nil)
        #expect(f.model.snapshot.current?.title == "again")
    }

    @Test func problemsReachTheMessageLineAndWhoeverListens() throws {
        let f = try Fixture()
        var heard: [[String]] = []
        f.model.didReport = { heard.append($0) }
        f.model.report(["could not save"])
        f.model.report([])
        #expect(f.model.problems == ["could not save"])
        #expect(heard == [["could not save"]], "once, and not for nothing")
    }

    /// The engine stamps the timer with the real clock, so this follows how
    /// the time moves, not what it reads.
    @Test func pausingFreezesTheClockAndSurvivesAReopen() throws {
        let f = try Fixture()
        f.model.add("ship", asCurrent: true)
        // The engine stamped the task with the real time: start the test's
        // clock there, and let the model read it, so the first reading is
        // not cut short at zero.
        f.clock.date = Date()
        f.model.tick()
        let task = { try #require(f.model.snapshot.current) }
        let running = try #require(f.model.elapsed(of: try task()))
        f.clock.advance(120)
        f.model.tick()
        #expect(try f.model.elapsed(of: try task()) == running + 120, "it runs with the clock")
        f.model.togglePause()
        let paused = try #require(f.model.elapsed(of: try task()))
        f.clock.advance(600)
        f.model.tick()
        #expect(try f.model.elapsed(of: try task()) == paused, "and stops while paused")
        let reopened = QueueModel(engine: try QueueEngine(dir: f.dir.path), clock: { f.clock.date })
        let stored = try #require(reopened.snapshot.current)
        #expect(stored.pausedAt == (try task()).pausedAt && stored.pausedAt != nil, "the pause is in the file")
        #expect(reopened.elapsed(of: stored) == paused)
        f.model.togglePause()
        let resumed = try #require(f.model.elapsed(of: try task()))
        f.clock.advance(60)
        f.model.tick()
        #expect(try f.model.elapsed(of: try task()) == resumed + 60, "and runs again once resumed")
    }

    @Test func everySettingSurvivesAReopen() throws {
        let f = try Fixture()
        var settings = f.model.settings
        settings.intervalMin = 25
        settings.vary.toggle()
        settings.intensity = .strong
        settings.color = .orange
        settings.quietPaused.toggle()
        settings.quietHours.toggle()
        settings.quietFrom = TimeOfDay(hour: 8, minute: 30)
        settings.quietTo = TimeOfDay(hour: 17, minute: 45)
        settings.theme = .dark
        settings.showTimer.toggle()
        settings.defaultBucket = .later
        f.model.setSettings(settings)
        #expect(f.model.settings == settings)
        // The engine writes settings on its next tick.
        f.model.tick()
        #expect(try QueueEngine(dir: f.dir.path).settings() == settings)
    }

    @Test func blankTextAddsNothingAndSaysNothing() throws {
        let f = try Fixture()
        #expect(!f.model.add("   "))
        #expect(f.model.actionError == nil)
        #expect(f.model.snapshot.revision == 0)
    }

    @Test func aRefusedAddSaysWhy() throws {
        let f = try Fixture()
        #expect(!f.model.add("#w @later"))
        #expect(f.model.actionError == "empty title")
        // The next request that works clears the message.
        #expect(f.model.add("real"))
        #expect(f.model.actionError == nil)
    }

    @Test func aCompletionCanBeUndoneWhileNothingElseChanges() throws {
        let f = try Fixture()
        f.model.add("first", asCurrent: true)
        f.model.add("second")
        let done = try #require(f.model.completeCurrent().task)
        f.model.offerUndo(for: done)
        #expect(f.model.snapshot.current?.title == "second")
        #expect(f.model.liveUndoOffer?.title == "first")

        f.model.undo()
        #expect(f.model.snapshot.current?.title == "first")
        #expect(f.model.snapshot.next.map(\.title) == ["second"])
        #expect(f.model.liveUndoOffer == nil)
        #expect(f.model.actionError == nil)
    }

    @Test func completingASideTaskOffersToUndoItToo() throws {
        let f = try Fixture()
        f.model.add("beside @side")
        let side = try #require(f.model.snapshot.side.first)
        #expect(f.model.complete(side))
        f.model.offerUndo(for: side)
        #expect(f.model.snapshot.side.isEmpty)
        f.model.undo()
        #expect(f.model.snapshot.side.map(\.title) == ["beside"])
    }

    @Test func anyOtherChangeWithdrawsTheOffer() throws {
        let f = try Fixture()
        f.model.add("first", asCurrent: true)
        f.model.add("second")
        let done = try #require(f.model.completeCurrent().task)
        f.model.offerUndo(for: done)
        f.model.togglePause()
        #expect(f.model.liveUndoOffer == nil)

        // Asked anyway, the engine refuses and the model says why.
        f.model.undo()
        #expect(f.model.actionError == "Nothing to undo: the queue changed since.")
        #expect(f.model.snapshot.current?.title == "second")
    }

    /// An undo that could not be saved can be tried again once the trouble
    /// is gone: the offer stays, as the engine's record does.
    @Test func anUndoThatCannotBeSavedCanBeTriedAgain() throws {
        let f = try Fixture()
        f.model.add("first", asCurrent: true)
        f.model.offerUndo(for: try #require(f.model.completeCurrent().task))
        let tasks = f.dir.appendingPathComponent("tasks.json")
        let saved = f.dir.appendingPathComponent("tasks.json.saved")
        // A directory where the file belongs makes every save fail.
        try FileManager.default.moveItem(at: tasks, to: saved)
        try FileManager.default.createDirectory(at: tasks, withIntermediateDirectories: false)

        f.model.undo()
        #expect(f.model.actionError?.contains("could not save") == true)
        #expect(f.model.snapshot.current == nil, "nothing was restored")
        #expect(f.model.liveUndoOffer != nil, "the offer stays for another try")

        try FileManager.default.removeItem(at: tasks)
        try FileManager.default.moveItem(at: saved, to: tasks)
        f.model.undo()
        #expect(f.model.actionError == nil)
        #expect(f.model.snapshot.current?.title == "first")
        #expect(f.model.liveUndoOffer == nil)
    }

    @Test func theOfferLastsEightSeconds() throws {
        let f = try Fixture()
        f.model.add("first", asCurrent: true)
        f.model.offerUndo(for: try #require(f.model.completeCurrent().task))
        f.clock.advance(7.9)
        f.model.tick()
        #expect(f.model.liveUndoOffer != nil)
        f.clock.advance(0.1)
        f.model.tick()
        #expect(f.model.liveUndoOffer == nil)
    }

    @Test func theTickDeliversADueFlash() throws {
        let f = try Fixture()
        f.model.add("focus #p", asCurrent: true)
        var settings = f.model.settings
        settings.intervalMin = 1
        settings.vary = false
        f.model.setSettings(settings)
        f.model.tick()
        #expect(f.flashes.isEmpty)
        f.clock.advance(60)
        f.model.tick()
        let flash = try #require(f.flashes.first)
        #expect(flash.title == "focus")
        #expect(flash.style == .edges)
        #expect(flash.palette == .orange)
    }

    @Test func aSettingsWriteFailureIsReportedOnce() throws {
        let f = try Fixture()
        // A directory where the file belongs makes every write fail.
        try FileManager.default.createDirectory(at: f.dir.appendingPathComponent("settings.json"), withIntermediateDirectories: true)
        var settings = f.model.settings
        settings.showTimer = false
        f.model.setSettings(settings)
        #expect(!f.model.settings.showTimer, "the value in memory is the user's at once")
        for _ in 0..<5 {
            f.clock.advance(1)
            f.model.tick()
        }
        #expect(f.model.problems.count == 1)
        #expect(f.model.problems.first?.contains("settings.json") == true)
        f.model.dismissProblems()
        #expect(f.model.problems.isEmpty)
    }

    @Test func everyTickMovesTheClockAndRedraws() throws {
        let f = try Fixture()
        f.model.add("t", asCurrent: true)
        let before = f.changes
        let start = f.model.unixNow
        f.clock.advance(125)
        f.model.tick()
        #expect(f.model.unixNow == start + 125)
        #expect(f.changes == before + 1)
        let current = try #require(f.model.snapshot.current)
        #expect(f.model.elapsed(of: current) != nil)
    }

    @Test func theWindowsRequestsReachTheEngine() throws {
        let f = try Fixture()
        f.model.add("a")
        f.model.add("b")
        f.model.add("c @later")
        let ids = f.model.snapshot.next.map(\.id)
        let later = try #require(f.model.snapshot.later.first)

        #expect(f.model.move(id: later.id, to: .next, before: ids[0]))
        #expect(f.model.snapshot.next.map(\.title) == ["c", "a", "b"])
        #expect(!f.model.move(id: ids[0], to: .side, before: ids[1]), "b is not in Side")
        #expect(f.model.actionError == nil, "a stale row is no error")

        f.model.shift(id: later.id, by: 1)
        #expect(f.model.snapshot.next.map(\.title) == ["a", "c", "b"])
        f.model.cycleTag(id: later.id)
        #expect(f.model.task(later.id)?.tag == .work)
        f.model.setTag(id: later.id, .personal)
        #expect(f.model.task(later.id)?.tag == .personal)
        f.model.move(id: later.id, to: .side)
        #expect(f.model.snapshot.side.map(\.title) == ["c"])
        f.model.promote(id: later.id)
        #expect(f.model.snapshot.current?.title == "c")
        f.model.remove(id: ids[1])
        #expect(f.model.task(ids[1]) == nil)
    }

    @Test func aBlankRenameAsksNothingOfTheEngine() throws {
        let f = try Fixture()
        f.model.add("kept")
        let id = try #require(f.model.snapshot.next.first?.id)
        let revision = f.model.snapshot.revision
        #expect(!f.model.rename(id: id, to: "   "))
        #expect(f.model.actionError == nil)
        #expect(f.model.snapshot.revision == revision)
        #expect(f.model.rename(id: id, to: " renamed "))
        #expect(f.model.task(id)?.title == "renamed")
    }

    /// A completion from a window can be undone from the popover, which the
    /// GNOME windows do not offer.
    @Test func aWindowCompletionOffersUndo() throws {
        let f = try Fixture()
        f.model.add("first", asCurrent: true)
        let id = try #require(f.model.snapshot.current?.id)
        f.model.complete(id: id)
        #expect(f.model.snapshot.current == nil)
        #expect(f.model.liveUndoOffer?.id == id)
        f.model.complete(id: 999)
        #expect(f.model.liveUndoOffer?.id == id, "nothing to complete changes nothing")
    }

    @Test func flashNowGoesThroughTheSamePathAsAScheduledFlash() throws {
        let f = try Fixture()
        #expect(!f.model.flashNow(), "nothing in Now")
        #expect(f.flashes.isEmpty)
        f.model.add("focus", asCurrent: true)
        #expect(f.model.flashNow())
        #expect(f.flashes.map(\.title) == ["focus"])
        guard case .scheduled(let remaining)? = f.model.flashStatus() else {
            Issue.record("expected a scheduled flash")
            return
        }
        #expect(remaining == 15 * 60, "the wait started over")
    }

    @Test func theFlashStatusFollowsTheQueue() throws {
        let f = try Fixture()
        #expect(f.model.flashStatus() == .held(reason: .noCurrentTask))
        f.model.add("t", asCurrent: true)
        guard case .scheduled = f.model.flashStatus() else {
            Issue.record("expected a scheduled flash, got \(String(describing: f.model.flashStatus()))")
            return
        }
    }
}
