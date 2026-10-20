use super::*;
use crate::{Bucket, Tag};

fn empty() -> ServerState {
    let mut state = ServerState::new(Id::new_v4(), Id::new_v4()).unwrap();
    let command = Command::new(
        &state.snapshot,
        Action::Initialize {
            tasks: vec![],
            timer: None,
        },
    );
    state.apply(&command, 100).unwrap();
    state
}

fn queued(state: &ServerState, action: QueuedAction) -> Command {
    Command::new(&state.snapshot, Action::Queued { action })
}

fn focus(state: &ServerState, action: FocusAction, now: u64) -> Command {
    Command::new(
        &state.snapshot,
        Action::Focus {
            guard: FocusGuard::observed(&state.snapshot, now),
            action,
        },
    )
}

fn add(state: &mut ServerState, title: &str, bucket: Bucket) -> Id {
    let task = Id::new_v4();
    let command = queued(
        state,
        QueuedAction::Create {
            task,
            title: title.into(),
            tag: None,
            bucket,
        },
    );
    state.apply(&command, 100).unwrap();
    task
}

fn promote(state: &mut ServerState, task: Id, now: u64) {
    let command = focus(state, FocusAction::Promote { task }, now);
    state.apply(&command, now).unwrap();
}

fn anchor(state: &ServerState, task: Id) -> Anchor {
    Anchor {
        task,
        placement: state.snapshot.task(task).unwrap().versions.placement,
    }
}

fn ids(state: &ServerState, bucket: Bucket) -> Vec<Id> {
    state
        .snapshot
        .tasks
        .iter()
        .filter(|t| t.bucket == bucket)
        .map(|t| t.id)
        .collect()
}

fn rejected_unchanged(state: &mut ServerState, command: &Command, now: u64, error: Rejection) {
    let old = state.clone();
    assert_eq!(state.apply(command, now), Err(error));
    assert_eq!(*state, old, "rejected transition changed state");
}

#[test]
fn restore_preserves_tombstones_but_invalidates_undo_and_old_tokens() {
    let mut state = empty();
    let task = add(&mut state, "complete me", Bucket::Next);
    promote(&mut state, task, 100);
    let complete = focus(&state, FocusAction::Complete, 101);
    state.apply(&complete, 101).unwrap();
    assert!(state.snapshot.undo.is_some());
    let before = state.clone();
    assert!(state.reset_generation(Id::nil()).is_err());
    assert_eq!(state, before);
    assert!(state.reset_generation(state.snapshot.generation).is_err());
    assert_eq!(state, before);
    state.reset_generation(Id::new_v4()).unwrap();
    assert!(state.snapshot.undo.is_none());
    assert!(state.used_ids.contains(&task));
    assert_eq!(state.snapshot.revision, before.snapshot.revision);
    assert_ne!(state.snapshot.history, before.snapshot.history);
    state.validate().unwrap();
    let recreate = queued(
        &state,
        QueuedAction::Create {
            task,
            title: "must not resurrect".into(),
            tag: None,
            bucket: Bucket::Next,
        },
    );
    assert!(state.apply(&recreate, 102).is_err());
}

#[test]
fn independent_creates_and_chained_offline_edits_preserve_identity() {
    let mut state = empty();
    let task = Id::new_v4();
    let create = queued(
        &state,
        QueuedAction::Create {
            task,
            title: "first".into(),
            tag: None,
            bucket: Bucket::Next,
        },
    );
    let rename = queued(
        &state,
        QueuedAction::Rename {
            task,
            expected: create.id,
            title: "second".into(),
        },
    );
    let rename_again = queued(
        &state,
        QueuedAction::Rename {
            task,
            expected: rename.id,
            title: "third".into(),
        },
    );
    let other = add(&mut state, "other device", Bucket::Next);
    for command in [&create, &rename, &rename_again] {
        state.apply(command, 100).unwrap();
    }
    assert_eq!(ids(&state, Bucket::Next), [other, task]);
    assert_eq!(state.snapshot.task(task).unwrap().title, "third");
    assert!(
        state.snapshot.current().is_none(),
        "ordinary create must not acquire Now"
    );
}

#[test]
fn independent_fields_merge_but_competing_renames_conflict() {
    let mut state = empty();
    let task = add(&mut state, "before", Bucket::Later);
    let versions = state.snapshot.task(task).unwrap().versions.clone();
    let first = queued(
        &state,
        QueuedAction::Rename {
            task,
            expected: versions.title,
            title: "first".into(),
        },
    );
    let second = queued(
        &state,
        QueuedAction::Rename {
            task,
            expected: versions.title,
            title: "second".into(),
        },
    );
    let tag = queued(
        &state,
        QueuedAction::SetTag {
            task,
            expected: versions.tag,
            tag: Some(Tag::Work),
        },
    );
    state.apply(&first, 100).unwrap();
    state.apply(&tag, 100).unwrap();
    rejected_unchanged(&mut state, &second, 100, Rejection::Conflict);
    let task = state.snapshot.task(task).unwrap();
    assert_eq!(task.title, "first");
    assert_eq!(task.tag, Some(Tag::Work));
}

#[test]
fn deleted_tasks_cannot_be_recreated_or_edited() {
    let mut state = empty();
    let task = add(&mut state, "a", Bucket::Next);
    let version = state.snapshot.task(task).unwrap().versions.title;
    let edit = queued(
        &state,
        QueuedAction::Rename {
            task,
            expected: version,
            title: "offline draft".into(),
        },
    );
    let delete = queued(
        &state,
        QueuedAction::Delete {
            task,
            title_version: version,
        },
    );
    state.apply(&delete, 100).unwrap();
    rejected_unchanged(&mut state, &edit, 100, Rejection::MissingTask);
    let recreate = queued(
        &state,
        QueuedAction::Create {
            task,
            title: "a".into(),
            tag: None,
            bucket: Bucket::Next,
        },
    );
    rejected_unchanged(&mut state, &recreate, 100, Rejection::Conflict);
}

#[test]
fn retag_does_not_block_queued_completion_but_focus_does() {
    let mut state = empty();
    let a = add(&mut state, "a", Bucket::Next);
    let b = add(&mut state, "b", Bucket::Next);
    let delete_a = queued(
        &state,
        QueuedAction::Delete {
            task: a,
            title_version: state.snapshot.task(a).unwrap().versions.title,
        },
    );
    let delete_b = queued(
        &state,
        QueuedAction::Delete {
            task: b,
            title_version: state.snapshot.task(b).unwrap().versions.title,
        },
    );
    let tag = queued(
        &state,
        QueuedAction::SetTag {
            task: a,
            expected: state.snapshot.task(a).unwrap().versions.tag,
            tag: Some(Tag::Work),
        },
    );
    state.apply(&tag, 100).unwrap();
    state.apply(&delete_a, 100).unwrap();
    assert!(
        state.snapshot.undo.is_none(),
        "queued delete offers no server undo"
    );
    promote(&mut state, b, 100);
    rejected_unchanged(&mut state, &delete_b, 100, Rejection::Conflict);
}

#[test]
fn stale_anchor_detects_same_bucket_and_away_back_moves() {
    for away in [false, true] {
        let mut state = empty();
        let a = add(&mut state, "a", Bucket::Next);
        let b = add(&mut state, "b", Bucket::Next);
        let c = add(&mut state, "c", Bucket::Side);
        let stale = queued(
            &state,
            QueuedAction::MoveBefore {
                task: c,
                expected: state.snapshot.task(c).unwrap().versions.placement,
                bucket: Bucket::Next,
                before: Some(anchor(&state, b)),
            },
        );
        let movement = queued(
            &state,
            QueuedAction::MoveBefore {
                task: b,
                expected: state.snapshot.task(b).unwrap().versions.placement,
                bucket: if away { Bucket::Later } else { Bucket::Next },
                before: if away { None } else { Some(anchor(&state, a)) },
            },
        );
        state.apply(&movement, 100).unwrap();
        if away {
            let back = queued(
                &state,
                QueuedAction::MoveBefore {
                    task: b,
                    expected: movement.id,
                    bucket: Bucket::Next,
                    before: None,
                },
            );
            state.apply(&back, 100).unwrap();
        }
        rejected_unchanged(&mut state, &stale, 100, Rejection::Conflict);
    }
}

#[test]
fn queued_move_cannot_remove_now_or_enter_now() {
    let mut state = empty();
    let a = add(&mut state, "a", Bucket::Next);
    let b = add(&mut state, "b", Bucket::Next);
    promote(&mut state, a, 100);
    for (task, bucket) in [(a, Bucket::Later), (b, Bucket::Now)] {
        let command = queued(
            &state,
            QueuedAction::MoveBefore {
                task,
                expected: state.snapshot.task(task).unwrap().versions.placement,
                bucket,
                before: None,
            },
        );
        rejected_unchanged(&mut state, &command, 100, Rejection::Conflict);
    }
}

#[test]
fn completing_now_is_atomic_and_stale_completion_never_touches_successor() {
    let mut state = empty();
    let a = add(&mut state, "a", Bucket::Next);
    let b = add(&mut state, "b", Bucket::Next);
    let c = add(&mut state, "c", Bucket::Next);
    promote(&mut state, a, 100);
    let first = focus(&state, FocusAction::Complete, 101);
    let second_device = focus(&state, FocusAction::Complete, 101);
    state.apply(&first, 101).unwrap();
    assert_eq!(state.snapshot.current().unwrap().id, b);
    assert_eq!(state.snapshot.focus.session.as_ref().unwrap().anchor, 101);
    assert_eq!(state.snapshot.undo.as_ref().unwrap().command, first.id);
    rejected_unchanged(&mut state, &second_device, 102, Rejection::Conflict);
    // The reducer is also safe if accidentally called twice, though transport
    // must return the durable receipt rather than report a retry as rejected.
    rejected_unchanged(&mut state, &first, 102, Rejection::Conflict);
    let next = focus(&state, FocusAction::Complete, 102);
    state.apply(&next, 102).unwrap();
    rejected_unchanged(&mut state, &first, 103, Rejection::Conflict);
    assert_eq!(state.snapshot.current().unwrap().id, c);
}

#[test]
fn focus_aba_changes_session_and_version_even_when_task_matches() {
    let mut state = empty();
    let a = add(&mut state, "a", Bucket::Next);
    let b = add(&mut state, "b", Bucket::Next);
    promote(&mut state, a, 100);
    let session = state.snapshot.focus.session.as_ref().unwrap().id;
    let pause = focus(&state, FocusAction::SetPaused { paused: true }, 100);
    promote(&mut state, b, 101);
    promote(&mut state, a, 102);
    assert_ne!(state.snapshot.focus.session.as_ref().unwrap().id, session);
    // Even pretending the stale control saw the latest revision is not enough.
    let mut forged_revision = pause;
    if let Action::Focus { guard, .. } = &mut forged_revision.action {
        guard.revision = state.snapshot.revision;
    }
    rejected_unchanged(&mut state, &forged_revision, 103, Rejection::Conflict);
}

#[test]
fn pause_resume_is_explicit_and_preserves_elapsed_through_sleep() {
    let mut state = empty();
    let a = add(&mut state, "a", Bucket::Next);
    promote(&mut state, a, 100);
    let session = state.snapshot.focus.session.as_ref().unwrap().id;
    let pause = focus(&state, FocusAction::SetPaused { paused: true }, 160);
    state.apply(&pause, 160).unwrap();
    assert_eq!(
        state
            .snapshot
            .focus
            .session
            .as_ref()
            .unwrap()
            .elapsed_at(10_000),
        60
    );
    let resume = focus(&state, FocusAction::SetPaused { paused: false }, 200);
    state.apply(&resume, 200).unwrap();
    let timer = state.snapshot.focus.session.as_ref().unwrap();
    assert_eq!(timer.elapsed_at(3_800), 3_660);
    assert_eq!(timer.id, session);
    assert_eq!(
        timer.elapsed_at(190),
        60,
        "backwards samples cannot subtract elapsed"
    );
}

#[test]
fn expired_and_far_future_focus_requests_are_rejected_without_effects() {
    let mut state = empty();
    let a = add(&mut state, "a", Bucket::Next);
    let command = focus(&state, FocusAction::Promote { task: a }, 100);
    rejected_unchanged(&mut state, &command, 110, Rejection::Expired);
    let future = focus(&state, FocusAction::Promote { task: a }, 150);
    let old = state.clone();
    assert!(matches!(
        state.apply(&future, 100),
        Err(Rejection::Invalid(_))
    ));
    assert_eq!(state, old);
}

#[test]
fn completion_rejects_changed_next_order_instead_of_promoting_an_unseen_task() {
    let mut state = empty();
    let a = add(&mut state, "a", Bucket::Next);
    let b = add(&mut state, "b", Bucket::Next);
    let c = add(&mut state, "c", Bucket::Next);
    promote(&mut state, a, 100);
    let complete = focus(&state, FocusAction::Complete, 100);
    let reorder = queued(
        &state,
        QueuedAction::MoveBefore {
            task: c,
            expected: state.snapshot.task(c).unwrap().versions.placement,
            bucket: Bucket::Next,
            before: Some(anchor(&state, b)),
        },
    );
    state.apply(&reorder, 100).unwrap();
    rejected_unchanged(&mut state, &complete, 101, Rejection::Conflict);
}

#[test]
fn undo_preserves_elapsed_but_refreshes_restored_tokens_and_session() {
    let mut state = empty();
    let a = add(&mut state, "a", Bucket::Next);
    let b = add(&mut state, "b", Bucket::Next);
    promote(&mut state, a, 100);
    let old_session = state.snapshot.focus.session.clone().unwrap();
    let stale_rename = queued(
        &state,
        QueuedAction::Rename {
            task: a,
            expected: state.snapshot.task(a).unwrap().versions.title,
            title: "stale".into(),
        },
    );
    let completion = focus(&state, FocusAction::Complete, 160);
    state.apply(&completion, 160).unwrap();
    let revision = state.snapshot.revision;
    let undo = focus(
        &state,
        FocusAction::Undo {
            completion: completion.id,
        },
        162,
    );
    state.apply(&undo, 162).unwrap();
    assert_eq!(state.snapshot.revision, revision + 1);
    assert_eq!(state.snapshot.current().unwrap().id, a);
    assert_eq!(ids(&state, Bucket::Next), [b]);
    assert!(state.snapshot.undo.is_none());
    let restored = state.snapshot.focus.session.as_ref().unwrap();
    assert_ne!(restored.id, old_session.id);
    assert_eq!(restored.elapsed_at(163), 63);
    assert_eq!(
        state.snapshot.task(a).unwrap().versions,
        Versions::new(undo.id)
    );
    rejected_unchanged(&mut state, &stale_rename, 163, Rejection::Conflict);
}

#[test]
fn undo_expires_and_any_accepted_mutation_invalidates_it() {
    for after_mutation in [false, true] {
        let mut state = empty();
        let a = add(&mut state, "a", Bucket::Next);
        promote(&mut state, a, 100);
        let complete = focus(&state, FocusAction::Complete, 100);
        state.apply(&complete, 100).unwrap();
        let now = if after_mutation {
            add(&mut state, "new", Bucket::Later);
            101
        } else {
            108
        };
        let undo = focus(
            &state,
            FocusAction::Undo {
                completion: complete.id,
            },
            now,
        );
        rejected_unchanged(
            &mut state,
            &undo,
            now,
            if after_mutation {
                Rejection::Conflict
            } else {
                Rejection::Expired
            },
        );
    }
}

#[test]
fn remove_and_move_of_current_leave_now_empty() {
    for remove in [false, true] {
        let mut state = empty();
        let a = add(&mut state, "a", Bucket::Next);
        let b = add(&mut state, "b", Bucket::Next);
        promote(&mut state, a, 100);
        let command = focus(
            &state,
            if remove {
                FocusAction::Remove
            } else {
                FocusAction::MoveToQueue {
                    bucket: Bucket::Side,
                    before: None,
                }
            },
            100,
        );
        state.apply(&command, 100).unwrap();
        assert!(state.snapshot.current().is_none());
        assert!(state.snapshot.focus.session.is_none());
        assert_eq!(ids(&state, Bucket::Next), [b]);
        assert!(state.snapshot.undo.is_none());
    }
}

#[test]
fn initialization_is_atomic_bounded_and_once_only() {
    let mut state = ServerState::new(Id::new_v4(), Id::new_v4()).unwrap();
    let id = Id::new_v4();
    let seed = SeedTask {
        id,
        title: "existing".into(),
        tag: Some(Tag::Work),
        bucket: Bucket::Now,
        created_at: 1,
    };
    let invalid = Command::new(
        &state.snapshot,
        Action::Initialize {
            tasks: vec![seed.clone()],
            timer: None,
        },
    );
    let old = state.clone();
    assert!(state.apply(&invalid, 100).is_err());
    assert_eq!(state, old);
    let command = Command::new(
        &state.snapshot,
        Action::Initialize {
            tasks: vec![seed],
            timer: Some(SeedTimer {
                elapsed: 500,
                running: false,
            }),
        },
    );
    state.apply(&command, 100).unwrap();
    assert_eq!(
        state
            .snapshot
            .focus
            .session
            .as_ref()
            .unwrap()
            .elapsed_at(999),
        500
    );
    rejected_unchanged(&mut state, &command, 100, Rejection::AlreadyInitialized);
    let mut uninitialized = ServerState::new(Id::new_v4(), Id::new_v4()).unwrap();
    let create = queued(
        &uninitialized,
        QueuedAction::Create {
            task: id,
            title: "a".into(),
            tag: None,
            bucket: Bucket::Next,
        },
    );
    rejected_unchanged(&mut uninitialized, &create, 100, Rejection::NotInitialized);
}

#[test]
fn snapshot_rejects_duplicate_ids_malformed_focus_and_unbounded_titles() {
    let mut state = empty();
    let a = add(&mut state, "a", Bucket::Next);
    let good = state.snapshot.clone();
    let mut bad = good.clone();
    bad.tasks.push(bad.tasks[0].clone());
    assert!(bad.validate().is_err());
    let mut bad = good.clone();
    bad.tasks[0].title = "x".repeat(257);
    assert!(bad.validate().is_err());
    let mut bad = good.clone();
    bad.focus.session = Some(Session {
        id: Id::new_v4(),
        task: a,
        elapsed: 0,
        running: true,
        anchor: 0,
    });
    assert!(bad.validate().is_err());
    let mut bad = good.clone();
    bad.focus.version = Id::nil();
    assert!(bad.validate().is_err());
    let bytes = serde_json::to_vec(&good).unwrap();
    assert_eq!(serde_json::from_slice::<Snapshot>(&bytes).unwrap(), good);
}

#[test]
fn task_limit_and_invalid_create_leave_state_unchanged() {
    let mut state = ServerState::new(Id::new_v4(), Id::new_v4()).unwrap();
    let tasks = (0..MAX_TASKS)
        .map(|_| SeedTask {
            id: Id::new_v4(),
            title: "x".into(),
            tag: None,
            bucket: Bucket::Later,
            created_at: 1,
        })
        .collect();
    let initialize = Command::new(&state.snapshot, Action::Initialize { tasks, timer: None });
    state.apply(&initialize, 100).unwrap();
    let create = queued(
        &state,
        QueuedAction::Create {
            task: Id::new_v4(),
            title: "over limit".into(),
            tag: None,
            bucket: Bucket::Next,
        },
    );
    rejected_unchanged(&mut state, &create, 100, Rejection::Capacity);
}

#[test]
fn history_fingerprint_distinguishes_replayed_timer_outcomes_and_forks() {
    let mut first = empty();
    let a = add(&mut first, "a", Bucket::Next);
    let base = first.clone();
    let command = focus(&first, FocusAction::Promote { task: a }, 100);
    let mut restored = base.clone();
    first.apply(&command, 100).unwrap();
    restored.apply(&command, 101).unwrap();
    assert_eq!(first.snapshot.revision, restored.snapshot.revision);
    assert_ne!(first.snapshot.history, restored.snapshot.history);
    let checkpoint = first.snapshot.checkpoint();
    let historical = restored.snapshot.history.clone();
    add(&mut restored, "catch up", Bucket::Later);
    assert_eq!(
        checkpoint.verify(&restored.snapshot, Some(&historical)),
        Err(Rejection::HistoryMismatch)
    );
    assert_eq!(
        checkpoint.verify(&restored.snapshot, None),
        Err(Rejection::HistoryMismatch)
    );
    assert!(base
        .snapshot
        .checkpoint()
        .verify(&first.snapshot, Some(&base.snapshot.history))
        .is_ok());
}

#[test]
fn deterministic_reduction_and_serialized_restart_keep_state_identical() {
    let mut first = empty();
    let mut second: ServerState =
        serde_json::from_slice(&serde_json::to_vec(&first).unwrap()).unwrap();
    let command = queued(
        &first,
        QueuedAction::Create {
            task: Id::new_v4(),
            title: "same".into(),
            tag: None,
            bucket: Bucket::Next,
        },
    );
    first.apply(&command, 123).unwrap();
    second.apply(&command, 123).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        command.fingerprint().unwrap(),
        serde_json::from_slice::<Command>(&serde_json::to_vec(&command).unwrap())
            .unwrap()
            .fingerprint()
            .unwrap()
    );
}

#[test]
fn bootstrap_reads_are_explicit_and_never_authorize_a_mutation() {
    let state = empty();
    let command = queued(
        &state,
        QueuedAction::Delete {
            task: Id::new_v4(),
            title_version: Id::new_v4(),
        },
    );
    let mut request = ExchangeRequest {
        version: PROTOCOL_VERSION,
        checkpoint: None,
        command: None,
    };
    assert!(request.validate().is_ok());
    request.command = Some(command);
    assert!(request.validate().is_err());
    request.checkpoint = Some(state.snapshot.checkpoint());
    assert!(request.validate().is_ok());
    request.version = PROTOCOL_VERSION + 1;
    assert_eq!(request.validate(), Err(Rejection::UnsupportedVersion));
}

#[test]
fn wrong_generation_or_schema_never_changes_state() {
    let mut state = empty();
    let mut command = queued(
        &state,
        QueuedAction::Create {
            task: Id::new_v4(),
            title: "a".into(),
            tag: None,
            bucket: Bucket::Next,
        },
    );
    command.generation = Id::new_v4();
    rejected_unchanged(&mut state, &command, 100, Rejection::WrongDataset);
    command.generation = state.snapshot.generation;
    command.version += 1;
    rejected_unchanged(&mut state, &command, 100, Rejection::UnsupportedVersion);
}

#[test]
fn accepted_queued_commands_cannot_apply_their_effect_twice() {
    let mut state = empty();
    let a = add(&mut state, "a", Bucket::Next);
    let b = add(&mut state, "b", Bucket::Next);
    let rename = queued(
        &state,
        QueuedAction::Rename {
            task: a,
            expected: state.snapshot.task(a).unwrap().versions.title,
            title: "renamed".into(),
        },
    );
    let tag = queued(
        &state,
        QueuedAction::SetTag {
            task: a,
            expected: state.snapshot.task(a).unwrap().versions.tag,
            tag: Some(Tag::Personal),
        },
    );
    let movement = queued(
        &state,
        QueuedAction::MoveBefore {
            task: a,
            expected: state.snapshot.task(a).unwrap().versions.placement,
            bucket: Bucket::Next,
            before: Some(anchor(&state, b)),
        },
    );
    for command in [rename, tag, movement] {
        state.apply(&command, 100).unwrap();
        rejected_unchanged(&mut state, &command, 100, Rejection::Conflict);
    }
    let delete = queued(
        &state,
        QueuedAction::Delete {
            task: a,
            title_version: state.snapshot.task(a).unwrap().versions.title,
        },
    );
    state.apply(&delete, 100).unwrap();
    rejected_unchanged(&mut state, &delete, 100, Rejection::MissingTask);
}

#[test]
fn corrupt_private_state_is_detected_before_use() {
    let mut state = empty();
    let a = add(&mut state, "a", Bucket::Next);
    let mut missing_identity = state.clone();
    missing_identity.used_ids.remove(&a);
    assert!(missing_identity.validate().is_err());
    promote(&mut state, a, 100);
    let complete = focus(&state, FocusAction::Complete, 100);
    state.apply(&complete, 100).unwrap();
    assert!(state.validate().is_ok());
    let mut wrong_hint = state.clone();
    wrong_hint.snapshot.undo.as_mut().unwrap().until += 1;
    assert!(wrong_hint.validate().is_err());
    let mut wrong_revision = state;
    wrong_revision.snapshot.revision += 1;
    assert!(wrong_revision.validate().is_err());
}
