//! The queue and the file it lives in. Every change is saved before it counts,
//! and the most recent completion can be taken back while nothing else has
//! changed since.
//!
//! This is plain data with plain methods: no shared handles, no interior
//! mutability and no listeners. A host that has to tell anyone about a change
//! compares `revision` before and after.

use crate::{Completed, SaveError, Store, Task};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// A change reached the task file, but syncing its containing directory
/// failed, so the change may not survive a crash immediately afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurabilityWarning {
    path: PathBuf,
    error: String,
}

impl fmt::Display for DurabilityWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "saved {} but could not make the change crash-safe: {}",
            self.path.display(),
            self.error
        )
    }
}

/// What a saved change returned. Whoever asked for the change reports the
/// warning through its own channel (dialog, stderr, notification or D-Bus
/// signal), so the user hears about it exactly once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome<R> {
    pub value: R,
    /// Set when the change committed but may not be crash-safe yet.
    pub warning: Option<DurabilityWarning>,
}

impl<R> Outcome<R> {
    /// A change that needed no save, or saved cleanly.
    pub fn saved(value: R) -> Self {
        Outcome {
            value,
            warning: None,
        }
    }

    pub fn into_parts(self) -> (R, Option<DurabilityWarning>) {
        (self.value, self.warning)
    }

    pub fn map<T>(self, f: impl FnOnce(R) -> T) -> Outcome<T> {
        Outcome {
            value: f(self.value),
            warning: self.warning,
        }
    }
}

/// The most recent completion, reversible only while nothing else has changed.
#[derive(Debug)]
struct LastCompletion {
    completed: Completed,
    revision: u64,
}

#[derive(Debug)]
pub struct Tasks {
    store: Store,
    path: PathBuf,
    /// Counts committed changes; lets the undo record tell whether it is stale.
    revision: u64,
    undo: Option<LastCompletion>,
}

impl Tasks {
    /// Read the task file. An absent file is an empty queue. A file that
    /// cannot be read is an error, so the caller can refuse to start rather
    /// than replace it.
    pub fn load(path: PathBuf) -> io::Result<Tasks> {
        let store = crate::load(&path).map_err(|e| {
            io::Error::new(e.kind(), format!("could not read {}: {e}", path.display()))
        })?;
        Ok(Tasks::new(store, path))
    }

    /// The queue in `store`, saved to `path` from the next change on.
    pub fn new(store: Store, path: PathBuf) -> Tasks {
        Tasks {
            store,
            path,
            revision: 0,
            undo: None,
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many changes have been saved since the file was loaded.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Apply a change and save it. A failure before the atomic rename rolls
    /// the change back. A failure syncing the directory after the rename comes
    /// back as a warning, and the committed change stays a success, so a
    /// caller does not retry an operation that is not idempotent. A change
    /// that changes nothing is not saved and does not count as one.
    pub fn update<R>(&mut self, f: impl FnOnce(&mut Store) -> R) -> io::Result<Outcome<R>> {
        self.update_with(f, crate::save)
    }

    fn update_with<R>(
        &mut self,
        f: impl FnOnce(&mut Store) -> R,
        save: impl FnOnce(&Path, &Store) -> Result<(), SaveError>,
    ) -> io::Result<Outcome<R>> {
        let original = self.store.clone();
        let value = f(&mut self.store);
        if self.store == original {
            // Nothing to save and nothing to tell anyone; in particular a
            // rejected or no-op request does not make the undo record stale.
            return Ok(Outcome::saved(value));
        }
        let warning = match save(&self.path, &self.store) {
            Ok(()) => None,
            Err(error) if error.is_committed() => Some(DurabilityWarning {
                path: self.path.clone(),
                error: error.to_string(),
            }),
            Err(error) => {
                self.store = original;
                return Err(io::Error::new(
                    error.kind(),
                    format!("could not save {}: {error}", self.path.display()),
                ));
            }
        };
        self.revision += 1;
        Ok(Outcome { value, warning })
    }

    /// Complete the current task and remember it for `undo_complete`.
    pub fn complete_current(&mut self) -> io::Result<Outcome<Option<Task>>> {
        let outcome = self.update(|s| s.complete_current())?;
        Ok(outcome.map(|completed| self.remember(completed)))
    }

    /// Mark a task done and remember it for `undo_complete`: the current task
    /// is completed (pulling from Next), any other task is simply deleted.
    pub fn complete(&mut self, id: u64) -> io::Result<Outcome<bool>> {
        let outcome = self.update(|s| s.complete(id))?;
        Ok(outcome.map(|completed| self.remember(completed).is_some()))
    }

    /// Keep a completion for `undo_complete`; the task it removed is returned.
    fn remember(&mut self, completed: Option<Completed>) -> Option<Task> {
        let completed = completed?;
        let task = completed.task.clone();
        self.undo = Some(LastCompletion {
            completed,
            revision: self.revision,
        });
        Some(task)
    }

    /// Reverse the completion of task `id`. `false` when that is not the last
    /// completion, it was already undone, or anything else changed since;
    /// nothing is written in that case.
    pub fn undo_complete(&mut self, id: u64) -> io::Result<Outcome<bool>> {
        self.undo_complete_with(id, crate::save)
    }

    fn undo_complete_with(
        &mut self,
        id: u64,
        save: impl FnOnce(&Path, &Store) -> Result<(), SaveError>,
    ) -> io::Result<Outcome<bool>> {
        let completed = match &self.undo {
            Some(last) if last.revision == self.revision && last.completed.task.id == id => {
                last.completed.clone()
            }
            _ => return Ok(Outcome::saved(false)),
        };
        let outcome = self.update_with(|s| s.undo_complete(completed), save)?;
        // Restored, so the record has served its purpose. A save that failed
        // above rolled the store back and left the record for another try.
        self.undo = None;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bucket;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("qf-tasks-{name}-{}-{nonce}", std::process::id()))
    }

    /// Tasks backed by a real, writable task file in a fresh directory.
    fn writable(name: &str) -> (PathBuf, Tasks) {
        let dir = temp_dir(name);
        fs::create_dir_all(&dir).unwrap();
        let tasks = Tasks::new(Store::new(), dir.join("tasks.json"));
        (dir, tasks)
    }

    fn ids(tasks: &Tasks, bucket: Bucket) -> Vec<u64> {
        tasks.store().in_bucket(bucket).map(|t| t.id).collect()
    }

    fn add(tasks: &mut Tasks, title: &str, bucket: Bucket) -> u64 {
        tasks.update(|s| s.add(title, bucket, None)).unwrap().value
    }

    #[test]
    fn malformed_file_is_not_replaced_with_an_empty_store() {
        let dir = temp_dir("malformed");
        let path = dir.join("tasks.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, b"{ definitely not valid json").unwrap();

        let error = Tasks::load(path.clone()).unwrap_err();
        assert!(error.to_string().contains("could not read"), "{error}");
        assert!(error.to_string().contains("tasks.json"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), b"{ definitely not valid json");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_save_rolls_back_and_does_not_count() {
        let dir = temp_dir("save-failure");
        let path = dir.join("tasks.json");
        // A directory at the destination makes the final atomic rename fail.
        fs::create_dir_all(&path).unwrap();
        let mut tasks = Tasks::new(Store::new(), path);

        let result = tasks.update(|store| store.add("lost", Bucket::Next, None));

        let error = result.unwrap_err();
        assert!(error.to_string().contains("could not save"), "{error}");
        assert!(tasks.store().is_empty());
        assert_eq!(tasks.store().next_id, 1);
        assert_eq!(tasks.revision(), 0);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn post_commit_failure_returns_a_warning_without_rollback() {
        let dir = temp_dir("post-commit-failure");
        let path = dir.join("tasks.json");
        let mut tasks = Tasks::new(Store::new(), path.clone());

        let outcome = tasks
            .update_with(
                |store| store.add("committed", Bucket::Next, None),
                |path, store| {
                    crate::save(path, store)?;
                    Err(SaveError::AfterCommit(io::Error::other(
                        "injected directory sync failure",
                    )))
                },
            )
            .unwrap();

        assert_eq!(outcome.value, 1);
        let warning = outcome.warning.unwrap().to_string();
        assert!(
            warning.contains("injected directory sync failure"),
            "{warning}"
        );
        assert!(warning.contains("tasks.json"), "{warning}");
        assert_eq!(tasks.store().tasks[0].title, "committed");
        assert_eq!(crate::load(&path).unwrap().tasks[0].title, "committed");
        assert_eq!(tasks.revision(), 1, "a committed change counts");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn completing_the_current_task_pulls_next_and_deleting_others_does_not() {
        let (dir, mut tasks) = writable("complete");
        let now = add(&mut tasks, "now", Bucket::Now);
        let next = add(&mut tasks, "next", Bucket::Next);
        let later = add(&mut tasks, "later", Bucket::Later);

        assert!(tasks.complete(later).unwrap().value);
        assert_eq!(tasks.store().current().map(|t| t.id), Some(now));
        assert!(tasks.complete(now).unwrap().value);
        assert_eq!(tasks.store().current().map(|t| t.id), Some(next));
        assert!(!tasks.complete(now).unwrap().value);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn undo_reverses_the_last_completion_once() {
        let (dir, mut tasks) = writable("undo");
        let a = add(&mut tasks, "a", Bucket::Now);
        let b = add(&mut tasks, "b", Bucket::Next);

        let done = tasks.complete_current().unwrap().value.unwrap();
        assert_eq!(done.id, a);
        assert_eq!(ids(&tasks, Bucket::Now), vec![b]);

        assert!(tasks.undo_complete(a).unwrap().value);
        assert_eq!(ids(&tasks, Bucket::Now), vec![a]);
        assert_eq!(ids(&tasks, Bucket::Next), vec![b]);
        assert_eq!(
            crate::load(&dir.join("tasks.json"))
                .unwrap()
                .current()
                .map(|t| t.id),
            Some(a),
            "undo is persisted like any other change"
        );
        assert!(!tasks.undo_complete(a).unwrap().value);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn undo_puts_a_deleted_side_task_back_in_place() {
        let (dir, mut tasks) = writable("undo-side");
        let now = add(&mut tasks, "now", Bucket::Now);
        let first = add(&mut tasks, "first", Bucket::Side);
        let second = add(&mut tasks, "second", Bucket::Side);

        assert!(tasks.complete(first).unwrap().value);
        assert_eq!(ids(&tasks, Bucket::Side), vec![second]);
        assert_eq!(tasks.store().current().map(|t| t.id), Some(now));

        assert!(tasks.undo_complete(first).unwrap().value);
        assert_eq!(ids(&tasks, Bucket::Side), vec![first, second]);
        assert_eq!(tasks.store().current().map(|t| t.id), Some(now));
        assert!(!tasks.undo_complete(first).unwrap().value);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn undo_is_refused_after_any_other_change() {
        let (dir, mut tasks) = writable("undo-stale");
        let a = add(&mut tasks, "a", Bucket::Now);
        tasks.complete_current().unwrap();
        add(&mut tasks, "meanwhile", Bucket::Later);

        assert!(!tasks.undo_complete(a).unwrap().value);
        assert!(tasks.store().current().is_none());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn undo_is_bound_to_the_task_that_was_completed_last() {
        let (dir, mut tasks) = writable("undo-bound");
        let a = add(&mut tasks, "a", Bucket::Now);
        let b = add(&mut tasks, "b", Bucket::Next);
        tasks.complete_current().unwrap();
        tasks.complete_current().unwrap();

        assert!(!tasks.undo_complete(a).unwrap().value, "a's offer is stale");
        assert!(tasks.undo_complete(b).unwrap().value);
        assert_eq!(ids(&tasks, Bucket::Now), vec![b]);
        assert!(tasks.store().get(a).is_none());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn requests_that_change_nothing_neither_save_nor_invalidate_undo() {
        let (dir, mut tasks) = writable("undo-noop");
        let a = add(&mut tasks, "a", Bucket::Now);
        tasks.complete_current().unwrap();
        let revision = tasks.revision();

        assert!(!tasks.update(|s| s.remove(999)).unwrap().value);
        assert!(!tasks.update(|s| s.shift(a, -1)).unwrap().value);
        assert!(tasks.complete_current().unwrap().value.is_none());
        assert_eq!(tasks.revision(), revision, "no-ops do not count");

        assert!(tasks.undo_complete(a).unwrap().value);
        assert_eq!(ids(&tasks, Bucket::Now), vec![a]);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_undo_save_keeps_the_record_for_another_try() {
        let (dir, mut tasks) = writable("undo-save-failure");
        let a = add(&mut tasks, "a", Bucket::Now);
        tasks.complete_current().unwrap();

        let failed = tasks.undo_complete_with(a, |_, _| {
            Err(SaveError::BeforeCommit(io::Error::other(
                "injected write failure",
            )))
        });
        assert!(failed.is_err());
        assert!(tasks.store().is_empty(), "rolled back");

        assert!(tasks.undo_complete(a).unwrap().value);
        assert_eq!(ids(&tasks, Bucket::Now), vec![a]);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn nothing_to_complete_leaves_nothing_to_undo() {
        let (dir, mut tasks) = writable("undo-empty");
        assert!(tasks.complete_current().unwrap().value.is_none());
        assert!(!tasks.undo_complete(1).unwrap().value);
        fs::remove_dir_all(dir).unwrap();
    }
}
