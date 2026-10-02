//! The settings and the file they live in.
//!
//! The value in memory is authoritative the moment it changes. The file
//! catches up on the host's next tick, a second later, because dragging the
//! interval slider moves the value on every pixel and each save is a write,
//! an fsync and a rename.
//!
//! The host owns the clock: it calls `tick` once a second and `flush` before
//! it exits, and reports whatever problem either one returns.

use crate::Settings;
use std::path::PathBuf;

/// Ticks to sit out before trying a failed write again.
const RETRY_TICKS: u32 = 30;

#[derive(Debug)]
pub struct SettingsStore {
    settings: Settings,
    path: PathBuf,
    /// Set while the file is behind the value in memory.
    dirty: bool,
    /// Set once a failure has been reported, so one outage is one complaint.
    reported: bool,
    /// Ticks to sit out before trying a failed write again.
    backoff: u32,
}

impl SettingsStore {
    /// Load the settings, falling back to the defaults. A file that cannot be
    /// read comes back as a warning rather than an error: losing the queue is
    /// worth refusing to start over, losing a switch is not. The user is still
    /// told, and the broken file is left alone until something changes.
    pub fn load(path: PathBuf) -> (SettingsStore, Option<String>) {
        let (settings, warning) = match crate::load_settings(&path) {
            Ok(settings) => (settings, None),
            Err(e) => (
                Settings::default(),
                Some(format!(
                    "could not read {}: {e}; using the default settings until you change one",
                    path.display()
                )),
            ),
        };
        let store = SettingsStore {
            settings,
            path,
            dirty: false,
            reported: false,
            backoff: 0,
        };
        (store, warning)
    }

    pub fn get(&self) -> &Settings {
        &self.settings
    }

    /// Change the settings. Returns whether anything changed; nothing is
    /// written when the change leaves the value it had.
    pub fn update(&mut self, f: impl FnOnce(&mut Settings)) -> bool {
        let before = self.settings.clone();
        f(&mut self.settings);
        self.settings.sanitize();
        self.changed_from(&before)
    }

    /// Apply a JSON object of changed keys. An unknown key or an unusable
    /// value leaves every setting as it was. Returns whether anything changed.
    pub fn apply_patch(&mut self, patch: &str) -> Result<bool, String> {
        let before = self.settings.clone();
        self.settings.apply_patch(patch)?;
        Ok(self.changed_from(&before))
    }

    fn changed_from(&mut self, before: &Settings) -> bool {
        let changed = self.settings != *before;
        self.dirty |= changed;
        changed
    }

    /// One second of the writer. A write that failed is not retried on the
    /// next tick: every attempt creates a temporary file and syncs it before
    /// it can fail, and a file that cannot be written now usually cannot be
    /// written a second later either. Returns a problem to report.
    pub fn tick(&mut self) -> Option<String> {
        if self.backoff > 0 {
            self.backoff -= 1;
            return None;
        }
        self.flush()
    }

    /// Whether a failure to write was reported and no write has worked since:
    /// an outage the user has been told of.
    pub fn in_outage(&self) -> bool {
        self.reported
    }

    /// Write if the file is behind. Called on the writer's tick, and once more
    /// before the process exits, which is why it never sits out a turn.
    /// Returns a problem to report, only the first time in an outage: a
    /// dialog a second would be worse than the trouble it reports.
    pub fn flush(&mut self) -> Option<String> {
        if !self.dirty {
            return None;
        }
        match crate::save_settings(&self.path, &self.settings) {
            Ok(()) => {
                self.dirty = false;
                self.reported = false;
                self.backoff = 0;
                None
            }
            Err(e) => {
                // The change is still unsaved.
                self.backoff = RETRY_TICKS;
                (!std::mem::replace(&mut self.reported, true))
                    .then(|| format!("could not save {}: {e}", self.path.display()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Intensity, Theme};
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
        let (mut settings, _) = SettingsStore::load(path.clone());

        assert!(settings.update(|s| s.vary = false));
        assert!(settings.tick().is_some(), "the first tick tries");
        // The whole backoff passes without another attempt: `dirty` is still
        // set, so any attempt would have to go through save_settings.
        for _ in 0..RETRY_TICKS {
            assert!(settings.tick().is_none());
        }
        assert!(settings.dirty, "still unsaved");
        assert_eq!(settings.backoff, 0, "and ready to try once more");

        // Shutdown does not sit out a turn, whatever the backoff says.
        settings.backoff = RETRY_TICKS;
        fs::remove_dir_all(&path).unwrap();
        assert!(settings.flush().is_none());
        assert!(!crate::load_settings(&path).unwrap().vary);
        assert_eq!(settings.backoff, 0, "cleared by the write that worked");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_change_is_visible_at_once_and_reaches_the_file_on_flush() {
        let dir = temp_dir("flush");
        let path = dir.join("settings.json");
        let (mut settings, warning) = SettingsStore::load(path.clone());
        assert!(warning.is_none());

        assert!(settings.update(|s| s.intensity = Intensity::Strong));
        assert_eq!(settings.get().intensity, Intensity::Strong);
        assert!(!path.exists(), "the file waits for the changes after it");

        assert!(settings.flush().is_none());
        assert_eq!(
            crate::load_settings(&path).unwrap().intensity,
            Intensity::Strong
        );
        // A flush with nothing outstanding writes nothing and does not fail.
        fs::remove_file(&path).unwrap();
        assert!(settings.flush().is_none());
        assert!(!path.exists());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_change_that_changes_nothing_is_not_a_change() {
        let dir = temp_dir("noop");
        let (mut settings, _) = SettingsStore::load(dir.join("settings.json"));

        assert!(settings.update(|s| s.theme = Theme::Dark));
        settings.dirty = false;
        assert!(!settings.update(|s| s.theme = Theme::Dark));
        assert!(!settings.apply_patch(r#"{"theme":"dark"}"#).unwrap());
        assert!(!settings.dirty, "nothing to write");
        assert!(settings.apply_patch(r#"{"theme":"light"}"#).unwrap());
        assert!(settings.dirty);
    }

    #[test]
    fn a_refused_patch_leaves_every_setting_alone() {
        let dir = temp_dir("patch");
        let (mut settings, _) = SettingsStore::load(dir.join("settings.json"));
        let before = settings.get().clone();

        assert!(settings.apply_patch(r#"{"intensity":"loud"}"#).is_err());
        assert_eq!(*settings.get(), before);
        assert!(!settings.dirty);
    }

    #[test]
    fn an_unreadable_file_warns_and_falls_back_without_replacing_it() {
        let dir = temp_dir("broken");
        let path = dir.join("settings.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, b"{ not settings").unwrap();

        let (mut settings, warning) = SettingsStore::load(path.clone());
        let warning = warning.unwrap();
        assert!(warning.contains("default settings"), "{warning}");
        assert!(warning.contains("settings.json"), "{warning}");
        assert_eq!(*settings.get(), Settings::default());
        assert!(settings.tick().is_none());
        assert_eq!(fs::read(&path).unwrap(), b"{ not settings");

        // Changing one setting is the point at which the file is rewritten.
        settings.update(|s| s.vary = false);
        assert!(settings.flush().is_none());
        assert!(!crate::load_settings(&path).unwrap().vary);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_write_that_fails_is_reported_once_per_outage() {
        let dir = temp_dir("unwritable");
        // A directory where the file belongs makes every rename fail.
        let path = dir.join("settings.json");
        fs::create_dir_all(&path).unwrap();
        let (mut settings, _) = SettingsStore::load(path.clone());

        settings.update(|s| s.vary = false);
        let problem = settings.flush().unwrap();
        assert!(problem.contains("could not save"), "{problem}");
        assert!(problem.contains("settings.json"), "{problem}");

        // Retried for as long as the app runs, the trouble is reported once
        // rather than once per attempt.
        for _ in 0..5 {
            assert!(settings.flush().is_none(), "one outage, one complaint");
        }

        // Clear the obstruction: the next failure is worth hearing about again.
        fs::remove_dir_all(&path).unwrap();
        assert!(settings.flush().is_none());
        assert!(
            !crate::load_settings(&path).unwrap().vary,
            "written at last"
        );
        // Put it back in the way; the file is a real file now.
        fs::remove_file(&path).unwrap();
        fs::create_dir_all(&path).unwrap();
        settings.update(|s| s.vary = true);
        assert!(settings.flush().is_some());

        fs::remove_dir_all(dir).unwrap();
    }
}
