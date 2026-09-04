//! The settings the user changes on the Settings page, held once for the whole
//! process and written to `settings.json`.
//!
//! The value in memory is authoritative the moment it changes: listeners hear
//! about it at once. The file catches up a second later, because dragging the
//! interval slider moves the value on every pixel and each save is a write, an
//! fsync and a rename.
//!
//! The store itself never touches the main loop — `persist_on_main_loop` hangs
//! the once-a-second flush off it — so everything here is an ordinary object
//! with ordinary tests.

use gtk::glib;
use qf_core::{Settings, TimeOfDay};
use std::cell::{Cell, Ref, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

pub type SharedSettings = Rc<SettingsStore>;

/// Seconds to wait before trying a failed write again.
const RETRY_TICKS: u32 = 30;

/// Told that something failed to reach the disk.
type Problem = Rc<dyn Fn(&str)>;

pub struct SettingsStore {
    settings: RefCell<Settings>,
    path: PathBuf,
    /// Set while the file is behind the value in memory.
    dirty: Cell<bool>,
    /// Set once a failure has been reported, so one outage is one complaint.
    reported: Cell<bool>,
    /// Ticks to sit out before trying a failed write again.
    backoff: Cell<u32>,
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
    /// Load the settings, falling back to the defaults. A file that cannot be
    /// read comes back as a warning rather than an error: losing the queue is
    /// worth refusing to start over, losing a switch is not — but the user is
    /// still told, and the broken file is left alone until something changes.
    pub fn load() -> (SharedSettings, Option<String>) {
        Self::load_from(qf_core::settings_path())
    }

    fn load_from(path: PathBuf) -> (SharedSettings, Option<String>) {
        let (settings, warning) = match qf_core::load_settings(&path) {
            Ok(settings) => (settings, None),
            Err(e) => (
                Settings::default(),
                Some(format!(
                    "could not read {}: {e}; using the default settings until you change one",
                    path.display()
                )),
            ),
        };
        let store = Rc::new(SettingsStore {
            settings: RefCell::new(settings),
            path,
            dirty: Cell::new(false),
            reported: Cell::new(false),
            backoff: Cell::new(0),
            listeners: RefCell::new(Vec::new()),
            problems: RefCell::new(Vec::new()),
        });
        (store, warning)
    }

    pub fn get(&self) -> Ref<'_, Settings> {
        self.settings.borrow()
    }

    /// Change the settings. Nothing is written and nobody is notified when the
    /// change leaves the value it had.
    pub fn update(&self, f: impl FnOnce(&mut Settings)) {
        let changed = {
            let mut settings = self.settings.borrow_mut();
            let before = settings.clone();
            f(&mut settings);
            settings.sanitize();
            *settings != before
        };
        if changed {
            self.commit();
        }
    }

    /// Apply a JSON object of changed keys (the D-Bus surface). An unknown key
    /// or an unusable value leaves every setting as it was.
    pub fn apply_patch(&self, patch: &str) -> Result<(), String> {
        let changed = {
            let mut settings = self.settings.borrow_mut();
            let before = settings.clone();
            settings.apply_patch(patch)?;
            *settings != before
        };
        if changed {
            self.commit();
        }
        Ok(())
    }

    fn commit(&self) {
        self.dirty.set(true);
        // Clone the handles out of the borrow so a listener may register one.
        let listeners: Vec<Rc<dyn Fn()>> = self.listeners.borrow().clone();
        for f in listeners {
            f();
        }
    }

    /// One second of the writer. A write that failed is not retried on the
    /// next tick: every attempt creates a temporary file and syncs it before
    /// it can fail, and a file that cannot be written now usually cannot be
    /// written a second later either.
    fn tick(&self) {
        if self.backoff.get() > 0 {
            self.backoff.set(self.backoff.get() - 1);
            return;
        }
        self.flush();
    }

    /// Write if the file is behind. Called on the writer's tick, and once more
    /// before the process exits — which is why it never sits out a turn.
    pub fn flush(&self) {
        if !self.dirty.replace(false) {
            return;
        }
        let settings = self.settings.borrow().clone();
        match qf_core::save_settings(&self.path, &settings) {
            Ok(()) => {
                self.reported.set(false);
                self.backoff.set(0);
            }
            Err(e) => {
                // Put the flag back: the change is still unsaved.
                self.dirty.set(true);
                self.backoff.set(RETRY_TICKS);
                // Complain only once per outage. A dialog a second would be
                // worse than the trouble it reports.
                if !self.reported.replace(true) {
                    self.report(&format!("could not save {}: {e}", self.path.display()));
                }
            }
        }
    }

    fn report(&self, message: &str) {
        eprintln!("queue-focus: {message}");
        let problems: Vec<Problem> = self.problems.borrow().clone();
        for f in problems {
            f(message);
        }
    }

    pub fn on_change(&self, f: impl Fn() + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn on_problem(&self, f: impl Fn(&str) + 'static) {
        self.problems.borrow_mut().push(Rc::new(f));
    }
}

/// The local time of day, for the quiet-hours rule. A clock the system cannot
/// read is treated as midday: the rule holds flashes back, and guessing a time
/// inside the usual working day is the least surprising failure.
pub fn local_time_of_day() -> TimeOfDay {
    match glib::DateTime::now_local() {
        Ok(now) => TimeOfDay::new(now.hour() as u32, now.minute() as u32)
            .unwrap_or_else(|| TimeOfDay::new(12, 0).expect("12:00")),
        Err(_) => TimeOfDay::new(12, 0).expect("12:00"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qf_core::{Intensity, Theme};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("qf-settings-{name}-{}-{nonce}", std::process::id()))
    }

    /// Every attempt creates a temporary file and syncs it before it can fail,
    /// so a file that cannot be written must not be retried once a second for
    /// the life of the session.
    #[test]
    fn a_failed_write_is_not_retried_on_every_tick() {
        let dir = temp_dir("backoff");
        let path = dir.join("settings.json");
        fs::create_dir_all(&path).unwrap();
        let (settings, _) = SettingsStore::load_from(path.clone());
        let attempts = Rc::new(Cell::new(0));
        let seen = attempts.clone();
        settings.on_problem(move |_| seen.set(seen.get() + 1));

        settings.update(|s| s.vary = false);
        settings.tick();
        assert_eq!(attempts.get(), 1, "the first tick tries");
        // The whole backoff passes without another attempt: `dirty` is still
        // set, so any attempt would have to go through save_settings.
        for _ in 0..RETRY_TICKS {
            settings.tick();
        }
        assert!(settings.dirty.get(), "still unsaved");
        assert_eq!(settings.backoff.get(), 0, "and ready to try once more");

        // Shutdown does not sit out a turn, whatever the backoff says.
        settings.backoff.set(RETRY_TICKS);
        fs::remove_dir_all(&path).unwrap();
        settings.flush();
        assert!(!qf_core::load_settings(&path).unwrap().vary);
        assert_eq!(
            settings.backoff.get(),
            0,
            "cleared by the write that worked"
        );

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_change_is_visible_at_once_and_reaches_the_file_on_flush() {
        let dir = temp_dir("flush");
        let path = dir.join("settings.json");
        let (settings, warning) = SettingsStore::load_from(path.clone());
        assert!(warning.is_none());

        settings.update(|s| s.intensity = Intensity::Strong);
        assert_eq!(settings.get().intensity, Intensity::Strong);
        assert!(!path.exists(), "the file waits for the changes after it");

        settings.flush();
        assert_eq!(
            qf_core::load_settings(&path).unwrap().intensity,
            Intensity::Strong
        );
        // A flush with nothing outstanding writes nothing and does not fail.
        settings.flush();

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_change_that_changes_nothing_notifies_nobody() {
        let dir = temp_dir("noop");
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
        settings.apply_patch(r#"{"theme":"light"}"#).unwrap();
        assert_eq!(seen.get(), 2);
    }

    #[test]
    fn a_refused_patch_leaves_every_setting_alone() {
        let dir = temp_dir("patch");
        let (settings, _) = SettingsStore::load_from(dir.join("settings.json"));
        let before = settings.get().clone();

        assert!(settings.apply_patch(r#"{"intensity":"loud"}"#).is_err());
        assert_eq!(*settings.get(), before);
    }

    #[test]
    fn an_unreadable_file_warns_and_falls_back_without_replacing_it() {
        let dir = temp_dir("broken");
        let path = dir.join("settings.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, b"{ not settings").unwrap();

        let (settings, warning) = SettingsStore::load_from(path.clone());
        assert!(warning.unwrap().contains("default settings"));
        assert_eq!(*settings.get(), Settings::default());
        assert_eq!(fs::read(&path).unwrap(), b"{ not settings");

        // Changing one setting is the point at which the file is rewritten.
        settings.update(|s| s.vary = false);
        settings.flush();
        assert!(!qf_core::load_settings(&path).unwrap().vary);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_write_that_fails_is_reported_and_tried_again() {
        let dir = temp_dir("unwritable");
        // A directory where the file belongs makes every rename fail.
        let path = dir.join("settings.json");
        fs::create_dir_all(&path).unwrap();
        let (settings, _) = SettingsStore::load_from(path.clone());
        let problems = Rc::new(RefCell::new(Vec::new()));
        let seen = problems.clone();
        settings.on_problem(move |m| seen.borrow_mut().push(m.to_string()));

        settings.update(|s| s.vary = false);
        settings.flush();
        assert_eq!(problems.borrow().len(), 1);
        assert!(problems.borrow()[0].contains("could not save"));

        // The write is retried every second for as long as the app runs, so
        // the trouble is reported once rather than once per attempt.
        for _ in 0..5 {
            settings.flush();
        }
        assert_eq!(problems.borrow().len(), 1, "one outage, one complaint");

        // Clear the obstruction: the next failure is worth hearing about again.
        fs::remove_dir_all(&path).unwrap();
        settings.flush();
        assert!(
            !qf_core::load_settings(&path).unwrap().vary,
            "written at last"
        );
        // Put it back in the way; the file is a real file now.
        fs::remove_file(&path).unwrap();
        fs::create_dir_all(&path).unwrap();
        settings.update(|s| s.vary = true);
        settings.flush();
        assert_eq!(problems.borrow().len(), 2);

        fs::remove_dir_all(dir).unwrap();
    }
}
