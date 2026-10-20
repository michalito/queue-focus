use super::protocol::{invalid, valid_id, valid_title};
use super::*;
use crate::Bucket;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UndoRecord {
    completion: Id,
    until: u64,
    before: Snapshot,
}

/// Durable server domain state. A storage transaction adds receipt/history
/// records around `apply`; duplicate receipt lookup must precede reduction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerState {
    pub snapshot: Snapshot,
    pub used_ids: BTreeSet<Id>,
    undo: Option<UndoRecord>,
}

impl ServerState {
    /// Establish a new root after an operator restores a backup. Existing
    /// clients must recover explicitly; all old concurrency tokens expire.
    pub fn reset_generation(&mut self, generation: Id) -> Result<(), Rejection> {
        valid_id(generation)?;
        if generation == self.snapshot.generation {
            return Err(invalid("restore must use a new generation"));
        }
        self.validate()?;
        let mut next = self.clone();
        next.undo = None;
        next.snapshot.undo = None;
        next.snapshot.generation = generation;
        next.snapshot.focus.version = generation;
        if let Some(session) = &mut next.snapshot.focus.session {
            session.id = generation;
        }
        for task in &mut next.snapshot.tasks {
            task.versions = Versions::new(generation);
        }
        let old_history = std::mem::take(&mut next.snapshot.history);
        next.snapshot.history =
            fingerprint(&("queue-focus-restore-v1", old_history, &next.snapshot))?;
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Validate persisted domain state before serving it. Server-private
    /// tombstones and undo information must agree with the public snapshot.
    pub fn validate(&self) -> Result<(), Rejection> {
        self.snapshot.validate()?;
        if self.used_ids.iter().any(Id::is_nil)
            || self
                .snapshot
                .tasks
                .iter()
                .any(|t| !self.used_ids.contains(&t.id))
        {
            return Err(invalid("used task identities disagree with live tasks"));
        }
        match (&self.undo, &self.snapshot.undo) {
            (None, None) => {}
            (Some(record), Some(hint)) => {
                record.before.validate()?;
                if record.completion != hint.command
                    || record.until != hint.until
                    || record.before.dataset != self.snapshot.dataset
                    || record.before.generation != self.snapshot.generation
                    || record.before.revision.checked_add(1) != Some(self.snapshot.revision)
                    || record.before.current().map(|t| (t.id, t.title.as_str()))
                        != Some((hint.task, hint.title.as_str()))
                    || !self.used_ids.contains(&hint.task)
                {
                    return Err(invalid("undo record disagrees with snapshot"));
                }
            }
            _ => return Err(invalid("undo eligibility disagrees with record")),
        }
        Ok(())
    }

    pub fn new(dataset: Id, generation: Id) -> Result<Self, Rejection> {
        valid_id(dataset)?;
        valid_id(generation)?;
        let mut snapshot = Snapshot {
            version: PROTOCOL_VERSION,
            dataset,
            generation,
            revision: 0,
            history: String::new(),
            initialized: false,
            tasks: Vec::new(),
            focus: Focus {
                version: generation,
                session: None,
            },
            undo: None,
        };
        snapshot.history = fingerprint(&("queue-focus-genesis-v1", &snapshot))?;
        snapshot.validate()?;
        Ok(Self {
            snapshot,
            used_ids: BTreeSet::new(),
            undo: None,
        })
    }

    /// Reject atomically: none of the receiver changes on error. `now` is the
    /// server's persisted, nondecreasing seconds timestamp, supplied by storage.
    pub fn apply(&mut self, command: &Command, now: u64) -> Result<(), Rejection> {
        if command.version != PROTOCOL_VERSION {
            return Err(Rejection::UnsupportedVersion);
        }
        if command.dataset != self.snapshot.dataset
            || command.generation != self.snapshot.generation
        {
            return Err(Rejection::WrongDataset);
        }
        valid_id(command.id)?;
        let mut next = self.clone();
        let previous = self.snapshot.history.clone();
        next.reduce(command, now)?;
        next.snapshot.revision = self
            .snapshot
            .revision
            .checked_add(1)
            .ok_or(Rejection::Capacity)?;
        next.snapshot.history.clear();
        next.snapshot.history =
            fingerprint(&("queue-focus-revision-v1", previous, command, &next.snapshot))?;
        next.validate()?;
        *self = next;
        Ok(())
    }

    fn reduce(&mut self, command: &Command, now: u64) -> Result<(), Rejection> {
        let is_undo = matches!(
            command.action,
            Action::Focus {
                action: FocusAction::Undo { .. },
                ..
            }
        );
        if !is_undo {
            self.undo = None;
            self.snapshot.undo = None;
        }
        match &command.action {
            Action::Initialize { tasks, timer } => {
                self.initialize(command.id, tasks, timer.as_ref(), now)
            }
            Action::Queued { action } => {
                if !self.snapshot.initialized {
                    return Err(Rejection::NotInitialized);
                }
                self.queued(command.id, action, now)
            }
            Action::Focus { guard, action } => {
                if !self.snapshot.initialized {
                    return Err(Rejection::NotInitialized);
                }
                self.check_focus(guard, now)?;
                self.focus(command.id, action, now)
            }
        }
    }

    fn initialize(
        &mut self,
        command: Id,
        tasks: &[SeedTask],
        timer: Option<&SeedTimer>,
        now: u64,
    ) -> Result<(), Rejection> {
        if self.snapshot.initialized {
            return Err(Rejection::AlreadyInitialized);
        }
        if tasks.len() > MAX_TASKS {
            return Err(Rejection::Capacity);
        }
        for t in tasks {
            self.create(command, t.id, &t.title, t.tag, t.bucket, t.created_at)?;
        }
        let current: Vec<_> = self
            .snapshot
            .tasks
            .iter()
            .filter(|t| t.bucket == Bucket::Now)
            .collect();
        self.snapshot.focus = Focus {
            version: command,
            session: match (current.as_slice(), timer) {
                ([], None) => None,
                ([task], Some(timer)) => Some(Session {
                    id: command,
                    task: task.id,
                    elapsed: timer.elapsed,
                    running: timer.running,
                    anchor: now,
                }),
                _ => return Err(invalid("initial Now requires exactly one matching timer")),
            },
        };
        self.snapshot.initialized = true;
        Ok(())
    }

    fn create(
        &mut self,
        command: Id,
        id: Id,
        title: &str,
        tag: Option<crate::Tag>,
        bucket: Bucket,
        created_at: u64,
    ) -> Result<(), Rejection> {
        valid_id(id)?;
        valid_title(title)?;
        if self.used_ids.contains(&id) {
            return Err(Rejection::Conflict);
        }
        if self.snapshot.tasks.len() >= MAX_TASKS {
            return Err(Rejection::Capacity);
        }
        self.used_ids.insert(id);
        self.snapshot.tasks.push(SyncTask {
            id,
            title: title.into(),
            tag,
            bucket,
            created_at,
            versions: Versions::new(command),
        });
        Ok(())
    }

    fn queued(&mut self, command: Id, action: &QueuedAction, now: u64) -> Result<(), Rejection> {
        match action {
            QueuedAction::Create {
                task,
                title,
                tag,
                bucket,
            } => {
                queued(*bucket)?;
                self.create(command, *task, title, *tag, *bucket, now)
            }
            QueuedAction::Rename {
                task,
                expected,
                title,
            } => {
                valid_title(title)?;
                let task = self.task_mut(*task)?;
                expect(task.versions.title, *expected)?;
                task.title.clone_from(title);
                task.versions.title = command;
                Ok(())
            }
            QueuedAction::SetTag {
                task,
                expected,
                tag,
            } => {
                let task = self.task_mut(*task)?;
                expect(task.versions.tag, *expected)?;
                task.tag = *tag;
                task.versions.tag = command;
                Ok(())
            }
            QueuedAction::MoveBefore {
                task,
                expected,
                bucket,
                before,
            } => {
                let existing = self.snapshot.task(*task)?;
                queued(existing.bucket)?;
                expect(existing.versions.placement, *expected)?;
                queued(*bucket)?;
                self.check_anchor(*task, *bucket, before.as_ref())?;
                self.place(*task, *bucket, before.as_ref().map(|a| a.task), command)
            }
            QueuedAction::Delete {
                task,
                title_version,
            } => {
                let existing = self.snapshot.task(*task)?;
                queued(existing.bucket)?;
                expect(existing.versions.title, *title_version)?;
                self.snapshot.tasks.retain(|t| t.id != *task);
                Ok(())
            }
        }
    }

    fn check_focus(&self, guard: &FocusGuard, now: u64) -> Result<(), Rejection> {
        if now >= guard.expires {
            return Err(Rejection::Expired);
        }
        if guard.expires > now.saturating_add(FOCUS_TTL_SECS) {
            return Err(invalid("focus expiry is too far in the future"));
        }
        let focus = &self.snapshot.focus;
        if guard.revision != self.snapshot.revision
            || guard.version != focus.version
            || guard.session != focus.session.as_ref().map(|s| s.id)
            || guard.task != focus.session.as_ref().map(|s| s.task)
        {
            return Err(Rejection::Conflict);
        }
        Ok(())
    }

    fn focus(&mut self, command: Id, action: &FocusAction, now: u64) -> Result<(), Rejection> {
        match action {
            FocusAction::Promote { task } => self.promote(*task, command, now)?,
            FocusAction::Create { task, title, tag } => {
                self.create(command, *task, title, *tag, Bucket::Next, now)?;
                self.promote(*task, command, now)?;
            }
            FocusAction::Complete => {
                let task = self.current_id()?;
                let old = self.snapshot.clone();
                let title = self.snapshot.task(task)?.title.clone();
                self.snapshot.tasks.retain(|t| t.id != task);
                self.snapshot.focus.session = None;
                if let Some(next) = self
                    .snapshot
                    .tasks
                    .iter()
                    .find(|t| t.bucket == Bucket::Next)
                    .map(|t| t.id)
                {
                    self.promote(next, command, now)?;
                }
                let until = now.saturating_add(UNDO_SECS);
                self.undo = Some(UndoRecord {
                    completion: command,
                    until,
                    before: old,
                });
                self.snapshot.undo = Some(UndoHint {
                    command,
                    task,
                    title,
                    until,
                });
            }
            FocusAction::Remove => {
                let task = self.current_id()?;
                self.snapshot.tasks.retain(|t| t.id != task);
                self.snapshot.focus.session = None;
            }
            FocusAction::MoveToQueue { bucket, before } => {
                queued(*bucket)?;
                let task = self.current_id()?;
                self.check_anchor(task, *bucket, before.as_ref())?;
                self.place(task, *bucket, before.as_ref().map(|a| a.task), command)?;
                self.snapshot.focus.session = None;
            }
            FocusAction::SetPaused { paused } => {
                let session = self
                    .snapshot
                    .focus
                    .session
                    .as_mut()
                    .ok_or(Rejection::MissingTask)?;
                session.elapsed = session.elapsed_at(now);
                session.anchor = now;
                session.running = !paused;
            }
            FocusAction::Undo { completion } => {
                let undo = self.undo.take().ok_or(Rejection::Conflict)?;
                if undo.completion != *completion {
                    return Err(Rejection::Conflict);
                }
                if now >= undo.until {
                    return Err(Rejection::Expired);
                }
                // Preserve identities but never restore obsolete concurrency
                // tokens. Unchanged live fields retain their current tokens.
                let current = &self.snapshot;
                let mut restored = undo.before;
                for t in &mut restored.tasks {
                    match current.tasks.iter().find(|v| v.id == t.id) {
                        Some(live) => {
                            t.versions = live.versions.clone();
                            if live.bucket != t.bucket {
                                t.versions.placement = command;
                            }
                        }
                        None => t.versions = Versions::new(command),
                    }
                }
                if let Some(session) = &mut restored.focus.session {
                    session.id = command;
                }
                restored.undo = None;
                self.snapshot = restored;
            }
        }
        self.snapshot.focus.version = command;
        Ok(())
    }

    fn current_id(&self) -> Result<Id, Rejection> {
        self.snapshot
            .focus
            .session
            .as_ref()
            .map(|s| s.task)
            .ok_or(Rejection::MissingTask)
    }

    fn promote(&mut self, task: Id, command: Id, now: u64) -> Result<(), Rejection> {
        self.snapshot.task(task)?;
        if self.current_id().ok() == Some(task) {
            return Ok(());
        }
        if let Some(current) = self.snapshot.current().map(|t| t.id) {
            let head = self
                .snapshot
                .tasks
                .iter()
                .find(|t| t.bucket == Bucket::Next && t.id != task)
                .map(|t| t.id);
            self.place(current, Bucket::Next, head, command)?;
        }
        self.place(task, Bucket::Now, None, command)?;
        self.snapshot.focus.session = Some(Session {
            id: command,
            task,
            elapsed: 0,
            running: true,
            anchor: now,
        });
        Ok(())
    }

    fn check_anchor(
        &self,
        task: Id,
        bucket: Bucket,
        before: Option<&Anchor>,
    ) -> Result<(), Rejection> {
        if let Some(anchor) = before {
            if anchor.task == task {
                return Err(invalid("a task cannot be its own move anchor"));
            }
            let target = self.snapshot.task(anchor.task)?;
            if target.bucket != bucket {
                return Err(Rejection::Conflict);
            }
            expect(target.versions.placement, anchor.placement)?;
        }
        Ok(())
    }

    fn place(
        &mut self,
        task: Id,
        bucket: Bucket,
        before: Option<Id>,
        command: Id,
    ) -> Result<(), Rejection> {
        let index = self
            .snapshot
            .tasks
            .iter()
            .position(|t| t.id == task)
            .ok_or(Rejection::MissingTask)?;
        let mut task = self.snapshot.tasks.remove(index);
        task.bucket = bucket;
        task.versions.placement = command;
        let at = match before {
            Some(anchor) => self
                .snapshot
                .tasks
                .iter()
                .position(|t| t.id == anchor)
                .ok_or(Rejection::MissingTask)?,
            None => self
                .snapshot
                .tasks
                .iter()
                .rposition(|t| t.bucket == bucket)
                .map_or(self.snapshot.tasks.len(), |i| i + 1),
        };
        self.snapshot.tasks.insert(at, task);
        Ok(())
    }

    fn task_mut(&mut self, id: Id) -> Result<&mut SyncTask, Rejection> {
        self.snapshot
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or(Rejection::MissingTask)
    }
}

fn queued(bucket: Bucket) -> Result<(), Rejection> {
    if bucket == Bucket::Now {
        Err(Rejection::Conflict)
    } else {
        Ok(())
    }
}
fn expect(actual: Id, expected: Id) -> Result<(), Rejection> {
    if actual != expected {
        Err(Rejection::Conflict)
    } else {
        Ok(())
    }
}
