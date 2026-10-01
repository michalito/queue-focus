//! The engine driven the way Swift drives it: only through what is exported.

use super::*;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("qf-ffi-{name}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn open(dir: &Path) -> Arc<QueueEngine> {
    let engine = QueueEngine::new(dir.to_string_lossy().into_owned()).unwrap();
    assert_eq!(engine.open_warning(), None);
    engine
}

fn noon() -> TimeOfDay {
    TimeOfDay {
        hour: 12,
        minute: 0,
    }
}

fn ids(tasks: &[QueueTask]) -> Vec<u64> {
    tasks.iter().map(|t| t.id).collect()
}

fn invalid_message<T: fmt::Debug>(result: Result<T, QfError>) -> String {
    match result {
        Err(QfError::InvalidArgument { message }) => message,
        other => panic!("expected an invalid argument, got {other:?}"),
    }
}

#[test]
fn a_fresh_directory_opens_empty_and_writes_nothing_until_a_change() {
    let dir = temp_dir("fresh");
    let engine = open(&dir);
    let snapshot = engine.snapshot();
    assert_eq!(snapshot.current, None);
    assert!(snapshot.side.is_empty() && snapshot.next.is_empty() && snapshot.later.is_empty());
    assert_eq!(snapshot.revision, 0);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_unreadable_task_file_refuses_to_open_and_is_left_alone() {
    let dir = temp_dir("bad-tasks");
    fs::write(dir.join("tasks.json"), b"{ broken").unwrap();
    match QueueEngine::new(dir.to_string_lossy().into_owned()) {
        Err(QfError::Persistence { message }) => {
            assert!(message.contains("tasks.json"), "{message}")
        }
        Err(other) => panic!("expected a persistence error, got {other:?}"),
        Ok(_) => panic!("opened over a malformed task file"),
    }
    assert_eq!(fs::read(dir.join("tasks.json")).unwrap(), b"{ broken");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_unreadable_settings_file_opens_with_the_defaults_and_a_warning() {
    let dir = temp_dir("bad-settings");
    fs::write(dir.join("settings.json"), b"[]").unwrap();
    let engine = QueueEngine::new(dir.to_string_lossy().into_owned()).unwrap();
    let warning = engine.open_warning().unwrap();
    assert!(warning.contains("settings.json"), "{warning}");
    assert_eq!(
        engine.settings(),
        QueueSettings::from(&qf_core::Settings::default())
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_snapshot_groups_the_queue_and_counts_saved_changes() {
    let dir = temp_dir("snapshot");
    let engine = open(&dir);
    let next = engine.add("fix login #w".into(), None).unwrap();
    let side = engine
        .add("wait for CI".into(), Some(Bucket::Side))
        .unwrap();
    let later = engine.add("call mum #p @later".into(), None).unwrap();
    let current = engine.add("ship it".into(), Some(Bucket::Now)).unwrap();

    let snapshot = engine.snapshot();
    let task = snapshot.current.clone().unwrap();
    assert_eq!((task.id, task.title.as_str()), (current, "ship it"));
    assert_eq!(task.bucket, Bucket::Now);
    assert!(task.started_at.is_some() && task.paused_at.is_none());
    assert_eq!(ids(&snapshot.side), [side]);
    assert_eq!(ids(&snapshot.next), [next]);
    assert_eq!(snapshot.next[0].tag, Some(TaskTag::Work));
    assert_eq!(ids(&snapshot.later), [later]);
    assert_eq!(snapshot.later[0].tag, Some(TaskTag::Personal));
    assert_eq!(snapshot.later[0].title, "call mum");
    assert_eq!(snapshot.revision, 4);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn completing_undoing_and_pausing_go_through_the_engine() {
    let dir = temp_dir("complete");
    let engine = open(&dir);
    let a = engine.add("a".into(), Some(Bucket::Now)).unwrap();
    let b = engine.add("b".into(), None).unwrap();

    assert!(engine.toggle_pause().unwrap());
    assert!(engine.snapshot().current.unwrap().paused_at.is_some());
    assert!(engine.toggle_pause().unwrap());

    let done = engine.complete_current().unwrap().unwrap();
    assert_eq!(done.id, a);
    assert_eq!(engine.snapshot().current.unwrap().id, b);
    assert!(!engine.undo_complete(b).unwrap(), "not the last completion");
    assert!(engine.undo_complete(a).unwrap());
    assert_eq!(engine.snapshot().current.unwrap().id, a);
    assert_eq!(ids(&engine.snapshot().next), [b]);

    engine.complete(b).unwrap();
    assert!(engine.snapshot().next.is_empty());
    engine.remove(a).unwrap();
    assert!(!engine.undo_complete(b).unwrap(), "stale after a remove");
    assert_eq!(engine.complete_current().unwrap(), None);
    assert!(!engine.toggle_pause().unwrap());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn moving_tagging_and_renaming_go_through_the_engine() {
    let dir = temp_dir("move");
    let engine = open(&dir);
    let a = engine.add("a".into(), None).unwrap();
    let b = engine.add("b".into(), None).unwrap();
    let c = engine.add("c".into(), Some(Bucket::Later)).unwrap();

    engine.move_task(c, Bucket::Next, Some(0)).unwrap();
    assert_eq!(ids(&engine.snapshot().next), [c, a, b]);
    engine.move_task(c, Bucket::Next, None).unwrap();
    assert_eq!(ids(&engine.snapshot().next), [a, b, c]);
    assert!(engine.shift(c, -1).unwrap());
    assert_eq!(ids(&engine.snapshot().next), [a, c, b]);
    assert!(!engine.shift(a, -1).unwrap(), "already first");

    assert!(engine.move_before(b, Bucket::Next, Some(a)).unwrap());
    assert_eq!(ids(&engine.snapshot().next), [b, a, c]);
    assert!(engine.move_before(a, Bucket::Side, None).unwrap());
    assert!(
        !engine.move_before(c, Bucket::Next, Some(a)).unwrap(),
        "a has left Next since the row was drawn"
    );
    assert_eq!(ids(&engine.snapshot().next), [b, c]);

    engine.promote(c).unwrap();
    assert_eq!(engine.snapshot().current.unwrap().id, c);

    engine.set_tag(b, Some(TaskTag::Personal)).unwrap();
    engine.cycle_tag(b).unwrap();
    assert_eq!(engine.snapshot().next[0].tag, None);
    engine.cycle_tag(b).unwrap();
    assert_eq!(engine.snapshot().next[0].tag, Some(TaskTag::Work));
    engine.set_tag(b, None).unwrap();
    assert_eq!(engine.snapshot().next[0].tag, None);
    engine.rename(b, " renamed ".into()).unwrap();
    assert_eq!(engine.snapshot().next[0].title, "renamed");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refusals_are_invalid_arguments_and_change_nothing() {
    let dir = temp_dir("refusals");
    let engine = open(&dir);
    let a = engine.add("a".into(), None).unwrap();
    let revision = engine.snapshot().revision;
    assert_eq!(
        invalid_message(engine.add(" #w ".into(), None)),
        "empty title"
    );
    assert_eq!(
        invalid_message(engine.rename(a, "  ".into())),
        "empty title"
    );
    for message in [
        invalid_message(engine.complete(99)),
        invalid_message(engine.promote(99)),
        invalid_message(engine.remove(99)),
        invalid_message(engine.move_task(99, Bucket::Side, None)),
        invalid_message(engine.move_before(99, Bucket::Side, None)),
        invalid_message(engine.shift(99, 1)),
        invalid_message(engine.set_tag(99, None)),
        invalid_message(engine.cycle_tag(99)),
        invalid_message(engine.rename(99, "x".into())),
    ] {
        assert_eq!(message, "no such task");
    }
    assert_eq!(engine.snapshot().revision, revision);
    assert_eq!(engine.snapshot().next[0].title, "a");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_change_that_cannot_be_saved_is_a_persistence_error_and_rolled_back() {
    let dir = temp_dir("unsaveable");
    let engine = open(&dir);
    // A directory where the file belongs makes the atomic rename fail.
    fs::create_dir_all(dir.join("tasks.json")).unwrap();
    match engine.add("lost".into(), None) {
        Err(QfError::Persistence { message }) => {
            assert!(message.contains("could not save"), "{message}")
        }
        other => panic!("expected a persistence error, got {other:?}"),
    }
    assert!(engine.snapshot().next.is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn settings_round_trip_through_the_record_and_the_file() {
    let dir = temp_dir("settings");
    let engine = open(&dir);
    let mut settings = engine.settings();
    assert_eq!(settings.interval_min, 15);
    assert_eq!(settings.quiet_from, TimeOfDay { hour: 9, minute: 0 });
    settings.interval_min = 25;
    settings.intensity = Intensity::Strong;
    settings.color = FlashColor::Orange;
    settings.quiet_hours = true;
    settings.quiet_from = TimeOfDay {
        hour: 22,
        minute: 30,
    };
    settings.quiet_to = TimeOfDay { hour: 6, minute: 5 };
    settings.theme = Theme::Dark;
    settings.default_bucket = Bucket::Side;
    assert!(engine.set_settings(settings.clone()).unwrap());
    assert!(!engine.set_settings(settings.clone()).unwrap(), "no change");
    assert_eq!(engine.settings(), settings);
    assert!(engine.flush().is_empty());

    let json = fs::read_to_string(dir.join("settings.json")).unwrap();
    assert!(json.contains("\"quiet_from\": \"22:30\""), "{json}");
    assert!(json.contains("\"quiet_to\": \"06:05\""), "{json}");
    assert_eq!(open(&dir).settings(), settings);

    // The chosen bucket is where an unmarked task goes.
    let id = engine.add("beside".into(), None).unwrap();
    assert_eq!(ids(&engine.snapshot().side), [id]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_interval_out_of_range_is_pulled_into_range() {
    let dir = temp_dir("interval");
    let engine = open(&dir);
    let bounds = interval_bounds();
    for (asked, kept) in [(0, bounds.min), (9_000, bounds.max)] {
        let mut settings = engine.settings();
        settings.interval_min = asked;
        engine.set_settings(settings).unwrap();
        assert_eq!(engine.settings().interval_min, kept);
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_time_that_is_not_on_the_clock_is_refused() {
    let dir = temp_dir("time");
    let engine = open(&dir);
    let before = engine.settings();
    for (hour, minute) in [(24, 0), (9, 60)] {
        let mut settings = before.clone();
        settings.quiet_to = TimeOfDay { hour, minute };
        let message = invalid_message(engine.set_settings(settings));
        assert!(message.contains("is not a time of day"), "{message}");
        assert_eq!(engine.settings(), before);

        let time = TimeOfDay { hour, minute };
        invalid_message(engine.tick(0, time, 0));
        invalid_message(engine.flash_status(0, time));
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_flash_status_says_when_or_why_not() {
    let dir = temp_dir("status");
    let engine = open(&dir);
    let now = qf_core::unix_now();
    assert_eq!(
        engine.flash_status(now, noon()).unwrap(),
        FlashStatus::Held {
            reason: HoldReason::NoCurrentTask
        }
    );
    engine.add("!focus".into(), None).unwrap();
    assert!(engine.flash_now(now, 0).is_some());
    assert_eq!(
        engine.flash_status(now, noon()).unwrap(),
        FlashStatus::Scheduled {
            remaining_secs: 15 * 60
        }
    );
    engine.toggle_pause().unwrap();
    assert_eq!(
        engine.flash_status(now, noon()).unwrap(),
        FlashStatus::Held {
            reason: HoldReason::Paused
        }
    );
    let mut settings = engine.settings();
    settings.quiet_paused = false;
    settings.quiet_hours = true;
    engine.set_settings(settings).unwrap();
    let night = TimeOfDay {
        hour: 23,
        minute: 0,
    };
    assert_eq!(
        engine.flash_status(now, night).unwrap(),
        FlashStatus::Held {
            reason: HoldReason::OutsideHours
        }
    );
    assert_eq!(hold_reason_text(HoldReason::Paused), "the timer is paused");
    assert_eq!(
        hold_reason_text(HoldReason::NoCurrentTask),
        "nothing in Now"
    );
    assert_eq!(
        hold_reason_text(HoldReason::OutsideHours),
        "outside the chosen hours"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_tick_delivers_a_due_flash_and_the_settings_write() {
    let dir = temp_dir("tick");
    let engine = open(&dir);
    engine.add("focus #p".into(), Some(Bucket::Now)).unwrap();
    let mut settings = engine.settings();
    settings.interval_min = 1;
    settings.vary = false;
    engine.set_settings(settings).unwrap();

    let now = qf_core::unix_now();
    let first = engine.tick(now, noon(), 0).unwrap();
    assert_eq!(first.flash, None);
    assert!(first.problems.is_empty());
    assert!(
        dir.join("settings.json").exists(),
        "the tick wrote the settings"
    );

    let flash = engine.tick(now + 60, noon(), 0).unwrap().flash.unwrap();
    assert_eq!(flash.title, "focus");
    assert_eq!(flash.style, FlashStyle::Edges);
    assert_eq!(flash.palette, Palette::Orange);
    assert_eq!(flash.intensity, Intensity::Normal);
    assert_eq!(flash.timer, "1m");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_settings_write_failure_is_one_problem_per_outage() {
    let dir = temp_dir("settings-failure");
    let engine = open(&dir);
    fs::create_dir_all(dir.join("settings.json")).unwrap();
    let mut settings = engine.settings();
    settings.vary = false;
    engine.set_settings(settings).unwrap();

    let problems = engine.tick(0, noon(), 0).unwrap().problems;
    assert_eq!(problems.len(), 1);
    assert!(problems[0].contains("settings.json"), "{problems:?}");
    for second in 1..=40 {
        assert!(engine.tick(second, noon(), 0).unwrap().problems.is_empty());
    }
    assert!(engine.flush().is_empty(), "one outage, one complaint");
    fs::remove_dir_all(dir).unwrap();
}

/// A change saved without being made crash-safe still succeeds, and its
/// warning comes back with the next tick, once.
#[test]
fn a_durability_warning_waits_for_the_next_tick() {
    let dir = temp_dir("durability");
    let engine = open(&dir);
    let warning = qf_core::DurabilityWarning::new(dir.join("tasks.json"), "injected".into());
    let value = engine
        .lock()
        .settle(Ok(Outcome {
            value: 7,
            warning: Some(warning.clone()),
        }))
        .unwrap();
    assert_eq!(value, 7);
    assert_eq!(
        engine.tick(0, noon(), 0).unwrap().problems,
        [warning.to_string()]
    );
    assert!(engine.tick(1, noon(), 0).unwrap().problems.is_empty());

    engine.lock().report("before quitting".into());
    assert_eq!(engine.flush(), ["before quitting"]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn problems_that_pile_up_keep_the_first_ones() {
    let dir = temp_dir("pile-up");
    let engine = open(&dir);
    for n in 0..MAX_PENDING_PROBLEMS + 5 {
        engine.lock().report(format!("problem {n}"));
    }
    let problems = engine.flush();
    assert_eq!(problems.len(), MAX_PENDING_PROBLEMS);
    assert_eq!(problems[0], "problem 0");
    fs::remove_dir_all(dir).unwrap();
}

/// A panic while the lock is held must not lock the app out of its queue.
#[test]
fn a_poisoned_lock_still_serves() {
    let dir = temp_dir("poison");
    let engine = open(&dir);
    engine.add("kept".into(), None).unwrap();
    let held = engine.clone();
    let panicked = std::thread::spawn(move || {
        let _state = held.state.lock().unwrap();
        panic!("injected while holding the lock");
    })
    .join();
    assert!(panicked.is_err());
    assert!(engine.state.is_poisoned());
    assert_eq!(engine.snapshot().next[0].title, "kept");
    engine.add("after".into(), None).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_clocks_read_like_the_core() {
    assert_eq!(short_elapsed(62 * 60, false), "1h02");
    assert_eq!(short_elapsed(12 * 60, true), "12m ⏸");
    assert_eq!(long_elapsed(3725), "1:02:05");

    let mut task = QueueTask {
        id: 1,
        title: "t".into(),
        bucket: Bucket::Now,
        tag: None,
        created_at: 0,
        started_at: Some(100),
        paused_at: None,
    };
    assert_eq!(elapsed_secs(task.clone(), 160), Some(60));
    task.paused_at = Some(130);
    assert_eq!(
        elapsed_secs(task.clone(), 1_000),
        Some(30),
        "frozen while paused"
    );
    task.started_at = None;
    assert_eq!(elapsed_secs(task, 1_000), None);
}

#[test]
fn the_limits_are_the_core_s() {
    assert_eq!(
        interval_bounds(),
        IntervalBounds {
            min: qf_core::INTERVAL_MIN,
            max: qf_core::INTERVAL_MAX
        }
    );
    assert_eq!(max_title_chars() as usize, qf_core::MAX_TITLE_CHARS);
    assert!(default_data_dir().ends_with("queue-focus"));
}

/// Every variant crosses both ways unchanged.
#[test]
fn every_enum_value_crosses_and_comes_back() {
    for bucket in qf_core::Bucket::ALL {
        assert_eq!(qf_core::Bucket::from(Bucket::from(bucket)), bucket);
    }
    for tag in [qf_core::Tag::Work, qf_core::Tag::Personal] {
        assert_eq!(qf_core::Tag::from(TaskTag::from(tag)), tag);
    }
    for intensity in qf_core::Intensity::ALL {
        assert_eq!(
            qf_core::Intensity::from(Intensity::from(intensity)),
            intensity
        );
    }
    for color in qf_core::FlashColor::ALL {
        assert_eq!(qf_core::FlashColor::from(FlashColor::from(color)), color);
    }
    for palette in [qf_core::Palette::Blue, qf_core::Palette::Orange] {
        assert_eq!(qf_core::Palette::from(Palette::from(palette)), palette);
    }
    for style in qf_core::FlashStyle::ALL {
        assert_eq!(qf_core::FlashStyle::from(FlashStyle::from(style)), style);
    }
    for theme in qf_core::Theme::ALL {
        assert_eq!(qf_core::Theme::from(Theme::from(theme)), theme);
    }
    for (hold, reason) in [
        (qf_core::Hold::NoCurrentTask, HoldReason::NoCurrentTask),
        (qf_core::Hold::Paused, HoldReason::Paused),
        (qf_core::Hold::OutsideHours, HoldReason::OutsideHours),
    ] {
        assert_eq!(qf_core::Hold::from(reason), hold);
        assert_eq!(
            FlashStatus::from(qf_core::FlashStatus {
                hold,
                remaining: None
            }),
            FlashStatus::Held { reason }
        );
    }
}
