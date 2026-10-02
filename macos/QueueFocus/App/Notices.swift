import Foundation

/// Whether notifications can be shown.
enum NotePermission: Equatable {
    case allowed
    case notAsked
    case denied
}

/// A notification, as the app posts one.
struct Note: Equatable {
    /// The Done note's: a newer completion replaces the older note.
    static let doneID = "done"

    /// Notes with the same id replace one another.
    let id: String
    let title: String
    let body: String?
    /// The task a Done note offers to undo.
    var undo: UInt64?

    static func done(_ task: QueueTask) -> Note {
        Note(id: doneID, title: "Done", body: task.title, undo: task.id)
    }

    static func message(_ title: String, _ body: String? = nil) -> Note {
        Note(id: UUID().uuidString, title: title, body: body)
    }
}

/// Where notifications go.
@MainActor
protocol Notifier: AnyObject {
    func permission() async -> NotePermission
    /// Ask to show notifications. The answer is for next time: nothing waits
    /// on it.
    func ask()
    /// Post `note`; whether Notification Center took it.
    func post(_ note: Note) async -> Bool
    func withdraw(_ id: String)
}

/// What the app tells you while none of its windows is in front: a
/// completion made from the global shortcut, with Undo; that Now was empty;
/// a problem the engine reported. As notifications, as GNOME does, or, while
/// those are off, in the popover.
///
/// Each request is handled in turn, the next once the last is done, so notes
/// and the popover follow the order things happened in, whatever order the
/// system answers in. The queue can still change meanwhile from a window,
/// so whatever waited checks it again before it says anything.
@MainActor
final class Notices {
    private let model: QueueModel
    private let notifier: Notifier
    private let popoverIsShown: @MainActor () -> Bool
    private let showPopover: @MainActor () -> Void
    /// The completion the Done note on screen offers to undo, and the
    /// queue's revision right after it.
    private var done: (id: UInt64, title: String, revision: UInt64)?
    /// Asked for permission already this run: the system asks the user once.
    private var asked = false
    /// The request being handled, which the next one waits for.
    private var last: Task<Void, Never>?

    init(model: QueueModel, notifier: Notifier, popoverIsShown: @escaping @MainActor () -> Bool,
         showPopover: @escaping @MainActor () -> Void) {
        self.model = model
        self.notifier = notifier
        self.popoverIsShown = popoverIsShown
        self.showPopover = showPopover
    }

    /// Complete the current task from anywhere, and offer to undo it.
    @discardableResult
    func completeCurrent() -> Task<Void, Never> {
        inTurn { [self] in
            switch model.completeCurrent() {
            case .done(let task):
                model.offerUndo(for: task)
                let revision = model.snapshot.revision
                // An open popover offers it in its own row, as GNOME's open
                // menu does instead of a notification.
                guard !popoverIsShown() else { return }
                let allowed = await mayPost()
                // Meanwhile the queue may have moved on, and the undo with
                // it, or the popover may have opened and be offering it.
                guard model.snapshot.revision == revision, !popoverIsShown() else { return }
                if allowed, await notifier.post(.done(task)) {
                    done = (task.id, task.title, revision)
                    // The queue may have moved on while the note was posted.
                    modelDidChange()
                } else if model.snapshot.revision == revision {
                    showPopover()
                }
            case .empty:
                if !(await tell(.message("Nothing in Now"))) {
                    showPopover()
                }
            case .failed(let message):
                if !(await tell(.message("Could not complete the task", message))) {
                    model.actionError = message
                    showPopover()
                }
            }
        }
    }

    /// Undo, from the Done note's button. The engine decides whether it
    /// still can, whatever the popover's own offer says.
    @discardableResult
    func undo(id: UInt64) -> Task<Void, Never> {
        inTurn { [self] in
            // The note this answers, if it is still the one on screen.
            let record = done?.id == id ? done : nil
            switch model.undo(id: id) {
            case .undone:
                if record != nil { withdrawDone() }
            case .stale:
                if record != nil { withdrawDone() }
                await tell(.message("Nothing to undo", "The queue changed since."))
            case .failed(let message):
                // Nothing changed and the engine still has the completion,
                // so it is offered again, as GNOME's menu keeps its offer;
                // unless the queue moves on before it can be.
                let revision = model.snapshot.revision
                let allowed = await mayPost()
                guard model.snapshot.revision == revision else { return }
                let retry = Note(id: Note.doneID, title: "Could not undo", body: message, undo: id)
                if allowed, await notifier.post(retry) {
                    done = (id, record?.title ?? "", revision)
                    modelDidChange()
                } else if model.snapshot.revision == revision {
                    if let record {
                        model.offerUndo(id: id, title: record.title)
                    }
                    model.actionError = message
                    showPopover()
                }
            }
        }
    }

    /// One note per problem the engine reported, as GNOME posts one per
    /// warning. The message line has them too, so nothing else is shown
    /// while notifications are off.
    @discardableResult
    func report(_ problems: [String]) -> Task<Void, Never> {
        inTurn { [self] in
            for problem in problems {
                await tell(.message("Queue Focus", problem))
            }
        }
    }

    /// A request made from outside the app failed: say so where it can be
    /// seen, in a note or, without one, on the popover's message line.
    @discardableResult
    func failed(_ title: String, _ message: String?) -> Task<Void, Never> {
        inTurn { [self] in
            if !(await tell(.message(title, message))) {
                model.actionError = message
                showPopover()
            }
        }
    }

    /// The queue changed: once the Done note's undo can no longer work, the
    /// note goes. Notification Center would keep it for hours.
    func modelDidChange() {
        if let done, model.snapshot.revision != done.revision {
            withdrawDone()
        }
    }

    /// The Done note goes, as the record of its completion does when the app
    /// stops.
    func withdrawDone() {
        done = nil
        notifier.withdraw(Note.doneID)
    }

    /// Run `work` once the request before it is done.
    private func inTurn(_ work: @escaping @MainActor () async -> Void) -> Task<Void, Never> {
        let before = last
        let task = Task { @MainActor in
            await before?.value
            await work()
        }
        last = task
        return task
    }

    /// Post `note` if notifications are on; whether it was.
    @discardableResult
    private func tell(_ note: Note) async -> Bool {
        guard await mayPost() else { return false }
        return await notifier.post(note)
    }

    /// Whether notes can be shown. The first time, ask, and say no: the
    /// answer is for next time.
    private func mayPost() async -> Bool {
        switch await notifier.permission() {
        case .allowed:
            return true
        case .notAsked:
            if !asked {
                asked = true
                notifier.ask()
            }
            return false
        case .denied:
            return false
        }
    }
}
