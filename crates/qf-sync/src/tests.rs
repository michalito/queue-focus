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

pub(crate) struct Server {
    pub dir: PathBuf,
    pub database: Database,
    pub config: Config,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
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
