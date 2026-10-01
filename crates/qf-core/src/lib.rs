//! Core task model for queue-focus: four buckets (Now / Next / Later / Side),
//! of which Now holds the one current task; optional work/personal tag,
//! ordered by position, no history. Plus the user
//! settings both the app and the shell extension read.

mod elapsed;
mod model;
mod reminder;
mod settings;
mod settings_store;
mod store;
mod tasks;

pub use elapsed::{long_elapsed, short_elapsed};

pub use model::{
    unix_now, Bucket, Completed, QuickAdd, Store, Tag, Task, MAX_QUICK_ADD_BYTES, MAX_TITLE_CHARS,
};
pub use reminder::{FlashEvent, FlashStatus, Reminder};
pub use settings::{
    pick_style, within_window, FlashColor, FlashStyle, Hold, Intensity, Palette, Settings, Theme,
    TimeOfDay, INTERVAL_MAX, INTERVAL_MIN,
};
pub use settings_store::SettingsStore;
pub use store::{
    data_dir, data_path, load, load_settings, save, save_settings, settings_path, SaveError,
};
pub use tasks::{DurabilityWarning, Outcome, Tasks};
