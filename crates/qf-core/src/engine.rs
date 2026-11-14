//! Everything Queue Focus does that is not drawing: the queue, the settings
//! and the reminder, behind the method set the D-Bus interface defines.
//!
//! Each host maps its own surface onto this one to one: the GNOME service
//! onto D-Bus, the macOS app onto Swift. The host calls `tick` once a second
//! and owns the reminder's clock, calendar and entropy.
//!
//! The queue is either this device's own file or a shared queue kept in step
//! with a sync server. Both answer the same requests; only the shared one can
//! answer that a change was sent and waits for the server.

mod shared;

use crate::reminder::Reminder;
use crate::settings_store::SettingsStore;
use crate::store::{SETTINGS_FILE, TASKS_FILE};
use crate::sync::{Client, ClockSample, Envelope, Id, RecoveryChange, SyncStatus};
use crate::tasks::Tasks;
use crate::{
    Bucket, FlashEvent, FlashStatus, Outcome, Settings, Store, Tag, Task, TimeOfDay,
    MAX_QUICK_ADD_BYTES,
};
use shared::Shared;
use std::fmt;
use std::io;
use std::path::Path;

/// Settings currently serialize to well under 1 KiB. Leave room for
/// whitespace and future fields without letting an untrusted caller hand
/// serde_json an arbitrarily expensive document.
pub const MAX_SETTINGS_PATCH_BYTES: usize = 4 * 1024;

/// How much of an unknown setting's name an error repeats.
const MAX_ERROR_FIELD_CHARS: usize = 64;

/// A refused request. Nothing changed.
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

/// What a request that was taken came to.
///
/// A change to the queue is applied at once, except a change to the shared
/// Now or its timer: that is a command the server has to confirm first. It
/// is saved and sent, the queue stays as it was, and the result arrives as a
/// settlement in the sync status. Until then nothing may report it done,
/// offer to undo it, or send it again.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum Submitted<T> {
    /// Applied and saved: the queue shows it now.
    Applied(Outcome<T>),
    /// Saved and sent; the server decides. The id names the command whose
    /// settlement will say how it went.
    Pending(Id),
}

impl<T> Submitted<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Submitted<U> {
        match self {
            Submitted::Applied(outcome) => Submitted::Applied(outcome.map(f)),
            Submitted::Pending(command) => Submitted::Pending(command),
        }
    }

    /// The applied change, or `None` while the server has it.
    pub fn applied(self) -> Option<Outcome<T>> {
        match self {
            Submitted::Applied(outcome) => Some(outcome),
            Submitted::Pending(_) => None,
        }
    }

    /// The command the server is deciding, if it is one.
    pub fn pending(&self) -> Option<Id> {
        match self {
            Submitted::Applied(_) => None,
            Submitted::Pending(command) => Some(*command),
        }
    }
}

/// How a host words a pending change where it can only answer with an
/// error, as D-Bus and Swift do. Clients read the command id back out of the
/// parentheses, so the form is part of the D-Bus interface.
pub fn pending_message(command: Id) -> String {
    format!("Sent, waiting for confirmation ({command})")
}

/// What one second of the engine produced.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tick {
    /// A flash to draw now.
    pub flash: Option<FlashEvent>,
    /// The settings could not be written. Reported once per outage.
    pub settings_problem: Option<String>,
    /// A write worked after an outage was reported: what it said is over.
    pub settings_outage_ended: bool,
}

#[derive(Debug)]
pub struct Engine {
    queue: Queue,
    settings: SettingsStore,
    reminder: Reminder,
}

/// Where the tasks live.
#[derive(Debug)]
enum Queue {
    /// This device's own task file.
    Local(Tasks),
    /// The shared queue, as this device's sync journal projects it. Boxed: it
    /// carries the whole journal.
    Shared(Box<Shared>),
}

impl Queue {
    fn store(&self) -> &Store {
        match self {
            Queue::Local(tasks) => tasks.store(),
            Queue::Shared(shared) => shared.store(),
        }
    }

    fn revision(&self) -> u64 {
        match self {
            Queue::Local(tasks) => tasks.revision(),
            Queue::Shared(shared) => shared.revision(),
        }
    }
}

impl Engine {
    /// Open the task and settings files in `dir`. The first flash is due a
    /// full wait after `now`, which is read once both files are loaded, so a
    /// slow disk does not eat into the wait.
    ///
    /// A task file that cannot be read is an error, so the host can refuse to
    /// start rather than replace it. A settings file that cannot be read is
    /// only a warning: the defaults apply and the file is left alone until a
    /// setting changes.
    pub fn open(dir: &Path, now: impl FnOnce() -> u64) -> io::Result<(Engine, Option<String>)> {
        let tasks = Tasks::load(dir.join(TASKS_FILE))?;
        let (settings, warning) = SettingsStore::load(dir.join(SETTINGS_FILE));
        let reminder = Reminder::new(now(), settings.get());
        let engine = Engine {
            queue: Queue::Local(tasks),
            settings,
            reminder,
        };
        Ok((engine, warning))
    }

    /// Open the shared queue held by `client`, and the settings in `dir`. The
    /// host owns the directory lock and the transport. The standalone task
    /// file is never read or written: sync has replaced it.
    pub fn open_with_sync(
        dir: &Path,
        client: Client,
        clock: ClockSample,
    ) -> (Engine, Option<String>) {
        let (settings, warning) = SettingsStore::load(dir.join(SETTINGS_FILE));
        let reminder = Reminder::new(clock.wall, settings.get());
        let engine = Engine {
            queue: Queue::Shared(Box::new(Shared::new(client, clock))),
            settings,
            reminder,
        };
        (engine, warning)
    }

    // ---- reading -------------------------------------------------------

    pub fn store(&self) -> &Store {
        self.queue.store()
    }

    /// `GetState`: the compact JSON snapshot of the queue, with the sync
    /// status beside it when the queue is shared.
    pub fn state_json(&self) -> String {
        let mut state = self.store().snapshot_value();
        if let Some(status) = self.sync_status() {
            state["sync"] = serde_json::to_value(status).expect("sync status is plain data");
        }
        state.to_string()
    }

    /// Changes to the queue seen since the engine opened: saved here, or, for
    /// a shared queue, arrived from the server. Polls that change nothing do
    /// not count.
    pub fn revision(&self) -> u64 {
        self.queue.revision()
    }

    pub fn settings(&self) -> &Settings {
        self.settings.get()
    }

    /// `GetSettings`.
    pub fn settings_json(&self) -> String {
        self.settings().to_json()
    }

    // ---- task changes --------------------------------------------------

    /// Apply any change to a standalone queue and save it. A shared queue
    /// refuses: every change to it has to be one of the named requests below,
    /// which the server can check.
    pub fn update<R>(
        &mut self,
        f: impl FnOnce(&mut Store) -> R,
    ) -> Result<Outcome<R>, EngineError> {
        match &mut self.queue {
            Queue::Local(tasks) => Ok(tasks.update(f)?),
            Queue::Shared(_) => Err(invalid("shared queue changes require an explicit command")),
        }
    }

    /// `Add(text, bucket)`: parse the add markers and create a task. Without
    /// a bucket the task goes where Settings says; a marker in the text wins
    /// either way. Standalone, a task added while Now is empty goes to Now,
    /// so that the first task is the one being done; shared, it does not, as
    /// taking Now needs the server. Returns the new task's id.
    pub fn add(
        &mut self,
        text: &str,
        bucket: Option<Bucket>,
    ) -> Result<Submitted<u64>, EngineError> {
        if text.len() > MAX_QUICK_ADD_BYTES {
            return Err(invalid("task text is too long"));
        }
        let default = self.settings.get().default_bucket;
        match &mut self.queue {
            Queue::Local(tasks) => {
                let default = bucket.unwrap_or(match tasks.store().current() {
                    Some(_) => default,
                    None => Bucket::Now,
                });
                let outcome = tasks.update(|s| s.quick_add(text, default))?;
                match outcome.value {
                    Some(id) => applied(outcome.map(|_| id)),
                    None => Err(invalid("empty title")),
                }
            }
            Queue::Shared(shared) => shared.add(text, bucket.unwrap_or(default)),
        }
    }

    /// `CompleteCurrent`: delete the current task and pull the head of Next
    /// into Now. Returns the task that was completed, if Now held one.
    pub fn complete_current(&mut self) -> Result<Submitted<Option<Task>>, EngineError> {
        match &mut self.queue {
            Queue::Local(tasks) => applied(tasks.complete_current()?),
            Queue::Shared(shared) => shared.complete_current(),
        }
    }

    /// `Complete(id)`: the current task is completed as by
    /// `complete_current`; any other task is deleted. Either way it is the
    /// completion `undo_complete` reverses, but a shared queue only keeps the
    /// current task's.
    pub fn complete(&mut self, id: u64) -> Result<Submitted<()>, EngineError> {
        match &mut self.queue {
            Queue::Local(tasks) => applied(found(tasks.complete(id)?)?),
            Queue::Shared(shared) => shared.delete(id, true),
        }
    }

    /// `UndoComplete(id)`: reverse the most recent completion. `false` when
    /// it was not `id`'s, or anything else has changed since.
    pub fn undo_complete(&mut self, id: u64) -> Result<Submitted<bool>, EngineError> {
        match &mut self.queue {
            Queue::Local(tasks) => applied(tasks.undo_complete(id)?),
            Queue::Shared(shared) => shared.undo(id),
        }
    }

    /// `TogglePause`: pause or resume the current task's timer. `false` when
    /// there is no current task.
    pub fn toggle_pause(&mut self) -> Result<Submitted<bool>, EngineError> {
        match &mut self.queue {
            Queue::Local(tasks) => applied(tasks.update(Store::toggle_pause)?),
            Queue::Shared(shared) => shared.toggle_pause(),
        }
    }

    /// `Promote(id)`: make a task the current one. The task it replaces moves
    /// to the front of Next.
    pub fn promote(&mut self, id: u64) -> Result<Submitted<()>, EngineError> {
        match &mut self.queue {
            Queue::Local(tasks) => applied(found(tasks.update(|s| s.promote(id))?)?),
            Queue::Shared(shared) => shared.promote(id),
        }
    }

    /// `Remove(id)`: delete a task. Unlike `complete`, it cannot be undone.
    pub fn remove(&mut self, id: u64) -> Result<Submitted<()>, EngineError> {
        match &mut self.queue {
            Queue::Local(tasks) => applied(found(tasks.update(|s| s.remove(id))?)?),
            Queue::Shared(shared) => shared.delete(id, false),
        }
    }

    /// `Move(id, bucket, index)`: move a task to a zero-based position, or to
    /// the end of the bucket without one. Now holds one task, so a move there
    /// is a promotion whatever the index.
    pub fn move_to(
        &mut self,
        id: u64,
        bucket: Bucket,
        index: Option<usize>,
    ) -> Result<Submitted<()>, EngineError> {
        match &mut self.queue {
            Queue::Local(tasks) => applied(found(tasks.update(|s| s.move_to(id, bucket, index))?)?),
            Queue::Shared(shared) => shared.move_to(id, bucket, index),
        }
    }

    /// `SetTag(id, tag)`: set or clear a task's tag.
    pub fn set_tag(&mut self, id: u64, tag: Option<Tag>) -> Result<Submitted<()>, EngineError> {
        match &mut self.queue {
            Queue::Local(tasks) => applied(found(tasks.update(|s| s.set_tag(id, tag))?)?),
            Queue::Shared(shared) => shared.set_tag(id, tag),
        }
    }

    // ---- what a window asks for ---------------------------------------
    //
    // Not on D-Bus, where the GNOME windows change the queue in-process.

    /// Cycle a task's tag: none, work, personal, none.
    pub fn cycle_tag(&mut self, id: u64) -> Result<Submitted<()>, EngineError> {
        let tag = self.require(id)?.tag;
        self.set_tag(id, Tag::cycle(tag))
    }

    /// Rename a task. An empty title is refused and the old one stays.
    pub fn rename(&mut self, id: u64, title: &str) -> Result<Submitted<()>, EngineError> {
        if title.trim().is_empty() {
            return Err(invalid("empty title"));
        }
        match &mut self.queue {
            Queue::Local(tasks) => applied(found(tasks.update(|s| s.rename(id, title))?)?),
            Queue::Shared(shared) => shared.rename(id, title),
        }
    }

    /// Move a task up (negative) or down within its bucket. Answers whether
    /// it moved: a task already at the edge stays where it is.
    pub fn shift(&mut self, id: u64, delta: i32) -> Result<Submitted<bool>, EngineError> {
        self.require(id)?;
        match &mut self.queue {
            Queue::Local(tasks) => applied(tasks.update(|s| s.shift(id, delta))?),
            Queue::Shared(shared) => shared.shift(id, delta),
        }
    }

    /// Drop a task before the row `before` of `bucket`, or at its end
    /// without one. Answers whether the drop was taken: a row that has gone
    /// or left the bucket since it was drawn is refused.
    pub fn move_before(
        &mut self,
        id: u64,
        bucket: Bucket,
        before: Option<u64>,
    ) -> Result<Submitted<bool>, EngineError> {
        self.require(id)?;
        match &mut self.queue {
            Queue::Local(tasks) => applied(tasks.update(|s| s.move_before(id, bucket, before))?),
            Queue::Shared(shared) => shared.move_before(id, bucket, before),
        }
    }

    fn require(&self, id: u64) -> Result<&Task, EngineError> {
        self.store().get(id).ok_or_else(|| invalid("no such task"))
    }

    // ---- sync ------------------------------------------------------------
    //
    // A standalone queue has no sync status, and refuses the requests that
    // only make sense for a shared one.

    /// How the shared queue stands; `None` for a standalone one.
    pub fn sync_status(&self) -> Option<&SyncStatus> {
        self.shared().map(Shared::status)
    }

    /// Bumped whenever `sync_status` changes, so a host can tell its
    /// listeners without comparing the status itself.
    pub fn sync_status_revision(&self) -> u64 {
        self.shared().map_or(0, Shared::status_revision)
    }

    /// The sync journal, for the host's worker to exchange with the server.
    pub fn sync_client_mut(&mut self) -> Option<&mut Client> {
        match &mut self.queue {
            Queue::Local(_) => None,
            Queue::Shared(shared) => Some(shared.client_mut()),
        }
    }

    /// The saved sync journal, for a recovery export.
    pub fn sync_envelope(&self) -> Option<&Envelope> {
        self.shared().map(|shared| shared.client().envelope())
    }

    /// Changes the server refused, readable, for the user to review.
    pub fn sync_recovery(&self) -> Vec<RecoveryChange> {
        self.shared()
            .map(|shared| shared.client().recovery_changes())
            .unwrap_or_default()
    }

    /// Put away one refused change, named by its command id.
    pub fn dismiss_sync_recovery(&mut self, command: &str) -> Result<Outcome<()>, EngineError> {
        self.shared_mut()?.dismiss_recovery(command)
    }

    /// Lift a suspension once its cause is repaired, so the worker sends the
    /// same commands again.
    pub fn retry_sync(&mut self) -> Result<Outcome<()>, EngineError> {
        self.shared_mut()?.retry()
    }

    /// Take in what the worker changed in the journal: the queue the views
    /// show and the status. Called after each exchange.
    pub fn refresh_sync(&mut self, clock: ClockSample) {
        if let Queue::Shared(shared) = &mut self.queue {
            shared.refresh(clock);
        }
    }

    fn shared(&self) -> Option<&Shared> {
        match &self.queue {
            Queue::Local(_) => None,
            Queue::Shared(shared) => Some(shared),
        }
    }

    fn shared_mut(&mut self) -> Result<&mut Shared, EngineError> {
        match &mut self.queue {
            Queue::Local(_) => Err(invalid("sync is not enabled")),
            Queue::Shared(shared) => Ok(shared),
        }
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
        let was_out = self.settings.in_outage();
        let settings_problem = self.settings.tick();
        let settings_outage_ended = was_out && !self.settings.in_outage();
        let flash = self.reminder.tick(
            self.queue.store(),
            self.settings.get(),
            now,
            local_time,
            random,
        );
        Tick {
            flash,
            settings_problem,
            settings_outage_ended,
        }
    }

    /// A flash now, whatever the quiet rules say, so long as Now holds a task.
    /// The wait for the next one starts over.
    pub fn flash_now(&mut self, now: u64, random: u32) -> Option<FlashEvent> {
        self.reminder
            .flash_now(self.queue.store(), self.settings.get(), now, random)
    }

    /// Why the next flash is held back, or how long until it arrives.
    pub fn flash_status(&self, now: u64, local_time: TimeOfDay) -> FlashStatus {
        self.reminder
            .status(self.queue.store(), self.settings.get(), now, local_time)
    }
}

/// A change to the standalone queue: always applied once it is saved.
fn applied<T>(outcome: Outcome<T>) -> Result<Submitted<T>, EngineError> {
    Ok(Submitted::Applied(outcome))
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
        let (engine, warning) = Engine::open(dir, || NOON).unwrap();
        assert!(warning.is_none());
        engine
    }

    fn ids(engine: &Engine, bucket: Bucket) -> Vec<u64> {
        engine.store().in_bucket(bucket).map(|t| t.id).collect()
    }

    /// A standalone request is applied, never pending.
    fn done<T: fmt::Debug>(result: Result<Submitted<T>, EngineError>) -> Outcome<T> {
        match result {
            Ok(Submitted::Applied(outcome)) => outcome,
            other => panic!("expected an applied change, got {other:?}"),
        }
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
        let id = done(engine.add("first #w", None)).value;
        assert!(engine.set_settings(r#"{"interval_min":5}"#).unwrap());
        assert!(engine.flush().is_none());

        let again = open(&dir);
        assert_eq!(again.store().get(id).unwrap().title, "first");
        assert_eq!(again.settings().interval_min, 5);
        assert_eq!(again.revision(), 0, "a fresh engine has saved nothing yet");

        fs::remove_dir_all(dir).unwrap();
    }

    /// The wait for the first flash starts once the files are loaded. Loading
    /// makes both files private, so the clock can see whether it came first.
    #[test]
    fn the_first_wait_is_timed_after_loading() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("first-wait");
        let mode = |name: &str| fs::metadata(dir.join(name)).unwrap().permissions().mode() & 0o777;
        fs::write(dir.join("tasks.json"), br#"{"next_id":1,"tasks":[]}"#).unwrap();
        fs::write(dir.join("settings.json"), br#"{"interval_min":1}"#).unwrap();
        for name in ["tasks.json", "settings.json"] {
            fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o644)).unwrap();
        }
        let (mut engine, _) = Engine::open(&dir, || {
            assert_eq!(mode("tasks.json"), 0o600, "the clock was read first");
            assert_eq!(mode("settings.json"), 0o600, "the clock was read first");
            NOON
        })
        .unwrap();
        done(engine.add("now", Some(Bucket::Now)));
        assert_eq!(engine.flash_status(NOON, noon()).remaining, Some(60));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unreadable_task_file_refuses_to_open_and_is_left_alone() {
        let dir = temp_dir("bad-tasks");
        fs::write(dir.join("tasks.json"), b"{ broken").unwrap();
        let error = Engine::open(&dir, || NOON).unwrap_err();
        assert!(error.to_string().contains("tasks.json"), "{error}");
        assert_eq!(fs::read(dir.join("tasks.json")).unwrap(), b"{ broken");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unreadable_settings_file_opens_with_the_defaults_and_a_warning() {
        let dir = temp_dir("bad-settings");
        fs::write(dir.join("settings.json"), b"[]").unwrap();
        let (engine, warning) = Engine::open(&dir, || NOON).unwrap();
        assert!(warning.unwrap().contains("settings.json"));
        assert_eq!(*engine.settings(), Settings::default());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn add_parses_markers_and_falls_back_to_the_chosen_bucket() {
        let dir = temp_dir("add");
        let mut engine = open(&dir);
        let current = done(engine.add("now #p", Some(Bucket::Now))).value;
        let next = done(engine.add("plain", None)).value;
        assert_eq!(ids(&engine, Bucket::Next), vec![next]);

        engine.update_settings(|s| s.default_bucket = Bucket::Side);
        let side = done(engine.add("side by default", None)).value;
        let later = done(engine.add("asked for later", Some(Bucket::Later))).value;
        let marked = done(engine.add("marker wins @next", Some(Bucket::Later))).value;
        assert_eq!(ids(&engine, Bucket::Side), vec![side]);
        assert_eq!(ids(&engine, Bucket::Later), vec![later]);
        assert_eq!(ids(&engine, Bucket::Next), vec![next, marked]);
        assert_eq!(engine.store().current().unwrap().id, current);
        assert_eq!(engine.store().current().unwrap().tag, Some(Tag::Personal));

        fs::remove_dir_all(dir).unwrap();
    }

    /// With nothing being done, a task added without a bucket is the one to
    /// do, whatever Settings says. A bucket that was asked for is kept.
    #[test]
    fn a_task_added_while_now_is_empty_is_the_current_one() {
        let dir = temp_dir("add-first");
        let mut engine = open(&dir);
        engine.update_settings(|s| s.default_bucket = Bucket::Later);
        let asked = done(engine.add("asked for", Some(Bucket::Next))).value;
        let marked = done(engine.add("marked @side", None)).value;
        assert!(engine.store().current().is_none(), "a named bucket is kept");
        assert_eq!(ids(&engine, Bucket::Next), vec![asked]);
        assert_eq!(ids(&engine, Bucket::Side), vec![marked]);

        let first = done(engine.add("first #w", None)).value;
        let current = engine.store().current().unwrap();
        assert_eq!((current.id, current.tag), (first, Some(Tag::Work)));
        assert!(current.started_at.is_some(), "and its clock runs");
        let second = done(engine.add("second", None)).value;
        assert_eq!(ids(&engine, Bucket::Now), vec![first]);
        assert_eq!(ids(&engine, Bucket::Later), vec![second], "Settings again");

        // Emptied by hand, as a completion with Next empty would leave it.
        done(engine.remove(first));
        let third = done(engine.add("third", None)).value;
        assert_eq!(ids(&engine, Bucket::Now), vec![third]);

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
        done(engine.add("only", None));
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

    #[test]
    fn a_window_s_requests_answer_whether_they_did_anything() {
        let dir = temp_dir("window");
        let mut engine = open(&dir);
        let a = done(engine.add("a", Some(Bucket::Next))).value;
        let b = done(engine.add("b", Some(Bucket::Next))).value;
        let side = done(engine.add("side", Some(Bucket::Side))).value;

        done(engine.cycle_tag(a));
        assert_eq!(engine.store().get(a).unwrap().tag, Some(Tag::Work));
        done(engine.rename(a, "  renamed "));
        assert_eq!(engine.store().get(a).unwrap().title, "renamed");
        assert_eq!(invalid_message(engine.rename(a, "   ")), "empty title");
        assert_eq!(engine.store().get(a).unwrap().title, "renamed");

        assert!(!done(engine.shift(a, -1)).value, "already first");
        assert!(done(engine.shift(a, 1)).value);
        assert_eq!(ids(&engine, Bucket::Next), vec![b, a]);

        assert!(done(engine.move_before(side, Bucket::Next, Some(a))).value);
        assert_eq!(ids(&engine, Bucket::Next), vec![b, side, a]);
        let revision = engine.revision();
        assert!(
            !done(engine.move_before(a, Bucket::Later, Some(b))).value,
            "b is not in Later"
        );
        assert_eq!(engine.revision(), revision, "a refused drop saves nothing");

        for message in [
            invalid_message(engine.cycle_tag(99)),
            invalid_message(engine.rename(99, "x")),
            invalid_message(engine.shift(99, 1)),
            invalid_message(engine.move_before(99, Bucket::Next, None)),
        ] {
            assert_eq!(message, "no such task");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    /// The D-Bus round trip of the queue, through the engine's own names.
    #[test]
    fn the_named_requests_do_what_the_interface_says() {
        let dir = temp_dir("requests");
        let mut engine = open(&dir);
        let a = done(engine.add("a", Some(Bucket::Now))).value;
        let b = done(engine.add("b", None)).value;
        let c = done(engine.add("c", Some(Bucket::Later))).value;

        assert!(done(engine.toggle_pause()).value);
        assert!(engine.store().current().unwrap().is_paused());
        done(engine.set_tag(b, Some(Tag::Work)));
        done(engine.set_tag(b, None));
        assert_eq!(engine.store().get(b).unwrap().tag, None);

        done(engine.promote(c));
        assert_eq!(engine.store().current().unwrap().id, c);
        assert_eq!(ids(&engine, Bucket::Next), vec![a, b]);

        done(engine.move_to(b, Bucket::Next, Some(0)));
        assert_eq!(ids(&engine, Bucket::Next), vec![b, a]);
        done(engine.move_to(b, Bucket::Next, None));
        assert_eq!(
            ids(&engine, Bucket::Next),
            vec![a, b],
            "no index is the end"
        );

        let completed = done(engine.complete_current()).value.unwrap();
        assert_eq!(completed.id, c);
        assert_eq!(engine.store().current().unwrap().id, a);
        assert!(done(engine.undo_complete(c)).value);
        assert_eq!(engine.store().current().unwrap().id, c);

        done(engine.complete(b));
        assert!(engine.store().get(b).is_none());
        done(engine.remove(a));
        assert!(!done(engine.undo_complete(b)).value, "stale after a remove");

        let state: serde_json::Value = serde_json::from_str(&engine.state_json()).unwrap();
        assert_eq!(state["current"]["id"], c);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn nothing_to_complete_or_pause_is_an_answer_not_an_error() {
        let dir = temp_dir("empty");
        let mut engine = open(&dir);
        assert!(done(engine.complete_current()).value.is_none());
        assert!(!done(engine.toggle_pause()).value);
        assert!(!done(engine.undo_complete(1)).value);
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

    /// The Shell extension finds the command id between the parentheses of
    /// the D-Bus error's message (`dbus.js`, `pendingCommand`).
    #[test]
    fn a_pending_message_carries_the_command_id_in_parentheses() {
        let command: Id = "0f8e6f8e-1c2d-4e5f-8a9b-0c1d2e3f4a5b".parse().unwrap();
        assert_eq!(
            pending_message(command),
            "Sent, waiting for confirmation (0f8e6f8e-1c2d-4e5f-8a9b-0c1d2e3f4a5b)"
        );
    }

    /// One tick drives both the settings writer and the reminder.
    #[test]
    fn a_tick_writes_the_settings_and_delivers_a_due_flash() {
        let dir = temp_dir("tick");
        let mut engine = open(&dir);
        done(engine.add("focus", Some(Bucket::Now)));
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
    fn the_tick_says_once_when_a_settings_outage_ends() {
        let dir = temp_dir("tick-outage-ends");
        let mut engine = open(&dir);
        fs::create_dir_all(dir.join("settings.json")).unwrap();
        engine.update_settings(|s| s.vary = false);
        assert!(engine.tick(NOON, noon(), 0).settings_problem.is_some());
        fs::remove_dir_all(dir.join("settings.json")).unwrap();
        let ended: Vec<u64> = (1..=40)
            .filter(|second| engine.tick(NOON + second, noon(), 0).settings_outage_ended)
            .collect();
        assert_eq!(ended.len(), 1, "once, on the write that worked: {ended:?}");
        assert!(
            !crate::load_settings(&dir.join("settings.json"))
                .unwrap()
                .vary,
            "and it wrote the change"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn flash_now_needs_a_current_task_and_restarts_the_wait() {
        let dir = temp_dir("flash-now");
        let mut engine = open(&dir);
        assert_eq!(engine.flash_status(NOON, noon()).hold, Hold::NoCurrentTask);
        assert!(engine.flash_now(NOON, 0).is_none());
        done(engine.add("!focus #p", None));
        let flash = engine.flash_now(NOON + 100, 0).unwrap();
        assert_eq!(flash.palette, crate::Palette::Orange);
        assert_eq!(
            engine.flash_status(NOON + 100, noon()).remaining,
            Some(15 * 60)
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
