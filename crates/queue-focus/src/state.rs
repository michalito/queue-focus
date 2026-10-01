//! The task queue, shared by the windows and the D-Bus service: every saved
//! change notifies the listeners. Saving, rolling back and undo are the core's.

use qf_core::{Outcome, Store, Task, Tasks};
use std::cell::{Ref, RefCell};
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

pub type SharedState = Rc<State>;

pub struct State {
    tasks: RefCell<Tasks>,
    listeners: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl State {
    pub fn load() -> io::Result<SharedState> {
        Self::load_from(qf_core::data_path())
    }

    pub(crate) fn load_from(path: PathBuf) -> io::Result<SharedState> {
        Ok(Rc::new(State {
            tasks: RefCell::new(Tasks::load(path)?),
            listeners: RefCell::new(Vec::new()),
        }))
    }

    pub fn store(&self) -> Ref<'_, Store> {
        Ref::map(self.tasks.borrow(), Tasks::store)
    }

    /// Apply a change and save it; see `Tasks::update`.
    pub fn update<R>(&self, f: impl FnOnce(&mut Store) -> R) -> io::Result<Outcome<R>> {
        self.change(|tasks| tasks.update(f))
    }

    pub fn complete_current(&self) -> io::Result<Outcome<Option<Task>>> {
        self.change(Tasks::complete_current)
    }

    pub fn complete(&self, id: u64) -> io::Result<Outcome<bool>> {
        self.change(|tasks| tasks.complete(id))
    }

    pub fn undo_complete(&self, id: u64) -> io::Result<Outcome<bool>> {
        self.change(|tasks| tasks.undo_complete(id))
    }

    /// Run a change, then tell the listeners if one was saved. The borrow
    /// ends first, so a listener can read the store.
    fn change<R>(&self, f: impl FnOnce(&mut Tasks) -> R) -> R {
        let (result, saved) = {
            let mut tasks = self.tasks.borrow_mut();
            let before = tasks.revision();
            let result = f(&mut tasks);
            (result, tasks.revision() != before)
        };
        if saved {
            self.notify();
        }
        result
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

    fn counted(state: &State) -> Rc<Cell<u32>> {
        let count = Rc::new(Cell::new(0));
        let seen = count.clone();
        state.on_change(move || seen.set(seen.get() + 1));
        count
    }

    /// Listeners hear about saved changes, and only those: not a failed save,
    /// not a request that changed nothing.
    #[test]
    fn listeners_hear_about_each_saved_change_and_nothing_else() {
        let dir = temp_dir("listeners");
        fs::create_dir_all(&dir).unwrap();
        let state = State::load_from(dir.join("tasks.json")).unwrap();
        let notifications = counted(&state);

        let a = state
            .update(|s| s.add("a", Bucket::Now, None))
            .unwrap()
            .value;
        assert_eq!(notifications.get(), 1);
        assert!(state.complete_current().unwrap().value.is_some());
        assert_eq!(notifications.get(), 2);
        assert!(state.undo_complete(a).unwrap().value);
        assert_eq!(notifications.get(), 3);

        assert!(!state.update(|s| s.remove(999)).unwrap().value);
        assert!(!state.complete(999).unwrap().value);
        assert!(!state.undo_complete(a).unwrap().value);
        assert_eq!(notifications.get(), 3, "no-ops are not broadcast");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_save_notifies_nobody() {
        let dir = temp_dir("save-failure");
        let path = dir.join("tasks.json");
        let state = State::load_from(path.clone()).unwrap();
        // A directory at the destination makes the final atomic rename fail.
        fs::create_dir_all(&path).unwrap();
        let notifications = counted(&state);

        assert!(state.update(|s| s.add("lost", Bucket::Next, None)).is_err());
        assert!(state.store().is_empty());
        assert_eq!(notifications.get(), 0);

        fs::remove_dir_all(dir).unwrap();
    }

    /// A listener reads the store it was told about, which it can only do
    /// once the change has let go of it.
    #[test]
    fn a_listener_can_read_the_store() {
        let dir = temp_dir("reentrant");
        fs::create_dir_all(&dir).unwrap();
        let state = State::load_from(dir.join("tasks.json")).unwrap();
        let seen = Rc::new(Cell::new(0));
        let (weak, count) = (Rc::downgrade(&state), seen.clone());
        state.on_change(move || count.set(weak.upgrade().unwrap().store().len()));

        state.update(|s| s.add("a", Bucket::Next, None)).unwrap();
        assert_eq!(seen.get(), 1);

        fs::remove_dir_all(dir).unwrap();
    }
}
