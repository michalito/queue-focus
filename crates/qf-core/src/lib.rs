//! Core task model for queue-focus: four buckets (Now / Next / Later / Side),
//! optional work/personal tag, ordered by position, no history.

mod model;
mod store;

pub use model::{unix_now, Bucket, Completed, QuickAdd, Store, Tag, Task};
pub use store::{data_path, load, save, SaveError};
