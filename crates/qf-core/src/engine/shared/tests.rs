use super::*;
use std::{fs, path::PathBuf};

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
        while !self
            .engine
            .sync
            .as_ref()
            .unwrap()
            .envelope()
            .pending
            .is_empty()
        {
            self.exchange();
        }
    }
    fn reopen(&mut self) {
        let device = self.engine.sync.as_ref().unwrap().envelope().device;
        let client = Client::open(&self.dir.join(STATE_FILE), device).unwrap();
        self.engine = Engine::open_with_sync(&self.dir, client, ClockSample::now().unwrap()).0;
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
fn pending<T: fmt::Debug>(result: Result<Outcome<T>, EngineError>) -> Id {
    match result {
        Err(EngineError::Pending { command_id }) => command_id,
        other => panic!("expected pending, got {other:?}"),
    }
}

#[test]
fn offline_capture_and_chained_edits_never_acquire_now_or_touch_legacy_file() {
    let mut f = Fixture::new();
    let a = f.engine.add("offline #w", None).unwrap().value;
    let b = f.engine.add("second", None).unwrap().value;
    assert!(f.engine.store().current().is_none());
    f.engine.rename(a, "renamed").unwrap();
    f.engine.cycle_tag(a).unwrap();
    f.engine.move_before(a, Bucket::Next, Some(b)).unwrap();
    f.engine.move_to(b, Bucket::Side, None).unwrap();
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
    let a = f.engine.add("a", None).unwrap().value;
    let b = f.engine.add("b", None).unwrap().value;
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
    f.engine.complete(b).unwrap();
    assert!(!f.engine.undo_complete(b).unwrap().value);
    f.exchange();
    assert!(!f.engine.undo_complete(b).unwrap().value);
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
    f.engine.add("still pending", None).unwrap();
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
    f.engine.add("preserved", None).unwrap();
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
    let a = f.engine.add("a", None).unwrap().value;
    let b = f.engine.add("b", None).unwrap().value;
    let c = f.engine.add("c", None).unwrap().value;
    assert!(!f.engine.shift(a, -1).unwrap().value);
    assert!(f.engine.shift(a, 1).unwrap().value);
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
    assert!(
        !f.engine
            .move_before(a, Bucket::Later, Some(b))
            .unwrap()
            .value
    );
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
    assert!(f
        .engine
        .sync
        .as_ref()
        .unwrap()
        .envelope()
        .pending
        .is_empty());
    let queued = f.engine.add("explicit @later", None).unwrap().value;
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
