use super::*;
use crate::{Bucket, Tag};
use std::fs;

fn clock(n: u64) -> ClockSample {
    ClockSample {
        wall: 1000 + n,
        boot: n * 1_000_000_000,
    }
}
struct Fixture {
    dir: PathBuf,
    server: ServerState,
    client: Client,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("qf-journal-{}", Id::new_v4()));
        let mut server = ServerState::new(Id::new_v4(), Id::new_v4()).unwrap();
        let init = Command::new(
            &server.snapshot,
            Action::Initialize {
                tasks: vec![],
                timer: None,
            },
        );
        server.apply(&init, 100).unwrap();
        let envelope =
            Envelope::new(Id::new_v4(), server.snapshot.clone(), 100, clock(0).wall).unwrap();
        let path = dir.join(STATE_FILE);
        save_private(&path, &envelope, MAX_ENVELOPE_BYTES).unwrap();
        let client = Client::open(&path, envelope.device).unwrap();
        Self {
            dir,
            server,
            client,
        }
    }
    fn exchange(&mut self, n: u64) -> Option<Settlement> {
        let request = self.client.request().unwrap();
        let response = self.response(&request, 100 + n);
        self.client
            .accept(&request, response, clock(n))
            .unwrap()
            .value
    }
    fn response(&mut self, request: &ExchangeRequest, time: u64) -> ExchangeResponse {
        let receipt = request.command.as_ref().map(|command| Receipt {
            command: command.id,
            outcome: match self.server.apply(command, time) {
                Ok(()) => ReceiptOutcome::Applied {
                    revision: self.server.snapshot.revision,
                },
                Err(reason) => ReceiptOutcome::Rejected { reason },
            },
        });
        ExchangeResponse {
            version: PROTOCOL_VERSION,
            device: self.client.envelope.device,
            server_time: time,
            snapshot: self.server.snapshot.clone(),
            receipt,
        }
    }
    fn add(&mut self, title: &str) -> Id {
        let task = Id::new_v4();
        self.client
            .enqueue_queued(QueuedAction::Create {
                task,
                title: title.into(),
                tag: None,
                bucket: Bucket::Next,
            })
            .unwrap();
        task
    }
    fn reopen(&mut self) {
        self.client =
            Client::open(&self.dir.join(STATE_FILE), self.client.envelope.device).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn offline_journal_survives_restart_and_chained_edits_have_stable_handles() {
    let mut f = Fixture::new();
    let task = f.add("first");
    let create = f.client.envelope.pending[0].id;
    let handle = f.client.envelope.handle(task).unwrap();
    let rename = f
        .client
        .enqueue_queued(QueuedAction::Rename {
            task,
            expected: create,
            title: "second".into(),
        })
        .unwrap()
        .value;
    f.client
        .enqueue_queued(QueuedAction::SetTag {
            task,
            expected: create,
            tag: Some(Tag::Work),
        })
        .unwrap();
    let pending = f.client.envelope.pending.clone();
    f.reopen();
    assert_eq!(f.client.envelope.pending, pending);
    assert_eq!(f.client.store(clock(1)).tasks[0].title, "second");
    assert_eq!(f.client.store(clock(1)).tasks[0].id, handle);
    assert!(f.client.store(clock(1)).current().is_none());
    assert_eq!(f.client.status(clock(1)).state, ConnectionState::Offline);
    f.exchange(2);
    assert_eq!(f.client.envelope.pending[0].id, rename);
    assert_eq!(f.client.store(clock(2)).tasks.len(), 1);
    f.exchange(3);
    f.exchange(4);
    assert!(f.client.envelope.pending.is_empty());
    assert_eq!(f.client.store(clock(4)).tasks[0].id, handle);
    assert_eq!(f.client.store(clock(4)).tasks[0].title, "second");
    assert_eq!(f.client.store(clock(4)).tasks[0].tag, Some(Tag::Work));
}

#[test]
fn failed_predecessor_keeps_dependent_intents_and_independent_capture() {
    let mut f = Fixture::new();
    let task = f.add("original");
    f.exchange(0);
    let token = f.server.snapshot.task(task).unwrap().versions.title;
    let first = f
        .client
        .enqueue_queued(QueuedAction::Rename {
            task,
            expected: token,
            title: "local first".into(),
        })
        .unwrap()
        .value;
    f.client
        .enqueue_queued(QueuedAction::Rename {
            task,
            expected: first,
            title: "local second".into(),
        })
        .unwrap();
    let independent = f.add("independent");
    let remote = Command::new(
        &f.server.snapshot,
        Action::Queued {
            action: QueuedAction::Rename {
                task,
                expected: token,
                title: "remote".into(),
            },
        },
    );
    f.server.apply(&remote, 101).unwrap();
    f.exchange(2);
    assert_eq!(f.client.envelope.recovery.len(), 1);
    assert_eq!(f.client.status(clock(2)).likely_conflicts, 1);
    assert_eq!(
        f.client.projected_snapshot().task(task).unwrap().title,
        "remote"
    );
    assert!(f.client.projected_snapshot().task(independent).is_ok());
    f.exchange(3);
    f.exchange(4);
    assert_eq!(f.client.envelope.recovery.len(), 2);
    assert_eq!(
        f.client.envelope.recovery[1].command.action,
        Action::Queued {
            action: QueuedAction::Rename {
                task,
                expected: first,
                title: "local second".into()
            }
        }
    );
    assert!(f.client.envelope.pending.is_empty());
    f.reopen();
    assert_eq!(f.client.envelope.recovery.len(), 2);
}

#[test]
fn focus_is_never_optimistic_and_blocks_other_focus_but_not_capture() {
    let mut f = Fixture::new();
    let task = f.add("one");
    assert!(f
        .client
        .enqueue_focus(FocusAction::Promote { task }, clock(0))
        .is_err());
    f.exchange(0);
    let command = f
        .client
        .enqueue_focus(FocusAction::Promote { task }, clock(1))
        .unwrap()
        .value;
    assert!(f.client.store(clock(1)).current().is_none());
    assert_eq!(f.client.status(clock(1)).focus_command, Some(command));
    assert!(f
        .client
        .enqueue_focus(FocusAction::Promote { task }, clock(1))
        .is_err());
    f.add("captured behind pending focus");
    f.exchange(2);
    assert_eq!(f.client.store(clock(2)).current().unwrap().title, "one");
    assert!(f.client.status(clock(2)).focus_command.is_none());
    f.exchange(3);
    assert!(f
        .client
        .enqueue_focus(FocusAction::SetPaused { paused: true }, clock(9))
        .is_err());
}

#[test]
fn stale_focus_expires_definitively_and_is_preserved_for_recovery() {
    let mut f = Fixture::new();
    let task = f.add("one");
    f.exchange(0);
    f.client
        .enqueue_focus(FocusAction::Promote { task }, clock(1))
        .unwrap();
    f.reopen();
    assert!(f.client.status(clock(1)).focus_command.is_some());
    let receipt = f.exchange(20).unwrap();
    assert_eq!(
        receipt.outcome,
        ReceiptOutcome::Rejected {
            reason: Rejection::Expired
        }
    );
    assert!(f.client.store(clock(20)).current().is_none());
    assert_eq!(f.client.envelope.recovery.len(), 1);
}

#[test]
fn fetch_started_before_capture_keeps_new_journal_and_replays_once() {
    let mut f = Fixture::new();
    let request = f.client.request().unwrap();
    f.add("while fetching");
    let response = f.response(&request, 100);
    f.client.accept(&request, response, clock(0)).unwrap();
    assert_eq!(f.client.envelope.pending.len(), 1);
    assert_eq!(f.client.store(clock(0)).tasks.len(), 1);
    f.exchange(1);
    assert_eq!(f.client.store(clock(1)).tasks.len(), 1);
}

#[test]
fn missing_or_wrong_receipt_never_discards_journal() {
    let mut f = Fixture::new();
    f.add("one");
    let request = f.client.request().unwrap();
    let mut response = f.response(&request, 100);
    let envelope = f.client.envelope.clone();
    let receipt = response.receipt.take().unwrap();
    assert!(f
        .client
        .accept(&request, response.clone(), clock(0))
        .is_err());
    response.receipt = Some(Receipt {
        command: Id::new_v4(),
        ..receipt
    });
    assert!(f.client.accept(&request, response, clock(0)).is_err());
    assert_eq!(f.client.envelope, envelope);
    f.reopen();
    assert_eq!(f.client.envelope, envelope);
}

#[test]
fn failed_ack_save_retries_the_identical_request_then_accepts_cached_receipt() {
    let mut f = Fixture::new();
    f.add("accepted remotely");
    let request = f.client.request().unwrap();
    let response = f.response(&request, 100);
    let before = f.client.envelope.clone();
    // A regular file in place of the parent forces a pre-rename save error.
    let parked = f.dir.with_extension("parked");
    fs::rename(&f.dir, &parked).unwrap();
    fs::write(&f.dir, b"not a directory").unwrap();
    let result = f.client.accept(&request, response.clone(), clock(0));
    fs::remove_file(&f.dir).unwrap();
    fs::rename(&parked, &f.dir).unwrap();
    assert!(result.is_err());
    assert_eq!(f.client.envelope, before);
    assert_eq!(f.client.request(), Some(request.clone()));
    let acknowledged = f.client.accept(&request, response, clock(1)).unwrap();
    assert!(acknowledged.value.is_some());
    assert!(f.client.envelope.pending.is_empty());
    f.reopen();
    assert_eq!(f.client.store(clock(1)).tasks.len(), 1);
}

#[test]
fn full_pending_journal_refuses_new_capture_without_changing_existing_ids() {
    let mut f = Fixture::new();
    let mut full = f.client.envelope.clone();
    for n in 0..MAX_PENDING {
        full.pending.push(Command::new(
            &full.confirmed,
            Action::Queued {
                action: QueuedAction::Create {
                    task: Id::new_v4(),
                    title: format!("task {n}"),
                    tag: None,
                    bucket: Bucket::Next,
                },
            },
        ));
    }
    full.register_handles().unwrap();
    // Capacity is checked before projection or a save.
    full.validate().unwrap();
    f.client.envelope = full.clone();
    assert!(f
        .client
        .enqueue_queued(QueuedAction::Create {
            task: Id::new_v4(),
            title: "keep the draft".into(),
            tag: None,
            bucket: Bucket::Next
        })
        .is_err());
    assert_eq!(f.client.envelope, full);
}

#[test]
fn save_failure_before_rename_keeps_original_but_after_rename_adopts_identity() {
    let mut f = Fixture::new();
    let old = f.client.envelope.clone();
    let task = Id::new_v4();
    let command = Command::new(
        &old.confirmed,
        Action::Queued {
            action: QueuedAction::Create {
                task,
                title: "once".into(),
                tag: None,
                bucket: Bucket::Next,
            },
        },
    );
    let mut candidate = old.clone();
    candidate.pending.push(command.clone());
    candidate.register_handles().unwrap();
    assert!(f
        .client
        .install_with(candidate.clone(), |_, _| Err(SaveError::BeforeCommit(
            io::Error::other("disk full")
        )))
        .is_err());
    assert_eq!(f.client.envelope, old);
    let result = f
        .client
        .install_with(candidate.clone(), |path, value| {
            let bytes = serde_json::to_vec_pretty(value).unwrap();
            crate::store::save_bytes_with_sync(path, &bytes, |_| {
                Err(io::Error::other("directory sync failed"))
            })
        })
        .unwrap();
    assert!(result.warning.is_some());
    assert_eq!(f.client.envelope, candidate);
    assert!(f.client.request().is_none());
    assert!(f
        .client
        .enqueue_queued(QueuedAction::Create {
            task: Id::new_v4(),
            title: "retry must not recreate".into(),
            tag: None,
            bucket: Bucket::Next
        })
        .is_err());
    f.client.retry_durability().unwrap();
    assert_eq!(f.client.request().unwrap().command.unwrap().id, command.id);
    f.reopen();
    assert_eq!(f.client.envelope, candidate);
}

#[test]
fn ack_post_rename_failure_adopts_removal_and_recovery_atomically() {
    let mut f = Fixture::new();
    f.add("one");
    let mut next = f.client.envelope.clone();
    let command = next.pending.remove(0);
    next.recovery.push(RecoveryItem {
        command: command.clone(),
        reason: Rejection::Conflict,
    });
    f.client
        .install_with(next.clone(), |path, value| {
            let bytes = serde_json::to_vec_pretty(value).unwrap();
            crate::store::save_bytes_with_sync(path, &bytes, |_| {
                Err(io::Error::other("directory sync failed"))
            })
        })
        .unwrap();
    assert!(f.client.request().is_none());
    assert!(f.client.envelope.pending.is_empty());
    f.reopen();
    assert_eq!(f.client.envelope, next);
    assert_eq!(f.client.envelope.recovery[0].command.id, command.id);
}

#[test]
fn timer_uses_boot_elapsed_across_sleep_and_wall_jumps_without_poll_churn() {
    let mut f = Fixture::new();
    let task = f.add("timed");
    f.exchange(0);
    f.client
        .enqueue_focus(FocusAction::Promote { task }, clock(0))
        .unwrap();
    f.exchange(0);
    let initial = f.client.store(clock(0));
    assert_eq!(
        initial.current().unwrap().elapsed_secs(clock(0).wall),
        Some(0)
    );
    f.exchange(2);
    assert_eq!(f.client.store(clock(2)), initial);
    let sleep = ClockSample {
        wall: clock(0).wall + 3600,
        boot: clock(0).boot + 3_600_000_000_000,
    };
    let after = f.client.store(sleep);
    assert_eq!(after, initial);
    assert_eq!(
        after.current().unwrap().elapsed_secs(sleep.wall),
        Some(3600)
    );
    let jumped = ClockSample {
        wall: sleep.wall + 600,
        boot: sleep.boot,
    };
    let corrected = f.client.store(jumped);
    assert_eq!(
        corrected.current().unwrap().elapsed_secs(jumped.wall),
        Some(3600)
    );
    assert_eq!(f.client.status(jumped).state, ConnectionState::Offline);
}

#[test]
fn paused_timer_is_stable_and_offline_restart_uses_persisted_anchor() {
    let mut f = Fixture::new();
    let task = f.add("timed");
    f.exchange(0);
    f.client
        .enqueue_focus(FocusAction::Promote { task }, clock(0))
        .unwrap();
    f.exchange(0);
    f.exchange(3);
    f.client
        .enqueue_focus(FocusAction::SetPaused { paused: true }, clock(3))
        .unwrap();
    f.exchange(3);
    let paused = f.client.store(clock(3));
    assert_eq!(
        paused.current().unwrap().elapsed_secs(clock(30).wall),
        Some(3)
    );
    f.reopen();
    let projected = f.client.store(clock(30));
    assert_eq!(
        projected.current().unwrap().elapsed_secs(clock(30).wall),
        Some(3)
    );
    assert_eq!(f.client.status(clock(30)).state, ConnectionState::Offline);
    assert!(f
        .client
        .enqueue_focus(FocusAction::SetPaused { paused: false }, clock(30))
        .is_err());
}

#[test]
fn undo_is_only_offered_for_confirmed_unexpired_completion() {
    let mut f = Fixture::new();
    let task = f.add("one");
    f.exchange(0);
    f.client
        .enqueue_focus(FocusAction::Promote { task }, clock(0))
        .unwrap();
    f.exchange(0);
    let handle = f.client.envelope.handle(task).unwrap();
    f.client
        .enqueue_focus(FocusAction::Complete, clock(1))
        .unwrap();
    assert!(f.client.status(clock(1)).undo.is_none());
    f.exchange(1);
    let hint = f.client.status(clock(1)).undo.unwrap();
    assert_eq!(hint.task, handle);
    assert!(f.client.status(clock(10)).undo.is_none());
    f.client
        .enqueue_focus(
            FocusAction::Undo {
                completion: hint.command,
            },
            clock(2),
        )
        .unwrap();
    assert!(f.client.store(clock(2)).current().is_none());
    f.exchange(2);
    assert_eq!(f.client.store(clock(2)).current().unwrap().id, handle);
    assert!(f.client.status(clock(2)).undo.is_none());
}

#[test]
fn auth_suspension_preserves_head_and_history_quarantine_cannot_resume() {
    let mut f = Fixture::new();
    f.add("keep");
    let command = f.client.envelope.pending[0].clone();
    f.client.suspend("fix device token", false).unwrap();
    assert!(f.client.request().is_none());
    f.reopen();
    assert_eq!(
        f.client.status(clock(0)).state,
        ConnectionState::NeedsAttention
    );
    f.client.resume_after_repair().unwrap();
    assert_eq!(f.client.request().unwrap().command, Some(command.clone()));
    f.client.suspend("server history differs", true).unwrap();
    f.client.suspend("another error", false).unwrap();
    assert!(f.client.resume_after_repair().is_err());
    assert!(f
        .client
        .enqueue_queued(QueuedAction::Delete {
            task: Id::new_v4(),
            title_version: Id::new_v4()
        })
        .is_err());
    assert_eq!(f.client.envelope.pending, [command]);
}

#[test]
fn private_files_bounds_identity_and_lock_are_enforced() {
    let f = Fixture::new();
    assert!(Client::open(&f.dir.join(STATE_FILE), Id::new_v4()).is_err());
    let lock = DataLock::acquire(&f.dir).unwrap();
    assert!(DataLock::acquire(&f.dir).is_err());
    drop(lock);
    let _lock = DataLock::acquire(&f.dir).unwrap();
    let path = f.dir.join("bounded.json");
    assert!(save_private(&path, &f.client.envelope, 10).is_err());
    assert!(!path.exists());
    assert!(read_private::<Envelope>(&f.dir.join(STATE_FILE), 10).is_err());
    let mut corrupt = f.client.envelope.clone();
    corrupt.next_handle = 0;
    assert!(corrupt.validate().is_err());
    std::os::unix::fs::symlink(f.dir.join(STATE_FILE), &path).unwrap();
    assert!(read_private::<Envelope>(&path, MAX_ENVELOPE_BYTES).is_err());
}

#[test]
fn malformed_persisted_intent_fails_closed_in_pending_and_recovery() {
    let mut f = Fixture::new();
    let task = f.add("valid title");
    let original = f.client.envelope.clone();
    let token = original.pending[0].id;
    let bad_actions = [
        Action::Queued {
            action: QueuedAction::Create {
                task,
                title: "".into(),
                tag: None,
                bucket: Bucket::Next,
            },
        },
        Action::Queued {
            action: QueuedAction::Create {
                task,
                title: "x".repeat(257),
                tag: None,
                bucket: Bucket::Next,
            },
        },
        Action::Queued {
            action: QueuedAction::Create {
                task,
                title: "valid".into(),
                tag: None,
                bucket: Bucket::Now,
            },
        },
        Action::Queued {
            action: QueuedAction::Rename {
                task: Id::nil(),
                expected: token,
                title: "valid".into(),
            },
        },
        Action::Queued {
            action: QueuedAction::SetTag {
                task,
                expected: Id::nil(),
                tag: None,
            },
        },
        Action::Queued {
            action: QueuedAction::MoveBefore {
                task,
                expected: token,
                bucket: Bucket::Next,
                before: Some(Anchor {
                    task: Id::new_v4(),
                    placement: Id::nil(),
                }),
            },
        },
        Action::Focus {
            guard: FocusGuard {
                revision: 1,
                version: token,
                session: Some(Id::new_v4()),
                task: None,
                expires: 110,
            },
            action: FocusAction::Complete,
        },
        Action::Focus {
            guard: FocusGuard::observed(&original.confirmed, 100),
            action: FocusAction::Undo {
                completion: Id::nil(),
            },
        },
    ];
    for action in bad_actions {
        for recovery in [false, true] {
            let mut corrupted = original.clone();
            corrupted.pending[0].action = action.clone();
            if recovery {
                let command = corrupted.pending.remove(0);
                corrupted.recovery.push(RecoveryItem {
                    command,
                    reason: Rejection::Conflict,
                });
            }
            let path = f.dir.join(STATE_FILE);
            save_private(&path, &corrupted, MAX_ENVELOPE_BYTES).unwrap();
            let bytes = fs::read(&path).unwrap();
            assert!(Client::open(&path, corrupted.device).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }
}

#[test]
fn valid_conflicts_and_expired_focus_remain_loadable() {
    let mut f = Fixture::new();
    let task = f.add("one");
    f.exchange(0);
    let mut envelope = f.client.envelope.clone();
    envelope.pending.push(Command::new(
        &envelope.confirmed,
        Action::Queued {
            action: QueuedAction::Rename {
                task,
                expected: Id::new_v4(),
                title: "conflicting".into(),
            },
        },
    ));
    envelope.pending.push(Command::new(
        &envelope.confirmed,
        Action::Focus {
            guard: FocusGuard::observed(&envelope.confirmed, 0),
            action: FocusAction::Promote { task },
        },
    ));
    envelope.validate().unwrap();
    save_private(&f.dir.join(STATE_FILE), &envelope, MAX_ENVELOPE_BYTES).unwrap();
    f.reopen();
    assert_eq!(f.client.envelope.pending, envelope.pending);
    assert_eq!(f.client.status(clock(100)).likely_conflicts, 1);
    assert_eq!(
        f.client.request().unwrap().command.as_ref(),
        envelope.pending.first()
    );
}

#[test]
fn boot_clock_reads_and_freshness_ends_at_five_seconds() {
    assert!(ClockSample::now().unwrap().boot > 0);
    let mut f = Fixture::new();
    f.exchange(0);
    assert!(f.client.fresh(clock(5)));
    assert!(!f.client.fresh(ClockSample {
        wall: clock(5).wall,
        boot: clock(5).boot + 1
    }));
    f.client.network_failed();
    assert!(!f.client.fresh(clock(1)));
}
