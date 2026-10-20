use super::*;
use qf_core::{sync::*, Bucket, Store};
use qf_sync_server::Database;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

#[test]
fn host_locks_standalone_and_refuses_partial_sync_directories() {
    let dir = std::env::temp_dir().join(format!("qf-host-{}", Id::new_v4()));
    let (host, mut engine, _) = Host::open(&dir).unwrap();
    engine.add("standalone", None).unwrap();
    assert!(Host::open(&dir).is_err());
    assert!(DataLock::acquire(&dir).is_err());
    // Engine recovery can reopen while the same host retains its lock.
    assert_eq!(open_engine(&dir).unwrap().0.store().tasks.len(), 1);
    drop(host);
    for marker in [setup::SETUP_FILE, STATE_FILE, CONFIG_FILE] {
        fs::write(dir.join(marker), b"broken").unwrap();
        assert!(
            Host::open(&dir).is_err(),
            "must not fall back with {marker}"
        );
        fs::remove_file(dir.join(marker)).unwrap();
    }
    assert_eq!(open_engine(&dir).unwrap().0.store().tasks.len(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn live_host_exports_pending_intent_and_unsaved_settings_without_changing_source() {
    let mut server = Server::new();
    let source = server.initialize();
    let (host, mut engine, _) = Host::open(&source).unwrap();
    engine.add("offline capture", None).unwrap();
    engine.update_settings(|settings| settings.show_timer = false);
    let before = fs::read(source.join(STATE_FILE)).unwrap();
    let destination = server.dir.join("live-export");
    host.export(&engine, &destination).unwrap();
    assert_eq!(
        qf_core::load(&destination.join("tasks.json"))
            .unwrap()
            .tasks[0]
            .title,
        "offline capture"
    );
    assert!(
        !qf_core::load_settings(&destination.join("settings.json"))
            .unwrap()
            .show_timer
    );
    let exported: Envelope = read_private(
        &destination.join("sync-intents-export.json"),
        MAX_ENVELOPE_BYTES,
    )
    .unwrap();
    assert_eq!(exported.pending, engine.sync_envelope().unwrap().pending);
    assert!(!destination.join(CONFIG_FILE).exists());
    assert!(!destination.join(STATE_FILE).exists());
    assert!(!destination.join(setup::SETUP_FILE).exists());
    assert_eq!(fs::read(source.join(STATE_FILE)).unwrap(), before);
    assert!(
        DataLock::acquire(&source).is_err(),
        "host retains source ownership"
    );
    assert!(
        host.export(&engine, &destination).is_err(),
        "never reuse a destination"
    );
    assert!(
        host.export(&engine, &source).is_err(),
        "never downgrade in place"
    );
}

#[test]
fn native_retry_restarts_suspended_transport_with_the_same_pending_command() {
    let mut server = Server::new();
    let directory = server.initialize();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    Config {
        url: format!("http://{}", listener.local_addr().unwrap()),
        ..server.config.clone()
    }
    .save(&directory)
    .unwrap();
    let mut database = Database::open(&server.dir.join("server/queue.db")).unwrap();
    let token = server.config.token.clone();
    let peer = thread::spawn(move || {
        let mut requests = vec![];
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut header = vec![];
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
                assert!(header.len() < 8192);
            }
            let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
            let size: usize = header
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert!(size < MAX_HTTP_BYTES);
            let mut body = vec![0; size];
            socket.read_exact(&mut body).unwrap();
            let request: ExchangeRequest = serde_json::from_slice(&body).unwrap();
            let (status, body) = if attempt == 0 {
                ("401 Unauthorized", vec![])
            } else {
                let response = database
                    .exchange(&token, &request, qf_core::unix_now())
                    .unwrap();
                ("200 OK", serde_json::to_vec(&response).unwrap())
            };
            requests.push(request);
            write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            socket.write_all(&body).unwrap();
        }
        requests
    });
    let (mut host, mut engine, _) = Host::open(&directory).unwrap();
    engine.add("survive suspended worker", None).unwrap();
    let pending = engine.sync_envelope().unwrap().pending.clone();
    let deadline = Instant::now() + Duration::from_secs(5);
    while engine.sync_envelope().unwrap().suspension.is_none() {
        host.pump(&mut engine);
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(engine.sync_envelope().unwrap().pending, pending);
    host.retry_sync(&mut engine).unwrap();
    while engine.sync_status().unwrap().pending != 0 {
        host.pump(&mut engine);
        assert!(
            Instant::now() < deadline,
            "retry must unhalt the existing worker"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let requests = peer.join().unwrap();
    assert_eq!(requests[0].command, requests[1].command);
    assert_eq!(requests[1].command.as_ref(), pending.first());
    assert_eq!(engine.store().tasks.len(), 1);
    engine
        .sync_client_mut()
        .unwrap()
        .suspend("history mismatch", true)
        .unwrap();
    let quarantined = engine.sync_envelope().unwrap().clone();
    assert!(host.retry_sync(&mut engine).is_err());
    assert_eq!(engine.sync_envelope(), Some(&quarantined));
}

#[test]
fn two_native_hosts_share_queue_now_pause_and_settlement() {
    let mut server = Server::new();
    let mac = server.initialize();
    let ubuntu = server.dir.join("ubuntu");
    let config = server.enroll();
    setup::setup(&ubuntu, config, setup::Mode::Shared).unwrap();
    let (mut mac_host, mut mac_engine, _) = Host::open(&mac).unwrap();
    let (mut ubuntu_host, mut ubuntu_engine, _) = Host::open(&ubuntu).unwrap();
    let task = mac_engine.add("shared", None).unwrap().value;
    fn converge(
        first: (&mut Host, &mut qf_core::Engine),
        second: (&mut Host, &mut qf_core::Engine),
        ready: impl Fn(&qf_core::Engine, &qf_core::Engine) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            first.0.wake();
            second.0.wake();
            assert!(first.0.pump(first.1).warning.is_none());
            assert!(second.0.pump(second.1).warning.is_none());
            if ready(first.1, second.1) {
                break;
            }
            assert!(Instant::now() < deadline, "hosts did not converge");
            thread::sleep(Duration::from_millis(10));
        }
    }
    converge(
        (&mut mac_host, &mut mac_engine),
        (&mut ubuntu_host, &mut ubuntu_engine),
        |a, b| a.sync_status().unwrap().pending == 0 && b.store().tasks.len() == 1,
    );
    let command = match mac_engine.promote(task) {
        Err(qf_core::EngineError::Pending { command_id }) => command_id,
        other => panic!("expected pending, got {other:?}"),
    };
    assert!(mac_engine.store().current().is_none());
    converge(
        (&mut mac_host, &mut mac_engine),
        (&mut ubuntu_host, &mut ubuntu_engine),
        |a, b| a.store().current().is_some() && b.store().current().is_some(),
    );
    assert_eq!(
        mac_engine
            .sync_status()
            .unwrap()
            .last_focus_settlement
            .as_ref()
            .unwrap()
            .command,
        command
    );
    assert!(matches!(
        ubuntu_engine.toggle_pause(),
        Err(qf_core::EngineError::Pending { .. })
    ));
    converge(
        (&mut mac_host, &mut mac_engine),
        (&mut ubuntu_host, &mut ubuntu_engine),
        |a, b| a.store().current().unwrap().is_paused() && b.store().current().unwrap().is_paused(),
    );
    assert_eq!(
        mac_engine
            .store()
            .current()
            .unwrap()
            .elapsed_secs(qf_core::unix_now()),
        ubuntu_engine
            .store()
            .current()
            .unwrap()
            .elapsed_secs(qf_core::unix_now())
    );
}

pub(crate) struct Server {
    pub dir: PathBuf,
    pub database: Database,
    pub config: Config,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

#[test]
fn worker_reopen_discards_old_response_and_retries_durable_head_once() {
    let mut server = Server::new();
    let directory = server.initialize();
    let (mut host, mut engine, _) = Host::open(&directory).unwrap();
    engine.add("survive reopen", None).unwrap();
    assert!(host.pump(&mut engine).warning.is_none());
    // The owned worker continues its first exchange while the recoverable
    // engine is reconstructed from the journal.
    engine = open_engine(&directory).unwrap().0;
    host.engine_reopened();
    let deadline = Instant::now() + Duration::from_secs(5);
    while engine.sync_status().unwrap().pending != 0 {
        assert!(host.pump(&mut engine).warning.is_none());
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(engine.store().tasks.len(), 1);
    let response = Transport::new(server.config.clone())
        .unwrap()
        .exchange(&engine.sync_client_mut().unwrap().request().unwrap())
        .unwrap();
    assert_eq!(
        response.snapshot.tasks.len(),
        1,
        "receipt retry cannot duplicate the create"
    );
    assert!(engine.sync_status().unwrap().message.is_none());
}
impl Server {
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("qf-transport-{}", Id::new_v4()));
        let mut database = Database::create(&dir.join("server/queue.db")).unwrap();
        let enrolled = database.enroll("Mac").unwrap();
        let http_db = Database::open(&dir.join("server/queue.db")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let thread = thread::spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                    axum::serve(listener, qf_sync_server::router(http_db))
                        .with_graceful_shutdown(async {
                            let _ = stopped.await;
                        })
                        .await
                        .unwrap();
                });
        });
        Self {
            dir,
            database,
            config: Config {
                version: PROTOCOL_VERSION,
                device: enrolled.device,
                token: enrolled.token,
                url: format!("http://{address}"),
                allow_loopback_http: true,
            },
            stop: Some(stop),
            thread: Some(thread),
        }
    }
    pub fn enroll(&mut self) -> Config {
        let e = self.database.enroll("Ubuntu").unwrap();
        Config {
            device: e.device,
            token: e.token,
            ..self.config.clone()
        }
    }
    pub fn initialize(&mut self) -> PathBuf {
        let path = self.dir.join("mac");
        qf_core::save(&path.join("tasks.json"), &Store::new()).unwrap();
        setup::setup(&path, self.config.clone(), setup::Mode::Initialize).unwrap();
        path
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn origins_require_https_or_explicit_literal_loopback_without_credentials() {
    let server = Server::new();
    for url in [
        "https://sync.example.com",
        "https://sync.example.com:8443/",
        "http://127.0.0.1:1234",
        "http://[::1]:1234",
    ] {
        let c = Config {
            url: url.into(),
            ..server.config.clone()
        };
        assert!(c.endpoint().is_ok());
    }
    for url in [
        "http://sync.example.com",
        "http://localhost",
        "https://user:secret@example.com",
        "https://example.com/?token=x",
        "https://example.com/path",
        "https://example.com/#x",
        "file:///tmp/x",
    ] {
        let c = Config {
            url: url.into(),
            ..server.config.clone()
        };
        assert!(c.endpoint().is_err(), "accepted {url}");
    }
    let c = Config {
        allow_loopback_http: false,
        ..server.config.clone()
    };
    assert!(c.endpoint().is_err());
}

fn fake_response(response: Vec<u8>) -> (Config, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let thread = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            let mut b = [0];
            socket.read_exact(&mut b).unwrap();
            header.push(b[0]);
            assert!(header.len() < 8192);
        }
        let _ = socket.write_all(&response);
    });
    (
        Config {
            version: 1,
            url: format!("http://{address}"),
            device: Id::new_v4(),
            token: "private".into(),
            allow_loopback_http: true,
        },
        thread,
    )
}
fn fetch() -> ExchangeRequest {
    ExchangeRequest {
        version: 1,
        checkpoint: None,
        command: None,
    }
}

#[test]
fn redirects_are_not_followed_and_oversized_bodies_are_rejected() {
    let (config, thread) = fake_response(b"HTTP/1.1 302 Found\r\nLocation: https://example.com/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec());
    assert!(matches!(
        Transport::new(config).unwrap().exchange(&fetch()),
        Err(Failure::Http { status: 302, .. })
    ));
    thread.join().unwrap();
    let (config, thread) = fake_response(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_HTTP_BYTES + 1
        )
        .into_bytes(),
    );
    assert_eq!(
        Transport::new(config).unwrap().exchange(&fetch()),
        Err(Failure::InvalidResponse)
    );
    thread.join().unwrap();
}

#[test]
fn worker_settles_without_blocking_and_auth_failure_keeps_exact_head() {
    let mut server = Server::new();
    let directory = server.initialize();
    let mut client = Client::open(&directory.join(STATE_FILE), server.config.device).unwrap();
    let command = client
        .enqueue_queued(QueuedAction::Create {
            task: Id::new_v4(),
            title: "from worker".into(),
            tag: None,
            bucket: Bucket::Next,
        })
        .unwrap()
        .value;
    let mut worker = Worker::start(server.config.clone()).unwrap();
    let start = Instant::now();
    let mut settled = None;
    while settled.is_none() && start.elapsed() < Duration::from_secs(5) {
        settled = worker
            .tick(&mut client, ClockSample::now().unwrap())
            .settlement;
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(settled.unwrap().command, command);
    assert!(client.envelope().pending.is_empty());
    drop(worker);
    client
        .enqueue_queued(QueuedAction::Create {
            task: Id::new_v4(),
            title: "retain through 401".into(),
            tag: None,
            bucket: Bucket::Next,
        })
        .unwrap();
    let pending = client.envelope().pending.clone();
    let mut worker = Worker::start(Config {
        token: "wrong".into(),
        ..server.config.clone()
    })
    .unwrap();
    let start = Instant::now();
    while client.envelope().suspension.is_none() && start.elapsed() < Duration::from_secs(5) {
        worker.tick(&mut client, ClockSample::now().unwrap());
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        client.status(ClockSample::now().unwrap()).state,
        ConnectionState::NeedsAttention
    );
    assert_eq!(client.envelope().pending, pending);
    assert!(!worker.busy());
}

#[test]
fn transport_rejects_wrong_device_and_exposes_history_recovery_without_queue_overwrite() {
    let mut server = Server::new();
    let directory = server.initialize();
    let client = Client::open(&directory.join(STATE_FILE), server.config.device).unwrap();
    let wrong = Config {
        device: Id::new_v4(),
        ..server.config.clone()
    };
    let transport = Transport::new(wrong).unwrap();
    assert!(matches!(
        transport.exchange(&fetch()),
        Err(Failure::Http { status: 401, .. })
    ));
    let mut request = client.request().unwrap();
    request.command = Some(Command::new(
        &client.envelope().confirmed,
        Action::Queued {
            action: QueuedAction::Create {
                task: Id::new_v4(),
                title: "must not run with wrong device".into(),
                tag: None,
                bucket: Bucket::Next,
            },
        },
    ));
    assert!(matches!(
        transport.exchange(&request),
        Err(Failure::Http { status: 401, .. })
    ));
    assert!(Transport::new(server.config.clone())
        .unwrap()
        .exchange(&fetch())
        .unwrap()
        .snapshot
        .tasks
        .is_empty());
    server.database.restore_generation().unwrap();
    let error = Transport::new(server.config.clone())
        .unwrap()
        .exchange(&client.request().unwrap())
        .unwrap_err();
    assert!(error.recovery_required());
    assert!(!error.retryable());
}
