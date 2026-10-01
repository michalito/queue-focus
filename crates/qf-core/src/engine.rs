//! Everything Queue Focus does that is not drawing: the queue, the settings
//! and the reminder, behind the method set the D-Bus interface defines.
//!
//! Each host maps its own surface onto this one to one: the GNOME service
//! onto D-Bus, the macOS app onto Swift. The host owns the clock, the
//! calendar and the entropy, and calls `tick` once a second.

use crate::reminder::Reminder;
use crate::settings_store::SettingsStore;
use crate::store::{SETTINGS_FILE, TASKS_FILE};
use crate::tasks::Tasks;
use crate::{
    Bucket, FlashEvent, FlashStatus, Outcome, Settings, Store, Tag, Task, TimeOfDay,
    MAX_QUICK_ADD_BYTES,
};
use std::fmt;
use std::io;
use std::path::Path;

/// Settings currently serialize to well under 1 KiB. Leave room for
/// whitespace and future fields without letting an untrusted caller hand
/// serde_json an arbitrarily expensive document.
pub const MAX_SETTINGS_PATCH_BYTES: usize = 4 * 1024;

/// How much of an unknown setting's name an error repeats.
const MAX_ERROR_FIELD_CHARS: usize = 64;

/// Why a request was refused. Nothing changed in either case.
#[derive(Debug)]
pub enum EngineError {
    /// The arguments could not be used, or named a task that is not there.
    InvalidArgument(String),
    /// The change could not be saved, so it was rolled back.
    Persistence(io::Error),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::InvalidArgument(message) => f.write_str(message),
            EngineError::Persistence(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EngineError::InvalidArgument(_) => None,
            EngineError::Persistence(error) => Some(error),
        }
    }
}

impl From<io::Error> for EngineError {
    fn from(error: io::Error) -> Self {
        EngineError::Persistence(error)
    }
}

fn invalid(message: &str) -> EngineError {
    EngineError::InvalidArgument(message.into())
}

/// What one second of the engine produced.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tick {
    /// A flash to draw now.
    pub flash: Option<FlashEvent>,
    /// The settings could not be written. Reported once per outage.
    pub settings_problem: Option<String>,
}

#[derive(Debug)]
pub struct Engine {
    tasks: Tasks,
    settings: SettingsStore,
    reminder: Reminder,
}

impl Engine {
    /// Open the task and settings files in `dir`; the first flash is due a
    /// full wait after `now`.
    ///
    /// A task file that cannot be read is an error, so the host can refuse to
    /// start rather than replace it. A settings file that cannot be read is
    /// only a warning: the defaults apply and the file is left alone until a
    /// setting changes.
    pub fn open(dir: &Path, now: u64) -> io::Result<(Engine, Option<String>)> {
        let tasks = Tasks::load(dir.join(TASKS_FILE))?;
        let (settings, warning) = SettingsStore::load(dir.join(SETTINGS_FILE));
        let reminder = Reminder::new(now, settings.get());
        Ok((
            Engine {
                tasks,
                settings,
                reminder,
            },
            warning,
        ))
    }

    // ---- reading -------------------------------------------------------

    pub fn store(&self) -> &Store {
        self.tasks.store()
    }

    /// `GetState`: the compact JSON snapshot of the queue.
    pub fn state_json(&self) -> String {
        self.store().snapshot_json()
    }

    /// How many task changes have been saved since the engine opened.
    pub fn revision(&self) -> u64 {
        self.tasks.revision()
    }

    pub fn settings(&self) -> &Settings {
        self.settings.get()
    }

    /// `GetSettings`.
    pub fn settings_json(&self) -> String {
        self.settings().to_json()
    }

    // ---- task changes --------------------------------------------------

    /// Apply any change to the queue and save it. The named methods below
    /// are the requests other processes can make; a window that drags and
    /// renames tasks uses this.
    pub fn update<R>(
        &mut self,
        f: impl FnOnce(&mut Store) -> R,
    ) -> Result<Outcome<R>, EngineError> {
        Ok(self.tasks.update(f)?)
    }

    /// `Add(text, bucket)`: parse the add markers and create a task. Without
    /// a bucket the task goes where Settings says; a marker in the text wins
    /// either way. Returns the new task's id.
    pub fn add(&mut self, text: &str, bucket: Option<Bucket>) -> Result<Outcome<u64>, EngineError> {
        if text.len() > MAX_QUICK_ADD_BYTES {
            return Err(invalid("task text is too long"));
        }
        let default = bucket.unwrap_or(self.settings().default_bucket);
        let outcome = self.update(|s| s.quick_add(text, default))?;
        match outcome.value {
            Some(id) => Ok(outcome.map(|_| id)),
            None => Err(invalid("empty title")),
        }
    }

    /// `CompleteCurrent`: delete the current task and pull the head of Next
    /// into Now. Returns the task that was completed, if Now held one.
    pub fn complete_current(&mut self) -> Result<Outcome<Option<Task>>, EngineError> {
        Ok(self.tasks.complete_current()?)
    }

    /// `Complete(id)`: the current task is completed as by
    /// `complete_current`; any other task is deleted. Either way it is the
    /// completion `undo_complete` reverses.
    pub fn complete(&mut self, id: u64) -> Result<Outcome<()>, EngineError> {
        found(self.tasks.complete(id)?)
    }

    /// `UndoComplete(id)`: reverse the most recent completion. `false` when
    /// it was not `id`'s, or anything else has changed since.
    pub fn undo_complete(&mut self, id: u64) -> Result<Outcome<bool>, EngineError> {
        Ok(self.tasks.undo_complete(id)?)
    }

    /// `TogglePause`: pause or resume the current task's timer. `false` when
    /// there is no current task.
    pub fn toggle_pause(&mut self) -> Result<Outcome<bool>, EngineError> {
        self.update(Store::toggle_pause)
    }

    /// `Promote(id)`: make a task the current one. The task it replaces moves
    /// to the front of Next.
    pub fn promote(&mut self, id: u64) -> Result<Outcome<()>, EngineError> {
        found(self.update(|s| s.promote(id))?)
    }

    /// `Remove(id)`: delete a task. Unlike `complete`, it cannot be undone.
    pub fn remove(&mut self, id: u64) -> Result<Outcome<()>, EngineError> {
        found(self.update(|s| s.remove(id))?)
    }

    /// `Move(id, bucket, index)`: move a task to a zero-based position, or to
    /// the end of the bucket without one. Now holds one task, so a move there
    /// is a promotion whatever the index.
    pub fn move_to(
        &mut self,
        id: u64,
        bucket: Bucket,
        index: Option<usize>,
    ) -> Result<Outcome<()>, EngineError> {
        found(self.update(|s| s.move_to(id, bucket, index))?)
    }

    /// `SetTag(id, tag)`: set or clear a task's tag.
    pub fn set_tag(&mut self, id: u64, tag: Option<Tag>) -> Result<Outcome<()>, EngineError> {
        found(self.update(|s| s.set_tag(id, tag))?)
    }

    // ---- settings ------------------------------------------------------

    /// Change the settings from a control. Returns whether anything changed.
    /// The file catches up on a later tick.
    pub fn update_settings(&mut self, f: impl FnOnce(&mut Settings)) -> bool {
        self.settings.update(f)
    }

    /// `SetSettings(json)`: apply a JSON object holding only the settings to
    /// change. An unknown key or an unusable value changes nothing. Returns
    /// whether anything changed.
    pub fn set_settings(&mut self, patch: &str) -> Result<bool, EngineError> {
        if patch.len() > MAX_SETTINGS_PATCH_BYTES {
            return Err(invalid("settings patch is too long"));
        }
        self.settings
            .apply_patch(patch)
            .map_err(|e| EngineError::InvalidArgument(bounded_settings_error(e)))
    }

    /// Write the settings if the file is behind, whatever the backoff says.
    /// Called before the host exits. Returns a problem to report.
    pub fn flush(&mut self) -> Option<String> {
        self.settings.flush()
    }

    // ---- the clock -----------------------------------------------------

    /// One second of the engine: the settings writer and the reminder. `now`
    /// is the unix time, `local_time` the time of day the quiet hours read,
    /// and `random` any number.
    pub fn tick(&mut self, now: u64, local_time: TimeOfDay, random: u32) -> Tick {
        let settings_problem = self.settings.tick();
        let flash = self.reminder.tick(
            self.tasks.store(),
            self.settings.get(),
            now,
            local_time,
            random,
        );
        Tick {
            flash,
            settings_problem,
        }
    }

    /// A flash now, whatever the quiet rules say, so long as Now holds a task.
    /// The wait for the next one starts over.
    pub fn flash_now(&mut self, now: u64, random: u32) -> Option<FlashEvent> {
        self.reminder
            .flash_now(self.tasks.store(), self.settings.get(), now, random)
    }

    /// Why the next flash is held back, or how long until it arrives.
    pub fn flash_status(&self, now: u64, local_time: TimeOfDay) -> FlashStatus {
        self.reminder
            .status(self.tasks.store(), self.settings.get(), now, local_time)
    }
}

/// A request that named a task found it; one that did not is refused.
fn found(outcome: Outcome<bool>) -> Result<Outcome<()>, EngineError> {
    if outcome.value {
        Ok(outcome.map(|_| ()))
    } else {
        Err(invalid("no such task"))
    }
}

/// Bound the only settings error that repeats a caller's text. Truncating by
/// character keeps the message valid UTF-8 while a long key cannot become a
/// long error.
fn bounded_settings_error(error: String) -> String {
    const PREFIX: &str = "unknown setting: ";
    let Some(field) = error.strip_prefix(PREFIX) else {
        return error;
    };
    let Some((end, _)) = field.char_indices().nth(MAX_ERROR_FIELD_CHARS) else {
        return error;
    };
    format!("{PREFIX}{}…", &field[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Hold, Intensity};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const NOON: u64 = 12 * 3600;

    fn noon() -> TimeOfDay {
        TimeOfDay::new(12, 0).unwrap()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("qf-engine-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open(dir: &Path) -> Engine {
        let (engine, warning) = Engine::open(dir, NOON).unwrap();
        assert!(warning.is_none());
        engine
    }

    fn ids(engine: &Engine, bucket: Bucket) -> Vec<u64> {
        engine.store().in_bucket(bucket).map(|t| t.id).collect()
    }

    fn invalid_message<T: fmt::Debug>(result: Result<T, EngineError>) -> String {
        match result {
            Err(EngineError::InvalidArgument(message)) => message,
            other => panic!("expected an invalid argument, got {other:?}"),
        }
    }

    #[test]
    fn opening_reads_both_files_from_one_directory() {
        let dir = temp_dir("open");
        let mut engine = open(&dir);
        let id = engine.add("first #w", None).unwrap().value;
        assert!(engine.set_settings(r#"{"interval_min":5}"#).unwrap());
        assert!(engine.flush().is_none());

        let again = open(&dir);
        assert_eq!(again.store().get(id).unwrap().title, "first");
        assert_eq!(again.settings().interval_min, 5);
        assert_eq!(again.revision(), 0, "a fresh engine has saved nothing yet");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unreadable_task_file_refuses_to_open_and_is_left_alone() {
        let dir = temp_dir("bad-tasks");
        fs::write(dir.join("tasks.json"), b"{ broken").unwrap();
        let error = Engine::open(&dir, NOON).unwrap_err();
        assert!(error.to_string().contains("tasks.json"), "{error}");
        assert_eq!(fs::read(dir.join("tasks.json")).unwrap(), b"{ broken");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unreadable_settings_file_opens_with_the_defaults_and_a_warning() {
        let dir = temp_dir("bad-settings");
        fs::write(dir.join("settings.json"), b"[]").unwrap();
        let (engine, warning) = Engine::open(&dir, NOON).unwrap();
        assert!(warning.unwrap().contains("settings.json"));
        assert_eq!(*engine.settings(), Settings::default());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn add_parses_markers_and_falls_back_to_the_chosen_bucket() {
        let dir = temp_dir("add");
        let mut engine = open(&dir);
        let next = engine.add("plain", None).unwrap().value;
        assert_eq!(ids(&engine, Bucket::Next), vec![next]);

        engine.update_settings(|s| s.default_bucket = Bucket::Side);
        let side = engine.add("side by default", None).unwrap().value;
        let later = engine
            .add("asked for later", Some(Bucket::Later))
            .unwrap()
            .value;
        let marked = engine
            .add("marker wins @next", Some(Bucket::Later))
            .unwrap()
            .value;
        let current = engine.add("now #p", Some(Bucket::Now)).unwrap().value;
        assert_eq!(ids(&engine, Bucket::Side), vec![side]);
        assert_eq!(ids(&engine, Bucket::Later), vec![later]);
        assert_eq!(ids(&engine, Bucket::Next), vec![next, marked]);
        assert_eq!(engine.store().current().unwrap().id, current);
        assert_eq!(engine.store().current().unwrap().tag, Some(Tag::Personal));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn add_refuses_an_empty_title_and_oversized_text_without_saving() {
        let dir = temp_dir("add-refused");
        let mut engine = open(&dir);
        assert_eq!(
            invalid_message(engine.add("  #w @later ", None)),
            "empty title"
        );
        let huge = "x".repeat(MAX_QUICK_ADD_BYTES + 1);
        assert_eq!(
            invalid_message(engine.add(&huge, None)),
            "task text is too long"
        );
        assert_eq!(engine.revision(), 0);
        assert!(!dir.join("tasks.json").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn requests_naming_a_missing_task_are_refused_and_change_nothing() {
        let dir = temp_dir("missing");
        let mut engine = open(&dir);
        engine.add("only", None).unwrap();
        let revision = engine.revision();
        for message in [
            invalid_message(engine.complete(99)),
            invalid_message(engine.promote(99)),
            invalid_message(engine.remove(99)),
            invalid_message(engine.move_to(99, Bucket::Side, None)),
            invalid_message(engine.set_tag(99, Some(Tag::Work))),
        ] {
            assert_eq!(message, "no such task");
        }
        assert_eq!(engine.revision(), revision);
        fs::remove_dir_all(dir).unwrap();
    }

    /// The D-Bus round trip of the queue, through the engine's own names.
    #[test]
    fn the_named_requests_do_what_the_interface_says() {
        let dir = temp_dir("requests");
        let mut engine = open(&dir);
        let a = engine.add("a", Some(Bucket::Now)).unwrap().value;
        let b = engine.add("b", None).unwrap().value;
        let c = engine.add("c", Some(Bucket::Later)).unwrap().value;

        assert!(engine.toggle_pause().unwrap().value);
        assert!(engine.store().current().unwrap().is_paused());
        engine.set_tag(b, Some(Tag::Work)).unwrap();
        engine.set_tag(b, None).unwrap();
        assert_eq!(engine.store().get(b).unwrap().tag, None);

        engine.promote(c).unwrap();
        assert_eq!(engine.store().current().unwrap().id, c);
        assert_eq!(ids(&engine, Bucket::Next), vec![a, b]);

        engine.move_to(b, Bucket::Next, Some(0)).unwrap();
        assert_eq!(ids(&engine, Bucket::Next), vec![b, a]);
        engine.move_to(b, Bucket::Next, None).unwrap();
        assert_eq!(
            ids(&engine, Bucket::Next),
            vec![a, b],
            "no index is the end"
        );

        let done = engine.complete_current().unwrap().value.unwrap();
        assert_eq!(done.id, c);
        assert_eq!(engine.store().current().unwrap().id, a);
        assert!(engine.undo_complete(c).unwrap().value);
        assert_eq!(engine.store().current().unwrap().id, c);

        engine.complete(b).unwrap();
        assert!(engine.store().get(b).is_none());
        engine.remove(a).unwrap();
        assert!(
            !engine.undo_complete(b).unwrap().value,
            "stale after a remove"
        );

        let state: serde_json::Value = serde_json::from_str(&engine.state_json()).unwrap();
        assert_eq!(state["current"]["id"], c);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn nothing_to_complete_or_pause_is_an_answer_not_an_error() {
        let dir = temp_dir("empty");
        let mut engine = open(&dir);
        assert!(engine.complete_current().unwrap().value.is_none());
        assert!(!engine.toggle_pause().unwrap().value);
        assert!(!engine.undo_complete(1).unwrap().value);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_change_that_cannot_be_saved_is_a_persistence_error() {
        let dir = temp_dir("unsaveable");
        let mut engine = open(&dir);
        // A directory where the file belongs makes the atomic rename fail.
        fs::create_dir_all(dir.join("tasks.json")).unwrap();
        match engine.add("lost", None) {
            Err(EngineError::Persistence(error)) => {
                assert!(error.to_string().contains("could not save"), "{error}")
            }
            other => panic!("expected a persistence error, got {other:?}"),
        }
        assert!(engine.store().is_empty(), "rolled back");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_patches_change_only_what_they_name_or_nothing_at_all() {
        let dir = temp_dir("settings");
        let mut engine = open(&dir);
        assert!(engine.set_settings(r#"{"intensity":"strong"}"#).unwrap());
        assert!(!engine.set_settings(r#"{"intensity":"strong"}"#).unwrap());
        let before = engine.settings().clone();
        assert!(engine
            .set_settings(r#"{"vary":false,"theme":"puce"}"#)
            .is_err());
        assert_eq!(*engine.settings(), before);
        let json: serde_json::Value = serde_json::from_str(&engine.settings_json()).unwrap();
        assert_eq!(json["intensity"], "strong");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_settings_patch_limit_is_a_byte_limit() {
        let dir = temp_dir("patch-limit");
        let mut engine = open(&dir);
        let at_limit = format!(
            "{{\"vary\":false{}}}",
            " ".repeat(MAX_SETTINGS_PATCH_BYTES - 14)
        );
        assert_eq!(at_limit.len(), MAX_SETTINGS_PATCH_BYTES);
        assert!(engine.set_settings(&at_limit).unwrap());
        assert_eq!(
            invalid_message(engine.set_settings(&format!("{at_limit} "))),
            "settings patch is too long"
        );
        let wide = "é".repeat(MAX_SETTINGS_PATCH_BYTES / 2 + 1);
        assert_eq!(
            invalid_message(engine.set_settings(&wide)),
            "settings patch is too long"
        );
        assert!(Settings::default().to_json().len() < MAX_SETTINGS_PATCH_BYTES);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_setting_names_are_truncated_without_splitting_utf8() {
        let field = "é".repeat(MAX_ERROR_FIELD_CHARS + 1);
        assert_eq!(
            bounded_settings_error(format!("unknown setting: {field}")),
            format!("unknown setting: {}…", "é".repeat(MAX_ERROR_FIELD_CHARS))
        );
        assert_eq!(
            bounded_settings_error("unknown setting: typo".into()),
            "unknown setting: typo"
        );
        assert_eq!(
            bounded_settings_error("settings patch is not JSON".into()),
            "settings patch is not JSON"
        );

        let dir = temp_dir("unknown-setting");
        let mut engine = open(&dir);
        let message = invalid_message(engine.set_settings(&format!("{{\"{field}\":1}}")));
        assert!(message.ends_with('…'), "{message}");
        fs::remove_dir_all(dir).unwrap();
    }

    /// One tick drives both the settings writer and the reminder.
    #[test]
    fn a_tick_writes_the_settings_and_delivers_a_due_flash() {
        let dir = temp_dir("tick");
        let mut engine = open(&dir);
        engine.add("focus", Some(Bucket::Now)).unwrap();
        engine.update_settings(|s| {
            s.interval_min = 1;
            s.intensity = Intensity::Strong;
        });

        let first = engine.tick(NOON + 1, noon(), 0);
        assert_eq!(first, Tick::default());
        assert_eq!(
            crate::load_settings(&dir.join("settings.json"))
                .unwrap()
                .interval_min,
            1,
            "the tick wrote the settings"
        );

        let due = engine.tick(NOON + 15 * 60, noon(), 0);
        let flash = due.flash.unwrap();
        assert_eq!(flash.title, "focus");
        assert_eq!(flash.intensity, Intensity::Strong);
        assert_eq!(
            engine.flash_status(NOON + 15 * 60, noon()).remaining,
            Some(60),
            "the shorter interval took over"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_settings_write_failure_comes_back_from_the_tick_once() {
        let dir = temp_dir("tick-problem");
        let mut engine = open(&dir);
        fs::create_dir_all(dir.join("settings.json")).unwrap();
        engine.update_settings(|s| s.vary = false);
        let problem = engine.tick(NOON, noon(), 0).settings_problem.unwrap();
        assert!(problem.contains("settings.json"), "{problem}");
        for second in 1..=40 {
            assert_eq!(engine.tick(NOON + second, noon(), 0), Tick::default());
        }
        assert!(engine.flush().is_none(), "one outage, one complaint");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn flash_now_needs_a_current_task_and_restarts_the_wait() {
        let dir = temp_dir("flash-now");
        let mut engine = open(&dir);
        assert_eq!(engine.flash_status(NOON, noon()).hold, Hold::NoCurrentTask);
        assert!(engine.flash_now(NOON, 0).is_none());
        engine.add("!focus #p", None).unwrap();
        let flash = engine.flash_now(NOON + 100, 0).unwrap();
        assert_eq!(flash.palette, crate::Palette::Orange);
        assert_eq!(
            engine.flash_status(NOON + 100, noon()).remaining,
            Some(15 * 60)
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
