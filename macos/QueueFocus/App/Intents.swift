import AppIntents

/// The app's queue, for the intents: the system runs them in this process,
/// starting the app first if it is not running.
@MainActor
enum IntentHost {
    static var model: QueueModel?
}

/// Why an intent could not do what it was asked.
enum IntentFailure: Error, Equatable, CustomLocalizedStringResourceConvertible {
    case notOpen
    case failed(String)

    var localizedStringResource: LocalizedStringResource {
        switch self {
        case .notOpen: "Queue Focus could not open its queue."
        case .failed(let message): "\(message)"
        }
    }
}

struct AddTaskIntent: AppIntent {
    static let title: LocalizedStringResource = "Add a Task"
    static let description = IntentDescription("Adds a task to Queue Focus. The add field's markers work here too: !now, #w, #p, @later, @side.")
    static let openAppWhenRun = false

    @Parameter(title: "Task")
    var text: String

    @Parameter(title: "Make It Current", default: false)
    var asCurrent: Bool

    @MainActor
    func perform() async throws -> some IntentResult & ProvidesDialog {
        guard let model = IntentHost.model else { throw IntentFailure.notOpen }
        // Blank text asks nothing of the engine, so it leaves no reason of
        // its own: an earlier one would be wrong.
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw IntentFailure.failed("There was nothing to add.")
        }
        guard model.add(text, asCurrent: asCurrent) else {
            throw IntentFailure.failed(model.actionError ?? "The task could not be added.")
        }
        return .result(dialog: "Added.")
    }
}

struct CompleteCurrentTaskIntent: AppIntent {
    static let title: LocalizedStringResource = "Complete the Current Task"
    static let description = IntentDescription("Marks the current task done and makes the next one current. The popover offers to undo it.")
    static let openAppWhenRun = false

    @MainActor
    func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        guard let model = IntentHost.model else { throw IntentFailure.notOpen }
        switch model.completeCurrent() {
        case .done(let task):
            model.offerUndo(for: task)
            return .result(value: task.title, dialog: "Done: \(task.title)")
        case .empty:
            return .result(value: "", dialog: "Nothing in Now.")
        case .failed(let message):
            throw IntentFailure.failed(message)
        }
    }
}

struct CurrentTaskIntent: AppIntent {
    static let title: LocalizedStringResource = "Get the Current Task"
    static let description = IntentDescription("The current task and how long it has been running, as the menu bar shows it.")
    static let openAppWhenRun = false

    @MainActor
    func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        guard let model = IntentHost.model else { throw IntentFailure.notOpen }
        let current = Self.describe(model)
        return .result(value: current, dialog: "\(current.isEmpty ? "Nothing in Now." : current)")
    }

    /// `ship v0.1 · 23m`, or empty with nothing in Now.
    @MainActor
    static func describe(_ model: QueueModel) -> String {
        guard let task = model.snapshot.current else { return "" }
        guard let secs = model.elapsed(of: task) else { return task.title }
        return "\(task.title) · \(shortElapsed(secs: secs, paused: task.pausedAt != nil))"
    }
}

/// The intents as Shortcuts offers them without setting anything up.
struct QueueFocusShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: AddTaskIntent(), phrases: ["Add a task to \(.applicationName)"],
                    shortTitle: "Add Task", systemImageName: "plus")
        AppShortcut(intent: CompleteCurrentTaskIntent(), phrases: ["Complete the current task in \(.applicationName)"],
                    shortTitle: "Complete Current Task", systemImageName: "checkmark")
        AppShortcut(intent: CurrentTaskIntent(), phrases: ["What is the current task in \(.applicationName)"],
                    shortTitle: "Current Task", systemImageName: "text.badge.checkmark")
    }
}
