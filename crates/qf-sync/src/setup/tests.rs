use super::*;
use crate::tests::Server;
use crate::CONFIG_FILE;

#[test]
fn first_device_preserves_timer_handles_backup_and_blocks_old_binaries() {
    let server = Server::new();
    let dir = server.dir.join("mac");
    let mut store = Store::new();
    let now = qf_core::unix_now();
    let id = store.add("current", Bucket::Now, None);
    store.tasks[0].started_at = Some(now - 50);
    store.tasks[0].paused_at = Some(now - 20);
    store.add("next", Bucket::Next, None);
    qf_core::save(&dir.join("tasks.json"), &store).unwrap();
    let original = fs::read(dir.join("tasks.json")).unwrap();
    setup(&dir, server.config.clone(), Mode::Initialize).unwrap();
    assert!(!dir.join(SETUP_FILE).exists());
    assert!(qf_core::load(&dir.join("tasks.json")).is_err());
    let envelope: Envelope = read_private(&dir.join(STATE_FILE), MAX_ENVELOPE_BYTES).unwrap();
    let current = envelope.confirmed.current().unwrap();
    assert_eq!(envelope.handle(current.id), Some(id));
    assert_eq!(
        envelope.confirmed.focus.session.as_ref().unwrap().elapsed,
        30
    );
    assert!(!envelope.confirmed.focus.session.as_ref().unwrap().running);
    let backup = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("sync-local-backup-")
        })
        .unwrap();
    assert_eq!(fs::read(backup.path()).unwrap(), original);
}

#[test]
fn second_device_imports_local_now_to_next_without_deduplication_or_focus_change() {
    let mut server = Server::new();
    let mac = server.dir.join("mac");
    let mut first = Store::new();
    first.add("same title", Bucket::Now, None);
    qf_core::save(&mac.join("tasks.json"), &first).unwrap();
    setup(&mac, server.config.clone(), Mode::Initialize).unwrap();
    let original: Envelope = read_private(&mac.join(STATE_FILE), MAX_ENVELOPE_BYTES).unwrap();
    let linux = server.dir.join("ubuntu");
    let mut second = Store::new();
    second.add("same title", Bucket::Now, None);
    second.add("same title", Bucket::Next, None);
    qf_core::save(&linux.join("tasks.json"), &second).unwrap();
    let config = server.enroll();
    setup(&linux, config, Mode::Import).unwrap();
    let joined: Envelope = read_private(&linux.join(STATE_FILE), MAX_ENVELOPE_BYTES).unwrap();
    assert_eq!(joined.confirmed.tasks.len(), 3);
    assert_eq!(joined.confirmed.focus, original.confirmed.focus);
    assert_eq!(
        joined
            .confirmed
            .tasks
            .iter()
            .filter(|t| t.bucket == Bucket::Next)
            .count(),
        2
    );
    assert!(joined.pending.is_empty());
}

#[test]
fn setup_staged_before_network_and_lost_ack_resumes_same_ids() {
    let mut server = Server::new();
    let dir = server.dir.join("mac");
    let mut store = Store::new();
    store.add("once", Bucket::Now, None);
    qf_core::save(&dir.join("tasks.json"), &store).unwrap();
    let bad = Config {
        token: "invalid".into(),
        ..server.config.clone()
    };
    assert!(setup(&dir, bad, Mode::Initialize).is_err());
    let mut stage = read_stage(&dir).unwrap();
    assert_eq!(stage.next, 0);
    assert!(stage.confirmed.is_none());
    server.config.save(&dir).unwrap();
    // Simulate discovery being durable, then a committed command whose
    // acknowledgement was lost before the setup record could be advanced.
    let response = server
        .database
        .exchange(&server.config.token, &discovery(), qf_core::unix_now())
        .unwrap();
    stage.confirmed = Some(response.snapshot);
    stage.server_time = response.server_time;
    stage.save(&dir).unwrap();
    let command = stage.bind(&stage.plans[0]);
    let request = ExchangeRequest {
        version: 1,
        checkpoint: Some(stage.confirmed.as_ref().unwrap().checkpoint()),
        command: Some(command.clone()),
    };
    let accepted = server
        .database
        .exchange(&server.config.token, &request, qf_core::unix_now())
        .unwrap();
    resume(&dir).unwrap();
    let joined: Envelope = read_private(&dir.join(STATE_FILE), MAX_ENVELOPE_BYTES).unwrap();
    assert_eq!(joined.confirmed, accepted.snapshot);
    assert_eq!(joined.confirmed.tasks.len(), 1);
}

#[test]
fn interrupted_activation_after_sentinel_is_idempotently_resumable() {
    let mut server = Server::new();
    let dir = server.dir.join("mac");
    let bad = Config {
        token: "invalid".into(),
        ..server.config.clone()
    };
    assert!(setup(&dir, bad, Mode::Initialize).is_err());
    server.config.save(&dir).unwrap();
    let stage_before = read_stage(&dir).unwrap();
    resume(&dir).unwrap();
    let envelope: Envelope = read_private(&dir.join(STATE_FILE), MAX_ENVELOPE_BYTES).unwrap();
    // Reconstitute the exact final stage that was removed as the last step.
    let mut final_stage = stage_before;
    final_stage.next = final_stage.plans.len();
    final_stage.confirmed = Some(envelope.confirmed.clone());
    final_stage.server_time = envelope.server_time;
    final_stage.wall_time = envelope.wall_time;
    final_stage.save(&dir).unwrap();
    let prior = fs::read(dir.join(STATE_FILE)).unwrap();
    resume(&dir).unwrap();
    assert_eq!(fs::read(dir.join(STATE_FILE)).unwrap(), prior);
    assert!(!dir.join(SETUP_FILE).exists());
    // Other device still sees one authoritative history.
    let config = server.enroll();
    let response = Transport::new(config)
        .unwrap()
        .exchange(&discovery())
        .unwrap();
    assert_eq!(response.snapshot.revision, envelope.confirmed.revision);
}

#[test]
fn changed_local_source_blocks_resume_and_exports_newer_local_work() {
    let server = Server::new();
    let dir = server.dir.join("mac");
    let mut store = Store::new();
    store.add("original", Bucket::Now, None);
    qf_core::save(&dir.join("tasks.json"), &store).unwrap();
    assert!(setup(
        &dir,
        Config {
            token: "invalid".into(),
            ..server.config.clone()
        },
        Mode::Initialize
    )
    .is_err());
    server.config.save(&dir).unwrap();
    store.add("new old-app edit", Bucket::Next, None);
    qf_core::save(&dir.join("tasks.json"), &store).unwrap();
    assert!(resume(&dir).is_err());
    let destination = server.dir.join("recovered");
    export(&dir, &destination).unwrap();
    assert_eq!(
        qf_core::load(&destination.join("tasks.json")).unwrap(),
        store
    );
    assert!(dir.join(SETUP_FILE).exists());
    assert!(!destination.join(CONFIG_FILE).exists());
    assert!(destination.join("sync-intents-export.json").exists());
}

#[test]
fn shared_only_is_explicit_and_export_never_overwrites_destination_or_source() {
    let mut server = Server::new();
    let mac = server.initialize();
    let dir = server.dir.join("ubuntu");
    let mut local = Store::new();
    local.add("keep in backup", Bucket::Now, None);
    qf_core::save(&dir.join("tasks.json"), &local).unwrap();
    let config = server.enroll();
    setup(&dir, config.clone(), Mode::Shared).unwrap();
    let before = fs::read(dir.join(STATE_FILE)).unwrap();
    let client = Client::open(&dir.join(STATE_FILE), config.device).unwrap();
    assert!(client.envelope().confirmed.tasks.is_empty());
    let out = server.dir.join("export");
    export(&dir, &out).unwrap();
    assert_eq!(fs::read(dir.join(STATE_FILE)).unwrap(), before);
    assert!(!out.join(CONFIG_FILE).exists());
    assert!(export(&dir, &out).is_err());
    assert!(export(&dir, &mac).is_err());
}

#[test]
fn locks_and_corrupt_backup_stop_setup_without_network_mutations() {
    let server = Server::new();
    let dir = server.dir.join("mac");
    let lock = DataLock::acquire(&dir).unwrap();
    assert!(setup(&dir, server.config.clone(), Mode::Initialize).is_err());
    drop(lock);
    assert!(setup(
        &dir,
        Config {
            token: "invalid".into(),
            ..server.config.clone()
        },
        Mode::Initialize
    )
    .is_err());
    let stage = read_stage(&dir).unwrap();
    fs::write(dir.join(stage.backup), b"{}").unwrap();
    assert!(resume(&dir).is_err());
    let snapshot = Transport::new(server.config.clone())
        .unwrap()
        .exchange(&discovery())
        .unwrap()
        .snapshot;
    assert!(!snapshot.initialized);
}
