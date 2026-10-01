// The engine as the app will use it: through the generated Swift, against
// the static library, in a directory of its own.
import Foundation
import QfCore
import Testing

/// A fresh data directory, removed when the test is done with it.
private final class DataDirectory {
    let url: URL

    init() throws {
        url = FileManager.default.temporaryDirectory
            .appendingPathComponent("qf-swift-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    }

    deinit {
        try? FileManager.default.removeItem(at: url)
    }

    func engine() throws -> QueueEngine {
        try QueueEngine(dir: url.path)
    }

    func write(_ name: String, _ contents: String) throws {
        try contents.write(to: url.appendingPathComponent(name), atomically: true, encoding: .utf8)
    }

    func read(_ name: String) throws -> String {
        try String(contentsOf: url.appendingPathComponent(name), encoding: .utf8)
    }
}

private let noon = TimeOfDay(hour: 12, minute: 0)

private func now() -> UInt64 {
    UInt64(Date().timeIntervalSince1970)
}

@Test func addingCompletingAndUndoingRoundTrip() throws {
    let dir = try DataDirectory()
    let engine = try dir.engine()
    #expect(engine.openWarning() == nil)

    let current = try engine.add(text: "ship it #w", bucket: .now)
    let next = try engine.add(text: "write notes", bucket: nil)
    var snapshot = engine.snapshot()
    #expect(snapshot.current?.id == current)
    #expect(snapshot.current?.tag == .work)
    #expect(snapshot.next.map(\.id) == [next])
    #expect(snapshot.revision == 2)

    let done = try #require(try engine.completeCurrent())
    #expect(done.title == "ship it")
    #expect(engine.snapshot().current?.id == next)
    #expect(try engine.undoComplete(id: current))
    snapshot = engine.snapshot()
    #expect(snapshot.current?.id == current)
    #expect(snapshot.next.map(\.id) == [next])

    let stored = try dir.read("tasks.json")
    #expect(stored.contains("\"ship it\""))
}

@Test func refusalsArriveAsTypedErrors() throws {
    let dir = try DataDirectory()
    let engine = try dir.engine()
    #expect(throws: QfError.InvalidArgument(message: "empty title")) {
        try engine.add(text: "  #w @later ", bucket: nil)
    }
    #expect(throws: QfError.InvalidArgument(message: "no such task")) {
        try engine.promote(id: 99)
    }
}

@Test func aMalformedTaskFileIsRefusedAndLeftAlone() throws {
    let dir = try DataDirectory()
    try dir.write("tasks.json", "{ broken")
    #expect {
        try dir.engine()
    } throws: { error in
        guard case QfError.Persistence(let message) = error else { return false }
        return message.contains("tasks.json")
    }
    #expect(try dir.read("tasks.json") == "{ broken")
}

@Test func settingsRoundTripThroughTheRecordAndTheFile() throws {
    let dir = try DataDirectory()
    let engine = try dir.engine()
    var settings = engine.settings()
    #expect(settings.intervalMin == 15)
    settings.intervalMin = 25
    settings.quietHours = true
    settings.quietFrom = TimeOfDay(hour: 22, minute: 30)
    settings.defaultBucket = .side
    #expect(try engine.setSettings(settings: settings))
    #expect(try !engine.setSettings(settings: settings))
    #expect(engine.flush().isEmpty)
    #expect(try dir.read("settings.json").contains("\"quiet_from\": \"22:30\""))
    #expect(try dir.engine().settings() == settings)

    settings.quietTo = TimeOfDay(hour: 24, minute: 0)
    #expect(throws: QfError.self) { try engine.setSettings(settings: settings) }
}

@Test func theClockDrivesTheReminder() throws {
    let dir = try DataDirectory()
    let engine = try dir.engine()
    #expect(try engine.flashStatus(now: now(), localTime: noon) == .held(reason: .noCurrentTask))
    #expect(holdReasonText(reason: .noCurrentTask) == "nothing in Now")

    _ = try engine.add(text: "!focus #p", bucket: nil)
    var settings = engine.settings()
    settings.intervalMin = 1
    settings.vary = false
    _ = try engine.setSettings(settings: settings)
    let start = now()
    #expect(try engine.tick(now: start, localTime: noon, random: 0).flash == nil)
    let tick = try engine.tick(now: start + 60, localTime: noon, random: UInt32.random(in: 0 ..< 30))
    let flash = try #require(tick.flash)
    #expect(flash.title == "focus")
    #expect(flash.style == .edges)
    #expect(flash.palette == .orange)
    #expect(tick.problems.isEmpty)
    #expect(try engine.flashStatus(now: start + 60, localTime: noon) == .scheduled(remainingSecs: 60))

    #expect(throws: QfError.self) {
        try engine.tick(now: start, localTime: TimeOfDay(hour: 9, minute: 60), random: 0)
    }
}

@Test func theFormattersAreTheCores() throws {
    #expect(shortElapsed(secs: 62 * 60, paused: false) == "1h02")
    #expect(shortElapsed(secs: 12 * 60, paused: true) == "12m ⏸")
    #expect(longElapsed(secs: 3725) == "1:02:05")
    let task = QueueTask(
        id: 1, title: "t", bucket: .now, tag: nil, createdAt: 0, startedAt: 100, pausedAt: 130
    )
    #expect(elapsedSecs(task: task, now: 1000) == 30)
    #expect(intervalBounds() == IntervalBounds(min: 1, max: 90))
    #expect(maxTitleChars() == 256)
    #expect(defaultDataDir().hasSuffix("queue-focus"))
}
