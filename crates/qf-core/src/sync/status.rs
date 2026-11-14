//! How a shared queue stands, as every host shows it.
//!
//! The rules the views follow live here, once: whether Now and its timer can
//! be changed, and the one line that says how sync is going. The GNOME app,
//! the Shell extension and the macOS app all read them from this status
//! rather than working them out for themselves.

use super::{Id, Settlement};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// A fresh answer from the server, and nothing stopping the next one.
    Online,
    /// No fresh answer yet: edits to the queue are saved here meanwhile.
    Offline,
    /// The server or the credentials need the user; nothing is sent.
    NeedsAttention,
    /// The server's history differs from this device's: export and rejoin.
    Recovery,
    /// A save could not be made crash-safe yet; nothing is sent until it is.
    Durability,
}

/// A completion the server still offers to undo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientUndo {
    /// The completion's command, which the undo names.
    pub command: Id,
    /// The completed task's handle.
    pub task: u64,
    pub title: String,
    /// Seconds left to undo it.
    pub remaining: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatus {
    pub state: ConnectionState,
    /// Why sync is held up, when it is: a suspension's or a save's message.
    pub message: Option<String>,
    /// The one line a view shows for sync.
    pub summary: String,
    /// Whether Now and its timer could be changed when the status was taken:
    /// each change to them needs a fresh answer from the server and nothing
    /// else waiting to be sent. The engine checks again when asked, so a
    /// status left unrefreshed past freshness only offers a change that is
    /// then refused, never lets one through.
    pub focus_available: bool,
    /// Commands saved here and not yet confirmed.
    pub pending: usize,
    /// The change to Now waiting for its receipt.
    pub focus_command: Option<Id>,
    /// Queued edits that will probably be refused.
    pub likely_conflicts: usize,
    /// Refused changes kept for the user to review.
    pub recovery_items: usize,
    pub undo: Option<ClientUndo>,
    /// The receipt of the last change to Now, kept across restarts so a
    /// view that missed the event can still settle what it was waiting for.
    pub last_focus_settlement: Option<Settlement>,
}

/// The line for a status: the problem when there is one, else the work in
/// hand, else whether this device is in step.
pub(super) fn summary(
    state: ConnectionState,
    message: Option<&str>,
    focus_command: Option<Id>,
    pending: usize,
    recovery_items: usize,
) -> String {
    if let Some(message) = message {
        return message.into();
    }
    if focus_command.is_some() {
        return "Sent, waiting for confirmation…".into();
    }
    match (recovery_items, pending) {
        (1, _) => "Sync needs attention: 1 saved change needs review.".into(),
        (n, _) if n > 1 => format!("Sync needs attention: {n} saved changes need review."),
        (_, 1) => "1 change saved on this device, waiting to sync.".into(),
        (_, n) if n > 1 => format!("{n} changes saved on this device, waiting to sync."),
        _ if state == ConnectionState::Online => "Synced".into(),
        _ => {
            "Offline — queue edits are saved here. Now and timer changes need a connection.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(state: ConnectionState, pending: usize, recovery: usize) -> String {
        summary(state, None, None, pending, recovery)
    }

    #[test]
    fn the_line_puts_a_problem_first_then_the_work_in_hand() {
        let command = Some(Id::new_v4());
        assert_eq!(
            summary(
                ConnectionState::NeedsAttention,
                Some("Bad token."),
                command,
                3,
                2
            ),
            "Bad token."
        );
        assert_eq!(
            summary(ConnectionState::Online, None, command, 1, 2),
            "Sent, waiting for confirmation…"
        );
        assert_eq!(
            line(ConnectionState::Online, 4, 1),
            "Sync needs attention: 1 saved change needs review."
        );
        assert_eq!(
            line(ConnectionState::Offline, 0, 3),
            "Sync needs attention: 3 saved changes need review."
        );
        assert_eq!(
            line(ConnectionState::Offline, 1, 0),
            "1 change saved on this device, waiting to sync."
        );
        assert_eq!(
            line(ConnectionState::Online, 2, 0),
            "2 changes saved on this device, waiting to sync."
        );
        assert_eq!(line(ConnectionState::Online, 0, 0), "Synced");
        assert!(line(ConnectionState::Offline, 0, 0).starts_with("Offline — "));
    }
}
