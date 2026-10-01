//! The running service's one engine, shared by the windows, the command line
//! and the D-Bus object.
//!
//! Everything the engine does is the core's. This adds what GNOME needs
//! around it: listeners for task and settings changes, a channel for settings
//! that could not be written, the emitter that puts a flash on the bus, and a
//! once-a-second tick on the GLib main loop that supplies the clock, the
//! local time of day and the randomness.

use gtk::glib;
use qf_core::{
    Engine, EngineError, FlashEvent, FlashStatus, Outcome, Settings, Store, Task, TimeOfDay,
};
use std::cell::{Ref, RefCell};
use std::io;
use std::path::Path;
use std::rc::{Rc, Weak};

pub type SharedService = Rc<Service>;

type Listener = Rc<dyn Fn()>;
/// Told that the settings could not be written.
type Problem = Rc<dyn Fn(&str)>;
/// Puts one flash on the bus.
type Emitter = Rc<dyn Fn(&FlashEvent)>;

pub struct Service {
    engine: RefCell<Engine>,
    /// Told after every saved task change.
    changed: RefCell<Vec<Listener>>,
    /// Told after every settings change, before the file catches up.
    settings_changed: RefCell<Vec<Listener>>,
    /// Told about a failure to write the settings, once per outage.
    problems: RefCell<Vec<Problem>>,
    emit: RefCell<Option<Emitter>>,
}

impl Service {
    /// Open the data files. A task file that cannot be read is an error; a
    /// settings file that cannot be read comes back as a warning.
    pub fn open() -> io::Result<(SharedService, Option<String>)> {
        Self::open_in(&qf_core::data_dir())
    }

    pub(crate) fn open_in(dir: &Path) -> io::Result<(SharedService, Option<String>)> {
        Self::open_at(dir, qf_core::unix_now)
    }

    /// Open with the first flash due a full wait after `now()`.
    fn open_at(
        dir: &Path,
        now: impl FnOnce() -> u64,
    ) -> io::Result<(SharedService, Option<String>)> {
        let (engine, warning) = Engine::open(dir, now)?;
        let service = Rc::new(Service {
            engine: RefCell::new(engine),
            changed: RefCell::new(Vec::new()),
            settings_changed: RefCell::new(Vec::new()),
            problems: RefCell::new(Vec::new()),
            emit: RefCell::new(None),
        });
        Ok((service, warning))
    }

    /// Tick once a second on the default main context, for as long as the
    /// service lives. Called once, when the application starts.
    pub fn run_on_main_loop(self: &Rc<Self>) {
        glib::timeout_add_seconds_local(1, wakeup(Rc::downgrade(self)));
    }

    // ---- reading -------------------------------------------------------

    /// The engine, to read. Hold it only as long as the read.
    pub fn engine(&self) -> Ref<'_, Engine> {
        self.engine.borrow()
    }

    pub fn store(&self) -> Ref<'_, Store> {
        Ref::map(self.engine.borrow(), Engine::store)
    }

    pub fn settings(&self) -> Ref<'_, Settings> {
        Ref::map(self.engine.borrow(), Engine::settings)
    }

    pub fn flash_status(&self) -> FlashStatus {
        let now = qf_core::unix_now();
        self.engine.borrow().flash_status(now, local_time(now))
    }

    // ---- changing ------------------------------------------------------

    /// Run one engine request, then tell the listeners what it changed. The
    /// engine is released first, so a listener can read it.
    pub fn request<R>(&self, f: impl FnOnce(&mut Engine) -> R) -> R {
        let (result, tasks_changed, settings_changed) = {
            let mut engine = self.engine.borrow_mut();
            let revision = engine.revision();
            let settings = engine.settings().clone();
            let result = f(&mut engine);
            (
                result,
                engine.revision() != revision,
                *engine.settings() != settings,
            )
        };
        if tasks_changed {
            notify(&self.changed);
        }
        if settings_changed {
            notify(&self.settings_changed);
        }
        result
    }

    /// Apply any change to the queue and save it.
    pub fn update<R>(&self, f: impl FnOnce(&mut Store) -> R) -> Result<Outcome<R>, EngineError> {
        self.request(|engine| engine.update(f))
    }

    pub fn complete_current(&self) -> Result<Outcome<Option<Task>>, EngineError> {
        self.request(Engine::complete_current)
    }

    /// Change the settings from a control; listeners hear about it at once.
    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) {
        self.request(|engine| engine.update_settings(f));
    }

    /// A flash now, whatever the quiet rules say. `false` when Now is empty.
    /// `true` means a flash was sent, not that the shell displayed it.
    pub fn flash_now(&self) -> bool {
        let flash = self
            .engine
            .borrow_mut()
            .flash_now(qf_core::unix_now(), random());
        self.draw(flash)
    }

    /// Write the settings if the file is behind. Called before the process
    /// exits, which is the one time the writer's wait cannot be afforded.
    pub fn flush(&self) {
        let problem = self.engine.borrow_mut().flush();
        self.report(problem);
    }

    fn tick(&self) {
        // One instant for the deadline, the timer and the quiet hours.
        let now = qf_core::unix_now();
        let tick = self
            .engine
            .borrow_mut()
            .tick(now, local_time(now), random());
        self.report(tick.settings_problem);
        self.draw(tick.flash);
    }

    // ---- listening -----------------------------------------------------

    pub fn on_change(&self, f: impl Fn() + 'static) {
        self.changed.borrow_mut().push(Rc::new(f));
    }

    pub fn on_settings_change(&self, f: impl Fn() + 'static) {
        self.settings_changed.borrow_mut().push(Rc::new(f));
    }

    pub fn on_problem(&self, f: impl Fn(&str) + 'static) {
        self.problems.borrow_mut().push(Rc::new(f));
    }

    /// Installed by `dbus::export`. Emission is a request to draw, not an
    /// acknowledgement that a connected shell displayed the reminder.
    pub fn set_flash_emitter(&self, f: impl Fn(&FlashEvent) + 'static) {
        *self.emit.borrow_mut() = Some(Rc::new(f));
    }

    /// Hand a flash to the emitter. The engine has been released by now, so
    /// the emitter may read it.
    fn draw(&self, flash: Option<FlashEvent>) -> bool {
        let Some(flash) = flash else {
            return false;
        };
        let emit = self.emit.borrow().clone();
        if let Some(emit) = emit {
            emit(&flash);
        }
        true
    }

    fn report(&self, problem: Option<String>) {
        let Some(message) = problem else {
            return;
        };
        eprintln!("queue-focus: {message}");
        // Cloned out of the borrow, so a listener may register another.
        let problems: Vec<Problem> = self.problems.borrow().clone();
        for f in problems {
            f(&message);
        }
    }
}

fn notify(listeners: &RefCell<Vec<Listener>>) {
    // Cloned out of the borrow, so a listener may register another.
    let listeners: Vec<Listener> = listeners.borrow().clone();
    for f in listeners {
        f();
    }
}

/// The once-a-second callback. It holds the service weakly, so the timer
/// does not keep a dropped service alive, and it stops once it is gone.
fn wakeup(service: Weak<Service>) -> impl FnMut() -> glib::ControlFlow {
    move || match service.upgrade() {
        Some(service) => {
            service.tick();
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
    use qf_core::{Bucket, FlashColor, FlashStyle, Hold, Intensity, Palette, Theme};
    use std::cell::Cell;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("qf-service-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A service with no timer. GLib's default main context belongs to one
    /// thread at a time and dispatches every thread's timers, so only the
    /// test of the timer itself registers one.
    fn service(dir: &Path) -> SharedService {
        let (service, warning) = Service::open_in(dir).unwrap();
        assert!(warning.is_none());
        service
    }

    fn count(register: impl FnOnce(Box<dyn Fn()>)) -> Rc<Cell<u32>> {
        let count = Rc::new(Cell::new(0));
        let seen = count.clone();
        register(Box::new(move || seen.set(seen.get() + 1)));
        count
    }

    /// A fresh full wait, as the real clock reads it. The flash and the look
    /// at the countdown may fall either side of a second boundary, so allow
    /// for the seconds that have passed since `since`.
    fn assert_full_wait(remaining: Option<u64>, since: u64) {
        let remaining = remaining.expect("a flash is scheduled");
        let passed = qf_core::unix_now() - since;
        assert!(
            remaining <= 15 * 60 && remaining + passed >= 15 * 60,
            "{remaining}s left, {passed}s after the flash was asked for"
        );
    }

    fn record(service: &Service) -> Rc<RefCell<Vec<FlashEvent>>> {
        let events = Rc::new(RefCell::new(Vec::new()));
        let recorded = events.clone();
        service.set_flash_emitter(move |event| recorded.borrow_mut().push(event.clone()));
        events
    }

    /// Listeners hear about saved changes, and only those: not a failed save,
    /// not a request that changed nothing.
    #[test]
    fn task_listeners_hear_about_each_saved_change_and_nothing_else() {
        let dir = temp_dir("listeners");
        let service = service(&dir);
        let changes = count(|f| service.on_change(f));
        let settings = count(|f| service.on_settings_change(f));

        let a = service
            .update(|s| s.add("a", Bucket::Now, None))
            .unwrap()
            .value;
        assert_eq!(changes.get(), 1);
        assert!(service.complete_current().unwrap().value.is_some());
        assert_eq!(changes.get(), 2);
        assert!(service.request(|e| e.undo_complete(a)).unwrap().value);
        assert_eq!(changes.get(), 3);

        assert!(!service.update(|s| s.remove(999)).unwrap().value);
        assert!(service.request(|e| e.complete(999)).is_err());
        assert!(!service.request(|e| e.undo_complete(a)).unwrap().value);
        assert_eq!(changes.get(), 3, "no-ops are not broadcast");

        fs::remove_file(dir.join("tasks.json")).unwrap();
        fs::create_dir_all(dir.join("tasks.json")).unwrap();
        assert!(service
            .update(|s| s.add("lost", Bucket::Next, None))
            .is_err());
        assert_eq!(changes.get(), 3, "a failed save is not a change");
        assert_eq!(settings.get(), 0);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_listeners_hear_about_real_changes_only() {
        let dir = temp_dir("settings");
        let service = service(&dir);
        let changes = count(|f| service.on_change(f));
        let settings = count(|f| service.on_settings_change(f));

        service.update_settings(|s| s.theme = Theme::Dark);
        assert_eq!(settings.get(), 1);
        service.update_settings(|s| s.theme = Theme::Dark);
        assert_eq!(settings.get(), 1);
        service
            .request(|e| e.set_settings(r#"{"theme":"dark"}"#))
            .unwrap();
        assert!(service
            .request(|e| e.set_settings(r#"{"theme":"puce"}"#))
            .is_err());
        assert_eq!(settings.get(), 1);
        service
            .request(|e| e.set_settings(r#"{"theme":"light"}"#))
            .unwrap();
        assert_eq!(settings.get(), 2);
        assert_eq!(changes.get(), 0);

        fs::remove_dir_all(dir).unwrap();
    }

    /// A listener reads the engine it was told about, which it can only do
    /// once the request has let go of it.
    #[test]
    fn a_listener_can_read_the_engine() {
        let dir = temp_dir("reentrant");
        let service = service(&dir);
        let seen = Rc::new(Cell::new(0));
        let (weak, count) = (Rc::downgrade(&service), seen.clone());
        service.on_change(move || count.set(weak.upgrade().unwrap().store().len()));
        service.update(|s| s.add("a", Bucket::Next, None)).unwrap();
        assert_eq!(seen.get(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_settings_write_is_reported_once_to_every_problem_listener() {
        let dir = temp_dir("unwritable");
        let service = service(&dir);
        // A directory where the file belongs makes every rename fail.
        fs::create_dir_all(dir.join("settings.json")).unwrap();
        let problems = Rc::new(RefCell::new(Vec::new()));
        let seen = problems.clone();
        service.on_problem(move |m| seen.borrow_mut().push(m.to_string()));

        service.update_settings(|s| s.vary = false);
        service.tick();
        service.flush();
        service.tick();
        assert_eq!(problems.borrow().len(), 1, "one outage, one complaint");
        assert!(problems.borrow()[0].contains("could not save"));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_flash_reads_the_live_queue_and_settings() {
        let dir = temp_dir("live");
        let service = service(&dir);
        let events = record(&service);
        assert_eq!(service.flash_status().hold, Hold::NoCurrentTask);
        assert!(!service.flash_now());
        assert!(events.borrow().is_empty());

        service
            .request(|e| e.add("real stores #p !now", None))
            .unwrap();
        service.update_settings(|s| {
            s.vary = false;
            s.color = FlashColor::Blue;
            s.intensity = Intensity::Strong;
        });
        let since = qf_core::unix_now();
        assert!(service.flash_now());
        let event = events.borrow()[0].clone();
        assert_eq!(event.title, "real stores");
        assert_eq!(event.style, FlashStyle::FIXED);
        assert_eq!(event.palette, Palette::Blue);
        assert_eq!(event.intensity, Intensity::Strong);
        assert_eq!(event.timer, "0m");
        let status = service.flash_status();
        assert_eq!(status.hold, Hold::None);
        assert_full_wait(status.remaining, since);

        fs::remove_dir_all(dir).unwrap();
    }

    /// The real GLib timer, the real local time and the real randomness: a
    /// service whose first flash is overdue flashes on its first second.
    #[test]
    fn the_timer_delivers_a_flash_that_is_due() {
        let dir = temp_dir("timer");
        let (service, _) = Service::open_at(&dir, || qf_core::unix_now() - 3_600).unwrap();
        service
            .request(|e| e.add("due", Some(Bucket::Now)))
            .unwrap();
        let events = record(&service);
        let context = glib::MainContext::default();
        let _guard = context.acquire().unwrap();
        let since = qf_core::unix_now();
        service.run_on_main_loop();

        let deadline = Instant::now() + Duration::from_secs(5);
        while events.borrow().is_empty() && Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }
        fs::remove_dir_all(dir).unwrap();
        assert_eq!(events.borrow().len(), 1, "the timer delivered no flash");
        assert_eq!(events.borrow()[0].title, "due");
        assert_full_wait(service.flash_status().remaining, since);
    }

    #[test]
    fn the_timer_does_not_keep_a_dropped_service_alive() {
        let dir = temp_dir("dropped");
        let service = service(&dir);
        let mut tick = wakeup(Rc::downgrade(&service));
        assert_eq!(tick(), glib::ControlFlow::Continue);

        let weak = Rc::downgrade(&service);
        drop(service);
        assert!(weak.upgrade().is_none(), "the timer held the service");
        assert_eq!(tick(), glib::ControlFlow::Break);

        fs::remove_dir_all(dir).unwrap();
    }

    /// The emitter runs with the engine released, so it may read the queue
    /// and the schedule while it draws.
    #[test]
    fn the_emitter_can_read_the_engine() {
        let dir = temp_dir("emitter");
        let service = service(&dir);
        service
            .request(|e| e.add("now", Some(Bucket::Now)))
            .unwrap();
        let seen = Rc::new(Cell::new(None));
        let (weak, observed) = (Rc::downgrade(&service), seen.clone());
        service.set_flash_emitter(move |_| {
            let service = weak.upgrade().unwrap();
            assert!(service.store().current().is_some());
            observed.set(service.flash_status().remaining);
        });
        let since = qf_core::unix_now();
        assert!(service.flash_now());
        assert_full_wait(seen.get(), since);

        fs::remove_dir_all(dir).unwrap();
    }
}
