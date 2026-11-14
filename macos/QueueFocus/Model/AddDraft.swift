import Observation

/// An add field's text, and the task for the shared Now it may be waiting
/// for. Every add field keeps one, so they all behave the same.
///
/// A task for the shared Now waits for the sync server. Until it answers,
/// the field keeps its text and takes no other; once the server confirms,
/// the text is cleared if it is still what was sent, and once it refuses,
/// the text stays for another try and the message line says why.
@MainActor
@Observable
final class AddDraft {
    var text = ""
    /// The add being waited for, and the text that was sent with it.
    private var awaiting: (command: String, sent: String)?

    /// Whether the field waits for the server, and so takes no edits.
    var isWaiting: Bool { awaiting != nil }

    /// Add the text. Returns whether the task is in the queue, its text
    /// cleared; a task the server has to confirm is not, yet.
    func submit(to model: QueueModel, asCurrent: Bool) -> Bool {
        guard awaiting == nil else { return false }
        switch model.add(text, asCurrent: asCurrent) {
        case .added:
            text = ""
            return true
        case .pending(let command):
            awaiting = (command, text)
            model.awaitSettlement(of: command)
            return settle(with: model)
        case .notAdded:
            return false
        }
    }

    /// Take the server's answer, if it has come. Returns whether the task
    /// was added and the text cleared, as `submit` does.
    func settle(with model: QueueModel) -> Bool {
        guard let (command, sent) = awaiting,
              let settlement = model.takeSettlement(of: command) else { return false }
        awaiting = nil
        guard settlement.applied else {
            model.actionError = settlement.message ?? "The task was not added. Its text is still here."
            return false
        }
        guard text == sent else { return false }
        text = ""
        return true
    }
}
