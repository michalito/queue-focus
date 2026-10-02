import Foundation
import Observation
import os

/// A completion that can still be taken back: offered for eight seconds, as
/// in GNOME, and only while nothing else in the queue has changed, which is
/// also when the engine would refuse it.
struct UndoOffer: Equatable {
    let id: UInt64
    let title: String
    /// The queue's revision right after the completion.
    let revision: UInt64
    let expires: Date
}

/// The app's one engine, and everything the views show of it.
///
/// Every request goes to the engine, then the model takes a fresh snapshot;
/// the views and the status item read only from here. All of it runs on the
/// main actor: the work is tiny, and it keeps the engine's calls in order.
@MainActor
@Observable
final class QueueModel {
    /// How long a completion can be taken back, as in GNOME.
    static let undoWindow: TimeInterval = 8
    /// The most problems kept for the popover; the log has every one.
    static let keptProblems = 20

    private(set) var snapshot: QueueSnapshot
    private(set) var settings: QueueSettings
    /// The time the views show. It moves with every tick.
    private(set) var now: Date
    private(set) var undoOffer: UndoOffer?
    /// Why the last request from a view failed, for the popover's message line.
    var actionError: String?
    /// Changes saved without being made crash-safe, and settings that could
    /// not be written, oldest first.
    private(set) var problems: [String] = []

    /// Told after every change and every tick, for what SwiftUI does not
    /// draw: the status item.
    @ObservationIgnored var didChange: @MainActor () -> Void = {}
    /// Told when the reminder says to flash.
    @ObservationIgnored var presentFlash: @MainActor (FlashEvent) -> Void = { _ in }

    @ObservationIgnored private let engine: QueueEngine
    @ObservationIgnored private let clock: () -> Date
    @ObservationIgnored private let calendar: Calendar
    @ObservationIgnored private let log = Logger(subsystem: "org.queuefocus.QueueFocus", category: "engine")

    init(engine: QueueEngine, clock: @escaping () -> Date = Date.init, calendar: Calendar = .current) {
        self.engine = engine
        self.clock = clock
        self.calendar = calendar
        snapshot = engine.snapshot()
        settings = engine.settings()
        now = clock()
    }

    // MARK: Reading

    /// Unix seconds, as the engine counts them.
    var unixNow: UInt64 {
        UInt64(max(0, now.timeIntervalSince1970))
    }

    /// Seconds on a task's clock now; `nil` for a task that is not current.
    func elapsed(of task: QueueTask) -> UInt64? {
        elapsedSecs(task: task, now: unixNow)
    }

    /// The offer, while it can still be taken.
    var liveUndoOffer: UndoOffer? {
        guard let offer = undoOffer, now < offer.expires, snapshot.revision == offer.revision else {
            return nil
        }
        return offer
    }

    func flashStatus() -> FlashStatus? {
        try? engine.flashStatus(now: unixNow, localTime: localTime(now))
    }

    // MARK: The queue

    /// Add a task from the add field: to the bucket Settings chose, or as the
    /// current task. Returns whether one was added; blank text adds nothing.
    @discardableResult
    func add(_ text: String, asCurrent: Bool = false) -> Bool {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return false }
        return perform { try engine.add(text: text, bucket: asCurrent ? .now : nil) } != nil
    }

    /// Complete the current task, pulling the head of Next into Now. Returns
    /// the task that was completed, for the caller to offer to undo.
    func completeCurrent() -> QueueTask? {
        perform { try engine.completeCurrent() } ?? nil
    }

    /// Mark a listed task done. Returns whether it was.
    @discardableResult
    func complete(_ task: QueueTask) -> Bool {
        perform { try engine.complete(id: task.id) } != nil
    }

    /// Offer to undo the completion of `task`, which has just been made. Only
    /// the latest completion can be undone, so this replaces any other offer.
    func offerUndo(for task: QueueTask) {
        now = clock()
        undoOffer = UndoOffer(
            id: task.id,
            title: task.title,
            revision: snapshot.revision,
            expires: now.addingTimeInterval(Self.undoWindow)
        )
    }

    /// Take back the completion on offer.
    /// Take back the completion on offer. The offer stays when the undo
    /// could not be saved, so it can be tried again; the engine keeps its
    /// record for the same reason.
    func undo() {
        guard let offer = undoOffer else { return }
        guard let undone = perform({ try engine.undoComplete(id: offer.id) }) else { return }
        undoOffer = nil
        if !undone {
            actionError = "Nothing to undo: the queue changed since."
        }
    }

    func togglePause() {
        perform { try engine.togglePause() }
    }

    func promote(_ task: QueueTask) {
        perform { try engine.promote(id: task.id) }
    }

    func promote(id: UInt64) {
        perform { try engine.promote(id: id) }
    }

    /// Move a task to the end of `bucket`; into Now, it becomes current.
    func move(id: UInt64, to bucket: Bucket) {
        perform { try engine.moveTask(id: id, bucket: bucket, index: nil) }
    }

    /// Drop a task in front of the row `before` in `bucket`, or at its end.
    /// `false` when that row has gone since it was drawn.
    @discardableResult
    func move(id: UInt64, to bucket: Bucket, before: UInt64?) -> Bool {
        perform { try engine.moveBefore(id: id, bucket: bucket, before: before) } ?? false
    }

    /// Move a task up (negative) or down within its bucket.
    func shift(id: UInt64, by delta: Int32) {
        perform { try engine.shift(id: id, delta: delta) }
    }

    func cycleTag(id: UInt64) {
        perform { try engine.cycleTag(id: id) }
    }

    func setTag(id: UInt64, _ tag: TaskTag?) {
        perform { try engine.setTag(id: id, tag: tag) }
    }

    /// Rename a task. A blank title is not a rename: nothing is asked of the
    /// engine and the old title stays.
    @discardableResult
    func rename(id: UInt64, to title: String) -> Bool {
        guard !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        return perform { try engine.rename(id: id, title: title) } != nil
    }

    /// Delete a task for good; unlike a completion, it cannot be undone.
    func remove(id: UInt64) {
        perform { try engine.remove(id: id) }
    }

    /// Mark a task done from a window: the current task as by Done, any other
    /// deleted. Either way the popover offers to undo it, which the GNOME
    /// windows do not.
    func complete(id: UInt64) {
        guard let task = task(id) else { return }
        if complete(task) {
            offerUndo(for: task)
        }
    }

    /// The task with this id, wherever it is.
    func task(_ id: UInt64) -> QueueTask? {
        if snapshot.current?.id == id { return snapshot.current }
        return (snapshot.side + snapshot.next + snapshot.later).first { $0.id == id }
    }

    // MARK: Settings

    func setSettings(_ settings: QueueSettings) {
        perform { try engine.setSettings(settings: settings) }
    }

    // MARK: The reminder

    /// Flash now, whatever the quiet rules say, so long as Now holds a task.
    /// The wait for the next one starts over. Returns whether there was one.
    @discardableResult
    func flashNow() -> Bool {
        now = clock()
        guard let flash = engine.flashNow(now: unixNow, random: .random(in: 0..<30)) else { return false }
        presentFlash(flash)
        refresh()
        return true
    }

    // MARK: The clock

    /// One second of the app: the engine writes changed settings and says
    /// whether to flash, and the views' clocks move on.
    func tick() {
        now = clock()
        do {
            let tick = try engine.tick(now: unixNow, localTime: localTime(now), random: .random(in: 0..<30))
            report(tick.problems)
            if let flash = tick.flash {
                presentFlash(flash)
            }
        } catch {
            log.error("tick failed: \(Self.describe(error), privacy: .public)")
        }
        refresh()
    }

    /// Write anything not yet written. Called as the app quits; returns the
    /// problems there is no longer time to show.
    func flush() -> [String] {
        engine.flush()
    }

    /// Show a problem the engine reported, and log it.
    func report(_ problems: [String]) {
        guard !problems.isEmpty else { return }
        for problem in problems {
            log.error("\(problem, privacy: .public)")
        }
        self.problems = Array((self.problems + problems).suffix(Self.keptProblems))
    }

    func dismissProblems() {
        problems = []
    }

    // MARK: Plumbing

    /// Run a request; on failure say why on the message line. Either way the
    /// snapshot is fresh afterwards, since a failure can follow a change.
    @discardableResult
    private func perform<T>(_ request: () throws -> T) -> T? {
        defer { refresh() }
        do {
            let value = try request()
            actionError = nil
            return value
        } catch {
            actionError = Self.describe(error)
            return nil
        }
    }

    private func refresh() {
        let snapshot = engine.snapshot()
        if snapshot != self.snapshot { self.snapshot = snapshot }
        let settings = engine.settings()
        if settings != self.settings { self.settings = settings }
        didChange()
    }

    private func localTime(_ date: Date) -> TimeOfDay {
        let parts = calendar.dateComponents([.hour, .minute], from: date)
        return TimeOfDay(hour: UInt8(parts.hour ?? 12), minute: UInt8(parts.minute ?? 0))
    }

    static func describe(_ error: Error) -> String {
        switch error {
        case QfError.Persistence(let message), QfError.InvalidArgument(let message):
            message
        default:
            String(describing: error)
        }
    }
}
