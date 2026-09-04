//! The reminder's clock: every so often, ask the shell to flash the current
//! task across the screen.
//!
//! The service owns the schedule rather than the extension, because the
//! service is the one that knows the queue, the pause, and the settings — and
//! it is running whether or not a window is open. What it sends is one
//! self-contained `Flash` signal per flash: everything needed to draw it, so a
//! shell that has only just connected still draws the right thing.

use crate::settings::{local_time_of_day, SharedSettings};
use crate::state::SharedState;
use gtk::glib;
use qf_core::{pick_style, FlashStyle, Hold, Intensity, Palette};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub type SharedFlash = Rc<FlashClock>;

/// Puts one flash on the bus.
type Emitter = Rc<dyn Fn(&FlashEvent)>;

/// One flash, as the extension receives it.
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

pub struct FlashClock {
    state: SharedState,
    settings: SharedSettings,
    /// Unix seconds at which the next flash is due.
    due: Cell<u64>,
    /// The style used last, so the next one differs.
    last: Cell<Option<FlashStyle>>,
    /// Set once the D-Bus object exists; before that a flash has nowhere to go.
    emit: RefCell<Option<Emitter>>,
}

impl FlashClock {
    pub fn new(state: SharedState, settings: SharedSettings) -> SharedFlash {
        let interval = settings.get().interval_secs();
        let clock = Rc::new(FlashClock {
            state,
            settings,
            due: Cell::new(qf_core::unix_now().saturating_add(interval)),
            last: Cell::new(None),
            emit: RefCell::new(None),
        });
        let weak = Rc::downgrade(&clock);
        glib::timeout_add_seconds_local(1, move || match weak.upgrade() {
            Some(clock) => {
                clock.tick();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        clock
    }

    /// Where a flash goes. Set by `dbus::export`.
    pub fn set_emitter(&self, f: impl Fn(&FlashEvent) + 'static) {
        *self.emit.borrow_mut() = Some(Rc::new(f));
    }

    /// Why the next flash is being held back, if it is.
    pub fn hold(&self) -> Hold {
        self.settings
            .get()
            .hold(self.state.store().current(), local_time_of_day())
    }

    /// Seconds until the next flash, or `None` while it is held back.
    pub fn remaining(&self) -> Option<u64> {
        self.hold()
            .is_none()
            .then(|| self.due.get().saturating_sub(qf_core::unix_now()))
    }

    /// Flash right now because the user asked to see one. The quiet rules are
    /// the reminder's own manners and do not apply to a request; an empty Now
    /// still has nothing to show, so that alone refuses.
    pub fn flash_now(&self) -> bool {
        self.fire()
    }

    fn tick(&self) {
        let now = qf_core::unix_now();
        let interval = self.settings.get().interval_secs();
        let (due, fire) = advance(now, self.due.get(), interval, !self.hold().is_none());
        self.due.set(due);
        if fire {
            self.fire();
        }
    }

    /// Send one flash. `false` when there is nothing in Now to flash.
    fn fire(&self) -> bool {
        let now = qf_core::unix_now();
        let event = {
            let store = self.state.store();
            let Some(task) = store.current() else {
                return false;
            };
            let settings = self.settings.get();
            // A multiple of both pool sizes — five styles to choose between,
            // or six when nothing has flashed yet — so none is favoured.
            let style = pick_style(
                settings.vary,
                self.last.get(),
                glib::random_int_range(0, 30) as u32,
            );
            FlashEvent {
                style,
                intensity: settings.intensity,
                palette: settings.color.palette(task.tag),
                title: task.title.clone(),
                timer: task
                    .elapsed_secs(now)
                    .map(|secs| short_elapsed(secs, task.is_paused()))
                    .unwrap_or_default(),
            }
        };
        self.last.set(Some(event.style));
        self.due
            .set(now.saturating_add(self.settings.get().interval_secs()));
        let emit = self.emit.borrow().clone();
        if let Some(emit) = emit {
            emit(&event);
        }
        true
    }
}

/// Decide one second of the clock: the time the next flash is due, and whether
/// to flash right now.
///
/// A held-back reminder keeps pushing the next flash away, so the wait starts
/// over once there is something to be reminded of rather than firing the
/// instant a task appears.
fn advance(now: u64, due: u64, interval: u64, held: bool) -> (u64, bool) {
    let full_wait = now.saturating_add(interval);
    // A shortened interval — or a clock that jumped forward — must not park
    // the next flash further away than the wait the user asked for.
    let due = due.min(full_wait);
    if held {
        return (full_wait, false);
    }
    if now >= due {
        return (full_wait, true);
    }
    (due, false)
}

/// The top bar's clock: `"12m"`, `"1h02"`, and `" ⏸"` while paused.
pub fn short_elapsed(secs: u64, paused: bool) -> String {
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    let t = if h > 0 {
        format!("{h}h{m:02}")
    } else {
        format!("{m}m")
    };
    if paused {
        format!("{t} ⏸")
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60;

    #[test]
    fn the_clock_waits_out_the_interval_and_then_flashes() {
        let interval = 15 * MIN;
        let start = 1_000;
        let (due, fire) = advance(start, start + interval, interval, false);
        assert_eq!((due, fire), (start + interval, false));

        let one_short = start + interval - 1;
        let (due, fire) = advance(one_short, due, interval, false);
        assert_eq!((due, fire), (start + interval, false), "not yet");

        let (due, fire) = advance(start + interval, due, interval, false);
        assert!(fire);
        assert_eq!(
            due,
            start + 2 * interval,
            "the next one is a full wait away"
        );
    }

    #[test]
    fn a_held_back_reminder_starts_its_wait_over() {
        let interval = 15 * MIN;
        let (due, fire) = advance(1_000, 1_001, interval, true);
        assert_eq!((due, fire), (1_000 + interval, false));

        // Held right up to the moment one was due: it does not go off late.
        let (due, fire) = advance(2_000, 1_000, interval, true);
        assert_eq!((due, fire), (2_000 + interval, false));
        // And the first free second is a full wait away, not immediate.
        let (_, fire) = advance(2_001, due, interval, false);
        assert!(!fire);
    }

    #[test]
    fn shortening_the_interval_brings_the_next_flash_forward() {
        let now = 1_000;
        let due = now + 90 * MIN;
        let (due, fire) = advance(now, due, 5 * MIN, false);
        assert_eq!((due, fire), (now + 5 * MIN, false));
    }

    #[test]
    fn lengthening_the_interval_leaves_a_flash_already_due_alone() {
        let now = 1_000;
        let due = now + 2 * MIN;
        let (due, fire) = advance(now, due, 90 * MIN, false);
        assert_eq!((due, fire), (now + 2 * MIN, false), "still in two minutes");
    }

    /// A clock that jumps forward past the due time flashes once, not once per
    /// interval it skipped; one that jumps back does not go quiet for hours.
    #[test]
    fn a_clock_that_jumps_does_not_leave_the_reminder_stuck() {
        let interval = 15 * MIN;
        let (due, fire) = advance(100_000, 1_000, interval, false);
        assert!(fire);
        assert_eq!(due, 100_000 + interval);

        let (due, fire) = advance(1_000, 100_000, interval, false);
        assert!(!fire);
        assert_eq!(due, 1_000 + interval, "not stuck until the old due time");
    }

    #[test]
    fn the_arithmetic_survives_the_end_of_time() {
        let (due, fire) = advance(u64::MAX, u64::MAX, 15 * MIN, false);
        assert!(fire);
        assert_eq!(due, u64::MAX);
        let (due, fire) = advance(u64::MAX - 1, u64::MAX, 15 * MIN, true);
        assert!(!fire);
        assert_eq!(due, u64::MAX);
    }

    #[test]
    fn the_flash_clock_reads_like_the_top_bar() {
        assert_eq!(short_elapsed(0, false), "0m");
        assert_eq!(short_elapsed(59, false), "0m");
        assert_eq!(short_elapsed(12 * MIN, false), "12m");
        assert_eq!(short_elapsed(3600, false), "1h00");
        assert_eq!(short_elapsed(3600 + 2 * MIN, false), "1h02");
        assert_eq!(short_elapsed(12 * MIN, true), "12m ⏸");
    }

    #[test]
    fn a_flash_carries_everything_needed_to_draw_it() {
        let event = FlashEvent {
            style: FlashStyle::TopbarBeam,
            intensity: Intensity::Strong,
            palette: Palette::Orange,
            title: "call \"mum\"".into(),
            timer: "1h02".into(),
        };
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
        assert_eq!(json["style"], "topbarBeam");
        assert_eq!(json["intensity"], "strong");
        assert_eq!(json["palette"], "orange");
        assert_eq!(json["title"], "call \"mum\"");
        assert_eq!(json["timer"], "1h02");
    }
}
