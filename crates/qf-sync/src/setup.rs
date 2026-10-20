use crate::{Config, Transport};
use qf_core::{sync::*, Bucket, Store};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

pub const SETUP_FILE: &str = "sync-setup.json";
#[cfg(test)]
mod tests;
const SENTINEL: &str = "Sync is enabled; use a sync-capable Queue Focus build";
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Initialize,
    Import,
    Shared,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    id: Id,
    action: Action,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stage {
    version: u32,
    device: Id,
    server: String,
    mode: Mode,
    backup: String,
    backup_hash: String,
    source_missing: bool,
    plans: Vec<Plan>,
    next: usize,
    confirmed: Option<Snapshot>,
    server_time: u64,
    wall_time: u64,
    recovery: Vec<RecoveryItem>,
    handles: BTreeMap<Id, u64>,
    next_handle: u64,
}
impl Stage {
    fn validate(&self) -> Result<()> {
        if self.version != PROTOCOL_VERSION
            || self.device.is_nil()
            || !valid_hash(&self.backup_hash)
            || !self.backup.starts_with("sync-local-backup-")
            || !self.backup.ends_with(".json")
            || self.backup.contains('/')
            || self.backup.contains('\\')
            || self.plans.len() > MAX_TASKS
            || self.next > self.plans.len()
            || (self.confirmed.is_none() && self.next != 0)
        {
            return Err("invalid sync setup record".into());
        }
        if let Some(snapshot) = &self.confirmed {
            snapshot.validate()?;
        }
        let mut commands = BTreeSet::new();
        let mut tasks_seen = BTreeSet::new();
        for plan in &self.plans {
            self.bind(plan).validate_syntax()?;
            if !commands.insert(plan.id) {
                return Err("duplicate setup command".into());
            }
            match (&self.mode, &plan.action) {
                (Mode::Initialize, Action::Initialize { tasks, .. }) if self.plans.len() == 1 => {
                    for task in tasks {
                        if !tasks_seen.insert(task.id) {
                            return Err("duplicate setup task".into());
                        }
                    }
                }
                (
                    Mode::Import,
                    Action::Queued {
                        action: QueuedAction::Create { task, .. },
                    },
                ) => {
                    if !tasks_seen.insert(*task) {
                        return Err("duplicate import task".into());
                    }
                }
                _ => return Err("setup mode and commands disagree".into()),
            }
        }
        if self.mode == Mode::Initialize && self.plans.len() != 1 {
            return Err("missing initialization command".into());
        }
        if tasks_seen != self.handles.keys().copied().collect()
            || self.next_handle == 0
            || self
                .handles
                .values()
                .any(|n| *n == 0 || *n >= self.next_handle)
            || self.handles.values().collect::<BTreeSet<_>>().len() != self.handles.len()
        {
            return Err("invalid setup handle mapping".into());
        }
        for item in &self.recovery {
            if !self.plans[..self.next]
                .iter()
                .any(|p| self.bind(p) == item.command)
            {
                return Err("unknown setup recovery command".into());
            }
        }
        Ok(())
    }
    fn bind(&self, plan: &Plan) -> Command {
        // Before discovery, placeholders are for syntax validation only.
        // IDs/payloads are durable before discovery; bind the discovered
        // dataset/generation durably before sending the first mutation.
        Command {
            version: PROTOCOL_VERSION,
            id: plan.id,
            action: plan.action.clone(),
            dataset: self
                .confirmed
                .as_ref()
                .map_or(Id::from_u128(1), |s| s.dataset),
            generation: self
                .confirmed
                .as_ref()
                .map_or(Id::from_u128(1), |s| s.generation),
        }
    }
    fn save(&self, directory: &Path) -> Result<()> {
        self.validate()?;
        save_private(&directory.join(SETUP_FILE), self, MAX_ENVELOPE_BYTES)?;
        Ok(())
    }
}

pub fn preview(directory: &Path, config: Config) -> Result<serde_json::Value> {
    let _lock = DataLock::acquire(directory)?;
    let local = local_store(directory)?;
    let shared = Transport::new(config)?.exchange(&discovery())?;
    Ok(
        serde_json::json!({ "local_tasks": local.tasks, "shared_tasks": shared.snapshot.tasks,
        "shared_initialized": shared.snapshot.initialized,
        "import_note": "Every local task gets a new identity; local Now goes to Next. Equal titles are not deduplicated." }),
    )
}

/// Explicit mode, app closed and one writer. No destructive default.
pub fn setup(directory: &Path, config: Config, mode: Mode) -> Result<()> {
    let _lock = DataLock::acquire(directory)?;
    config.endpoint()?;
    if directory.join(SETUP_FILE).exists() {
        let stage = read_stage(directory)?;
        if stage.device != config.device || stage.server != config.url || stage.mode != mode {
            return Err(
                "setup already exists with different choices; resume it or export recovery".into(),
            );
        }
        config.save(directory)?;
        return resume_locked(directory);
    }
    if directory.join(STATE_FILE).exists() {
        return Err("sync is already active in this directory".into());
    }
    let store = local_store(directory)?;
    if store.tasks.len() > MAX_TASKS {
        return Err("local queue exceeds the 2,000-task sync limit".into());
    }
    let (bytes, source_missing) =
        match read_private_bytes(&directory.join("tasks.json"), MAX_ENVELOPE_BYTES) {
            Ok(bytes) => (bytes, false),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                (serde_json::to_vec_pretty(&store)?, true)
            }
            Err(e) => return Err(e.into()),
        };
    let backup = format!("sync-local-backup-{}.json", Id::new_v4());
    save_private_bytes(&directory.join(&backup), &bytes, MAX_ENVELOPE_BYTES)?;
    let mut handles = BTreeMap::new();
    let tasks: Vec<_> = store
        .tasks
        .iter()
        .map(|task| {
            let id = Id::new_v4();
            if mode != Mode::Shared {
                handles.insert(id, task.id);
            }
            SeedTask {
                id,
                title: task.title.clone(),
                tag: task.tag,
                bucket: task.bucket,
                created_at: task.created_at,
            }
        })
        .collect();
    let plans = match mode {
        Mode::Initialize => vec![Plan {
            id: Id::new_v4(),
            action: Action::Initialize {
                tasks,
                timer: store.current().map(|t| SeedTimer {
                    elapsed: t.elapsed_secs(qf_core::unix_now()).unwrap_or(0),
                    running: !t.is_paused(),
                }),
            },
        }],
        Mode::Import => tasks
            .into_iter()
            .map(|t| Plan {
                id: Id::new_v4(),
                action: Action::Queued {
                    action: QueuedAction::Create {
                        task: t.id,
                        title: t.title,
                        tag: t.tag,
                        bucket: if t.bucket == Bucket::Now {
                            Bucket::Next
                        } else {
                            t.bucket
                        },
                    },
                },
            })
            .collect(),
        Mode::Shared => vec![],
    };
    Stage {
        version: PROTOCOL_VERSION,
        device: config.device,
        server: config.url.clone(),
        mode,
        backup,
        backup_hash: fingerprint(&bytes)?,
        source_missing,
        plans,
        next: 0,
        confirmed: None,
        server_time: 0,
        wall_time: qf_core::unix_now(),
        recovery: vec![],
        handles,
        next_handle: store.next_id,
    }
    .save(directory)?;
    config.save(directory)?;
    resume_locked(directory)
}
pub fn resume(directory: &Path) -> Result<()> {
    let _lock = DataLock::acquire(directory)?;
    resume_locked(directory)
}
fn resume_locked(directory: &Path) -> Result<()> {
    sync_parent(&directory.join(SETUP_FILE))?;
    let mut stage = read_stage(directory)?;
    verify_source(directory, &stage)?;
    let config = Config::load(directory)?;
    if stage.device != config.device || stage.server != config.url {
        return Err("setup destination and credentials disagree".into());
    }
    let transport = Transport::new(config)?;
    if stage.confirmed.is_none() {
        let response = transport.exchange(&discovery())?;
        if stage.mode == Mode::Initialize && response.snapshot.initialized {
            return Err("shared queue already initialized; export this setup into a new directory, then preview and import".into());
        }
        if stage.mode != Mode::Initialize && !response.snapshot.initialized {
            return Err("first device has not initialized the shared queue".into());
        }
        if stage.mode == Mode::Import
            && response.snapshot.tasks.len() + stage.plans.len() > MAX_TASKS
        {
            return Err(
                "combined queues exceed 2,000 tasks; backup and setup are preserved".into(),
            );
        }
        stage.confirmed = Some(response.snapshot);
        stage.server_time = response.server_time;
        stage.wall_time = qf_core::unix_now();
        stage.save(directory)?;
    }
    while stage.next < stage.plans.len() {
        let confirmed = stage.confirmed.as_ref().ok_or("missing setup checkpoint")?;
        let command = stage.bind(&stage.plans[stage.next]);
        let request = ExchangeRequest {
            version: PROTOCOL_VERSION,
            checkpoint: Some(confirmed.checkpoint()),
            command: Some(command.clone()),
        };
        let response = transport.exchange(&request)?;
        let receipt = response
            .receipt
            .as_ref()
            .ok_or("setup response lacks a receipt")?;
        if receipt.command != command.id
            || response.snapshot.dataset != confirmed.dataset
            || response.snapshot.generation != confirmed.generation
            || response.snapshot.revision < confirmed.revision
            || response.server_time < stage.server_time
            || (response.snapshot.revision == confirmed.revision && response.snapshot != *confirmed)
        {
            return Err("invalid setup response; saved command identity is unchanged".into());
        }
        match &receipt.outcome {
            ReceiptOutcome::Applied { revision }
                if *revision > confirmed.revision && *revision <= response.snapshot.revision => {}
            ReceiptOutcome::Rejected { reason } => stage.recovery.push(RecoveryItem {
                command,
                reason: reason.clone(),
            }),
            _ => return Err("invalid setup receipt revision".into()),
        }
        stage.next += 1;
        stage.confirmed = Some(response.snapshot);
        stage.server_time = response.server_time;
        stage.wall_time = qf_core::unix_now();
        stage.save(directory)?;
    }
    if stage.mode == Mode::Initialize && !stage.recovery.is_empty() {
        return Err("initialization rejected; backup and intent are preserved for export".into());
    }
    verify_source(directory, &stage)?;
    let mut envelope = Envelope::new(
        stage.device,
        stage.confirmed.clone().ok_or("missing setup snapshot")?,
        stage.server_time,
        stage.wall_time,
    )?;
    envelope.handles = stage.handles.clone();
    envelope.next_handle = stage.next_handle;
    envelope.recovery = stage.recovery.clone();
    envelope.register_handles()?;
    envelope.validate()?;
    let path = directory.join(STATE_FILE);
    if path.exists() {
        let existing: Envelope = read_private(&path, MAX_ENVELOPE_BYTES)?;
        if existing != envelope {
            return Err(
                "existing sync state differs from activation; refusing to replace it".into(),
            );
        }
    } else {
        save_private(&path, &envelope, MAX_ENVELOPE_BYTES)?;
    }
    save_private(
        &directory.join("tasks.json"),
        &serde_json::json!({ "tasks": SENTINEL }),
        MAX_ENVELOPE_BYTES,
    )?;
    fs::remove_file(directory.join(SETUP_FILE))?;
    sync_parent(&path)?;
    Ok(())
}
fn read_stage(directory: &Path) -> Result<Stage> {
    let stage: Stage = read_private(&directory.join(SETUP_FILE), MAX_ENVELOPE_BYTES)?;
    stage.validate()?;
    if fingerprint(&read_private_bytes(
        &directory.join(&stage.backup),
        MAX_ENVELOPE_BYTES,
    )?)? != stage.backup_hash
    {
        return Err("setup backup differs from its recorded identity".into());
    }
    Ok(stage)
}
fn verify_source(directory: &Path, stage: &Stage) -> Result<()> {
    match read_private_bytes(&directory.join("tasks.json"), MAX_ENVELOPE_BYTES) {
        Ok(bytes) if fingerprint(&bytes)? == stage.backup_hash => Ok(()),
        Ok(bytes) if directory.join(STATE_FILE).exists() &&
            serde_json::from_slice::<serde_json::Value>(&bytes)? == serde_json::json!({ "tasks": SENTINEL }) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound && stage.source_missing => Ok(()),
        _ => Err("local tasks changed during setup; quit the old app and preserve both versions before continuing".into()),
    }
}
fn local_store(directory: &Path) -> Result<Store> {
    let path = directory.join("tasks.json");
    if path.exists() {
        let _: serde_json::Value = read_private(&path, MAX_ENVELOPE_BYTES)?;
    }
    Ok(qf_core::load(&path)?)
}
fn discovery() -> ExchangeRequest {
    ExchangeRequest {
        version: PROTOCOL_VERSION,
        checkpoint: None,
        command: None,
    }
}

/// Export to a new directory, never a downgrade in place; omit credentials.
pub fn export(directory: &Path, destination: &Path) -> Result<()> {
    let _lock = DataLock::acquire(directory)?;
    if destination.exists() {
        return Err("export destination must be a new directory".into());
    }
    let (store, intents) = if directory.join(SETUP_FILE).exists() {
        let stage = read_stage(directory)?;
        let backup: Store = read_private(&directory.join(&stage.backup), MAX_ENVELOPE_BYTES)?;
        // Preserve newer local edits if an old binary ignored the setup lock.
        let store = match read_private_bytes(&directory.join("tasks.json"), MAX_ENVELOPE_BYTES) {
            Ok(bytes)
                if serde_json::from_slice::<serde_json::Value>(&bytes)?
                    == serde_json::json!({ "tasks": SENTINEL }) =>
            {
                backup
            }
            Ok(_) => local_store(directory)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => backup,
            Err(error) => return Err(error.into()),
        };
        (store, serde_json::to_value(&stage)?)
    } else {
        let envelope: Envelope = read_private(&directory.join(STATE_FILE), MAX_ENVELOPE_BYTES)?;
        envelope.validate()?;
        let mut client = Client::open(&directory.join(STATE_FILE), envelope.device)?;
        (
            client.store(ClockSample::now()?),
            serde_json::to_value(&envelope)?,
        )
    };
    fs::create_dir(destination)?;
    let _destination_lock = DataLock::acquire(destination)?;
    // Interrupted export is recognizable, never an apparently empty queue.
    save_private(
        &destination.join(SETUP_FILE),
        &serde_json::json!({ "export_incomplete": true }),
        4096,
    )?;
    qf_core::save(&destination.join("tasks.json"), &store)?;
    save_private(
        &destination.join("sync-intents-export.json"),
        &intents,
        MAX_ENVELOPE_BYTES,
    )?;
    qf_core::save_settings(
        &destination.join("settings.json"),
        &qf_core::load_settings(&directory.join("settings.json"))?,
    )?;
    fs::remove_file(destination.join(SETUP_FILE))?;
    sync_parent(&destination.join("tasks.json"))?;
    Ok(())
}
