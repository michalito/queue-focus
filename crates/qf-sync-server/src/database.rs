use qf_core::sync::*;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

const SCHEMA: i64 = 1;

#[derive(Debug)]
pub enum Error {
    Unauthorized,
    ReceiptMismatch,
    Protocol(Rejection),
    Invalid(String),
    Storage(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => f.write_str("Device token is missing, invalid or revoked."),
            Self::ReceiptMismatch => {
                f.write_str("Command ID was already used by different content or another device.")
            }
            Self::Protocol(e) => e.fmt(f),
            Self::Invalid(e) | Self::Storage(e) => f.write_str(e),
        }
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.to_string())
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Storage(e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Storage(e.to_string())
    }
}
impl From<Rejection> for Error {
    fn from(e: Rejection) -> Self {
        Self::Protocol(e)
    }
}

/// Do not derive Debug: an enrollment contains a bearer credential.
pub struct Enrollment {
    pub device: Id,
    pub token: String,
}

pub struct Database {
    connection: Connection,
}

impl Database {
    /// Explicit creation only. Starting the HTTP service never creates a DB.
    pub fn create(path: &Path) -> Result<Self, Error> {
        let parent = parent(path);
        if parent.exists() {
            if fs::metadata(parent)?.permissions().mode() & 0o077 != 0 {
                return Err(Error::Invalid(
                    "use a dedicated data directory with mode 0700".into(),
                ));
            }
        } else {
            fs::create_dir_all(parent)?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.sync_all()?;
        let mut connection = open_connection(path)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE dataset (id INTEGER PRIMARY KEY CHECK(id=1), state TEXT NOT NULL, last_time INTEGER NOT NULL);
             CREATE TABLE history (revision INTEGER PRIMARY KEY, fingerprint TEXT NOT NULL);
             CREATE TABLE devices (id TEXT PRIMARY KEY, label TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE, revoked INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE receipts (command TEXT PRIMARY KEY, device TEXT NOT NULL REFERENCES devices(id), fingerprint TEXT NOT NULL, outcome TEXT NOT NULL);
             PRAGMA user_version=1;"
        )?;
        let state = ServerState::new(Id::new_v4(), Id::new_v4())?;
        tx.execute(
            "INSERT INTO dataset VALUES (1, ?1, 0)",
            [serde_json::to_string(&state)?],
        )?;
        tx.execute(
            "INSERT INTO history VALUES (0, ?1)",
            [&state.snapshot.history],
        )?;
        tx.commit()?;
        fs::File::open(parent)?.sync_all()?;
        Ok(Self { connection })
    }

    pub fn open(path: &Path) -> Result<Self, Error> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(Error::Invalid(
                "database must be an existing regular file".into(),
            ));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        let connection = open_connection(path)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != SCHEMA {
            return Err(Error::Invalid(
                "unsupported database schema; refusing to serve".into(),
            ));
        }
        let state = read_state(&connection)?;
        state.validate()?;
        let known: Option<String> = connection
            .query_row(
                "SELECT fingerprint FROM history WHERE revision=?1",
                [integer(state.snapshot.revision)?],
                |row| row.get(0),
            )
            .optional()?;
        state
            .snapshot
            .checkpoint()
            .verify(&state.snapshot, known.as_deref())?;
        Ok(Self { connection })
    }

    pub fn enroll(&mut self, label: &str) -> Result<Enrollment, Error> {
        if label.trim().is_empty() || label.chars().count() > 128 {
            return Err(Error::Invalid(
                "device label must contain 1–128 characters".into(),
            ));
        }
        let enrollment = Enrollment {
            device: Id::new_v4(),
            token: new_token(),
        };
        self.connection.execute(
            "INSERT INTO devices(id,label,token_hash) VALUES (?1,?2,?3)",
            params![
                enrollment.device.to_string(),
                label,
                fingerprint(&enrollment.token)?
            ],
        )?;
        Ok(enrollment)
    }

    /// Rotation retains the device identity used by all old command receipts.
    pub fn rotate(&mut self, device: Id) -> Result<Enrollment, Error> {
        let enrollment = Enrollment {
            device,
            token: new_token(),
        };
        let changed = self.connection.execute(
            "UPDATE devices SET token_hash=?1, revoked=0 WHERE id=?2",
            params![fingerprint(&enrollment.token)?, device.to_string()],
        )?;
        if changed != 1 {
            return Err(Error::Invalid("unknown device".into()));
        }
        Ok(enrollment)
    }

    pub fn revoke(&mut self, device: Id) -> Result<(), Error> {
        if self.connection.execute(
            "UPDATE devices SET revoked=1 WHERE id=?1",
            [device.to_string()],
        )? != 1
        {
            return Err(Error::Invalid("unknown device".into()));
        }
        Ok(())
    }

    pub fn exchange(
        &mut self,
        token: &str,
        request: &ExchangeRequest,
        sampled_time: u64,
    ) -> Result<ExchangeResponse, Error> {
        self.exchange_inner(token, None, request, sampled_time)
    }

    /// Bind the token to the persisted installation before executing work.
    pub fn exchange_for_device(
        &mut self,
        token: &str,
        device: Id,
        request: &ExchangeRequest,
        sampled_time: u64,
    ) -> Result<ExchangeResponse, Error> {
        self.exchange_inner(token, Some(device), request, sampled_time)
    }

    fn exchange_inner(
        &mut self,
        token: &str,
        expected_device: Option<Id>,
        request: &ExchangeRequest,
        sampled_time: u64,
    ) -> Result<ExchangeResponse, Error> {
        request.validate()?;
        if token.len() > 256 {
            return Err(Error::Unauthorized);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device: Option<String> = tx
            .query_row(
                "SELECT id FROM devices WHERE token_hash=?1 AND revoked=0",
                [fingerprint(&token)?],
                |row| row.get(0),
            )
            .optional()?;
        let device = device
            .ok_or(Error::Unauthorized)?
            .parse::<Id>()
            .map_err(|_| Error::Storage("invalid enrolled device identity".into()))?;
        if expected_device.is_some_and(|expected| expected != device) {
            return Err(Error::Unauthorized);
        }
        let mut state = read_state(&tx)?;
        let last_time: i64 =
            tx.query_row("SELECT last_time FROM dataset WHERE id=1", [], |row| {
                row.get(0)
            })?;
        let server_time = sampled_time.max(
            u64::try_from(last_time).map_err(|_| Error::Storage("invalid server time".into()))?,
        );
        if let Some(checkpoint) = &request.checkpoint {
            let historical: Option<String> = tx
                .query_row(
                    "SELECT fingerprint FROM history WHERE revision=?1",
                    [integer(checkpoint.revision)?],
                    |row| row.get(0),
                )
                .optional()?;
            checkpoint.verify(&state.snapshot, historical.as_deref())?;
        }
        let receipt = if let Some(command) = &request.command {
            let hash = command.fingerprint()?;
            let prior: Option<(String, String, String)> = tx
                .query_row(
                    "SELECT device,fingerprint,outcome FROM receipts WHERE command=?1",
                    [command.id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let outcome = if let Some((owner, fingerprint, outcome)) = prior {
                if owner != device.to_string() || hash != fingerprint {
                    return Err(Error::ReceiptMismatch);
                }
                serde_json::from_str::<ReceiptOutcome>(&outcome)?
            } else {
                let outcome = match state.apply(command, server_time) {
                    Ok(()) => {
                        tx.execute(
                            "INSERT INTO history VALUES (?1,?2)",
                            params![integer(state.snapshot.revision)?, state.snapshot.history],
                        )?;
                        tx.execute(
                            "UPDATE dataset SET state=?1 WHERE id=1",
                            [serde_json::to_string(&state)?],
                        )?;
                        ReceiptOutcome::Applied {
                            revision: state.snapshot.revision,
                        }
                    }
                    Err(reason) => ReceiptOutcome::Rejected { reason },
                };
                tx.execute(
                    "INSERT INTO receipts VALUES (?1,?2,?3,?4)",
                    params![
                        command.id.to_string(),
                        device.to_string(),
                        hash,
                        serde_json::to_string(&outcome)?
                    ],
                )?;
                outcome
            };
            Some(Receipt {
                command: command.id,
                outcome,
            })
        } else {
            None
        };
        tx.execute(
            "UPDATE dataset SET last_time=?1 WHERE id=1",
            [integer(server_time)?],
        )?;
        tx.commit()?;
        Ok(ExchangeResponse {
            version: PROTOCOL_VERSION,
            device,
            server_time,
            snapshot: state.snapshot,
            receipt,
        })
    }

    /// Produces a consistent, standalone DB artifact. Never copy a live WAL DB.
    pub fn backup(&self, destination: &Path) -> Result<(), Error> {
        let temporary = parent(destination).join(format!(".backup-{}.tmp", Id::new_v4()));
        let result = (|| {
            let file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&temporary)?;
            let mut target = Connection::open_with_flags(
                sqlite_path(&temporary)?,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
            )?;
            let backup = rusqlite::backup::Backup::new(&self.connection, &mut target)?;
            backup.run_to_completion(64, Duration::from_millis(10), None)?;
            drop(backup);
            target.close().map_err(|(_, e)| Error::from(e))?;
            file.sync_all()?;
            // Atomic publication without replacing an existing backup.
            fs::hard_link(&temporary, destination)?;
            fs::File::open(parent(destination))?.sync_all()?;
            Ok(())
        })();
        let _ = fs::remove_file(&temporary);
        result
    }

    /// Run against a *copy* of a backup, while the service is stopped.
    /// Establish a new history root; keep used IDs, devices and receipts so
    /// recovering clients cannot accidentally reuse identities.
    pub fn restore_generation(&mut self) -> Result<(), Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = read_state(&tx)?;
        state.reset_generation(Id::new_v4())?;
        tx.execute("DELETE FROM history", [])?;
        tx.execute(
            "INSERT INTO history VALUES (?1,?2)",
            params![integer(state.snapshot.revision)?, state.snapshot.history],
        )?;
        tx.execute(
            "UPDATE dataset SET state=?1 WHERE id=1",
            [serde_json::to_string(&state)?],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn healthy(&self) -> bool {
        self.connection
            .query_row("SELECT 1 FROM dataset WHERE id=1", [], |r| {
                r.get::<_, i64>(0)
            })
            .is_ok()
    }
}

fn open_connection(path: &Path) -> Result<Connection, Error> {
    let conn = Connection::open_with_flags(
        sqlite_path(path)?,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
    )?;
    Ok(conn)
}
// macOS /var and /tmp are directory symlinks. Resolve the directory while
// keeping NOFOLLOW on the final filename (SQLite checks every component).
fn sqlite_path(path: &Path) -> Result<std::path::PathBuf, Error> {
    let filename = path
        .file_name()
        .ok_or_else(|| Error::Invalid("missing database filename".into()))?;
    Ok(fs::canonicalize(parent(path))?.join(filename))
}
fn read_state(conn: &Connection) -> Result<ServerState, Error> {
    let json: String = conn.query_row("SELECT state FROM dataset WHERE id=1", [], |r| r.get(0))?;
    let state: ServerState = serde_json::from_str(&json)?;
    state.validate()?;
    Ok(state)
}
fn integer(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::Invalid("integer exceeds database range".into()))
}
fn new_token() -> String {
    format!("{}{}", Id::new_v4().simple(), Id::new_v4().simple())
}
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
