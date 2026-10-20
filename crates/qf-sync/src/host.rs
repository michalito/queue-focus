//! Process lifetime resources stay outside the recoverable engine.
use crate::{setup::SETUP_FILE, Config, Worker, WorkerUpdate, CONFIG_FILE};
use qf_core::{sync::*, Engine};
use std::{fs, io, path::Path};

pub struct Host {
    _lock: DataLock,
    worker: Option<Worker>,
}

impl Host {
    /// Acquire before reading state. Even a standalone app prevents setup
    /// from converting its files underneath the running process.
    pub fn open(directory: &Path) -> io::Result<(Self, Engine, Option<String>)> {
        Self::open_at(directory, qf_core::unix_now)
    }

    pub fn open_at(
        directory: &Path,
        now: impl FnOnce() -> u64,
    ) -> io::Result<(Self, Engine, Option<String>)> {
        let lock = DataLock::acquire(directory)?;
        let (engine, warning) = open_engine_at(directory, now)?;
        let worker = if engine.sync_status().is_some() {
            Some(Worker::start(Config::load(directory)?)?)
        } else {
            None
        };
        Ok((
            Self {
                _lock: lock,
                worker,
            },
            engine,
            warning,
        ))
    }

    pub fn wake(&mut self) {
        if let Some(worker) = &mut self.worker {
            worker.wake();
        }
    }

    pub fn retry_sync(
        &mut self,
        engine: &mut Engine,
    ) -> Result<qf_core::Outcome<()>, qf_core::EngineError> {
        let outcome = engine.retry_sync()?;
        if let Some(worker) = &mut self.worker {
            worker.retry();
        }
        Ok(outcome)
    }

    /// Export without releasing the source lock or changing its pending work.
    pub fn export(&self, engine: &Engine, destination: &Path) -> crate::setup::Result<()> {
        let envelope = engine.sync_envelope().ok_or("sync is not enabled")?;
        crate::setup::export_snapshot(
            destination,
            engine.store(),
            &serde_json::to_value(envelope)?,
            engine.settings(),
        )
    }

    pub fn engine_reopened(&mut self) {
        if let Some(worker) = &mut self.worker {
            worker.engine_reopened();
        }
    }

    /// Channel receive/send and local persistence only. The worker owns HTTP.
    pub fn pump(&mut self, engine: &mut Engine) -> WorkerUpdate {
        let clock = match ClockSample::now() {
            Ok(clock) => clock,
            Err(error) => {
                return WorkerUpdate {
                    settlement: None,
                    warning: Some(error.to_string()),
                }
            }
        };
        let update = match (&mut self.worker, engine.sync_client_mut()) {
            (Some(worker), Some(client)) => worker.tick(client, clock),
            _ => WorkerUpdate::default(),
        };
        engine.refresh_sync(clock);
        update
    }
}

/// Reopen only while the host still holds its lock. Does not spawn a worker.
/// A partially activated or damaged sync directory must never fall back to
/// standalone mode, even if tasks.json looks readable.
pub fn open_engine(directory: &Path) -> io::Result<(Engine, Option<String>)> {
    open_engine_at(directory, qf_core::unix_now)
}

fn open_engine_at(
    directory: &Path,
    now: impl FnOnce() -> u64,
) -> io::Result<(Engine, Option<String>)> {
    if present(&directory.join(SETUP_FILE))? {
        return Err(io::Error::other("Sync setup is incomplete. Close the app and run qf-sync resume, or export to a new directory."));
    }
    let state = present(&directory.join(STATE_FILE))?;
    let config = present(&directory.join(CONFIG_FILE))?;
    match (state, config) {
        (false, false) => Engine::open(directory, now),
        (true, true) => {
            let config = Config::load(directory)?;
            let client = Client::open(&directory.join(STATE_FILE), config.device)?;
            let mut clock = ClockSample::now()?;
            clock.wall = now();
            Ok(Engine::open_with_sync(directory, client, clock))
        }
        _ => Err(io::Error::other("Sync state or credentials are missing. Restore the missing file or export to a new directory; the queue has not been changed.")),
    }
}

fn present(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}
