//! Explicit native intents. Never derive a command from a mutated Store.
use super::invalid;
use super::*;
use crate::sync::*;
use crate::{QuickAdd, MAX_TITLE_CHARS};

#[cfg(test)]
mod tests;

impl Engine {
    /// The host owns the directory lock and the transport separately. This
    /// constructor never reads or writes the obsolete standalone task file.
    pub fn open_with_sync(
        dir: &Path,
        mut client: Client,
        clock: ClockSample,
    ) -> (Engine, Option<String>) {
        let tasks = Tasks::new(client.store(clock), dir.join(TASKS_FILE));
        let (settings, warning) = SettingsStore::load(dir.join(SETTINGS_FILE));
        let reminder = Reminder::new(clock.wall, settings.get());
        let status = client.status(clock);
        (
            Engine {
                tasks,
                settings,
                reminder,
                sync: Some(client),
                sync_status: Some(status),
                sync_status_revision: 0,
            },
            warning,
        )
    }

    pub fn sync_client_mut(&mut self) -> Option<&mut Client> {
        self.sync.as_mut()
    }

    pub fn sync_envelope(&self) -> Option<&Envelope> {
        self.sync.as_ref().map(Client::envelope)
    }

    pub fn sync_recovery(&self) -> Vec<RecoveryChange> {
        self.sync
            .as_ref()
            .map(Client::recovery_changes)
            .unwrap_or_default()
    }

    pub fn dismiss_sync_recovery(&mut self, command: &str) -> Result<Outcome<()>, EngineError> {
        let clock = ClockSample::now()?;
        let command = command
            .parse()
            .map_err(|_| invalid("invalid command identity"))?;
        let client = self
            .sync
            .as_mut()
            .ok_or_else(|| invalid("sync is not enabled"))?;
        if !client
            .envelope()
            .recovery
            .iter()
            .any(|item| item.command.id == command)
        {
            return Err(invalid("no rejected change with that identity"));
        }
        let result = client.dismiss_recovery(command).map_err(shared_error);
        self.refresh_sync(clock);
        result
    }

    pub fn retry_sync(&mut self) -> Result<Outcome<()>, EngineError> {
        let clock = ClockSample::now()?;
        let result = self
            .sync
            .as_mut()
            .ok_or_else(|| invalid("sync is not enabled"))?
            .resume_after_repair()
            .map_err(shared_error);
        self.refresh_sync(clock);
        result
    }

    pub fn sync_status(&self) -> Option<&SyncStatus> {
        self.sync_status.as_ref()
    }

    pub fn sync_status_revision(&self) -> u64 {
        self.sync_status_revision
    }

    /// Called after each worker poll and native mutation. Status changes must
    /// notify hosts separately when a pending focus action leaves the queue unchanged.
    pub fn refresh_sync(&mut self, clock: ClockSample) {
        if let Some(client) = &mut self.sync {
            let status = client.status(clock);
            if self.sync_status.as_ref() != Some(&status) {
                self.sync_status_revision = self.sync_status_revision.wrapping_add(1);
            }
            self.tasks.replace_projection(client.store(clock));
            self.sync_status = Some(status);
        }
    }

    fn shared_task(&self, id: u64) -> Result<SyncTask, EngineError> {
        let client = self.sync.as_ref().unwrap();
        let identity = client
            .envelope()
            .identity(id)
            .ok_or_else(|| invalid("no such task"))?;
        client
            .projected_snapshot()
            .task(identity)
            .cloned()
            .map_err(|_| invalid("no such task"))
    }

    fn queued(&mut self, action: QueuedAction) -> Result<Outcome<()>, EngineError> {
        let clock = ClockSample::now()?;
        let result = self.sync.as_mut().unwrap().enqueue_queued(action);
        self.refresh_sync(clock);
        result
            .map(|outcome| outcome.map(|_| ()))
            .map_err(shared_error)
    }

    fn focus<T>(&mut self, action: FocusAction) -> Result<Outcome<T>, EngineError> {
        let clock = ClockSample::now()?;
        let result = self.sync.as_mut().unwrap().enqueue_focus(action, clock);
        self.refresh_sync(clock);
        // A post-rename warning is also retained in status; never return an
        // ordinary failure that could cause a caller to repeat this intent.
        match result {
            Ok(outcome) => Err(EngineError::Pending {
                command_id: outcome.value,
            }),
            Err(error) => Err(shared_error(error)),
        }
    }

    pub(super) fn shared_add(
        &mut self,
        text: &str,
        bucket: Option<Bucket>,
    ) -> Result<Outcome<u64>, EngineError> {
        let parsed = QuickAdd::parse(text).ok_or_else(|| invalid("empty title"))?;
        let bucket = parsed
            .bucket
            .or(bucket)
            .unwrap_or(self.settings().default_bucket);
        let title: String = parsed.title.chars().take(MAX_TITLE_CHARS).collect();
        let task = Id::new_v4();
        if bucket == Bucket::Now {
            return self.focus(FocusAction::Create {
                task,
                title,
                tag: parsed.tag,
            });
        }
        let result = self.queued(QueuedAction::Create {
            task,
            title,
            tag: parsed.tag,
            bucket,
        })?;
        let handle = self
            .sync
            .as_ref()
            .unwrap()
            .envelope()
            .handle(task)
            .expect("persisted create handle");
        Ok(result.map(|_| handle))
    }

    pub(super) fn shared_complete_current(&mut self) -> Result<Outcome<Option<Task>>, EngineError> {
        if self.store().current().is_none() {
            return Ok(Outcome::saved(None));
        }
        self.focus(FocusAction::Complete)
    }

    pub(super) fn shared_delete(
        &mut self,
        id: u64,
        complete: bool,
    ) -> Result<Outcome<()>, EngineError> {
        let task = self.shared_task(id)?;
        if task.bucket == Bucket::Now {
            self.focus(if complete {
                FocusAction::Complete
            } else {
                FocusAction::Remove
            })
        } else {
            self.queued(QueuedAction::Delete {
                task: task.id,
                title_version: task.versions.title,
            })
        }
    }

    pub(super) fn shared_undo(&mut self, id: u64) -> Result<Outcome<bool>, EngineError> {
        let clock = ClockSample::now()?;
        let undo = self.sync.as_ref().unwrap().status(clock).undo;
        match undo.filter(|undo| undo.task == id) {
            Some(undo) => self.focus(FocusAction::Undo {
                completion: undo.command,
            }),
            None => Ok(Outcome::saved(false)),
        }
    }

    pub(super) fn shared_pause(&mut self) -> Result<Outcome<bool>, EngineError> {
        let Some(task) = self.store().current() else {
            return Ok(Outcome::saved(false));
        };
        self.focus(FocusAction::SetPaused {
            paused: !task.is_paused(),
        })
    }

    pub(super) fn shared_promote(&mut self, id: u64) -> Result<Outcome<()>, EngineError> {
        let task = self.shared_task(id)?;
        if task.bucket == Bucket::Now {
            return Ok(Outcome::saved(()));
        }
        self.focus(FocusAction::Promote { task: task.id })
    }

    pub(super) fn shared_tag(
        &mut self,
        id: u64,
        tag: Option<Tag>,
    ) -> Result<Outcome<()>, EngineError> {
        let task = self.shared_task(id)?;
        if task.tag == tag {
            return Ok(Outcome::saved(()));
        }
        self.queued(QueuedAction::SetTag {
            task: task.id,
            expected: task.versions.tag,
            tag,
        })
    }

    pub(super) fn shared_rename(
        &mut self,
        id: u64,
        title: &str,
    ) -> Result<Outcome<()>, EngineError> {
        let task = self.shared_task(id)?;
        let title: String = title.trim().chars().take(MAX_TITLE_CHARS).collect();
        if task.title == title {
            return Ok(Outcome::saved(()));
        }
        self.queued(QueuedAction::Rename {
            task: task.id,
            expected: task.versions.title,
            title,
        })
    }

    pub(super) fn shared_move_to(
        &mut self,
        id: u64,
        bucket: Bucket,
        index: Option<usize>,
    ) -> Result<Outcome<()>, EngineError> {
        self.require(id)?;
        if bucket == Bucket::Now {
            return self.shared_promote(id);
        }
        let before = index.and_then(|index| {
            self.store()
                .in_bucket(bucket)
                .filter(|t| t.id != id)
                .nth(index)
                .map(|t| t.id)
        });
        self.shared_move_before(id, bucket, before)
            .map(|outcome| outcome.map(|_| ()))
    }

    pub(super) fn shared_move_before(
        &mut self,
        id: u64,
        bucket: Bucket,
        before: Option<u64>,
    ) -> Result<Outcome<bool>, EngineError> {
        let task = self.shared_task(id)?;
        if let Some(before) = before {
            if !self.store().get(before).is_some_and(|t| t.bucket == bucket) {
                return Ok(Outcome::saved(false));
            }
            if before == id {
                return Ok(Outcome::saved(true));
            }
        }
        if bucket == Bucket::Now {
            return self.shared_promote(id).map(|o| o.map(|_| true));
        }
        let anchor = before
            .map(|id| {
                self.shared_task(id).map(|task| Anchor {
                    task: task.id,
                    placement: task.versions.placement,
                })
            })
            .transpose()?;
        if task.bucket == Bucket::Now {
            self.focus(FocusAction::MoveToQueue {
                bucket,
                before: anchor,
            })
        } else {
            self.queued(QueuedAction::MoveBefore {
                task: task.id,
                expected: task.versions.placement,
                bucket,
                before: anchor,
            })
            .map(|o| o.map(|_| true))
        }
    }

    pub(super) fn shared_shift(
        &mut self,
        id: u64,
        delta: i32,
    ) -> Result<Outcome<bool>, EngineError> {
        let bucket = self
            .store()
            .get(id)
            .ok_or_else(|| invalid("no such task"))?
            .bucket;
        let ids: Vec<_> = self.store().in_bucket(bucket).map(|t| t.id).collect();
        let index = ids.iter().position(|candidate| *candidate == id).unwrap();
        let target = (index as i64 + delta as i64).clamp(0, ids.len() as i64 - 1) as usize;
        if index == target {
            return Ok(Outcome::saved(false));
        }
        self.shared_move_to(id, bucket, Some(target))
            .map(|o| o.map(|_| true))
    }
}

fn shared_error(error: io::Error) -> EngineError {
    if matches!(
        error.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData
    ) {
        EngineError::InvalidArgument(error.to_string())
    } else {
        EngineError::Persistence(error)
    }
}
