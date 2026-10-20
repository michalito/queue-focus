//! The flash reminder: every so often, flash the current task across the
//! screen.
//!
//! The schedule is plain data. The host owns the clock, the calendar and the
//! entropy: once a second it hands `tick` the queue, the settings, the unix
//! time, the local time of day and a random number, and draws whatever flash
//! comes back. So every decision here is an ordinary function call with an
//! ordinary test.

use crate::{
    pick_style, short_elapsed, FlashStyle, Hold, Intensity, Palette, Settings, Store, TimeOfDay,
};

/// One flash, resolved: everything needed to draw it, so a shell that has
/// only just connected still draws the right thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashEvent {
    pub style: FlashStyle,
    pub intensity: Intensity,
    pub palette: Palette,
    pub title: String,
    /// Already formatted, so the flash and the top bar always read the same.
    pub timer: String,
}

impl FlashEvent {
    /// The `Flash` signal's payload, as the shell extension reads it.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "style": self.style.as_str(),
            "intensity": self.intensity.as_str(),
            "palette": self.palette.as_str(),
            "title": self.title,
            "timer": self.timer,
        })
        .to_string()
    }
}

/// One observation for the settings view. Both values describe the same instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlashStatus {
    pub hold: Hold,
    /// Seconds until the next flash; `None` while one is held back.
    pub remaining: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    /// Unix seconds at which the next flash is due.
    due: u64,
    /// The style of the flash before, which the next one avoids.
    last: Option<FlashStyle>,
}

impl Reminder {
    /// A reminder whose first flash is a full wait after `now`.
    pub fn new(now: u64, settings: &Settings) -> Reminder {
        Reminder {
            due: now.saturating_add(settings.interval_secs()),
            last: None,
        }
    }

    /// One second of the reminder: the flash to draw, if one is due.
    ///
    /// Whenever the reminder is held back the wait starts over, so a flash
    /// never arrives the moment a task is promoted or the pause ends.
    pub fn tick(
        &mut self,
        store: &Store,
        settings: &Settings,
        now: u64,
        local_time: TimeOfDay,
        random: u32,
    ) -> Option<FlashEvent> {
        let full_wait = now.saturating_add(settings.interval_secs());
        // A shorter interval or a backwards clock jump cannot strand a flash
        // beyond a full wait. Lengthening an interval keeps an earlier deadline.
        let due = self.due.min(full_wait);
        if !settings.hold(store.current(), local_time).is_none() {
            self.due = full_wait;
            None
        } else if now >= due {
            self.fire(store, settings, now, random)
        } else {
            self.due = due;
            None
        }
    }

    /// A manual preview. It bypasses the quiet rules, but still needs a
    /// current task, and it restarts the wait like any other flash.
    pub fn flash_now(
        &mut self,
        store: &Store,
        settings: &Settings,
        now: u64,
        random: u32,
    ) -> Option<FlashEvent> {
        self.fire(store, settings, now, random)
    }

    /// Why the next flash is held back, or how long until it arrives.
    pub fn status(
        &self,
        store: &Store,
        settings: &Settings,
        now: u64,
        local_time: TimeOfDay,
    ) -> FlashStatus {
        let hold = settings.hold(store.current(), local_time);
        FlashStatus {
            hold,
            remaining: hold.is_none().then(|| self.due.saturating_sub(now)),
        }
    }

    fn fire(
        &mut self,
        store: &Store,
        settings: &Settings,
        now: u64,
        random: u32,
    ) -> Option<FlashEvent> {
        let task = store.current()?;
        let style = pick_style(settings.vary, self.last, random);
        self.last = Some(style);
        self.due = now.saturating_add(settings.interval_secs());
        Some(FlashEvent {
            style,
            intensity: settings.intensity,
            palette: settings.color.palette(task.tag),
            title: task.title.clone(),
            timer: task
                .elapsed_secs(now)
                .map(|secs| short_elapsed(secs, task.is_paused()))
                .unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bucket, Tag, Task};

    const MIN: u64 = 60;

    fn at(hour: u32, minute: u32) -> TimeOfDay {
        TimeOfDay::new(hour, minute).unwrap()
    }

    /// A personal task that started at 400, a reminder made at 1000 with the
    /// default settings, and every flash it hands back.
    struct Fixture {
        store: Store,
        settings: Settings,
        local_time: TimeOfDay,
        reminder: Reminder,
        events: Vec<FlashEvent>,
    }

    impl Fixture {
        fn new() -> Self {
            let mut store = Store::new();
            store.add("call \"mum\"", Bucket::Now, Some(Tag::Personal));
            store.tasks[0].started_at = Some(400);
            let settings = Settings::default();
            let reminder = Reminder::new(1_000, &settings);
            Fixture {
                store,
                settings,
                local_time: at(12, 0),
                reminder,
                events: Vec::new(),
            }
        }

        fn current(&mut self) -> &mut Task {
            &mut self.store.tasks[0]
        }

        /// One tick at `now`. The random number is always 0, so a varied
        /// flash takes the first style it is allowed.
        fn step(&mut self, now: u64) {
            let event = self
                .reminder
                .tick(&self.store, &self.settings, now, self.local_time, 0);
            self.events.extend(event);
        }

        fn flash_now(&mut self, now: u64) -> bool {
            let event = self.reminder.flash_now(&self.store, &self.settings, now, 0);
            let fired = event.is_some();
            self.events.extend(event);
            fired
        }

        fn status(&self, now: u64) -> FlashStatus {
            self.reminder
                .status(&self.store, &self.settings, now, self.local_time)
        }

        fn remaining(&self, now: u64) -> Option<u64> {
            self.status(now).remaining
        }
    }

    #[test]
    fn scheduled_flash_carries_the_current_task_and_restarts_the_wait() {
        let mut f = Fixture::new();
        assert_eq!(f.remaining(1_000), Some(15 * MIN));
        f.step(1_899);
        assert!(f.events.is_empty());
        assert_eq!(f.remaining(1_899), Some(1));
        f.step(1_900);
        let event = f.events[0].clone();
        assert_eq!(event.title, "call \"mum\"");
        assert_eq!(event.timer, "25m");
        assert_eq!(event.palette, Palette::Orange);
        assert_eq!(event.intensity, Settings::default().intensity);
        assert_eq!(event.style, FlashStyle::ALL[0]);
        assert_eq!(f.remaining(1_900), Some(15 * MIN));
        f.step(1_900);
        assert_eq!(f.events.len(), 1, "no duplicate at the same instant");
    }

    #[test]
    fn pause_resume_and_preview_share_the_schedule() {
        let mut f = Fixture::new();
        f.current().paused_at = Some(1_000);
        f.step(1_900);
        assert_eq!(
            f.status(1_900),
            FlashStatus {
                hold: Hold::Paused,
                remaining: None
            }
        );
        assert!(f.events.is_empty());
        assert!(f.flash_now(1_900), "manual preview bypasses pause");
        assert_eq!(f.events[0].timer, "10m ⏸");
        {
            let task = f.current();
            task.paused_at = None;
            task.started_at = Some(1_300); // resume preserves the ten elapsed minutes
        }
        f.step(1_901);
        assert_eq!(f.remaining(1_901), Some(899));
        f.step(2_800);
        assert_eq!(f.events.len(), 2);
        assert_eq!(f.events[1].timer, "25m");
        assert_ne!(f.events[0].style, f.events[1].style);
    }

    #[test]
    fn empty_now_refuses_preview_without_touching_the_style_history() {
        let mut f = Fixture::new();
        let task = f.store.tasks.remove(0);
        f.step(1_900);
        assert_eq!(
            f.status(1_900),
            FlashStatus {
                hold: Hold::NoCurrentTask,
                remaining: None
            }
        );
        let before = f.reminder.clone();
        assert!(!f.flash_now(1_900));
        assert_eq!(f.reminder, before, "a refused preview changes nothing");
        f.store.tasks.push(task);
        f.step(1_901);
        assert_eq!(f.remaining(1_901), Some(899));
        f.step(2_800);
        assert_eq!(f.events.len(), 1);
        assert_eq!(
            f.events[0].style,
            FlashStyle::ALL[0],
            "nothing was avoided, so the first style is allowed"
        );
    }

    #[test]
    fn quiet_hours_hold_automatic_flashes_but_allow_preview() {
        let mut f = Fixture::new();
        f.settings.quiet_hours = true;
        f.settings.quiet_from = at(22, 0);
        f.settings.quiet_to = at(6, 0);
        f.step(1_900);
        assert_eq!(f.status(1_900).hold, Hold::OutsideHours);
        assert_eq!(f.remaining(1_900), None);
        assert!(f.flash_now(1_900));
        f.local_time = at(23, 0);
        f.step(1_901);
        assert_eq!(f.remaining(1_901), Some(899));
        f.step(2_800);
        assert_eq!(f.events.len(), 2);
        f.local_time = at(6, 0);
        f.step(3_700);
        assert_eq!(f.events.len(), 2, "end of the allowed window is exclusive");
    }

    #[test]
    fn disabling_quiet_rules_allows_a_paused_task_to_flash() {
        let mut f = Fixture::new();
        f.current().paused_at = Some(1_000);
        f.settings.quiet_paused = false;
        f.step(1_900);
        assert_eq!(f.events[0].timer, "10m ⏸");
        assert_eq!(f.status(1_900).hold, Hold::None);
    }

    #[test]
    fn interval_edits_and_manual_preview_adjust_the_next_reminder() {
        let mut f = Fixture::new();
        f.settings.interval_min = 5;
        f.step(1_100);
        assert_eq!(f.remaining(1_100), Some(5 * MIN));
        f.settings.interval_min = 90;
        f.step(1_200);
        assert_eq!(
            f.remaining(1_200),
            Some(200),
            "lengthening keeps an earlier deadline"
        );
        assert!(f.flash_now(1_200));
        assert_eq!(f.remaining(1_200), Some(90 * MIN));
        f.step(1_400);
        assert_eq!(f.events.len(), 1, "preview replaced the old deadline");
        f.step(6_600);
        assert_eq!(f.events.len(), 2);
    }

    #[test]
    fn clock_jumps_emit_once_and_do_not_strand_the_next_reminder() {
        let mut f = Fixture::new();
        f.step(100_000);
        assert_eq!(f.events.len(), 1);
        assert_eq!(f.remaining(100_000), Some(15 * MIN));
        f.step(1_000);
        assert_eq!(f.remaining(1_000), Some(15 * MIN));
        assert_eq!(f.events.len(), 1);
        f.step(1_900);
        assert_eq!(f.events.len(), 2);
        f.step(u64::MAX);
        assert_eq!(
            f.remaining(u64::MAX),
            Some(0),
            "deadline arithmetic saturates"
        );
    }

    #[test]
    fn replacing_the_task_and_settings_changes_the_next_event() {
        let mut f = Fixture::new();
        assert!(f.flash_now(1_000));
        {
            let task = f.current();
            task.id = 2;
            task.title = "replacement".into();
            task.tag = Some(Tag::Work);
            task.started_at = Some(1_300);
        }
        f.settings.vary = false;
        f.settings.intensity = Intensity::Strong;
        f.step(1_900);
        let event = f.events[1].clone();
        assert_eq!(event.title, "replacement");
        assert_eq!(event.timer, "10m");
        assert_eq!(event.palette, Palette::Blue);
        assert_eq!(event.style, FlashStyle::FIXED);
        assert_eq!(event.intensity, Intensity::Strong);
        assert!(f.flash_now(1_900));
        assert_eq!(f.events[2].style, FlashStyle::FIXED);
        f.settings.vary = true;
        assert!(f.flash_now(1_900));
        assert_ne!(f.events[3].style, FlashStyle::FIXED);
    }

    /// The deadline is committed before the flash is handed back, so whatever
    /// the host does while drawing it sees the wait that follows it.
    #[test]
    fn a_flash_comes_back_with_its_deadline_already_committed() {
        let mut f = Fixture::new();
        f.step(1_900);
        assert_eq!(f.events.len(), 1);
        f.settings.interval_min = 90;
        assert_eq!(f.remaining(1_900), Some(15 * MIN));
    }

    #[test]
    fn a_flash_carries_everything_needed_to_draw_it() {
        let mut f = Fixture::new();
        f.settings.vary = false;
        f.settings.intensity = Intensity::Strong;
        assert!(f.flash_now(4_120));
        let json: serde_json::Value = serde_json::from_str(&f.events[0].to_json()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "style": "edges",
                "intensity": "strong",
                "palette": "orange",
                "title": "call \"mum\"",
                "timer": "1h02",
            })
        );
    }
}
