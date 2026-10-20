use super::Id;
use crate::{Bucket, Tag, MAX_TITLE_CHARS};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_TASKS: usize = 2_000;
pub const MAX_PENDING: usize = 1_000;
pub const MAX_HTTP_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 3 * 1024 * 1024;
pub const MAX_ENVELOPE_BYTES: usize = 32 * 1024 * 1024;
pub const FOCUS_TTL_SECS: u64 = 10;
pub const FRESH_SECS: u64 = 5;
pub const UNDO_SECS: u64 = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Versions {
    pub title: Id,
    pub tag: Id,
    pub placement: Id,
}

impl Versions {
    pub fn new(command: Id) -> Self {
        Self {
            title: command,
            tag: command,
            placement: command,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncTask {
    pub id: Id,
    pub title: String,
    pub tag: Option<Tag>,
    pub bucket: Bucket,
    pub created_at: u64,
    pub versions: Versions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub id: Id,
    pub task: Id,
    pub elapsed: u64,
    pub running: bool,
    pub anchor: u64,
}

impl Session {
    pub fn elapsed_at(&self, now: u64) -> u64 {
        self.elapsed.saturating_add(if self.running {
            now.saturating_sub(self.anchor)
        } else {
            0
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Focus {
    pub version: Id,
    pub session: Option<Session>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoHint {
    pub command: Id,
    pub task: Id,
    pub title: String,
    pub until: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u32,
    pub dataset: Id,
    pub generation: Id,
    pub revision: u64,
    pub history: String,
    pub initialized: bool,
    /// Relative order within each bucket is authoritative.
    pub tasks: Vec<SyncTask>,
    pub focus: Focus,
    pub undo: Option<UndoHint>,
}

impl Snapshot {
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            dataset: self.dataset,
            generation: self.generation,
            revision: self.revision,
            history: self.history.clone(),
        }
    }

    pub fn task(&self, id: Id) -> Result<&SyncTask, Rejection> {
        self.tasks
            .iter()
            .find(|t| t.id == id)
            .ok_or(Rejection::MissingTask)
    }

    pub fn current(&self) -> Option<&SyncTask> {
        self.tasks.iter().find(|t| t.bucket == Bucket::Now)
    }

    pub fn validate(&self) -> Result<(), Rejection> {
        if self.version != PROTOCOL_VERSION {
            return Err(Rejection::UnsupportedVersion);
        }
        valid_id(self.dataset)?;
        valid_id(self.generation)?;
        valid_id(self.focus.version)?;
        if !valid_hash(&self.history) {
            return Err(invalid("invalid history fingerprint"));
        }
        if self.tasks.len() > MAX_TASKS {
            return Err(Rejection::Capacity);
        }
        let mut ids = BTreeSet::new();
        for t in &self.tasks {
            valid_id(t.id)?;
            valid_title(&t.title)?;
            valid_id(t.versions.title)?;
            valid_id(t.versions.tag)?;
            valid_id(t.versions.placement)?;
            if !ids.insert(t.id) {
                return Err(invalid("duplicate task ID"));
            }
        }
        let current: Vec<_> = self
            .tasks
            .iter()
            .filter(|t| t.bucket == Bucket::Now)
            .collect();
        match (&self.focus.session, current.as_slice()) {
            (None, []) => {}
            (Some(s), [t]) if s.task == t.id => {
                valid_id(s.id)?;
            }
            _ => return Err(invalid("focus and Now disagree")),
        }
        if !self.initialized
            && (!self.tasks.is_empty() || self.revision != 0 || self.undo.is_some())
        {
            return Err(invalid("uninitialized dataset contains changes"));
        }
        if self.initialized && self.revision == 0 {
            return Err(invalid(
                "initialized dataset has no initialization revision",
            ));
        }
        if let Some(undo) = &self.undo {
            valid_id(undo.command)?;
            valid_id(undo.task)?;
            valid_title(&undo.title)?;
            if ids.contains(&undo.task) {
                return Err(invalid("undo task is still live"));
            }
        }
        if serde_json::to_vec(self)
            .map_err(|_| invalid("cannot serialize snapshot"))?
            .len()
            > MAX_SNAPSHOT_BYTES
        {
            return Err(Rejection::Capacity);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub dataset: Id,
    pub generation: Id,
    pub revision: u64,
    pub history: String,
}

impl Checkpoint {
    /// The caller must obtain `historical_hash` from durable server history,
    /// not merely compare the client's revision with the current revision.
    pub fn verify(
        &self,
        current: &Snapshot,
        historical_hash: Option<&str>,
    ) -> Result<(), Rejection> {
        if !valid_hash(&self.history) {
            return Err(Rejection::HistoryMismatch);
        }
        if self.dataset != current.dataset || self.generation != current.generation {
            return Err(Rejection::WrongDataset);
        }
        if self.revision > current.revision || historical_hash != Some(self.history.as_str()) {
            return Err(Rejection::HistoryMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub task: Id,
    pub placement: Id,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum QueuedAction {
    Create {
        task: Id,
        title: String,
        tag: Option<Tag>,
        bucket: Bucket,
    },
    Rename {
        task: Id,
        expected: Id,
        title: String,
    },
    SetTag {
        task: Id,
        expected: Id,
        tag: Option<Tag>,
    },
    MoveBefore {
        task: Id,
        expected: Id,
        bucket: Bucket,
        before: Option<Anchor>,
    },
    Delete {
        task: Id,
        title_version: Id,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusGuard {
    pub revision: u64,
    pub version: Id,
    pub session: Option<Id>,
    pub task: Option<Id>,
    pub expires: u64,
}

impl FocusGuard {
    pub fn observed(snapshot: &Snapshot, server_time: u64) -> Self {
        Self {
            revision: snapshot.revision,
            version: snapshot.focus.version,
            session: snapshot.focus.session.as_ref().map(|s| s.id),
            task: snapshot.focus.session.as_ref().map(|s| s.task),
            expires: server_time.saturating_add(FOCUS_TTL_SECS),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FocusAction {
    Promote {
        task: Id,
    },
    Create {
        task: Id,
        title: String,
        tag: Option<Tag>,
    },
    Complete,
    Remove,
    MoveToQueue {
        bucket: Bucket,
        before: Option<Anchor>,
    },
    SetPaused {
        paused: bool,
    },
    Undo {
        completion: Id,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedTask {
    pub id: Id,
    pub title: String,
    pub tag: Option<Tag>,
    pub bucket: Bucket,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedTimer {
    pub elapsed: u64,
    pub running: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Queued {
        action: QueuedAction,
    },
    Focus {
        guard: FocusGuard,
        action: FocusAction,
    },
    Initialize {
        tasks: Vec<SeedTask>,
        timer: Option<SeedTimer>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub version: u32,
    pub dataset: Id,
    pub generation: Id,
    pub id: Id,
    pub action: Action,
}

impl Command {
    /// Validate stored intent without consulting mutable queue state or time.
    /// A stale token, deleted target or expired focus deadline is still valid
    /// intent that must be retained for a definitive server receipt.
    pub fn validate_syntax(&self) -> Result<(), Rejection> {
        if self.version != PROTOCOL_VERSION {
            return Err(Rejection::UnsupportedVersion);
        }
        for id in [self.id, self.dataset, self.generation] {
            valid_id(id)?;
        }
        let queued_bucket = |bucket: Bucket| {
            if bucket == Bucket::Now {
                Err(invalid("queued destination cannot be Now"))
            } else {
                Ok(())
            }
        };
        let anchor = |before: &Option<Anchor>, target: Option<Id>| {
            if let Some(before) = before {
                valid_id(before.task)?;
                valid_id(before.placement)?;
                if Some(before.task) == target {
                    return Err(invalid("a task cannot anchor itself"));
                }
            }
            Ok(())
        };
        match &self.action {
            Action::Queued { action } => match action {
                QueuedAction::Create {
                    task,
                    title,
                    bucket,
                    ..
                } => {
                    valid_id(*task)?;
                    valid_title(title)?;
                    queued_bucket(*bucket)?;
                }
                QueuedAction::Rename {
                    task,
                    expected,
                    title,
                } => {
                    valid_id(*task)?;
                    valid_id(*expected)?;
                    valid_title(title)?;
                }
                QueuedAction::SetTag { task, expected, .. } => {
                    valid_id(*task)?;
                    valid_id(*expected)?;
                }
                QueuedAction::MoveBefore {
                    task,
                    expected,
                    bucket,
                    before,
                } => {
                    valid_id(*task)?;
                    valid_id(*expected)?;
                    queued_bucket(*bucket)?;
                    anchor(before, Some(*task))?;
                }
                QueuedAction::Delete {
                    task,
                    title_version,
                } => {
                    valid_id(*task)?;
                    valid_id(*title_version)?;
                }
            },
            Action::Focus { guard, action } => {
                valid_id(guard.version)?;
                if guard.expires == 0 || guard.task.is_some() != guard.session.is_some() {
                    return Err(invalid("invalid focus guard structure"));
                }
                for id in [guard.task, guard.session].into_iter().flatten() {
                    valid_id(id)?;
                }
                match action {
                    FocusAction::Create { task, title, .. } => {
                        valid_id(*task)?;
                        valid_title(title)?;
                    }
                    FocusAction::Promote { task } => valid_id(*task)?,
                    FocusAction::Undo { completion } => valid_id(*completion)?,
                    FocusAction::MoveToQueue { bucket, before } => {
                        queued_bucket(*bucket)?;
                        anchor(before, guard.task)?;
                    }
                    FocusAction::Complete | FocusAction::Remove | FocusAction::SetPaused { .. } => {
                    }
                }
            }
            Action::Initialize { tasks, timer } => {
                if tasks.len() > MAX_TASKS {
                    return Err(Rejection::Capacity);
                }
                let mut ids = BTreeSet::new();
                for task in tasks {
                    valid_id(task.id)?;
                    valid_title(&task.title)?;
                    if !ids.insert(task.id) {
                        return Err(invalid("duplicate seed task"));
                    }
                }
                let current = tasks.iter().filter(|t| t.bucket == Bucket::Now).count();
                if !matches!((current, timer), (0, None) | (1, Some(_))) {
                    return Err(invalid("initial Now requires exactly one matching timer"));
                }
            }
        }
        Ok(())
    }

    pub fn new(snapshot: &Snapshot, action: Action) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            dataset: snapshot.dataset,
            generation: snapshot.generation,
            id: Id::new_v4(),
            action,
        }
    }

    pub fn is_focus(&self) -> bool {
        matches!(self.action, Action::Focus { .. })
    }

    pub fn fingerprint(&self) -> Result<String, Rejection> {
        fingerprint(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReceiptOutcome {
    Applied { revision: u64 },
    Rejected { reason: Rejection },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub command: Id,
    pub outcome: ReceiptOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExchangeRequest {
    pub version: u32,
    /// None is authenticated setup discovery only, and cannot carry a command.
    pub checkpoint: Option<Checkpoint>,
    pub command: Option<Command>,
}

impl ExchangeRequest {
    pub fn validate(&self) -> Result<(), Rejection> {
        if self.version != PROTOCOL_VERSION {
            return Err(Rejection::UnsupportedVersion);
        }
        if self.command.is_some() && self.checkpoint.is_none() {
            return Err(invalid("commands require a confirmed checkpoint"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExchangeResponse {
    pub version: u32,
    pub device: Id,
    pub server_time: u64,
    pub snapshot: Snapshot,
    pub receipt: Option<Receipt>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "detail", rename_all = "snake_case")]
pub enum Rejection {
    UnsupportedVersion,
    WrongDataset,
    HistoryMismatch,
    NotInitialized,
    AlreadyInitialized,
    MissingTask,
    Conflict,
    Expired,
    Capacity,
    Invalid(String),
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedVersion => "This sync protocol version is not supported.",
            Self::WrongDataset => "The shared queue identity changed. Recovery is required.",
            Self::HistoryMismatch => {
                "The server history differs from the confirmed queue. Recovery is required."
            }
            Self::NotInitialized => "The shared queue has not been initialized.",
            Self::AlreadyInitialized => "The shared queue is already initialized.",
            Self::MissingTask => "The task no longer exists.",
            Self::Conflict => "The queue changed on another device. Review this change.",
            Self::Expired => "The focus request expired before it could be confirmed.",
            Self::Capacity => "The sync storage limit was reached.",
            Self::Invalid(message) => message,
        })
    }
}
impl std::error::Error for Rejection {}

pub(crate) fn invalid(message: &str) -> Rejection {
    Rejection::Invalid(message.into())
}
pub(crate) fn valid_id(id: Id) -> Result<(), Rejection> {
    if id.is_nil() {
        Err(invalid("empty identifier"))
    } else {
        Ok(())
    }
}
pub(crate) fn valid_title(title: &str) -> Result<(), Rejection> {
    if title.is_empty() || title.trim() != title || title.chars().count() > MAX_TITLE_CHARS {
        Err(invalid(
            "task title must be trimmed, nonempty and at most 256 characters",
        ))
    } else {
        Ok(())
    }
}
pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Struct field order, UUID formatting and ordered collections define the
/// version-1 canonical encoding. Never hash a JSON object with unordered keys.
pub fn fingerprint(value: &impl Serialize) -> Result<String, Rejection> {
    let bytes =
        serde_json::to_vec(value).map_err(|_| invalid("cannot serialize protocol value"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
