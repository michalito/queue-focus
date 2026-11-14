import Observation

/// What an add field asks of the model. The model is the one host; tests
/// stand in for it.
@MainActor
protocol DraftHost: AnyObject {
    func add(_ text: String, asCurrent: Bool) -> AddResult
    /// Keep the server's answer to `command` for `draft` while it is there.
    func awaitSettlement(of command: String, by draft: AddDraft)
    /// The answer, once it has come; taking it ends the wait.
    func takeSettlement(of command: String) -> CommandSettlement?
}

/// An add field's text, and the task for the shared Now it may be waiting
/// for. Every add field keeps one, so they all behave the same.
///
/// A task for the shared Now waits for the sync server. Until it answers,
/// the field keeps its text and takes no other; once the server confirms,
/// the text is cleared if it is still what was sent, and once it refuses,
/// the text stays for another try. The host says why on its message line.
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
    func submit(to host: DraftHost, asCurrent: Bool) -> Bool {
        guard awaiting == nil else { return false }
        switch host.add(text, asCurrent: asCurrent) {
        case .added:
            text = ""
            return true
        case .pending(let command):
            awaiting = (command, text)
            host.awaitSettlement(of: command, by: self)
            return settle(with: host)
        case .notAdded:
            return false
        }
    }

    /// Take the server's answer, if it has come. Returns whether the task
    /// was added and the text cleared, as `submit` does.
    func settle(with host: DraftHost) -> Bool {
        guard let (command, sent) = awaiting,
              let settlement = host.takeSettlement(of: command) else { return false }
        awaiting = nil
        guard settlement.applied, text == sent else { return false }
        text = ""
        return true
    }
}
