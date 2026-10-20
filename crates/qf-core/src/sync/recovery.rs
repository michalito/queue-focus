//! Readable recovery records derived from immutable rejected intent.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryChange {
    pub command: String,
    pub title: String,
    pub attempt: String,
    pub reason: String,
    /// The full rejected intent, including identity and concurrency guards.
    pub copy_text: String,
}

impl Client {
    pub fn recovery_changes(&self) -> Vec<RecoveryChange> {
        self.envelope()
            .recovery
            .iter()
            .map(|item| {
                let task_title = |id: Id| {
                    self.envelope()
                        .confirmed
                        .task(id)
                        .map(|t| t.title.clone())
                        .unwrap_or_else(|_| format!("Task {id}"))
                };
                let (title, attempt) = match &item.command.action {
                    Action::Queued { action } => match action {
                        QueuedAction::Create { title, bucket, .. } => {
                            (title.clone(), format!("Add to {bucket:?}"))
                        }
                        QueuedAction::Rename { title, .. } => (title.clone(), "Rename task".into()),
                        QueuedAction::SetTag { task, tag, .. } => (
                            task_title(*task),
                            match tag {
                                Some(crate::Tag::Work) => "Set work tag",
                                Some(crate::Tag::Personal) => "Set personal tag",
                                None => "Remove tag",
                            }
                            .into(),
                        ),
                        QueuedAction::MoveBefore { task, bucket, .. } => {
                            (task_title(*task), format!("Move to {bucket:?}"))
                        }
                        QueuedAction::Delete { task, .. } => {
                            (task_title(*task), "Remove queued task".into())
                        }
                    },
                    Action::Focus { guard, action } => {
                        let title = match action {
                            FocusAction::Create { title, .. } => title.clone(),
                            FocusAction::Promote { task } => task_title(*task),
                            _ => guard.task.map(task_title).unwrap_or_else(|| "Now".into()),
                        };
                        let attempt = match action {
                            FocusAction::Create { .. } => "Add to Now",
                            FocusAction::Promote { .. } => "Make current",
                            FocusAction::Complete => "Complete Now",
                            FocusAction::Remove => "Remove Now",
                            FocusAction::MoveToQueue { .. } => "Move Now to a queue",
                            FocusAction::SetPaused { paused: true } => "Pause timer",
                            FocusAction::SetPaused { paused: false } => "Resume timer",
                            FocusAction::Undo { .. } => "Undo completion",
                        };
                        (title, attempt.into())
                    }
                    Action::Initialize { .. } => ("Local queue".into(), "Initialize sync".into()),
                };
                RecoveryChange {
                    command: item.command.id.to_string(),
                    title,
                    attempt,
                    reason: item.reason.to_string(),
                    copy_text: serde_json::to_string_pretty(item)
                        .expect("recovery intent contains only serializable protocol values"),
                }
            })
            .collect()
    }
}
