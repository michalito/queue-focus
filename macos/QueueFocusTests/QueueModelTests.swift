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
        let done = try #require(f.model.completeCurrent())
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
        let done = try #require(f.model.completeCurrent())
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
        f.model.offerUndo(for: try #require(f.model.completeCurrent()))
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
        f.model.offerUndo(for: try #require(f.model.completeCurrent()))
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
