//! Core of queue-focus: four buckets (Now / Next / Later / Side), of which Now
//! holds the one current task; optional work/personal tag, ordered by
//! position, no history. Plus the user settings, the flash reminder, and the
//! `Engine` that saves every change and that each platform's app drives.
//!
//! Nothing here has a clock, randomness or a toolkit of its own: the host
//! supplies the time and a random number, so every decision is an ordinary
//! function with an ordinary test.

mod elapsed;
mod engine;
mod model;
mod reminder;
mod settings;
mod settings_store;
mod store;
mod tasks;

pub use elapsed::{long_elapsed, short_elapsed};
pub use engine::{Engine, EngineError, Tick, MAX_SETTINGS_PATCH_BYTES};

pub use model::{
    unix_now, Bucket, Completed, QuickAdd, Store, Tag, Task, MAX_QUICK_ADD_BYTES, MAX_TITLE_CHARS,
};
pub use reminder::{FlashEvent, FlashStatus};
pub use settings::{
    pick_style, within_window, FlashColor, FlashStyle, Hold, Intensity, Palette, Settings, Theme,
    TimeOfDay, INTERVAL_MAX, INTERVAL_MIN,
};
pub use store::{data_dir, load, load_settings, save, save_settings, SaveError};
pub use tasks::{DurabilityWarning, Outcome};
