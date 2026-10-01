//! The settings the user changes on the Settings page, held once for the whole
//! process and shared with the D-Bus service. Writing them is the core's; this
//! adds the listeners and hangs the once-a-second writer off the main loop.

use gtk::glib;
use qf_core::Settings;
use std::cell::{Ref, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

pub type SharedSettings = Rc<SettingsStore>;

/// Told that something failed to reach the disk.
type Problem = Rc<dyn Fn(&str)>;

pub struct SettingsStore {
    store: RefCell<qf_core::SettingsStore>,
    listeners: RefCell<Vec<Rc<dyn Fn()>>>,
    /// Told about a failure to write the file, once per failure.
    problems: RefCell<Vec<Problem>>,
}

/// Write changed settings once a second, for as long as the store lives.
/// Called once, when the application starts.
pub fn persist_on_main_loop(settings: &SharedSettings) {
    let weak = Rc::downgrade(settings);
    glib::timeout_add_seconds_local(1, move || match weak.upgrade() {
        Some(settings) => {
            settings.tick();
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
}

impl SettingsStore {
    /// Load the settings; see `qf_core::SettingsStore::load` for the warning.
    pub fn load() -> (SharedSettings, Option<String>) {
        Self::load_from(qf_core::settings_path())
    }

    pub(crate) fn load_from(path: PathBuf) -> (SharedSettings, Option<String>) {
        let (store, warning) = qf_core::SettingsStore::load(path);
        let store = Rc::new(SettingsStore {
            store: RefCell::new(store),
            listeners: RefCell::new(Vec::new()),
            problems: RefCell::new(Vec::new()),
        });
        (store, warning)
    }

    pub fn get(&self) -> Ref<'_, Settings> {
        Ref::map(self.store.borrow(), qf_core::SettingsStore::get)
    }

    /// Change the settings. Nobody is notified when the change leaves the
    /// value it had.
    pub fn update(&self, f: impl FnOnce(&mut Settings)) {
        let changed = self.store.borrow_mut().update(f);
        if changed {
            self.notify();
        }
    }

    /// Apply a JSON object of changed keys (the D-Bus surface). An unknown key
    /// or an unusable value leaves every setting as it was.
    pub fn apply_patch(&self, patch: &str) -> Result<(), String> {
        let changed = self.store.borrow_mut().apply_patch(patch)?;
        if changed {
            self.notify();
        }
        Ok(())
    }

    fn tick(&self) {
        let problem = self.store.borrow_mut().tick();
        self.report(problem);
    }

    /// Write if the file is behind, whatever the backoff says. Called once
    /// more before the process exits.
    pub fn flush(&self) {
        let problem = self.store.borrow_mut().flush();
        self.report(problem);
    }

    fn notify(&self) {
        // Clone the handles out of the borrow so a listener may register one.
        let listeners: Vec<Rc<dyn Fn()>> = self.listeners.borrow().clone();
        for f in listeners {
            f();
        }
    }

    fn report(&self, problem: Option<String>) {
        let Some(message) = problem else {
            return;
        };
        eprintln!("queue-focus: {message}");
        let problems: Vec<Problem> = self.problems.borrow().clone();
        for f in problems {
            f(&message);
        }
    }

    pub fn on_change(&self, f: impl Fn() + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn on_problem(&self, f: impl Fn(&str) + 'static) {
        self.problems.borrow_mut().push(Rc::new(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qf_core::Theme;
    use std::cell::Cell;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("qf-settings-{name}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn listeners_hear_about_real_changes_only() {
        let dir = temp_dir("listeners");
        let (settings, _) = SettingsStore::load_from(dir.join("settings.json"));
        let seen = Rc::new(Cell::new(0));
        let count = seen.clone();
        settings.on_change(move || count.set(count.get() + 1));

        settings.update(|s| s.theme = Theme::Dark);
        assert_eq!(seen.get(), 1);
        settings.update(|s| s.theme = Theme::Dark);
        assert_eq!(seen.get(), 1);
        settings.apply_patch(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(seen.get(), 1);
        assert!(settings.apply_patch(r#"{"theme":"puce"}"#).is_err());
        assert_eq!(seen.get(), 1);
        settings.apply_patch(r#"{"theme":"light"}"#).unwrap();
        assert_eq!(seen.get(), 2);
    }

    #[test]
    fn a_failed_write_is_reported_to_every_problem_listener_once() {
        let dir = temp_dir("unwritable");
        // A directory where the file belongs makes every rename fail.
        let path = dir.join("settings.json");
        fs::create_dir_all(&path).unwrap();
        let (settings, _) = SettingsStore::load_from(path);
        let problems = Rc::new(RefCell::new(Vec::new()));
        let seen = problems.clone();
        settings.on_problem(move |m| seen.borrow_mut().push(m.to_string()));

        settings.update(|s| s.vary = false);
        settings.tick();
        settings.flush();
        settings.tick();
        assert_eq!(problems.borrow().len(), 1, "one outage, one complaint");
        assert!(problems.borrow()[0].contains("could not save"));

        fs::remove_dir_all(dir).unwrap();
    }
}
