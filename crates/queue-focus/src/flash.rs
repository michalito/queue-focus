//! The reminder's clock: every so often, ask the shell to flash the current
//! task across the screen.
//!
//! The service owns the schedule rather than the extension, because the
//! service is the one that knows the queue, the pause, and the settings — and
//! it is running whether or not a window is open. What it sends is one
//! self-contained `Flash` signal per flash: everything needed to draw it, so a
//! shell that has only just connected still draws the right thing.

use crate::settings::SharedSettings;
use crate::state::SharedState;
use gtk::glib;
use qf_core::{pick_style, short_elapsed, FlashEvent, FlashStyle, Hold, Settings, Task, TimeOfDay};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub type SharedFlash = Rc<FlashClock>;

/// Puts one flash on the bus.
type Emitter = Rc<dyn Fn(&FlashEvent)>;

/// One observation for the settings view. Both values describe the same instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlashStatus {
    pub hold: Hold,
    pub remaining: Option<u64>,
}

/// Inputs are owned so no store borrow survives into an emission callback.
#[derive(Clone)]
struct Snapshot {
    now: u64,
    local_time: TimeOfDay,
    current: Option<Task>,
    settings: Settings,
}

impl Snapshot {
    fn hold(&self) -> Hold {
        self.settings.hold(self.current.as_ref(), self.local_time)
    }
}

type Tick = Box<dyn Fn() -> glib::ControlFlow>;

/// Private runtime seam: production supplies GNOME time and wakeups; tests
/// supply explicit snapshots and invoke the very same scheduled callback.
trait Runtime {
    fn snapshot(&self) -> Snapshot;
    fn random(&self) -> u32;
    fn every_second(&self, tick: Tick);
}

struct GnomeRuntime {
    state: SharedState,
    settings: SharedSettings,
}

impl Runtime for GnomeRuntime {
    fn snapshot(&self) -> Snapshot {
        let now = qf_core::unix_now();
        // Derive quiet hours from the same instant as the deadline and timer.
        // Preserve the previous midday fallback when local time is unavailable.
        let local_time = i64::try_from(now)
            .ok()
            .and_then(|unix| glib::DateTime::from_unix_local(unix).ok())
            .and_then(|date| TimeOfDay::new(date.hour() as u32, date.minute() as u32))
            .unwrap_or_else(|| TimeOfDay::new(12, 0).expect("12:00"));
        Snapshot {
            now,
            local_time,
            current: self.state.store().current().cloned(),
            settings: self.settings.get().clone(),
        }
    }

    fn random(&self) -> u32 {
        // Multiple of both style pool sizes (six initially, then five).
        glib::random_int_range(0, 30) as u32
    }

    fn every_second(&self, tick: Tick) {
        glib::timeout_add_seconds_local(1, tick);
    }
}

pub struct FlashClock {
    runtime: Rc<dyn Runtime>,
    /// Unix seconds at which the next flash is due.
    due: Cell<u64>,
    last: Cell<Option<FlashStyle>>,
    emit: RefCell<Option<Emitter>>,
}

impl FlashClock {
    pub fn new(state: SharedState, settings: SharedSettings) -> SharedFlash {
        Self::with_runtime(Rc::new(GnomeRuntime { state, settings }))
    }

    fn with_runtime(runtime: Rc<dyn Runtime>) -> SharedFlash {
        let snapshot = runtime.snapshot();
        let clock = Rc::new(Self {
            runtime: runtime.clone(),
            due: Cell::new(
                snapshot
                    .now
                    .saturating_add(snapshot.settings.interval_secs()),
            ),
            last: Cell::new(None),
            emit: RefCell::new(None),
        });
        let weak = Rc::downgrade(&clock);
        runtime.every_second(Box::new(move || match weak.upgrade() {
            Some(clock) => {
                clock.tick();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        }));
        clock
    }

    /// Installed by `dbus::export`. Emission is a request to draw, not an
    /// acknowledgement that a connected shell displayed the reminder.
    pub fn set_emitter(&self, f: impl Fn(&FlashEvent) + 'static) {
        *self.emit.borrow_mut() = Some(Rc::new(f));
    }

    pub fn status(&self) -> FlashStatus {
        let snapshot = self.runtime.snapshot();
        let hold = snapshot.hold();
        FlashStatus {
            hold,
            remaining: hold
                .is_none()
                .then(|| self.due.get().saturating_sub(snapshot.now)),
        }
    }

    /// A manual preview bypasses quiet rules, but still needs a current task.
    /// `true` means a flash was prepared, not that the shell displayed it.
    pub fn flash_now(&self) -> bool {
        self.fire(self.runtime.snapshot())
    }

    fn tick(&self) {
        let snapshot = self.runtime.snapshot();
        let full_wait = snapshot
            .now
            .saturating_add(snapshot.settings.interval_secs());
        // A shorter interval or a backwards clock jump cannot strand a flash
        // beyond a full wait. Lengthening an interval keeps an earlier deadline.
        let due = self.due.get().min(full_wait);
        if !snapshot.hold().is_none() {
            self.due.set(full_wait);
        } else if snapshot.now >= due {
            self.fire(snapshot);
        } else {
            self.due.set(due);
        }
    }

    fn fire(&self, snapshot: Snapshot) -> bool {
        let Some(task) = snapshot.current else {
            return false;
        };
        let style = pick_style(
            snapshot.settings.vary,
            self.last.get(),
            self.runtime.random(),
        );
        let event = FlashEvent {
            style,
            intensity: snapshot.settings.intensity,
            palette: snapshot.settings.color.palette(task.tag),
            timer: task
                .elapsed_secs(snapshot.now)
                .map(|secs| short_elapsed(secs, task.is_paused()))
                .unwrap_or_default(),
            title: task.title,
        };
        self.last.set(Some(style));
        self.due.set(
            snapshot
                .now
                .saturating_add(snapshot.settings.interval_secs()),
        );
        let emit = self.emit.borrow().clone();
        if let Some(emit) = emit {
            emit(&event);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qf_core::{Intensity, Palette};

    const MIN: u64 = 60;

    struct TestRuntime {
        input: RefCell<Snapshot>,
        tick: RefCell<Option<Tick>>,
        samples: Cell<usize>,
        draws: Cell<usize>,
    }

    impl Runtime for TestRuntime {
        fn snapshot(&self) -> Snapshot {
            self.samples.set(self.samples.get() + 1);
            self.input.borrow().clone()
        }

        fn random(&self) -> u32 {
            self.draws.set(self.draws.get() + 1);
            0
        }

        fn every_second(&self, tick: Tick) {
            *self.tick.borrow_mut() = Some(tick);
        }
    }

    struct Fixture {
        runtime: Rc<TestRuntime>,
        clock: SharedFlash,
        events: Rc<RefCell<Vec<FlashEvent>>>,
    }

    impl Fixture {
        fn new() -> Self {
            let runtime = Rc::new(TestRuntime {
                input: RefCell::new(Snapshot {
                    now: 1_000,
                    local_time: TimeOfDay::new(12, 0).unwrap(),
                    current: Some(Task {
                        id: 1,
                        title: "call \"mum\"".into(),
                        bucket: qf_core::Bucket::Now,
                        tag: Some(qf_core::Tag::Personal),
                        created_at: 400,
                        started_at: Some(400),
                        paused_at: None,
                    }),
                    settings: Settings::default(),
                }),
                tick: RefCell::new(None),
                samples: Cell::new(0),
                draws: Cell::new(0),
            });
            let clock = FlashClock::with_runtime(runtime.clone());
            let events = Rc::new(RefCell::new(Vec::new()));
            let recorded = events.clone();
            clock.set_emitter(move |event| recorded.borrow_mut().push(event.clone()));
            Self {
                runtime,
                clock,
                events,
            }
        }

        fn step(&self, now: u64) {
            self.runtime.input.borrow_mut().now = now;
            assert_eq!(
                self.runtime.tick.borrow().as_ref().unwrap()(),
                glib::ControlFlow::Continue
            );
        }

        fn remaining(&self) -> Option<u64> {
            self.clock.status().remaining
        }
    }

    #[test]
    fn scheduled_flash_carries_the_current_task_and_restarts_the_wait() {
        let f = Fixture::new();
        assert_eq!(f.remaining(), Some(15 * MIN));
        f.step(1_899);
        assert!(f.events.borrow().is_empty());
        assert_eq!(f.remaining(), Some(1));
        let samples = f.runtime.samples.get();
        f.step(1_900);
        assert_eq!(
            f.runtime.samples.get(),
            samples + 1,
            "one snapshot per decision"
        );
        let event = f.events.borrow()[0].clone();
        assert_eq!(event.title, "call \"mum\"");
        assert_eq!(event.timer, "25m");
        assert_eq!(event.palette, Palette::Orange);
        assert_eq!(event.intensity, Settings::default().intensity);
        assert_eq!(event.style, FlashStyle::ALL[0]);
        assert_eq!(f.remaining(), Some(15 * MIN));
        f.step(1_900);
        assert_eq!(
            f.events.borrow().len(),
            1,
            "no duplicate at the same instant"
        );
    }

    #[test]
    fn pause_resume_and_preview_share_the_schedule() {
        let f = Fixture::new();
        f.runtime
            .input
            .borrow_mut()
            .current
            .as_mut()
            .unwrap()
            .paused_at = Some(1_000);
        f.step(1_900);
        assert_eq!(
            f.clock.status(),
            FlashStatus {
                hold: Hold::Paused,
                remaining: None
            }
        );
        assert!(f.events.borrow().is_empty());
        assert!(f.clock.flash_now(), "manual preview bypasses pause");
        assert_eq!(f.events.borrow()[0].timer, "10m ⏸");
        {
            let mut input = f.runtime.input.borrow_mut();
            let task = input.current.as_mut().unwrap();
            task.paused_at = None;
            task.started_at = Some(1_300); // resume preserves the ten elapsed minutes
        }
        f.step(1_901);
        assert_eq!(f.remaining(), Some(899));
        f.step(2_800);
        assert_eq!(f.events.borrow().len(), 2);
        assert_eq!(f.events.borrow()[1].timer, "25m");
        assert_ne!(f.events.borrow()[0].style, f.events.borrow()[1].style);
    }

    #[test]
    fn empty_now_refuses_preview_without_using_randomness_or_style_history() {
        let f = Fixture::new();
        let task = f.runtime.input.borrow_mut().current.take();
        f.step(1_900);
        assert_eq!(
            f.clock.status(),
            FlashStatus {
                hold: Hold::NoCurrentTask,
                remaining: None
            }
        );
        assert!(!f.clock.flash_now());
        assert_eq!(f.runtime.draws.get(), 0);
        f.runtime.input.borrow_mut().current = task;
        f.step(1_901);
        assert_eq!(f.remaining(), Some(899));
        f.step(2_800);
        assert_eq!(f.events.borrow().len(), 1);
        assert_eq!(f.events.borrow()[0].style, FlashStyle::ALL[0]);
    }

    #[test]
    fn quiet_hours_hold_automatic_flashes_but_allow_preview() {
        let f = Fixture::new();
        {
            let mut input = f.runtime.input.borrow_mut();
            input.settings.quiet_hours = true;
            input.settings.quiet_from = TimeOfDay::new(22, 0).unwrap();
            input.settings.quiet_to = TimeOfDay::new(6, 0).unwrap();
        }
        f.step(1_900);
        assert_eq!(f.clock.status().hold, Hold::OutsideHours);
        assert_eq!(f.remaining(), None);
        assert!(f.clock.flash_now());
        f.runtime.input.borrow_mut().local_time = TimeOfDay::new(23, 0).unwrap();
        f.step(1_901);
        assert_eq!(f.remaining(), Some(899));
        f.step(2_800);
        assert_eq!(f.events.borrow().len(), 2);
        f.runtime.input.borrow_mut().local_time = TimeOfDay::new(6, 0).unwrap();
        f.step(3_700);
        assert_eq!(
            f.events.borrow().len(),
            2,
            "end of the allowed window is exclusive"
        );
    }

    #[test]
    fn disabling_quiet_rules_allows_a_paused_task_to_flash() {
        let f = Fixture::new();
        {
            let mut input = f.runtime.input.borrow_mut();
            input.current.as_mut().unwrap().paused_at = Some(1_000);
            input.settings.quiet_paused = false;
        }
        f.step(1_900);
        assert_eq!(f.events.borrow()[0].timer, "10m ⏸");
        assert_eq!(f.clock.status().hold, Hold::None);
    }

    #[test]
    fn interval_edits_and_manual_preview_adjust_the_next_reminder() {
        let f = Fixture::new();
        f.runtime.input.borrow_mut().settings.interval_min = 5;
        f.step(1_100);
        assert_eq!(f.remaining(), Some(5 * MIN));
        f.runtime.input.borrow_mut().settings.interval_min = 90;
        f.step(1_200);
        assert_eq!(
            f.remaining(),
            Some(200),
            "lengthening keeps an earlier deadline"
        );
        assert!(f.clock.flash_now());
        assert_eq!(f.remaining(), Some(90 * MIN));
        f.step(1_400);
        assert_eq!(
            f.events.borrow().len(),
            1,
            "preview replaced the old deadline"
        );
        f.step(6_600);
        assert_eq!(f.events.borrow().len(), 2);
    }

    #[test]
    fn clock_jumps_emit_once_and_do_not_strand_the_next_reminder() {
        let f = Fixture::new();
        f.step(100_000);
        assert_eq!(f.events.borrow().len(), 1);
        assert_eq!(f.remaining(), Some(15 * MIN));
        f.step(1_000);
        assert_eq!(f.remaining(), Some(15 * MIN));
        assert_eq!(f.events.borrow().len(), 1);
        f.step(1_900);
        assert_eq!(f.events.borrow().len(), 2);
        f.step(u64::MAX);
        assert_eq!(f.remaining(), Some(0), "deadline arithmetic saturates");
    }

    #[test]
    fn replacing_the_task_and_settings_changes_the_next_event() {
        let f = Fixture::new();
        assert!(f.clock.flash_now());
        {
            let mut input = f.runtime.input.borrow_mut();
            let task = input.current.as_mut().unwrap();
            task.id = 2;
            task.title = "replacement".into();
            task.tag = Some(qf_core::Tag::Work);
            task.started_at = Some(1_300);
            input.settings.vary = false;
            input.settings.intensity = Intensity::Strong;
        }
        f.step(1_900);
        let event = f.events.borrow()[1].clone();
        assert_eq!(event.title, "replacement");
        assert_eq!(event.timer, "10m");
        assert_eq!(event.palette, Palette::Blue);
        assert_eq!(event.style, FlashStyle::FIXED);
        assert_eq!(event.intensity, Intensity::Strong);
        assert!(f.clock.flash_now());
        assert_eq!(f.events.borrow()[2].style, FlashStyle::FIXED);
        f.runtime.input.borrow_mut().settings.vary = true;
        assert!(f.clock.flash_now());
        assert_ne!(f.events.borrow()[3].style, FlashStyle::FIXED);
    }

    #[test]
    fn emitter_can_change_inputs_and_observe_the_committed_deadline() {
        let f = Fixture::new();
        let weak = Rc::downgrade(&f.clock);
        let runtime = f.runtime.clone();
        f.clock.set_emitter(move |_| {
            runtime.input.borrow_mut().settings.interval_min = 90;
            assert_eq!(weak.upgrade().unwrap().status().remaining, Some(15 * MIN));
        });
        f.step(1_900);
        assert_eq!(f.remaining(), Some(15 * MIN));
    }

    #[test]
    fn wakeup_does_not_keep_a_dropped_clock_alive() {
        let f = Fixture::new();
        let runtime = f.runtime.clone();
        let weak = Rc::downgrade(&f.clock);
        drop(f);
        assert!(weak.upgrade().is_none());
        assert_eq!(
            runtime.tick.borrow().as_ref().unwrap()(),
            glib::ControlFlow::Break
        );
    }

    #[test]
    fn gnome_adapter_reads_live_stores_and_runs_its_timer() {
        use crate::settings::SettingsStore;
        use crate::state::State;
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        let dir = std::env::temp_dir().join(format!(
            "qf-flash-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let state = State::load_from(dir.join("tasks.json")).unwrap();
        let (settings, warning) = SettingsStore::load_from(dir.join("settings.json"));
        assert!(warning.is_none());
        let context = glib::MainContext::default();
        let _guard = context.acquire().unwrap();
        let clock = FlashClock::new(state.clone(), settings.clone());
        assert_eq!(clock.status().hold, Hold::NoCurrentTask);
        assert!(!clock.flash_now());
        let events = Rc::new(RefCell::new(Vec::new()));
        let recorded = events.clone();
        clock.set_emitter(move |event| recorded.borrow_mut().push(event.clone()));
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
        assert_eq!(clock.status().hold, Hold::None);

        // Exercise the real GLib wakeup registration as well as the deterministic
        // callbacks above. All callbacks return Break or hold only a weak clock.
        drop(clock);
        let fired = Rc::new(Cell::new(false));
        let observed = fired.clone();
        GnomeRuntime { state, settings }.every_second(Box::new(move || {
            observed.set(true);
            glib::ControlFlow::Break
        }));
        let timed_out = Rc::new(Cell::new(false));
        let timeout_flag = timed_out.clone();
        let watchdog = glib::timeout_add_local(std::time::Duration::from_secs(5), move || {
            timeout_flag.set(true);
            glib::ControlFlow::Break
        });
        while !fired.get() && !timed_out.get() {
            context.iteration(true);
        }
        if !timed_out.get() {
            watchdog.remove();
        }
        fs::remove_dir_all(dir).unwrap();
        assert!(
            fired.get(),
            "GNOME runtime did not invoke its scheduled callback"
        );
    }

    #[test]
    fn a_flash_carries_everything_needed_to_draw_it() {
        let f = Fixture::new();
        {
            let mut input = f.runtime.input.borrow_mut();
            input.now = 4_120;
            input.settings.vary = false;
            input.settings.intensity = Intensity::Strong;
        }
        assert!(f.clock.flash_now());
        let event = f.events.borrow()[0].clone();
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
        assert_eq!(json["style"], "edges");
        assert_eq!(json["intensity"], "strong");
        assert_eq!(json["palette"], "orange");
        assert_eq!(json["title"], "call \"mum\"");
        assert_eq!(json["timer"], "1h02");
    }
}
