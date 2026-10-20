use crate::{Config, Failure, Transport};
use qf_core::sync::*;
use std::{
    io,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread,
};

struct WorkResult {
    request: ExchangeRequest,
    result: Result<(ExchangeResponse, ClockSample), Failure>,
}
#[derive(Default)]
pub struct WorkerUpdate {
    pub settlement: Option<Settlement>,
    pub warning: Option<String>,
}

pub struct Worker {
    sender: SyncSender<ExchangeRequest>,
    receiver: Receiver<WorkResult>,
    busy: bool,
    next: u64,
    backoff: u64,
    wake: bool,
    halted: bool,
    discard_inflight: bool,
    last_tick: Option<u64>,
}
impl Worker {
    pub fn start(config: Config) -> io::Result<Self> {
        config.endpoint().map_err(io::Error::other)?;
        let (sender, incoming) = mpsc::sync_channel(1);
        let (outgoing, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("queue-focus-sync".into())
            .spawn(move || {
                let transport = Transport::new(config);
                while let Ok(request) = incoming.recv() {
                    let result = match &transport {
                        Ok(transport) => transport.exchange(&request).and_then(|response| {
                            ClockSample::now()
                                .map(|clock| (response, clock))
                                .map_err(|_| Failure::Clock)
                        }),
                        Err(error) => Err(error.clone()),
                    };
                    if outgoing.send(WorkResult { request, result }).is_err() {
                        break;
                    }
                }
            })?;
        // Dropping these channels stops the worker after its bounded in-flight
        // request. Do not join here: UI shutdown must not wait on the network.
        Ok(Self {
            sender,
            receiver,
            busy: false,
            next: 0,
            backoff: 1,
            wake: true,
            halted: false,
            discard_inflight: false,
            last_tick: None,
        })
    }
    pub fn wake(&mut self) {
        self.wake = true;
    }
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// An engine reopened its durable journal after a panic. Let any old
    /// request finish, then exchange the reopened head/checkpoint. Its receipt
    /// is still idempotent; applying a response to a different journal is not.
    pub fn engine_reopened(&mut self) {
        self.discard_inflight = self.busy;
        self.wake = true;
        self.next = 0;
    }

    /// Call on the existing serialized engine boundary. Only file saves and
    /// bounded channel operations happen here; HTTP is entirely on the worker.
    pub fn tick(&mut self, client: &mut Client, clock: ClockSample) -> WorkerUpdate {
        let mut update = WorkerUpdate::default();
        // The boot clock includes sleep. A resumed main loop should reconnect
        // promptly rather than wait out a pre-sleep network backoff.
        if self
            .last_tick
            .is_some_and(|last| clock.boot.saturating_sub(last) > 5_000_000_000)
        {
            self.wake = true;
        }
        self.last_tick = Some(clock.boot);
        if self.busy {
            match self.receiver.try_recv() {
                Ok(work) => {
                    self.busy = false;
                    if self.discard_inflight {
                        self.discard_inflight = false;
                        client.network_failed();
                        return update;
                    }
                    match work.result {
                        Ok((response, received)) => {
                            self.backoff = 1;
                            self.next = received.boot.saturating_add(2_000_000_000);
                            match client.accept(&work.request, response, received) {
                                Ok(outcome) => {
                                    update.settlement = outcome.value;
                                    update.warning = outcome.warning.map(|w| w.to_string());
                                    if !client.envelope().pending.is_empty() {
                                        self.next = 0;
                                    }
                                }
                                Err(error) => {
                                    client.network_failed();
                                    update.warning = Some(error.to_string());
                                    if error.kind() == io::ErrorKind::InvalidData {
                                        self.halted = true;
                                        let _ = client.suspend("Sync could not reconcile the response. Check storage and server configuration, then retry; pending commands are retained.", false);
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            client.network_failed();
                            if error.retryable() {
                                self.next = clock.boot.saturating_add(self.backoff * 1_000_000_000);
                                self.backoff = (self.backoff * 2).min(30);
                            } else {
                                self.halted = true;
                                update.warning = Some(error.to_string());
                                if let Err(save) =
                                    client.suspend(&error.to_string(), error.recovery_required())
                                {
                                    update.warning = Some(save.to_string());
                                }
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.busy = false;
                    self.halted = true;
                    client.network_failed();
                    let message = "Sync worker stopped. Restart the app; pending commands have been retained.";
                    let _ = client.suspend(message, false);
                    update.warning = Some(message.into());
                }
            }
        }
        if client.status(clock).state == ConnectionState::Durability {
            if let Err(error) = client.retry_durability() {
                update.warning = Some(error.to_string());
            }
        }
        if !self.halted && !self.busy && (self.wake || clock.boot >= self.next) {
            if let Some(request) = client.request() {
                self.wake = false;
                match self.sender.try_send(request) {
                    Ok(()) => self.busy = true,
                    Err(_) => {
                        client.network_failed();
                        update.warning =
                            Some("Sync worker is unavailable; restart the app.".into());
                    }
                }
            }
        }
        update
    }
}
