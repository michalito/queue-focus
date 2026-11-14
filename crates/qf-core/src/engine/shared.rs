//! The shared queue: the engine's requests as sync commands.
//!
//! Every request becomes one explicit command, resolved against the queue the
//! user sees: a cycled tag becomes the tag it cycles to, a shift becomes a
//! move before a named neighbour. A command is never derived by diffing a
//! mutated `Store`.
//!
//! Queued edits are applied to the view at once, from the journal. Changes to
//! Now and its timer are focus commands: they need a fresh view of the server
//! and wait for its receipt before anything changes.

use super::{invalid, EngineError, Submitted};
use crate::sync::{
    Anchor, Client, ClockSample, FocusAction, Id, QueuedAction, SyncStatus, SyncTask,
};
use crate::{Bucket, Outcome, QuickAdd, Store, Tag, Task, MAX_TITLE_CHARS};
use std::io;

#[cfg(test)]
mod tests;

/// The sync journal, and what the views read of it.
#[derive(Debug)]
pub(super) struct Shared {
    client: Client,
    /// The journal's projection: the confirmed queue with the queued edits
    /// replayed over it, in the form every view reads.
    store: Store,
    /// Bumped when the projected tasks change.
    revision: u64,
    status: SyncStatus,
    /// Bumped when `status` changes.
    status_revision: u64,
}

impl Shared {
    pub(super) fn new(mut client: Client, clock: ClockSample) -> Shared {
        Shared {
            store: client.store(clock),
            status: client.status(clock),
            client,
            revision: 0,
            status_revision: 0,
        }
    }

    pub(super) fn store(&self) -> &Store {
        &self.store
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn status(&self) -> &SyncStatus {
        &self.status
    }

    pub(super) fn status_revision(&self) -> u64 {
        self.status_revision
    }

    pub(super) fn client(&self) -> &Client {
        &self.client
    }

    pub(super) fn client_mut(&mut self) -> &mut Client {
        &mut self.client
    }

    /// Read the journal again. A poll that brings nothing new changes
    /// neither revision, so it does not rebuild the views.
    pub(super) fn refresh(&mut self, clock: ClockSample) {
        let status = self.client.status(clock);
        if status != self.status {
            self.status = status;
            self.status_revision = self.status_revision.wrapping_add(1);
        }
        let store = self.client.store(clock);
        // Reserving a handle for a pending create moves next_id, which no
        // view shows: only the tasks count.
        if store.tasks != self.store.tasks {
            self.revision = self.revision.wrapping_add(1);
        }
        self.store = store;
    }

    // ---- recovery ------------------------------------------------------

    pub(super) fn dismiss_recovery(&mut self, command: &str) -> Result<Outcome<()>, EngineError> {
        let clock = ClockSample::now()?;
        let command: Id = command
            .parse()
            .map_err(|_| invalid("invalid command identity"))?;
        let known = self
            .client
            .envelope()
            .recovery
            .iter()
            .any(|item| item.command.id == command);
        if !known {
            return Err(invalid("no rejected change with that identity"));
        }
        let result = self.client.dismiss_recovery(command);
        self.refresh(clock);
        result.map_err(command_error)
    }

    pub(super) fn retry(&mut self) -> Result<Outcome<()>, EngineError> {
        let clock = ClockSample::now()?;
        let result = self.client.resume_after_repair();
        self.refresh(clock);
        result.map_err(command_error)
    }

    // ---- the requests ----------------------------------------------------

    /// `bucket` is where a task goes when the text names none.
    pub(super) fn add(
        &mut self,
        text: &str,
        bucket: Bucket,
    ) -> Result<Submitted<u64>, EngineError> {
        let parsed = QuickAdd::parse(text).ok_or_else(|| invalid("empty title"))?;
        let title = bounded(&parsed.title);
        let task = Id::new_v4();
        let tag = parsed.tag;
        match parsed.bucket.unwrap_or(bucket) {
            Bucket::Now => self.focus(FocusAction::Create { task, title, tag }),
            bucket => {
                let outcome = self.queued(QueuedAction::Create {
                    task,
                    title,
                    tag,
                    bucket,
                })?;
                // The journal gave the new task its handle when it saved it.
                let handle = self
                    .client
                    .envelope()
                    .handle(task)
                    .expect("a saved create has a handle");
                Ok(Submitted::Applied(outcome.map(|()| handle)))
            }
        }
    }

    pub(super) fn complete_current(&mut self) -> Result<Submitted<Option<Task>>, EngineError> {
        if self.store.current().is_none() {
            return Ok(Submitted::Applied(Outcome::saved(None)));
        }
        self.focus(FocusAction::Complete)
    }

    /// Complete or remove a task: the current one through the server, any
    /// other as a queued edit. A queued deletion cannot be undone.
    pub(super) fn delete(&mut self, id: u64, complete: bool) -> Result<Submitted<()>, EngineError> {
        let task = self.task(id)?;
        if task.bucket == Bucket::Now {
            return self.focus(if complete {
                FocusAction::Complete
            } else {
                FocusAction::Remove
            });
        }
        self.queued(QueuedAction::Delete {
            task: task.id,
            title_version: task.versions.title,
        })
        .map(Submitted::Applied)
    }

    /// Only the completion the server still offers to undo can be undone.
    pub(super) fn undo(&mut self, id: u64) -> Result<Submitted<bool>, EngineError> {
        let clock = ClockSample::now()?;
        let offer = self
            .client
            .status(clock)
            .undo
            .filter(|offer| offer.task == id);
        match offer {
            Some(offer) => self.focus_at(
                FocusAction::Undo {
                    completion: offer.command,
                },
                clock,
            ),
            None => Ok(Submitted::Applied(Outcome::saved(false))),
        }
    }

    pub(super) fn toggle_pause(&mut self) -> Result<Submitted<bool>, EngineError> {
        let Some(current) = self.store.current() else {
            return Ok(Submitted::Applied(Outcome::saved(false)));
        };
        let paused = !current.is_paused();
        self.focus(FocusAction::SetPaused { paused })
    }

    pub(super) fn promote(&mut self, id: u64) -> Result<Submitted<()>, EngineError> {
        let task = self.task(id)?;
        if task.bucket == Bucket::Now {
            return Ok(Submitted::Applied(Outcome::saved(())));
        }
        self.focus(FocusAction::Promote { task: task.id })
    }

    pub(super) fn set_tag(
        &mut self,
        id: u64,
        tag: Option<Tag>,
    ) -> Result<Submitted<()>, EngineError> {
        let task = self.task(id)?;
        if task.tag == tag {
            return Ok(Submitted::Applied(Outcome::saved(())));
        }
        self.queued(QueuedAction::SetTag {
            task: task.id,
            expected: task.versions.tag,
            tag,
        })
        .map(Submitted::Applied)
    }

    pub(super) fn rename(&mut self, id: u64, title: &str) -> Result<Submitted<()>, EngineError> {
        let task = self.task(id)?;
        let title = bounded(title.trim());
        if task.title == title {
            return Ok(Submitted::Applied(Outcome::saved(())));
        }
        self.queued(QueuedAction::Rename {
            task: task.id,
            expected: task.versions.title,
            title,
        })
        .map(Submitted::Applied)
    }

    /// `index` counts the bucket's tasks other than this one, as the
    /// standalone queue counts them.
    pub(super) fn move_to(
        &mut self,
        id: u64,
        bucket: Bucket,
        index: Option<usize>,
    ) -> Result<Submitted<()>, EngineError> {
        if bucket == Bucket::Now {
            return self.promote(id);
        }
        self.task(id)?;
        let before = index.and_then(|index| {
            self.store
                .in_bucket(bucket)
                .filter(|t| t.id != id)
                .nth(index)
                .map(|t| t.id)
        });
        Ok(self.move_before(id, bucket, before)?.map(|_| ()))
    }

    pub(super) fn shift(&mut self, id: u64, delta: i32) -> Result<Submitted<bool>, EngineError> {
        let bucket = self.task(id)?.bucket;
        let ids: Vec<u64> = self.store.in_bucket(bucket).map(|t| t.id).collect();
        let index = ids
            .iter()
            .position(|candidate| *candidate == id)
            .expect("a task is in its own bucket");
        let last = ids.len() as i64 - 1;
        let target = (index as i64 + i64::from(delta)).clamp(0, last) as usize;
        if index == target {
            return Ok(Submitted::Applied(Outcome::saved(false)));
        }
        Ok(self.move_to(id, bucket, Some(target))?.map(|()| true))
    }

    /// `false` when `before` has gone or left `bucket` since it was drawn.
    pub(super) fn move_before(
        &mut self,
        id: u64,
        bucket: Bucket,
        before: Option<u64>,
    ) -> Result<Submitted<bool>, EngineError> {
        let task = self.task(id)?;
        if let Some(before) = before {
            if !self.store.get(before).is_some_and(|t| t.bucket == bucket) {
                return Ok(Submitted::Applied(Outcome::saved(false)));
            }
            if before == id {
                return Ok(Submitted::Applied(Outcome::saved(true)));
            }
        }
        if bucket == Bucket::Now {
            return Ok(self.promote(id)?.map(|()| true));
        }
        let before = match before {
            Some(before) => {
                let neighbour = self.task(before)?;
                Some(Anchor {
                    task: neighbour.id,
                    placement: neighbour.versions.placement,
                })
            }
            None => None,
        };
        if task.bucket == Bucket::Now {
            return self.focus(FocusAction::MoveToQueue { bucket, before });
        }
        let outcome = self.queued(QueuedAction::MoveBefore {
            task: task.id,
            expected: task.versions.placement,
            bucket,
            before,
        })?;
        Ok(Submitted::Applied(outcome.map(|()| true)))
    }

    // ---- plumbing --------------------------------------------------------

    /// The task behind a handle, as the journal projects it, with the field
    /// versions a command has to name.
    fn task(&self, id: u64) -> Result<SyncTask, EngineError> {
        let identity = self
            .client
            .envelope()
            .identity(id)
            .ok_or_else(|| invalid("no such task"))?;
        self.client
            .projected_snapshot()
            .task(identity)
            .cloned()
            .map_err(|_| invalid("no such task"))
    }

    /// Save a queued edit to the journal; the view shows it at once.
    fn queued(&mut self, action: QueuedAction) -> Result<Outcome<()>, EngineError> {
        let clock = ClockSample::now()?;
        let result = self.client.enqueue_queued(action);
        self.refresh(clock);
        result
            .map(|outcome| outcome.map(|_| ()))
            .map_err(command_error)
    }

    fn focus<T>(&mut self, action: FocusAction) -> Result<Submitted<T>, EngineError> {
        let clock = ClockSample::now()?;
        self.focus_at(action, clock)
    }

    /// Save a focus command and send it; nothing changes until the receipt.
    /// `clock` is the one instant the request was checked against.
    ///
    /// A save that committed but could not be made crash-safe is not lost:
    /// the status says so until it is, and holds the worker back meanwhile.
    fn focus_at<T>(
        &mut self,
        action: FocusAction,
        clock: ClockSample,
    ) -> Result<Submitted<T>, EngineError> {
        let result = self.client.enqueue_focus(action, clock);
        self.refresh(clock);
        let outcome = result.map_err(command_error)?;
        Ok(Submitted::Pending(outcome.value))
    }
}

/// A title as the protocol takes it.
fn bounded(title: &str) -> String {
    title.chars().take(MAX_TITLE_CHARS).collect()
}

/// The journal refuses a command it cannot take with invalid input or data;
/// anything else is a failure to save.
fn command_error(error: io::Error) -> EngineError {
    if matches!(
        error.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData
    ) {
        EngineError::InvalidArgument(error.to_string())
    } else {
        EngineError::Persistence(error)
    }
}
