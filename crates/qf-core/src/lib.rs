//! Core task model for queue-focus: four buckets (Now / Next / Later / Side),
//! optional work/personal tag, ordered by position, no history. Plus the user
//! settings both the app and the shell extension read.

mod model;
mod settings;
mod store;

pub use model::{
    unix_now, Bucket, Completed, QuickAdd, Store, Tag, Task, MAX_QUICK_ADD_BYTES, MAX_TITLE_CHARS,
};
pub use settings::{
    pick_style, within_window, FlashColor, FlashStyle, Hold, Intensity, Palette, Settings, Theme,
    TimeOfDay, INTERVAL_MAX, INTERVAL_MIN,
};
pub use store::{
    data_dir, data_path, load, load_settings, save, save_settings, settings_path, SaveError,
};
