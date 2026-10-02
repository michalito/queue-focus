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
@MainActor
final class Notices {
    private let model: QueueModel
    private let notifier: Notifier
    private let popoverIsShown: @MainActor () -> Bool
    private let showPopover: @MainActor () -> Void
    /// The completion the Done note on screen offers to undo, and the
    /// queue's revision right after it.
    private var done: (id: UInt64, revision: UInt64)?
    /// Asked for permission already this run: the system asks the user once.
    private var asked = false

    init(model: QueueModel, notifier: Notifier, popoverIsShown: @escaping @MainActor () -> Bool,
         showPopover: @escaping @MainActor () -> Void) {
        self.model = model
        self.notifier = notifier
        self.popoverIsShown = popoverIsShown
        self.showPopover = showPopover
    }

    /// Complete the current task from anywhere, and offer to undo it.
    func completeCurrent() async {
        switch model.completeCurrent() {
        case .done(let task):
            model.offerUndo(for: task)
            let revision = model.snapshot.revision
            // An open popover offers it in its own row, as GNOME's open menu
            // does instead of a notification.
            guard !popoverIsShown() else { return }
            let allowed = await mayPost()
            // Meanwhile the queue may have moved on, and the undo with it, or
            // the popover may have opened and be offering it already.
            guard model.snapshot.revision == revision, !popoverIsShown() else { return }
            if allowed, await notifier.post(.done(task)) {
                done = (task.id, revision)
                // The queue may have moved on while the note was posted.
                modelDidChange()
            } else {
                showPopover()
            }
        case .empty:
            if !(await tell(.message("Nothing in Now"))) {
                showPopover()
            }
        case .failed(let message):
            // The popover's message line says it too.
            if !(await tell(.message("Could not complete the task", message))) {
                showPopover()
            }
        }
    }

    /// Undo, from the Done note's button. The engine decides whether it
    /// still can, whatever the popover's own offer says.
    func undo(id: UInt64) async {
        switch model.undo(id: id) {
        case .undone:
            withdrawDone()
        case .stale:
            withdrawDone()
            await tell(.message("Nothing to undo", "The queue changed since."))
        case .failed(let message):
            // Nothing changed and the engine still has the completion, so
            // the note offers to try again, as GNOME's menu keeps its offer.
            let retry = Note(id: Note.doneID, title: "Could not undo", body: message, undo: id)
            if await mayPost(), await notifier.post(retry) {
                done = (id, model.snapshot.revision)
                modelDidChange()
            } else {
                showPopover()
            }
        }
    }

    /// One note per problem the engine reported, as GNOME posts one per
    /// warning. The message line has them too, so nothing else is shown
    /// while notifications are off.
    func report(_ problems: [String]) async {
        for problem in problems {
            await tell(.message("Queue Focus", problem))
        }
    }

    /// A request made from outside the app failed: say so where it can be
    /// seen, in a note or, without one, on the popover's message line.
    func failed(_ title: String, _ message: String?) async {
        if !(await tell(.message(title, message))) {
            model.actionError = message
            showPopover()
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
