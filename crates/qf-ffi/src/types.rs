//! The records and enums Swift sees, and their conversions to the core's.
//!
//! Names are chosen so they never shadow Swift or SwiftUI: `QueueTask` not
//! `Task` (Swift Concurrency), `QueueSettings` not `Settings` (the SwiftUI
//! scene), `TaskTag` not `Tag` (Swift Testing).

use qf_core::{Hold, Store};

macro_rules! mirror_enum {
    ($(#[$meta:meta])* $name:ident => $core:path { $($variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
        pub enum $name {
            $($variant),+
        }

        impl From<$core> for $name {
            fn from(value: $core) -> Self {
                match value {
                    $(<$core>::$variant => $name::$variant),+
                }
            }
        }

        impl From<$name> for $core {
            fn from(value: $name) -> Self {
                match value {
                    $($name::$variant => <$core>::$variant),+
                }
            }
        }
    };
}

mirror_enum!(
    /// Now holds the current task; Next is the queue; Later the backlog;
    /// Side what carries on beside the current task.
    Bucket => qf_core::Bucket { Now, Next, Later, Side }
);
mirror_enum!(TaskTag => qf_core::Tag { Work, Personal });
mirror_enum!(Intensity => qf_core::Intensity { Subtle, Normal, Strong });
mirror_enum!(
    /// `Tag` follows the current task's tag; untagged flashes blue.
    FlashColor => qf_core::FlashColor { Tag, Blue, Orange }
);
mirror_enum!(
    /// The resolved colour of one flash.
    Palette => qf_core::Palette { Blue, Orange }
);
mirror_enum!(FlashStyle => qf_core::FlashStyle { Wash, Wash2, Edges, EdgesSoft, Topbar, TopbarBeam });
mirror_enum!(Theme => qf_core::Theme { System, Light, Dark });

/// Why the reminder is holding back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HoldReason {
    /// Now is empty: there is nothing to be reminded of.
    NoCurrentTask,
    /// The current task's timer is paused.
    Paused,
    /// The time of day is outside the hours the user chose.
    OutsideHours,
}

impl From<HoldReason> for Hold {
    fn from(reason: HoldReason) -> Self {
        match reason {
            HoldReason::NoCurrentTask => Hold::NoCurrentTask,
            HoldReason::Paused => Hold::Paused,
            HoldReason::OutsideHours => Hold::OutsideHours,
        }
    }
}

/// A time of day, as a clock face shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
}

impl From<qf_core::TimeOfDay> for TimeOfDay {
    fn from(time: qf_core::TimeOfDay) -> Self {
        // Both fit: hours run to 23 and minutes to 59.
        TimeOfDay {
            hour: time.hour() as u8,
            minute: time.minute() as u8,
        }
    }
}

impl TryFrom<TimeOfDay> for qf_core::TimeOfDay {
    type Error = String;

    fn try_from(time: TimeOfDay) -> Result<Self, String> {
        qf_core::TimeOfDay::new(time.hour.into(), time.minute.into())
            .ok_or_else(|| format!("{:02}:{:02} is not a time of day", time.hour, time.minute))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QueueTask {
    pub id: u64,
    pub title: String,
    pub bucket: Bucket,
    pub tag: Option<TaskTag>,
    /// Unix seconds.
    pub created_at: u64,
    /// Unix seconds since the task became current; only the current task has one.
    pub started_at: Option<u64>,
    /// Unix seconds at which its timer was paused; `None` while it runs.
    pub paused_at: Option<u64>,
}

impl From<&qf_core::Task> for QueueTask {
    fn from(task: &qf_core::Task) -> Self {
        QueueTask {
            id: task.id,
            title: task.title.clone(),
            bucket: task.bucket.into(),
            tag: task.tag.map(Into::into),
            created_at: task.created_at,
            started_at: task.started_at,
            paused_at: task.paused_at,
        }
    }
}

impl From<QueueTask> for qf_core::Task {
    fn from(task: QueueTask) -> Self {
        qf_core::Task {
            id: task.id,
            title: task.title,
            bucket: task.bucket.into(),
            tag: task.tag.map(Into::into),
            created_at: task.created_at,
            started_at: task.started_at,
            paused_at: task.paused_at,
        }
    }
}

/// The queue as the views show it: the current task, then Side, Next and
/// Later in order, and how many changes have been saved so far.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QueueSnapshot {
    pub current: Option<QueueTask>,
    pub side: Vec<QueueTask>,
    pub next: Vec<QueueTask>,
    pub later: Vec<QueueTask>,
    pub revision: u64,
}

impl QueueSnapshot {
    pub(crate) fn of(store: &Store, revision: u64) -> Self {
        let list = |bucket| store.in_bucket(bucket).map(QueueTask::from).collect();
        QueueSnapshot {
            current: store.current().map(QueueTask::from),
            side: list(qf_core::Bucket::Side),
            next: list(qf_core::Bucket::Next),
            later: list(qf_core::Bucket::Later),
            revision,
        }
    }
}

/// Everything the user can change, as `settings.json` holds it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QueueSettings {
    /// Minutes between flashes; see `interval_bounds`.
    pub interval_min: u32,
    /// Pick a different style each time rather than always the edges.
    pub vary: bool,
    pub intensity: Intensity,
    pub color: FlashColor,
    /// Hold flashes back while the current task's timer is paused.
    pub quiet_paused: bool,
    /// Only flash between `quiet_from` and `quiet_to`.
    pub quiet_hours: bool,
    pub quiet_from: TimeOfDay,
    pub quiet_to: TimeOfDay,
    pub theme: Theme,
    /// Show the elapsed time beside the title in the menu bar.
    pub show_timer: bool,
    /// Where an added task without a bucket marker goes.
    pub default_bucket: Bucket,
}

impl From<&qf_core::Settings> for QueueSettings {
    fn from(s: &qf_core::Settings) -> Self {
        QueueSettings {
            interval_min: s.interval_min,
            vary: s.vary,
            intensity: s.intensity.into(),
            color: s.color.into(),
            quiet_paused: s.quiet_paused,
            quiet_hours: s.quiet_hours,
            quiet_from: s.quiet_from.into(),
            quiet_to: s.quiet_to.into(),
            theme: s.theme.into(),
            show_timer: s.show_timer,
            default_bucket: s.default_bucket.into(),
        }
    }
}

impl TryFrom<QueueSettings> for qf_core::Settings {
    type Error = String;

    fn try_from(s: QueueSettings) -> Result<Self, String> {
        Ok(qf_core::Settings {
            interval_min: s.interval_min,
            vary: s.vary,
            intensity: s.intensity.into(),
            color: s.color.into(),
            quiet_paused: s.quiet_paused,
            quiet_hours: s.quiet_hours,
            quiet_from: s.quiet_from.try_into()?,
            quiet_to: s.quiet_to.try_into()?,
            theme: s.theme.into(),
            show_timer: s.show_timer,
            default_bucket: s.default_bucket.into(),
        })
    }
}

/// One flash, resolved: everything needed to draw it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FlashEvent {
    pub style: FlashStyle,
    pub intensity: Intensity,
    pub palette: Palette,
    pub title: String,
    /// Already formatted, so the flash and the menu bar read the same.
    pub timer: String,
}

impl From<qf_core::FlashEvent> for FlashEvent {
    fn from(event: qf_core::FlashEvent) -> Self {
        FlashEvent {
            style: event.style.into(),
            intensity: event.intensity.into(),
            palette: event.palette.into(),
            title: event.title,
            timer: event.timer,
        }
    }
}

/// When the next flash comes, or why it will not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FlashStatus {
    Scheduled { remaining_secs: u64 },
    Held { reason: HoldReason },
}

impl From<qf_core::FlashStatus> for FlashStatus {
    fn from(status: qf_core::FlashStatus) -> Self {
        let reason = match status.hold {
            Hold::None => {
                return FlashStatus::Scheduled {
                    remaining_secs: status.remaining.unwrap_or(0),
                }
            }
            Hold::NoCurrentTask => HoldReason::NoCurrentTask,
            Hold::Paused => HoldReason::Paused,
            Hold::OutsideHours => HoldReason::OutsideHours,
        };
        FlashStatus::Held { reason }
    }
}

/// What one second of the engine produced.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TickResult {
    /// A flash to draw now.
    pub flash: Option<FlashEvent>,
    /// Failures to report, one notification each: changes saved without
    /// being made crash-safe, and settings that could not be written.
    pub problems: Vec<String>,
}

/// The range of the "flash every" slider, in minutes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct IntervalBounds {
    pub min: u32,
    pub max: u32,
}
