//! Shared, observable task store: every mutation persists and notifies listeners.

use qf_core::Store;
use std::cell::{Ref, RefCell};
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

pub type SharedState = Rc<State>;
type DurabilityWarningListener = Rc<dyn Fn(&DurabilityWarning)>;

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

/// The successful outcome of a persisted mutation.
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
}

pub struct State {
    store: RefCell<Store>,
    path: PathBuf,
    listeners: RefCell<Vec<Rc<dyn Fn()>>>,
    durability_warning_listeners: RefCell<Vec<DurabilityWarningListener>>,
}

impl State {
    pub fn load() -> io::Result<SharedState> {
        Self::load_from(qf_core::data_path())
    }

    fn load_from(path: PathBuf) -> io::Result<SharedState> {
        let store = qf_core::load(&path).map_err(|e| {
            io::Error::new(e.kind(), format!("could not read {}: {e}", path.display()))
        })?;
        Ok(Rc::new(State {
            store: RefCell::new(store),
            path,
            listeners: RefCell::new(Vec::new()),
            durability_warning_listeners: RefCell::new(Vec::new()),
        }))
    }

    pub fn store(&self) -> Ref<'_, Store> {
        self.store.borrow()
    }

    /// Apply a mutation, persist, then notify. Failures before the atomic
    /// rename are rolled back. A failure syncing the directory after rename
    /// is returned and emitted as a durability warning, but the committed
    /// mutation remains successful so callers do not retry a non-idempotent
    /// operation.
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

        self.notify();
        if let Some(error) = durability_warning {
            let warning = DurabilityWarning {
                path: self.path.clone(),
                error: error.to_string(),
            };
            self.notify_durability_warning(&warning);
            Ok(UpdateOutcome::CommittedWithWarning {
                value: result,
                warning,
            })
        } else {
            Ok(UpdateOutcome::Saved(result))
        }
    }

    pub fn on_change(&self, f: impl Fn() + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn on_durability_warning(&self, f: impl Fn(&DurabilityWarning) + 'static) {
        self.durability_warning_listeners
            .borrow_mut()
            .push(Rc::new(f));
    }

    fn notify(&self) {
        // Clone handles out of the borrow so listeners may register new listeners.
        let listeners: Vec<Rc<dyn Fn()>> = self.listeners.borrow().clone();
        for f in listeners {
            f();
        }
    }

    fn notify_durability_warning(&self, warning: &DurabilityWarning) {
        let listeners: Vec<DurabilityWarningListener> =
            self.durability_warning_listeners.borrow().clone();
        for f in listeners {
            f(warning);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qf_core::Bucket;
    use std::cell::Cell;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("qf-state-{name}-{}-{nonce}", std::process::id()))
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
        let state = Rc::new(State {
            store: RefCell::new(Store::new()),
            path,
            listeners: RefCell::new(Vec::new()),
            durability_warning_listeners: RefCell::new(Vec::new()),
        });
        let notifications = Rc::new(Cell::new(0));
        let seen = notifications.clone();
        state.on_change(move || seen.set(seen.get() + 1));

        let result = state.update(|store| store.add("lost", Bucket::Next, None, false));

        assert!(result.is_err());
        assert!(state.store().is_empty());
        assert_eq!(state.store().next_id, 1);
        assert_eq!(notifications.get(), 0);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn post_commit_failure_returns_and_emits_a_warning_without_rollback() {
        let dir = temp_dir("post-commit-failure");
        let path = dir.join("tasks.json");
        let state = Rc::new(State {
            store: RefCell::new(Store::new()),
            path: path.clone(),
            listeners: RefCell::new(Vec::new()),
            durability_warning_listeners: RefCell::new(Vec::new()),
        });
        let notifications = Rc::new(Cell::new(0));
        let seen = notifications.clone();
        state.on_change(move || seen.set(seen.get() + 1));
        let warnings = Rc::new(RefCell::new(Vec::new()));
        let seen = warnings.clone();
        state.on_durability_warning(move |warning| seen.borrow_mut().push(warning.to_string()));

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
        assert_eq!(warnings.borrow().len(), 1);
        assert!(warnings.borrow()[0].contains("injected directory sync failure"));

        fs::remove_dir_all(dir).unwrap();
    }
}
