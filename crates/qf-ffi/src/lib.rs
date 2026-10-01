//! The queue-focus engine for Swift. Every method maps one to one onto
//! `qf_core::Engine`; this crate only converts types and collects problems.
//!
//! `scripts/build-mac-core.sh` builds it as a universal static library and
//! generates the `QfCore` Swift module from it.

mod types;

pub use types::*;

use qf_core::{Engine, EngineError, Outcome};
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

uniffi::setup_scaffolding!();

/// Problems held for the next `tick` or `flush`. More than this many between
/// two ticks means something is badly wrong, and the first say why.
const MAX_PENDING_PROBLEMS: usize = 16;

/// Why a request was refused. Nothing changed in either case.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum QfError {
    /// A file could not be read or a change could not be saved.
    Persistence { message: String },
    /// The arguments could not be used, or named a task that is not there.
    InvalidArgument { message: String },
}

impl fmt::Display for QfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QfError::Persistence { message } | QfError::InvalidArgument { message } => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for QfError {}

impl From<EngineError> for QfError {
    fn from(error: EngineError) -> Self {
        let message = error.to_string();
        match error {
            EngineError::InvalidArgument(_) => QfError::InvalidArgument { message },
            EngineError::Persistence(_) => QfError::Persistence { message },
        }
    }
}

fn invalid(message: String) -> QfError {
    QfError::InvalidArgument { message }
}

/// The queue, the settings and the reminder, saved to one data directory.
/// Safe to share between threads, though the app keeps every call on the
/// main actor.
#[derive(uniffi::Object)]
pub struct QueueEngine {
    state: Mutex<State>,
    open_warning: Option<String>,
}

struct State {
    engine: Engine,
    problems: Vec<String>,
}

impl State {
    fn report(&mut self, problem: String) {
        if self.problems.len() < MAX_PENDING_PROBLEMS {
            self.problems.push(problem);
        }
    }

    /// Hand back a change's value; a durability warning waits for the tick.
    fn settle<T>(&mut self, result: Result<Outcome<T>, EngineError>) -> Result<T, QfError> {
        let (value, warning) = result?.into_parts();
        if let Some(warning) = warning {
            self.report(warning.to_string());
        }
        Ok(value)
    }
}

impl QueueEngine {
    /// A panic elsewhere must not lock the app out of its queue, so a
    /// poisoned lock is taken as it stands: every change either saved or
    /// rolled back before the panic could happen.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn change<T>(
        &self,
        f: impl FnOnce(&mut Engine) -> Result<Outcome<T>, EngineError>,
    ) -> Result<T, QfError> {
        let mut state = self.lock();
        let result = f(&mut state.engine);
        state.settle(result)
    }
}

#[uniffi::export]
impl QueueEngine {
    /// Open the task and settings files in `dir`, creating nothing until the
    /// first change. A task file that cannot be read is an error, so the app
    /// can refuse to start rather than replace it. A settings file that
    /// cannot be read leaves the defaults in place and an `open_warning`.
    #[uniffi::constructor]
    pub fn new(dir: String) -> Result<Arc<Self>, QfError> {
        let (engine, open_warning) =
            Engine::open(Path::new(&dir), qf_core::unix_now).map_err(|e| QfError::Persistence {
                message: e.to_string(),
            })?;
        Ok(Arc::new(QueueEngine {
            state: Mutex::new(State {
                engine,
                problems: Vec::new(),
            }),
            open_warning,
        }))
    }

    /// Why the settings file could not be read at open, if it could not.
    pub fn open_warning(&self) -> Option<String> {
        self.open_warning.clone()
    }

    // ---- reading -------------------------------------------------------

    pub fn snapshot(&self) -> QueueSnapshot {
        let state = self.lock();
        QueueSnapshot::of(state.engine.store(), state.engine.revision())
    }

    pub fn settings(&self) -> QueueSettings {
        QueueSettings::from(self.lock().engine.settings())
    }

    /// When the next flash comes, or why it will not.
    pub fn flash_status(&self, now: u64, local_time: TimeOfDay) -> Result<FlashStatus, QfError> {
        let local_time = local_time.try_into().map_err(invalid)?;
        Ok(self.lock().engine.flash_status(now, local_time).into())
    }

    // ---- the queue -----------------------------------------------------

    /// Parse the add markers and create a task. Without a bucket it goes
    /// where the settings say; a marker in the text wins either way. Returns
    /// the new task's id.
    pub fn add(&self, text: String, bucket: Option<Bucket>) -> Result<u64, QfError> {
        self.change(|e| e.add(&text, bucket.map(Into::into)))
    }

    /// Delete the current task and pull the head of Next into Now. Returns the
    /// task that was completed, or nothing when Now was empty.
    pub fn complete_current(&self) -> Result<Option<QueueTask>, QfError> {
        let task = self.change(|e| e.complete_current())?;
        Ok(task.as_ref().map(QueueTask::from))
    }

    /// Mark a task done: the current task as by `complete_current`, any
    /// other simply deleted. Either way `undo_complete` can reverse it.
    pub fn complete(&self, id: u64) -> Result<(), QfError> {
        self.change(|e| e.complete(id))
    }

    /// Reverse the most recent completion. `false` when it was not `id`'s or
    /// anything else has changed since.
    pub fn undo_complete(&self, id: u64) -> Result<bool, QfError> {
        self.change(|e| e.undo_complete(id))
    }

    /// Pause or resume the current task's timer. `false` when Now is empty.
    pub fn toggle_pause(&self) -> Result<bool, QfError> {
        self.change(|e| e.toggle_pause())
    }

    /// Make a task current. The one it replaces leads Next.
    pub fn promote(&self, id: u64) -> Result<(), QfError> {
        self.change(|e| e.promote(id))
    }

    /// Delete a task for good; unlike `complete`, it cannot be undone.
    pub fn remove(&self, id: u64) -> Result<(), QfError> {
        self.change(|e| e.remove(id))
    }

    /// Move a task to a zero-based position in a bucket, or to its end
    /// without one. Into Now it is a promotion, whatever the index.
    pub fn move_task(&self, id: u64, bucket: Bucket, index: Option<u32>) -> Result<(), QfError> {
        let index = index.map(|i| i as usize);
        self.change(|e| e.move_to(id, bucket.into(), index))
    }

    /// Drop a task before the row `before` of a bucket, or at its end without
    /// one. `false` when that row has gone or left the bucket since it was
    /// drawn, and nothing moved.
    pub fn move_before(
        &self,
        id: u64,
        bucket: Bucket,
        before: Option<u64>,
    ) -> Result<bool, QfError> {
        self.change(|e| e.move_before(id, bucket.into(), before))
    }

    /// Move a task up (negative) or down within its bucket. `false` when it
    /// is already at that edge.
    pub fn shift(&self, id: u64, delta: i32) -> Result<bool, QfError> {
        self.change(|e| e.shift(id, delta))
    }

    pub fn set_tag(&self, id: u64, tag: Option<TaskTag>) -> Result<(), QfError> {
        self.change(|e| e.set_tag(id, tag.map(Into::into)))
    }

    /// No tag, work, personal, and round again.
    pub fn cycle_tag(&self, id: u64) -> Result<(), QfError> {
        self.change(|e| e.cycle_tag(id))
    }

    /// An empty title is refused and the old one stays.
    pub fn rename(&self, id: u64, title: String) -> Result<(), QfError> {
        self.change(|e| e.rename(id, &title))
    }

    // ---- settings ------------------------------------------------------

    /// Replace the settings. An interval out of range is pulled into range.
    /// Returns whether anything changed; the file catches up on a tick.
    pub fn set_settings(&self, settings: QueueSettings) -> Result<bool, QfError> {
        let settings: qf_core::Settings = settings.try_into().map_err(invalid)?;
        Ok(self
            .lock()
            .engine
            .update_settings(|current| *current = settings))
    }

    // ---- the clock -----------------------------------------------------

    /// Call once a second. `now` is the unix time, `local_time` the time of
    /// day the quiet hours read, and `random` any number.
    pub fn tick(
        &self,
        now: u64,
        local_time: TimeOfDay,
        random: u32,
    ) -> Result<TickResult, QfError> {
        let local_time = local_time.try_into().map_err(invalid)?;
        let mut state = self.lock();
        let tick = state.engine.tick(now, local_time, random);
        if let Some(problem) = tick.settings_problem {
            state.report(problem);
        }
        Ok(TickResult {
            flash: tick.flash.map(Into::into),
            problems: std::mem::take(&mut state.problems),
        })
    }

    /// A flash now, whatever the quiet rules say, so long as Now holds a
    /// task. The wait for the next one starts over.
    pub fn flash_now(&self, now: u64, random: u32) -> Option<FlashEvent> {
        self.lock().engine.flash_now(now, random).map(Into::into)
    }

    /// Write the settings if the file is behind. Call before the app quits.
    /// Returns every problem not yet reported.
    pub fn flush(&self) -> Vec<String> {
        let mut state = self.lock();
        if let Some(problem) = state.engine.flush() {
            state.report(problem);
        }
        std::mem::take(&mut state.problems)
    }
}

/// Where the data lives: `$XDG_DATA_HOME/queue-focus` when that is set to an
/// absolute path, otherwise `~/Library/Application Support/queue-focus`.
#[uniffi::export]
pub fn default_data_dir() -> String {
    qf_core::data_dir().to_string_lossy().into_owned()
}

/// The menu bar's clock: `"12m"`, `"1h02"`, and `" ⏸"` while paused.
#[uniffi::export]
pub fn short_elapsed(secs: u64, paused: bool) -> String {
    qf_core::short_elapsed(secs, paused)
}

/// The windows' clock: `"00:00"`, `"1:02:05"`.
#[uniffi::export]
pub fn long_elapsed(secs: u64) -> String {
    qf_core::long_elapsed(secs)
}

/// Seconds on a task's clock at `now`, frozen while paused; `None` for a task
/// that is not current.
#[uniffi::export]
pub fn elapsed_secs(task: QueueTask, now: u64) -> Option<u64> {
    qf_core::Task::from(task).elapsed_secs(now)
}

/// Shown in place of the countdown while the reminder holds back.
#[uniffi::export]
pub fn hold_reason_text(reason: HoldReason) -> String {
    qf_core::Hold::from(reason).reason().to_string()
}

#[uniffi::export]
pub fn interval_bounds() -> IntervalBounds {
    IntervalBounds {
        min: qf_core::INTERVAL_MIN,
        max: qf_core::INTERVAL_MAX,
    }
}

/// Titles are cut to this many characters.
#[uniffi::export]
pub fn max_title_chars() -> u32 {
    qf_core::MAX_TITLE_CHARS as u32
}

#[cfg(test)]
mod tests;
