//! Shared, observable task store: every mutation persists and notifies listeners.

use qf_core::{Completed, Store, Task};
use std::cell::{Cell, Ref, RefCell};
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

pub type SharedState = Rc<State>;

/// A mutation reached the task file, but syncing its containing directory
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

/// The successful outcome of a persisted mutation. Whoever asked for the
/// mutation reports the warning through its own channel (dialog, stderr, or
/// D-Bus signal), so the user hears about it exactly once.
#[derive(Debug)]
pub enum UpdateOutcome<R> {
    Saved(R),
    CommittedWithWarning {
        value: R,
        warning: DurabilityWarning,
    },
}

impl<R> UpdateOutcome<R> {
    pub fn warning(&self) -> Option<&DurabilityWarning> {
        match self {
            Self::Saved(_) => None,
            Self::CommittedWithWarning { warning, .. } => Some(warning),
        }
    }

    pub fn into_value(self) -> R {
        match self {
            Self::Saved(value) | Self::CommittedWithWarning { value, .. } => value,
        }
    }

    pub fn into_parts(self) -> (R, Option<DurabilityWarning>) {
        match self {
            Self::Saved(value) => (value, None),
            Self::CommittedWithWarning { value, warning } => (value, Some(warning)),
        }
    }

    pub fn map<T>(self, f: impl FnOnce(R) -> T) -> UpdateOutcome<T> {
        match self {
            Self::Saved(value) => UpdateOutcome::Saved(f(value)),
            Self::CommittedWithWarning { value, warning } => UpdateOutcome::CommittedWithWarning {
                value: f(value),
                warning,
            },
        }
    }
}

/// The most recent completion, reversible only while nothing else has changed.
struct LastCompletion {
    completed: Completed,
    revision: u64,
}

pub struct State {
    store: RefCell<Store>,
    path: PathBuf,
    /// Counts committed mutations; lets the undo entry tell whether it is stale.
    revision: Cell<u64>,
    undo: RefCell<Option<LastCompletion>>,
    listeners: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl State {
    pub fn load() -> io::Result<SharedState> {
        Self::load_from(qf_core::data_path())
    }

    fn load_from(path: PathBuf) -> io::Result<SharedState> {
        let store = qf_core::load(&path).map_err(|e| {
            io::Error::new(e.kind(), format!("could not read {}: {e}", path.display()))
        })?;
        Ok(Self::with_store(store, path))
    }

    fn with_store(store: Store, path: PathBuf) -> SharedState {
        Rc::new(State {
            store: RefCell::new(store),
            path,
            revision: Cell::new(0),
            undo: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
        })
    }

    pub fn store(&self) -> Ref<'_, Store> {
        self.store.borrow()
    }

    /// Apply a mutation, persist, then notify. Failures before the atomic
    /// rename are rolled back. A failure syncing the directory after rename
    /// is returned as a durability warning, but the committed mutation
    /// remains successful so callers do not retry a non-idempotent operation.
    pub fn update<R>(&self, f: impl FnOnce(&mut Store) -> R) -> io::Result<UpdateOutcome<R>> {
        self.update_with(f, qf_core::save)
    }

    fn update_with<R>(
        &self,
        f: impl FnOnce(&mut Store) -> R,
        save: impl FnOnce(&std::path::Path, &Store) -> Result<(), qf_core::SaveError>,
    ) -> io::Result<UpdateOutcome<R>> {
        let (result, durability_warning) = {
            let mut store = self.store.borrow_mut();
            let original = store.clone();
            let r = f(&mut store);
            if *store == original {
                // Nothing to persist and nothing to tell anyone; in particular
                // a rejected or no-op request does not invalidate undo.
                return Ok(UpdateOutcome::Saved(r));
            }
            match save(&self.path, &store) {
                Ok(()) => (r, None),
                Err(error) if error.is_committed() => (r, Some(error)),
                Err(error) => {
                    *store = original;
                    return Err(io::Error::new(
                        error.kind(),
                        format!("could not save {}: {error}", self.path.display()),
                    ));
                }
            }
        };
        self.revision.set(self.revision.get() + 1);

        self.notify();
        Ok(match durability_warning {
            Some(error) => UpdateOutcome::CommittedWithWarning {
                value: result,
                warning: DurabilityWarning {
                    path: self.path.clone(),
                    error: error.to_string(),
                },
            },
            None => UpdateOutcome::Saved(result),
        })
    }

    /// Complete the current task and remember it for `undo_complete`.
    pub fn complete_current(&self) -> io::Result<UpdateOutcome<Option<Task>>> {
        let outcome = self.update(|s| s.complete_current())?;
        Ok(outcome.map(|completed| {
            if let Some(completed) = &completed {
                *self.undo.borrow_mut() = Some(LastCompletion {
                    completed: completed.clone(),
                    revision: self.revision.get(),
                });
            }
            completed.map(|c| c.task)
        }))
    }

    /// Mark a task done: the current task is completed (pulling from Next,
    /// reversible), any other task is simply deleted.
    pub fn complete(&self, id: u64) -> io::Result<UpdateOutcome<bool>> {
        let is_current = self.store().current().is_some_and(|t| t.id == id);
        if is_current {
            self.complete_current().map(|o| o.map(|t| t.is_some()))
        } else {
            self.update(|s| s.remove(id))
        }
    }

    /// Reverse the completion of task `id`. `false` when that is not the last
    /// completion, it was already undone, or anything else changed since;
    /// nothing is written in that case.
    pub fn undo_complete(&self, id: u64) -> io::Result<UpdateOutcome<bool>> {
        self.undo_complete_with(id, qf_core::save)
    }

    fn undo_complete_with(
        &self,
        id: u64,
        save: impl FnOnce(&std::path::Path, &Store) -> Result<(), qf_core::SaveError>,
    ) -> io::Result<UpdateOutcome<bool>> {
        let completed = match self.undo.borrow().as_ref() {
            Some(last) if last.revision == self.revision.get() && last.completed.task.id == id => {
                last.completed.clone()
            }
            _ => return Ok(UpdateOutcome::Saved(false)),
        };
        let outcome = self.update_with(|s| s.undo_complete(completed), save)?;
        // Restored, so the record has served its purpose. A save that failed
        // above rolled the store back and left the record for another try.
        *self.undo.borrow_mut() = None;
        Ok(outcome)
    }

    pub fn on_change(&self, f: impl Fn() + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    fn notify(&self) {
        // Clone handles out of the borrow so listeners may register new listeners.
        let listeners: Vec<Rc<dyn Fn()>> = self.listeners.borrow().clone();
        for f in listeners {
            f();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qf_core::Bucket;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("qf-state-{name}-{}-{nonce}", std::process::id()))
    }

    /// A state backed by a real, writable task file in a fresh directory.
    fn writable_state(name: &str) -> (PathBuf, SharedState) {
        let dir = temp_dir(name);
        fs::create_dir_all(&dir).unwrap();
        let state = State::with_store(Store::new(), dir.join("tasks.json"));
        (dir, state)
    }

    fn ids(state: &State, bucket: Bucket) -> Vec<u64> {
        state.store().in_bucket(bucket).map(|t| t.id).collect()
    }

    #[test]
    fn malformed_file_is_not_replaced_with_an_empty_store() {
        let dir = temp_dir("malformed");
        let path = dir.join("tasks.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, b"{ definitely not valid json").unwrap();

        assert!(State::load_from(path.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{ definitely not valid json");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_save_rolls_back_and_does_not_notify() {
        let dir = temp_dir("save-failure");
        let path = dir.join("tasks.json");
        // A directory at the destination makes the final atomic rename fail.
        fs::create_dir_all(&path).unwrap();
        let state = State::with_store(Store::new(), path);
        let notifications = Rc::new(Cell::new(0));
        let seen = notifications.clone();
        state.on_change(move || seen.set(seen.get() + 1));

        let result = state.update(|store| store.add("lost", Bucket::Next, None, false));

        assert!(result.is_err());
        assert!(state.store().is_empty());
        assert_eq!(state.store().next_id, 1);
        assert_eq!(notifications.get(), 0);
        assert_eq!(state.revision.get(), 0);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn post_commit_failure_returns_a_warning_without_rollback() {
        let dir = temp_dir("post-commit-failure");
        let path = dir.join("tasks.json");
        let state = State::with_store(Store::new(), path.clone());
        let notifications = Rc::new(Cell::new(0));
        let seen = notifications.clone();
        state.on_change(move || seen.set(seen.get() + 1));

        let result = state.update_with(
            |store| store.add("committed", Bucket::Next, None, false),
            |path, store| {
                qf_core::save(path, store)?;
                Err(qf_core::SaveError::AfterCommit(io::Error::other(
                    "injected directory sync failure",
                )))
            },
        );

        let outcome = result.unwrap();
        assert!(matches!(
            &outcome,
            UpdateOutcome::CommittedWithWarning { value: 1, .. }
        ));
        assert!(outcome
            .warning()
            .unwrap()
            .to_string()
            .contains("injected directory sync failure"));
        assert_eq!(state.store().tasks[0].title, "committed");
        assert_eq!(qf_core::load(&path).unwrap().tasks[0].title, "committed");
        assert_eq!(notifications.get(), 1);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn completing_the_current_task_pulls_next_and_deleting_others_does_not() {
        let (dir, state) = writable_state("complete");
        let now = state
            .update(|s| s.add("now", Bucket::Now, None, false))
            .unwrap()
            .into_value();
        let next = state
            .update(|s| s.add("next", Bucket::Next, None, false))
            .unwrap()
            .into_value();
        let later = state
            .update(|s| s.add("later", Bucket::Later, None, false))
            .unwrap()
            .into_value();

        assert!(state.complete(later).unwrap().into_value());
        assert_eq!(state.store().current().map(|t| t.id), Some(now));
        assert!(state.complete(now).unwrap().into_value());
        assert_eq!(state.store().current().map(|t| t.id), Some(next));
        assert!(!state.complete(now).unwrap().into_value());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn undo_reverses_the_last_completion_once() {
        let (dir, state) = writable_state("undo");
        let a = state
            .update(|s| s.add("a", Bucket::Now, None, false))
            .unwrap()
            .into_value();
        let b = state
            .update(|s| s.add("b", Bucket::Next, None, false))
            .unwrap()
            .into_value();

        let done = state.complete_current().unwrap().into_value().unwrap();
        assert_eq!(done.id, a);
        assert_eq!(ids(&state, Bucket::Now), vec![b]);

        assert!(state.undo_complete(a).unwrap().into_value());
        assert_eq!(ids(&state, Bucket::Now), vec![a]);
        assert_eq!(ids(&state, Bucket::Next), vec![b]);
        assert_eq!(
            qf_core::load(&dir.join("tasks.json"))
                .unwrap()
                .current()
                .map(|t| t.id),
            Some(a),
            "undo is persisted like any other change"
        );
        assert!(!state.undo_complete(a).unwrap().into_value());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn undo_is_refused_after_any_other_change() {
        let (dir, state) = writable_state("undo-stale");
        let a = state
            .update(|s| s.add("a", Bucket::Now, None, false))
            .unwrap()
            .into_value();
        state.complete_current().unwrap();
        state
            .update(|s| s.add("meanwhile", Bucket::Later, None, false))
            .unwrap();

        assert!(!state.undo_complete(a).unwrap().into_value());
        assert!(state.store().current().is_none());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn undo_is_bound_to_the_task_that_was_completed_last() {
        let (dir, state) = writable_state("undo-bound");
        let a = state
            .update(|s| s.add("a", Bucket::Now, None, false))
            .unwrap()
            .into_value();
        let b = state
            .update(|s| s.add("b", Bucket::Next, None, false))
            .unwrap()
            .into_value();
        state.complete_current().unwrap();
        state.complete_current().unwrap();

        assert!(
            !state.undo_complete(a).unwrap().into_value(),
            "a's offer is stale"
        );
        assert!(state.undo_complete(b).unwrap().into_value());
        assert_eq!(ids(&state, Bucket::Now), vec![b]);
        assert!(state.store().get(a).is_none());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn requests_that_change_nothing_neither_save_nor_invalidate_undo() {
        let (dir, state) = writable_state("undo-noop");
        let a = state
            .update(|s| s.add("a", Bucket::Now, None, false))
            .unwrap()
            .into_value();
        let notifications = Rc::new(Cell::new(0));
        let seen = notifications.clone();
        state.on_change(move || seen.set(seen.get() + 1));
        state.complete_current().unwrap();
        assert_eq!(notifications.get(), 1);

        assert!(!state.update(|s| s.remove(999)).unwrap().into_value());
        assert!(!state.update(|s| s.shift(a, -1)).unwrap().into_value());
        assert!(state.complete_current().unwrap().into_value().is_none());
        assert_eq!(notifications.get(), 1, "no-ops are not broadcast");

        assert!(state.undo_complete(a).unwrap().into_value());
        assert_eq!(ids(&state, Bucket::Now), vec![a]);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_undo_save_keeps_the_record_for_another_try() {
        let (dir, state) = writable_state("undo-save-failure");
        let a = state
            .update(|s| s.add("a", Bucket::Now, None, false))
            .unwrap()
            .into_value();
        state.complete_current().unwrap();

        let failed = state.undo_complete_with(a, |_, _| {
            Err(qf_core::SaveError::BeforeCommit(io::Error::other(
                "injected write failure",
            )))
        });
        assert!(failed.is_err());
        assert!(state.store().is_empty(), "rolled back");

        assert!(state.undo_complete(a).unwrap().into_value());
        assert_eq!(ids(&state, Bucket::Now), vec![a]);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn nothing_to_complete_leaves_nothing_to_undo() {
        let (dir, state) = writable_state("undo-empty");
        assert!(state.complete_current().unwrap().into_value().is_none());
        assert!(!state.undo_complete(1).unwrap().into_value());
        fs::remove_dir_all(dir).unwrap();
    }
}
