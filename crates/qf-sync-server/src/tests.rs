use super::*;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use qf_core::{sync::*, Bucket};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use tower::ServiceExt;

struct Fixture {
    directory: PathBuf,
    database: Database,
    device: Enrollment,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("qf-server-test-{}", Id::new_v4()));
        let mut database = Database::create(&directory.join("queue.db")).unwrap();
        let device = database.enroll("test Mac").unwrap();
        Self {
            directory,
            database,
            device,
        }
    }
    fn discover(&mut self) -> Snapshot {
        self.database
            .exchange(&self.device.token, &fetch(None), 100)
            .unwrap()
            .snapshot
    }
    fn send(&mut self, snapshot: &Snapshot, command: Command, time: u64) -> ExchangeResponse {
        self.database
            .exchange(&self.device.token, &request(snapshot, command), time)
            .unwrap()
    }
    fn initialize(&mut self) -> Snapshot {
        let snapshot = self.discover();
        let command = Command::new(
            &snapshot,
            Action::Initialize {
                tasks: vec![],
                timer: None,
            },
        );
        self.send(&snapshot, command, 100).snapshot
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
fn fetch(snapshot: Option<&Snapshot>) -> ExchangeRequest {
    ExchangeRequest {
        version: PROTOCOL_VERSION,
        checkpoint: snapshot.map(Snapshot::checkpoint),
        command: None,
    }
}
fn request(snapshot: &Snapshot, command: Command) -> ExchangeRequest {
    ExchangeRequest {
        command: Some(command),
        ..fetch(Some(snapshot))
    }
}
fn create(snapshot: &Snapshot, title: &str) -> Command {
    Command::new(
        snapshot,
        Action::Queued {
            action: QueuedAction::Create {
                task: Id::new_v4(),
                title: title.into(),
                tag: None,
                bucket: Bucket::Next,
            },
        },
    )
}
fn focus(snapshot: &Snapshot, action: FocusAction, time: u64) -> Command {
    Command::new(
        snapshot,
        Action::Focus {
            guard: FocusGuard::observed(snapshot, time),
            action,
        },
    )
}
fn rejection(response: &ExchangeResponse) -> &Rejection {
    match &response.receipt.as_ref().unwrap().outcome {
        ReceiptOutcome::Rejected { reason } => reason,
        _ => panic!("expected rejection"),
    }
}

#[test]
fn lost_ack_retry_survives_reopen_and_does_not_duplicate() {
    let mut f = Fixture::new();
    let snapshot = f.initialize();
    let command = create(&snapshot, "created once");
    let first = f.send(&snapshot, command.clone(), 101);
    f.database = Database::open(&f.directory.join("queue.db")).unwrap();
    let second = f.send(&snapshot, command, 200);
    assert_eq!(first.receipt, second.receipt);
    assert_eq!(first.snapshot, second.snapshot);
    assert_eq!(second.snapshot.tasks.len(), 1);
    assert_eq!(second.device, f.device.device);
}

#[test]
fn focus_retry_reads_receipt_before_expiry_and_keeps_current_state() {
    let mut f = Fixture::new();
    let snapshot = f.initialize();
    let snapshot = f.send(&snapshot, create(&snapshot, "one"), 100).snapshot;
    let snapshot = f
        .send(
            &snapshot,
            focus(
                &snapshot,
                FocusAction::Promote {
                    task: snapshot.tasks[0].id,
                },
                100,
            ),
            100,
        )
        .snapshot;
    let complete = focus(&snapshot, FocusAction::Complete, 100);
    let first = f.send(&snapshot, complete.clone(), 101);
    let newer = f
        .send(&first.snapshot, create(&first.snapshot, "two"), 200)
        .snapshot;
    let retry = f.send(&snapshot, complete, 300);
    assert_eq!(first.receipt, retry.receipt);
    assert_eq!(retry.snapshot, newer);
    assert_eq!(retry.snapshot.tasks[0].title, "two");
}

#[test]
fn rejected_receipt_is_stable_after_prerequisites_change() {
    let mut f = Fixture::new();
    let initial = f.discover();
    let command = create(&initial, "not initialized");
    let rejected = f.send(&initial, command.clone(), 100);
    assert_eq!(rejection(&rejected), &Rejection::NotInitialized);
    f.initialize();
    let retried = f.send(&initial, command, 110);
    assert_eq!(rejected.receipt, retried.receipt);
    assert!(retried.snapshot.tasks.is_empty());
}

#[test]
fn receipts_are_bound_to_content_and_device() {
    let mut f = Fixture::new();
    let snapshot = f.initialize();
    let command = create(&snapshot, "one");
    f.send(&snapshot, command.clone(), 100);
    let mut different = create(&snapshot, "two");
    different.id = command.id;
    assert!(matches!(
        f.database
            .exchange(&f.device.token, &request(&snapshot, different), 101),
        Err(Error::ReceiptMismatch)
    ));
    let other = f.database.enroll("Ubuntu").unwrap();
    assert!(matches!(
        f.database
            .exchange(&other.token, &request(&snapshot, command), 101),
        Err(Error::ReceiptMismatch)
    ));
}

#[test]
fn revocation_and_rotation_preserve_receipt_owner() {
    let mut f = Fixture::new();
    let snapshot = f.initialize();
    let command = create(&snapshot, "once");
    let original = f.send(&snapshot, command.clone(), 100);
    f.database.revoke(f.device.device).unwrap();
    assert!(matches!(
        f.database.exchange(&f.device.token, &fetch(None), 101),
        Err(Error::Unauthorized)
    ));
    let rotated = f.database.rotate(f.device.device).unwrap();
    assert!(matches!(
        f.database.exchange(&f.device.token, &fetch(None), 101),
        Err(Error::Unauthorized)
    ));
    let retry = f
        .database
        .exchange(&rotated.token, &request(&snapshot, command), 102)
        .unwrap();
    assert_eq!(retry.device, f.device.device);
    assert_eq!(retry.receipt, original.receipt);
}

#[test]
fn time_never_moves_backwards_across_restart() {
    let mut f = Fixture::new();
    f.database
        .exchange(&f.device.token, &fetch(None), 500)
        .unwrap();
    f.database = Database::open(&f.directory.join("queue.db")).unwrap();
    assert_eq!(
        f.database
            .exchange(&f.device.token, &fetch(None), 400)
            .unwrap()
            .server_time,
        500
    );
}

#[test]
fn simultaneous_completions_only_advance_once() {
    let mut f = Fixture::new();
    let snapshot = f.initialize();
    let snapshot = f.send(&snapshot, create(&snapshot, "one"), 100).snapshot;
    let snapshot = f.send(&snapshot, create(&snapshot, "two"), 100).snapshot;
    let snapshot = f
        .send(
            &snapshot,
            focus(
                &snapshot,
                FocusAction::Promote {
                    task: snapshot.tasks[0].id,
                },
                100,
            ),
            100,
        )
        .snapshot;
    let second = focus(&snapshot, FocusAction::Complete, 100);
    let first = f.send(&snapshot, focus(&snapshot, FocusAction::Complete, 100), 101);
    let mut other_connection = Database::open(&f.directory.join("queue.db")).unwrap();
    let loser = other_connection
        .exchange(&f.device.token, &request(&snapshot, second), 101)
        .unwrap();
    assert_eq!(rejection(&loser), &Rejection::Conflict);
    assert_eq!(first.snapshot, loser.snapshot);
    assert_eq!(loser.snapshot.current().unwrap().title, "two");
}

#[test]
fn transaction_rolls_back_state_if_receipt_write_fails() {
    let mut f = Fixture::new();
    let snapshot = f.initialize();
    let sabotage = rusqlite::Connection::open(f.directory.join("queue.db")).unwrap();
    sabotage.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON receipts BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    let command = create(&snapshot, "must be atomic");
    assert!(matches!(
        f.database
            .exchange(&f.device.token, &request(&snapshot, command.clone()), 100),
        Err(Error::Storage(_))
    ));
    assert_eq!(f.discover(), snapshot);
    sabotage.execute_batch("DROP TRIGGER fail_receipt").unwrap();
    let committed = f.send(&snapshot, command, 100);
    assert_eq!(committed.snapshot.revision, snapshot.revision + 1);
    assert_eq!(committed.snapshot.tasks.len(), 1);
}

#[test]
fn raw_restored_fork_is_detected_even_after_passing_client_revision() {
    let mut f = Fixture::new();
    let snapshot = f.initialize();
    let backup = f.directory.join("backup.db");
    f.database.backup(&backup).unwrap();
    let command = create(&snapshot, "same command different result time");
    let original = f.send(&snapshot, command.clone(), 101).snapshot;
    let mut restored = Database::open(&backup).unwrap();
    let fork = restored
        .exchange(&f.device.token, &request(&snapshot, command), 102)
        .unwrap()
        .snapshot;
    assert_ne!(original.history, fork.history);
    restored
        .exchange(
            &f.device.token,
            &request(&fork, create(&fork, "past client")),
            103,
        )
        .unwrap();
    for req in [
        fetch(Some(&original)),
        request(&original, create(&original, "must not execute")),
    ] {
        assert!(matches!(
            restored.exchange(&f.device.token, &req, 104),
            Err(Error::Protocol(Rejection::HistoryMismatch))
        ));
    }
    assert_eq!(
        restored
            .exchange(&f.device.token, &fetch(None), 105)
            .unwrap()
            .snapshot
            .tasks
            .len(),
        2
    );
}

#[test]
fn backup_is_standalone_and_restore_rotates_generation() {
    let mut f = Fixture::new();
    let snapshot = f.initialize();
    let snapshot = f.send(&snapshot, create(&snapshot, "one"), 100).snapshot;
    let snapshot = f
        .send(
            &snapshot,
            focus(
                &snapshot,
                FocusAction::Promote {
                    task: snapshot.tasks[0].id,
                },
                100,
            ),
            100,
        )
        .snapshot;
    let backup = f.directory.join("backup.db");
    f.database.backup(&backup).unwrap();
    let saved = fs::read(&backup).unwrap();
    assert!(f.database.backup(&backup).is_err());
    assert_eq!(fs::read(&backup).unwrap(), saved);
    assert!(!f.directory.join("backup.db-wal").exists());
    let mut restored = Database::open(&backup).unwrap();
    assert_eq!(
        restored
            .exchange(&f.device.token, &fetch(Some(&snapshot)), 101)
            .unwrap()
            .snapshot,
        snapshot
    );
    restored.restore_generation().unwrap();
    assert!(matches!(
        restored.exchange(&f.device.token, &fetch(Some(&snapshot)), 101),
        Err(Error::Protocol(Rejection::WrongDataset))
    ));
    let fresh = restored
        .exchange(&f.device.token, &fetch(None), 101)
        .unwrap()
        .snapshot;
    assert_eq!(snapshot.revision, fresh.revision);
    assert_ne!(snapshot.generation, fresh.generation);
    assert_ne!(
        snapshot.focus.session.as_ref().unwrap().id,
        fresh.focus.session.as_ref().unwrap().id
    );
    assert_eq!(
        snapshot.focus.session.as_ref().unwrap().elapsed_at(105),
        fresh.focus.session.as_ref().unwrap().elapsed_at(105)
    );
    assert_ne!(snapshot.tasks[0].versions, fresh.tasks[0].versions);
}

#[test]
fn startup_never_creates_or_overwrites_and_storage_is_private() {
    let f = Fixture::new();
    assert!(Database::open(&f.directory.join("missing.db")).is_err());
    assert!(!f.directory.join("missing.db").exists());
    assert!(Database::create(&f.directory.join("queue.db")).is_err());
    assert_eq!(
        fs::metadata(&f.directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for file in ["queue.db", "queue.db-wal", "queue.db-shm"] {
        assert_eq!(
            fs::metadata(f.directory.join(file))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
    std::os::unix::fs::symlink(f.directory.join("queue.db"), f.directory.join("link.db")).unwrap();
    assert!(Database::open(&f.directory.join("link.db")).is_err());
    let connection = rusqlite::Connection::open(f.directory.join("queue.db")).unwrap();
    let token_hash: String = connection
        .query_row("SELECT token_hash FROM devices", [], |r| r.get(0))
        .unwrap();
    assert_ne!(token_hash, f.device.token);
}

#[tokio::test]
async fn http_auth_limits_bootstrap_and_round_trip() {
    let mut f = Fixture::new();
    let snapshot = f.initialize();
    let app = router(Database::open(&f.directory.join("queue.db")).unwrap());
    let send = |token: Option<&str>, bytes: Vec<u8>| {
        let mut request = Request::post("/v1/exchange").header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        app.clone()
            .oneshot(request.body(Body::from(bytes)).unwrap())
    };
    let bytes = serde_json::to_vec(&fetch(None)).unwrap();
    assert_eq!(
        send(None, bytes.clone()).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(Some("invalid"), bytes.clone()).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let response = send(Some(&f.device.token), bytes).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let data = to_bytes(response.into_body(), MAX_HTTP_BYTES)
        .await
        .unwrap();
    let response: ExchangeResponse = serde_json::from_slice(&data).unwrap();
    assert_eq!(response.snapshot, snapshot);
    let mut req = fetch(None);
    req.command = Some(create(&snapshot, "no checkpoint"));
    assert_eq!(
        send(Some(&f.device.token), serde_json::to_vec(&req).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(Some(&f.device.token), vec![b' '; MAX_HTTP_BYTES + 1])
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        app.oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}
