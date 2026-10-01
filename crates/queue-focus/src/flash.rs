//! The reminder's clock: every so often, ask the shell to flash the current
//! task across the screen.
//!
//! The service owns the schedule rather than the extension, because the
//! service is the one that knows the queue, the pause, and the settings — and
//! it is running whether or not a window is open. What it sends is one
//! self-contained `Flash` signal per flash: everything needed to draw it, so a
//! shell that has only just connected still draws the right thing.
//!
//! The schedule is the core's `Reminder`. This supplies what it asks for —
//! the unix time, the local time of day from GLib, and GLib's randomness —
//! once a second, and puts each flash it returns on the bus.

use crate::settings::SharedSettings;
use crate::state::SharedState;
use gtk::glib;
use qf_core::{FlashEvent, FlashStatus, Reminder, TimeOfDay};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

pub type SharedFlash = Rc<FlashClock>;

/// Puts one flash on the bus.
type Emitter = Rc<dyn Fn(&FlashEvent)>;

pub struct FlashClock {
    state: SharedState,
    settings: SharedSettings,
    reminder: RefCell<Reminder>,
    emit: RefCell<Option<Emitter>>,
}

impl FlashClock {
    pub fn new(state: SharedState, settings: SharedSettings) -> SharedFlash {
        Self::scheduled(state, settings, qf_core::unix_now())
    }

    /// A clock whose first flash is a full wait after `now`, ticking once a
    /// second on the default main context.
    fn scheduled(state: SharedState, settings: SharedSettings, now: u64) -> SharedFlash {
        let clock = Self::unscheduled(state, settings, now);
        glib::timeout_add_seconds_local(1, wakeup(Rc::downgrade(&clock)));
        clock
    }

    /// The same clock without its timer: it only moves when asked to.
    fn unscheduled(state: SharedState, settings: SharedSettings, now: u64) -> SharedFlash {
        let reminder = Reminder::new(now, &settings.get());
        Rc::new(Self {
            state,
            settings,
            reminder: RefCell::new(reminder),
            emit: RefCell::new(None),
        })
    }

    /// Installed by `dbus::export`. Emission is a request to draw, not an
    /// acknowledgement that a connected shell displayed the reminder.
    pub fn set_emitter(&self, f: impl Fn(&FlashEvent) + 'static) {
        *self.emit.borrow_mut() = Some(Rc::new(f));
    }

    pub fn status(&self) -> FlashStatus {
        let now = qf_core::unix_now();
        self.reminder.borrow().status(
            &self.state.store(),
            &self.settings.get(),
            now,
            local_time(now),
        )
    }

    /// A manual preview bypasses quiet rules, but still needs a current task.
    /// `true` means a flash was sent, not that the shell displayed it.
    pub fn flash_now(&self) -> bool {
        let event = self.reminder.borrow_mut().flash_now(
            &self.state.store(),
            &self.settings.get(),
            qf_core::unix_now(),
            random(),
        );
        self.emit(event)
    }

    fn tick(&self) {
        // One instant for the deadline, the timer and the quiet hours.
        let now = qf_core::unix_now();
        let event = self.reminder.borrow_mut().tick(
            &self.state.store(),
            &self.settings.get(),
            now,
            local_time(now),
            random(),
        );
        self.emit(event);
    }

    /// Hand a flash to the emitter. Every borrow has ended by now, so the
    /// emitter may read the queue, the settings or this clock.
    fn emit(&self, event: Option<FlashEvent>) -> bool {
        let Some(event) = event else {
            return false;
        };
        let emit = self.emit.borrow().clone();
        if let Some(emit) = emit {
            emit(&event);
        }
        true
    }
}

/// The once-a-second callback. It holds the clock weakly, so the timer does
/// not keep a dropped clock alive, and it stops once the clock is gone.
fn wakeup(clock: Weak<FlashClock>) -> impl FnMut() -> glib::ControlFlow {
    move || match clock.upgrade() {
        Some(clock) => {
            clock.tick();
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    }
}

/// The local time of day at `now`, for the quiet hours. Midday when GLib
/// cannot tell.
fn local_time(now: u64) -> TimeOfDay {
    i64::try_from(now)
        .ok()
        .and_then(|unix| glib::DateTime::from_unix_local(unix).ok())
        .and_then(|date| TimeOfDay::new(date.hour() as u32, date.minute() as u32))
        .unwrap_or_else(|| TimeOfDay::new(12, 0).expect("12:00"))
}

fn random() -> u32 {
    // A multiple of both style pool sizes (six initially, then five), so
    // every style is equally likely.
    glib::random_int_range(0, 30) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SettingsStore;
    use crate::state::State;
    use qf_core::{FlashStyle, Hold, Intensity, Palette};
    use std::cell::Cell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("qf-flash-{name}-{}-{nonce}", std::process::id()))
    }

    fn stores(dir: &Path) -> (SharedState, SharedSettings) {
        fs::create_dir_all(dir).unwrap();
        let state = State::load_from(dir.join("tasks.json")).unwrap();
        let (settings, warning) = SettingsStore::load_from(dir.join("settings.json"));
        assert!(warning.is_none());
        (state, settings)
    }

    /// A clock with no timer. GLib's default main context belongs to one
    /// thread at a time and dispatches every thread's timers, so only the test
    /// of the timer itself registers one.
    fn clock(state: &SharedState, settings: &SharedSettings) -> SharedFlash {
        FlashClock::unscheduled(state.clone(), settings.clone(), qf_core::unix_now())
    }

    fn record(clock: &FlashClock) -> Rc<RefCell<Vec<FlashEvent>>> {
        let events = Rc::new(RefCell::new(Vec::new()));
        let recorded = events.clone();
        clock.set_emitter(move |event| recorded.borrow_mut().push(event.clone()));
        events
    }

    #[test]
    fn the_clock_reads_the_live_queue_and_settings() {
        let dir = temp_dir("live");
        let (state, settings) = stores(&dir);
        let clock = clock(&state, &settings);
        let events = record(&clock);
        assert_eq!(clock.status().hold, Hold::NoCurrentTask);
        assert!(!clock.flash_now());
        assert!(events.borrow().is_empty());

        state
            .update(|s| s.quick_add("real stores #p !now", qf_core::Bucket::Next))
            .unwrap();
        settings.update(|s| {
            s.vary = false;
            s.color = qf_core::FlashColor::Blue;
            s.intensity = Intensity::Strong;
        });
        assert!(clock.flash_now());
        let event = events.borrow()[0].clone();
        assert_eq!(event.title, "real stores");
        assert_eq!(event.style, FlashStyle::FIXED);
        assert_eq!(event.palette, Palette::Blue);
        assert_eq!(event.intensity, Intensity::Strong);
        assert_eq!(event.timer, "0m");
        let status = clock.status();
        assert_eq!(status.hold, Hold::None);
        assert_eq!(
            status.remaining,
            Some(15 * 60),
            "the preview restarted the wait"
        );

        fs::remove_dir_all(dir).unwrap();
    }

    /// The real GLib timer, the real local time and the real randomness: a
    /// clock whose deadline has passed flashes on its first second.
    #[test]
    fn the_timer_delivers_a_flash_that_is_due() {
        let dir = temp_dir("timer");
        let (state, settings) = stores(&dir);
        state
            .update(|s| s.add("due", qf_core::Bucket::Now, None))
            .unwrap();
        let context = glib::MainContext::default();
        let _guard = context.acquire().unwrap();
        let clock = FlashClock::scheduled(state, settings, qf_core::unix_now() - 3_600);
        let events = record(&clock);

        let deadline = Instant::now() + Duration::from_secs(5);
        while events.borrow().is_empty() && Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }
        fs::remove_dir_all(dir).unwrap();
        assert_eq!(events.borrow().len(), 1, "the timer delivered no flash");
        assert_eq!(events.borrow()[0].title, "due");
        assert_eq!(clock.status().remaining, Some(15 * 60));
    }

    #[test]
    fn the_timer_does_not_keep_a_dropped_clock_alive() {
        let dir = temp_dir("dropped");
        let (state, settings) = stores(&dir);
        let clock = clock(&state, &settings);
        let mut tick = wakeup(Rc::downgrade(&clock));
        assert_eq!(tick(), glib::ControlFlow::Continue);

        let weak = Rc::downgrade(&clock);
        drop(clock);
        assert!(weak.upgrade().is_none(), "the timer held the clock");
        assert_eq!(tick(), glib::ControlFlow::Break);

        fs::remove_dir_all(dir).unwrap();
    }

    /// The emitter runs with every borrow released, so it may read the queue
    /// and the clock while it draws.
    #[test]
    fn the_emitter_can_read_the_clock_and_the_queue() {
        let dir = temp_dir("reentrant");
        let (state, settings) = stores(&dir);
        state
            .update(|s| s.add("now", qf_core::Bucket::Now, None))
            .unwrap();
        let clock = clock(&state, &settings);
        let seen = Rc::new(Cell::new(None));
        let (weak, observed) = (Rc::downgrade(&clock), seen.clone());
        clock.set_emitter(move |_| {
            assert!(state.store().current().is_some());
            observed.set(weak.upgrade().unwrap().status().remaining);
        });
        assert!(clock.flash_now());
        assert_eq!(seen.get(), Some(15 * 60));

        fs::remove_dir_all(dir).unwrap();
    }
}
