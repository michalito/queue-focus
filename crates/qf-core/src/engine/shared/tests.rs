use super::super::{Engine, EngineError, Submitted};
use crate::store::TASKS_FILE;
use crate::sync::*;
use crate::{Bucket, Outcome, Tag};
use std::{fmt, fs, path::PathBuf};

struct Fixture {
    dir: PathBuf,
    engine: Engine,
    server: ServerState,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("qf-shared-engine-{}", Id::new_v4()));
        let mut server = ServerState::new(Id::new_v4(), Id::new_v4()).unwrap();
        server
            .apply(
                &Command::new(
                    &server.snapshot,
                    Action::Initialize {
                        tasks: vec![],
                        timer: None,
                    },
                ),
                crate::unix_now(),
            )
            .unwrap();
        let envelope = Envelope::new(
            Id::new_v4(),
            server.snapshot.clone(),
            crate::unix_now(),
            crate::unix_now(),
        )
        .unwrap();
        save_private(&dir.join(STATE_FILE), &envelope, MAX_ENVELOPE_BYTES).unwrap();
        // Deliberately invalid: sync must never load or overwrite this file.
        fs::write(dir.join(TASKS_FILE), b"legacy sentinel").unwrap();
        let client = Client::open(&dir.join(STATE_FILE), envelope.device).unwrap();
        let (engine, _) = Engine::open_with_sync(&dir, client, ClockSample::now().unwrap());
        Self {
            dir,
            engine,
            server,
        }
    }
    fn exchange(&mut self) -> Option<Settlement> {
        let clock = ClockSample::now().unwrap();
        let client = self.engine.sync_client_mut().unwrap();
        let request = client.request().unwrap();
        let receipt = request.command.as_ref().map(|command| Receipt {
            command: command.id,
            outcome: match self.server.apply(command, clock.wall) {
                Ok(()) => ReceiptOutcome::Applied {
                    revision: self.server.snapshot.revision,
                },
                Err(reason) => ReceiptOutcome::Rejected { reason },
            },
        });
        let response = ExchangeResponse {
            version: PROTOCOL_VERSION,
            device: client.envelope().device,
            server_time: clock.wall,
            snapshot: self.server.snapshot.clone(),
            receipt,
        };
        let settlement = client.accept(&request, response, clock).unwrap().value;
        self.engine.refresh_sync(clock);
        settlement
    }
    fn drain(&mut self) {
        while !self.engine.sync_envelope().unwrap().pending.is_empty() {
            self.exchange();
        }
    }
    fn reopen(&mut self) {
        let device = self.engine.sync_envelope().unwrap().device;
        let client = Client::open(&self.dir.join(STATE_FILE), device).unwrap();
        self.engine = Engine::open_with_sync(&self.dir, client, ClockSample::now().unwrap()).0;
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
fn pending<T: fmt::Debug>(result: Result<Submitted<T>, EngineError>) -> Id {
    match result {
        Ok(Submitted::Pending(command)) => command,
        other => panic!("expected pending, got {other:?}"),
    }
}

/// A queued edit is applied at once.
fn done<T: fmt::Debug>(result: Result<Submitted<T>, EngineError>) -> Outcome<T> {
    match result {
        Ok(Submitted::Applied(outcome)) => outcome,
        other => panic!("expected an applied change, got {other:?}"),
    }
}

#[test]
fn offline_capture_and_chained_edits_never_acquire_now_or_touch_legacy_file() {
    let mut f = Fixture::new();
    let a = done(f.engine.add("offline #w", None)).value;
    let b = done(f.engine.add("second", None)).value;
    assert!(f.engine.store().current().is_none());
    done(f.engine.rename(a, "renamed"));
    done(f.engine.cycle_tag(a));
    done(f.engine.move_before(a, Bucket::Next, Some(b)));
    done(f.engine.move_to(b, Bucket::Side, None));
    assert!(f.engine.update(|s| s.remove(a)).is_err());
    assert!(f.engine.promote(a).is_err(), "offline focus refused");
    f.reopen();
    assert_eq!(f.engine.store().get(a).unwrap().title, "renamed");
    assert_eq!(f.engine.store().get(a).unwrap().tag, Some(Tag::Personal));
    assert_eq!(f.engine.store().get(b).unwrap().bucket, Bucket::Side);
    f.drain();
    assert_eq!(f.server.snapshot.tasks.len(), 2);
    assert_eq!(
        fs::read(f.dir.join(TASKS_FILE)).unwrap(),
        b"legacy sentinel"
    );
}

#[test]
fn focus_is_pending_until_receipt_and_settlement_survives_restart() {
    let mut f = Fixture::new();
    f.exchange();
    let revision = f.engine.revision();
    let status_revision = f.engine.sync_status_revision();
    let command = pending(f.engine.add("!shared now", None));
    assert!(f.engine.store().current().is_none());
    assert_eq!(
        f.engine.revision(),
        revision,
        "status is not a queue mutation"
    );
    assert!(f.engine.sync_status_revision() > status_revision);
    assert_eq!(f.engine.sync_status().unwrap().focus_command, Some(command));
    let settlement = f.exchange().unwrap();
    assert_eq!(settlement.command, command);
    let current = f.engine.store().current().unwrap().id;
    pending(f.engine.toggle_pause());
    assert!(!f.engine.store().current().unwrap().is_paused());
    f.exchange();
    assert!(f.engine.store().current().unwrap().is_paused());
    f.reopen();
    assert_eq!(f.engine.store().current().unwrap().id, current);
    assert!(f
        .engine
        .sync_status()
        .unwrap()
        .last_focus_settlement
        .is_some());
    assert!(
        f.engine.toggle_pause().is_err(),
        "restart requires fresh observation"
    );
    let json: serde_json::Value = serde_json::from_str(&f.engine.state_json()).unwrap();
    assert_eq!(json["sync"]["state"], "offline");
}

#[test]
fn shared_completion_undo_uses_server_metadata_and_queued_completion_cannot_undo() {
    let mut f = Fixture::new();
    let a = done(f.engine.add("a", None)).value;
    let b = done(f.engine.add("b", None)).value;
    f.drain();
    pending(f.engine.promote(a));
    f.exchange();
    pending(f.engine.complete_current());
    assert_eq!(f.engine.store().current().unwrap().id, a);
    f.exchange();
    assert_eq!(f.engine.store().current().unwrap().id, b);
    assert_eq!(
        f.engine.sync_status().unwrap().undo.as_ref().unwrap().task,
        a
    );
    pending(f.engine.undo_complete(a));
    f.exchange();
    assert_eq!(f.engine.store().current().unwrap().id, a);
    done(f.engine.complete(b));
    assert!(!done(f.engine.undo_complete(b)).value);
    f.exchange();
    assert!(!done(f.engine.undo_complete(b)).value);
    assert!(f.engine.sync_status().unwrap().undo.is_none());
}

#[test]
fn stale_focus_rejection_keeps_server_state_and_persists_result() {
    let mut f = Fixture::new();
    f.exchange();
    let command = pending(f.engine.add("!draft", None));
    let remote = Command::new(
        &f.server.snapshot,
        Action::Queued {
            action: QueuedAction::Create {
                task: Id::new_v4(),
                title: "remote".into(),
                tag: None,
                bucket: Bucket::Next,
            },
        },
    );
    f.server.apply(&remote, crate::unix_now()).unwrap();
    let result = f.exchange().unwrap();
    assert_eq!(result.command, command);
    assert!(matches!(result.outcome, ReceiptOutcome::Rejected { .. }));
    assert!(f.engine.store().current().is_none());
    f.reopen();
    assert_eq!(
        f.engine.sync_status().unwrap().last_focus_settlement,
        Some(result)
    );
    assert_eq!(f.engine.sync_status().unwrap().recovery_items, 1);
    let recovery = f.engine.sync_recovery();
    assert_eq!(recovery.len(), 1);
    assert_eq!(recovery[0].title, "draft");
    assert_eq!(recovery[0].attempt, "Add to Now");
    let copy: RecoveryItem = serde_json::from_str(&recovery[0].copy_text).unwrap();
    assert_eq!(copy.command.id, command);
    // Dismissal removes only a definitive rejection, never uncertain work.
    done(f.engine.add("still pending", None));
    let pending = f.engine.sync_envelope().unwrap().pending.clone();
    assert!(f
        .engine
        .dismiss_sync_recovery(&pending[0].id.to_string())
        .is_err());
    let confirmed = f.engine.sync_envelope().unwrap().confirmed.clone();
    f.engine
        .dismiss_sync_recovery(&command.to_string())
        .unwrap();
    f.reopen();
    assert!(f.engine.sync_recovery().is_empty());
    assert_eq!(f.engine.sync_envelope().unwrap().pending, pending);
    assert_eq!(f.engine.sync_envelope().unwrap().confirmed, confirmed);
}

#[test]
fn quarantined_recovery_cannot_be_dismissed_or_retried() {
    let mut f = Fixture::new();
    done(f.engine.add("preserved", None));
    f.engine
        .sync_client_mut()
        .unwrap()
        .suspend("history mismatch", true)
        .unwrap();
    f.engine.refresh_sync(ClockSample::now().unwrap());
    let before = f.engine.sync_envelope().unwrap().clone();
    assert!(f.engine.retry_sync().is_err());
    assert!(f
        .engine
        .dismiss_sync_recovery(&before.pending[0].id.to_string())
        .is_err());
    assert_eq!(f.engine.sync_envelope(), Some(&before));
}

#[test]
fn shared_move_indices_and_anchor_refusals_match_native_contract() {
    let mut f = Fixture::new();
    let a = done(f.engine.add("a", None)).value;
    let b = done(f.engine.add("b", None)).value;
    let c = done(f.engine.add("c", None)).value;
    assert!(!done(f.engine.shift(a, -1)).value);
    assert!(done(f.engine.shift(a, 1)).value);
    assert_eq!(
        f.engine
            .store()
            .tasks
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![b, a, c]
    );
    let before = f.engine.revision();
    assert!(!done(f.engine.move_before(a, Bucket::Later, Some(b))).value);
    assert_eq!(before, f.engine.revision());
    f.drain();
    pending(f.engine.promote(a));
    f.exchange();
    pending(f.engine.move_before(a, Bucket::Next, Some(c)));
    assert!(f.engine.store().current().is_some());
    f.exchange();
    assert!(f.engine.store().current().is_none());
    assert_eq!(
        f.engine
            .store()
            .in_bucket(Bucket::Next)
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![b, a, c]
    );
}

#[test]
fn configured_now_refuses_offline_without_saving_elsewhere() {
    let mut f = Fixture::new();
    f.engine
        .update_settings(|settings| settings.default_bucket = Bucket::Now);
    assert!(matches!(
        f.engine.add("draft", None),
        Err(EngineError::InvalidArgument(_))
    ));
    assert!(f.engine.store().is_empty());
    assert!(f.engine.sync_envelope().unwrap().pending.is_empty());
    let queued = done(f.engine.add("explicit @later", None)).value;
    assert_eq!(f.engine.store().get(queued).unwrap().bucket, Bucket::Later);
    f.drain();
    pending(f.engine.add("configured Now online", None));
    assert!(f.engine.store().current().is_none());
    f.exchange();
    assert_eq!(
        f.engine.store().current().unwrap().title,
        "configured Now online"
    );
}

/// The status says when Now can change, and the engine takes a change to Now
/// exactly then: a view that follows the status never offers one that fails.
#[test]
fn focus_available_is_when_the_engine_takes_a_change_to_now() {
    let mut f = Fixture::new();
    let a = done(f.engine.add("a", None)).value;
    let status = f.engine.sync_status().unwrap();
    assert!(
        !status.focus_available,
        "no fresh answer, and an edit to send"
    );
    assert_eq!(
        status.summary,
        "1 change saved on this device, waiting to sync."
    );
    let b = done(f.engine.add("b", None)).value;
    assert!(f.engine.promote(a).is_err());

    f.drain();
    let status = f.engine.sync_status().unwrap();
    assert!(status.focus_available);
    assert_eq!(status.summary, "Synced");
    let command = pending(f.engine.promote(a));

    let status = f.engine.sync_status().unwrap();
    assert!(!status.focus_available, "one change to Now at a time");
    assert_eq!(status.focus_command, Some(command));
    assert_eq!(status.summary, "Sent, waiting for confirmation…");
    assert!(f.engine.promote(b).is_err());

    f.exchange();
    assert!(f.engine.sync_status().unwrap().focus_available);
    assert_eq!(f.engine.store().current().unwrap().id, a);
}

#[test]
fn get_state_carries_the_sync_status_and_a_standalone_queue_does_not() {
    let mut f = Fixture::new();
    f.exchange();
    let state: serde_json::Value = serde_json::from_str(&f.engine.state_json()).unwrap();
    assert_eq!(state["sync"]["state"], "online");
    assert_eq!(state["sync"]["summary"], "Synced");
    assert_eq!(state["sync"]["focus_available"], true);
    assert!(state["current"].is_null());

    let dir = f.dir.join("standalone");
    fs::create_dir_all(&dir).unwrap();
    let (standalone, _) = Engine::open(&dir, crate::unix_now).unwrap();
    assert!(standalone.sync_status().is_none());
    let state: serde_json::Value = serde_json::from_str(&standalone.state_json()).unwrap();
    assert!(state.get("sync").is_none());
}

#[test]
fn a_standalone_queue_refuses_the_sync_requests() {
    let dir = std::env::temp_dir().join(format!("qf-standalone-sync-{}", Id::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    let (mut engine, _) = Engine::open(&dir, crate::unix_now).unwrap();
    for result in [
        engine.retry_sync(),
        engine.dismiss_sync_recovery(&Id::new_v4().to_string()),
    ] {
        assert!(matches!(result, Err(EngineError::InvalidArgument(_))));
    }
    assert!(engine.sync_envelope().is_none() && engine.sync_recovery().is_empty());
    assert_eq!(engine.sync_status_revision(), 0);
    fs::remove_dir_all(dir).unwrap();
}
