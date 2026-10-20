use super::persistence::invalid;
use super::*;
use crate::{DurabilityWarning, Outcome, SaveError, Store, Task};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
};

pub const STATE_FILE: &str = "sync-state.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryItem {
    pub command: Command,
    pub reason: Rejection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suspension {
    /// Recovery is irreversible in this directory. Export into a new one.
    pub recovery: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u32,
    pub device: Id,
    pub confirmed: Snapshot,
    pub server_time: u64,
    pub wall_time: u64,
    pub pending: Vec<Command>,
    pub handles: BTreeMap<Id, u64>,
    pub next_handle: u64,
    pub recovery: Vec<RecoveryItem>,
    pub suspension: Option<Suspension>,
    #[serde(default)]
    pub last_focus_settlement: Option<Settlement>,
}

impl Envelope {
    pub fn new(
        device: Id,
        confirmed: Snapshot,
        server_time: u64,
        wall_time: u64,
    ) -> io::Result<Self> {
        let mut value = Self {
            version: PROTOCOL_VERSION,
            device,
            confirmed,
            server_time,
            wall_time,
            pending: vec![],
            handles: BTreeMap::new(),
            next_handle: 1,
            recovery: vec![],
            suspension: None,
            last_focus_settlement: None,
        };
        value.register_handles()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> io::Result<()> {
        if self.version != PROTOCOL_VERSION || self.device.is_nil() || !self.confirmed.initialized {
            return Err(invalid("invalid sync envelope identity or version"));
        }
        self.confirmed.validate().map_err(protocol_error)?;
        if self
            .confirmed
            .focus
            .session
            .as_ref()
            .is_some_and(|s| s.anchor > self.server_time)
        {
            return Err(invalid("timer anchor exceeds the observed server clock"));
        }
        if let Some(settlement) = &self.last_focus_settlement {
            if settlement.command.is_nil()
                || self.pending.iter().any(|c| c.id == settlement.command)
                || matches!(settlement.outcome, ReceiptOutcome::Applied { revision }
                    if revision == 0 || revision > self.confirmed.revision)
            {
                return Err(invalid("invalid persisted focus settlement"));
            }
        }
        let mut ids = BTreeSet::new();
        let mut focus = 0;
        if self.pending.len() > MAX_PENDING {
            return Err(invalid("too many pending commands"));
        }
        for command in self
            .pending
            .iter()
            .chain(self.recovery.iter().map(|r| &r.command))
        {
            command.validate_syntax().map_err(protocol_error)?;
            if command.version != PROTOCOL_VERSION
                || command.dataset != self.confirmed.dataset
                || command.generation != self.confirmed.generation
                || command.id.is_nil()
                || !ids.insert(command.id)
                || matches!(command.action, Action::Initialize { .. })
            {
                return Err(invalid("invalid persisted command identity"));
            }
        }
        for command in &self.pending {
            focus += usize::from(command.is_focus());
        }
        if focus > 1 {
            return Err(invalid("multiple pending focus commands"));
        }
        let handles: BTreeSet<_> = self.handles.values().copied().collect();
        if self.next_handle == 0
            || handles.len() != self.handles.len()
            || self
                .handles
                .iter()
                .any(|(id, h)| id.is_nil() || *h == 0 || *h >= self.next_handle)
        {
            return Err(invalid("invalid stable task handles"));
        }
        if self
            .required_ids()
            .iter()
            .any(|id| !self.handles.contains_key(id))
        {
            return Err(invalid("missing stable task handle"));
        }
        if self
            .suspension
            .as_ref()
            .is_some_and(|s| s.message.chars().count() > 512)
        {
            return Err(invalid("oversized suspension message"));
        }
        if serde_json::to_vec_pretty(self)?.len() > MAX_ENVELOPE_BYTES {
            return Err(invalid("sync envelope exceeds storage limit"));
        }
        Ok(())
    }
    fn required_ids(&self) -> BTreeSet<Id> {
        self.confirmed
            .tasks
            .iter()
            .map(|t| t.id)
            .chain(self.confirmed.undo.iter().map(|u| u.task))
            .chain(self.pending.iter().filter_map(created_task))
            .chain(
                self.recovery
                    .iter()
                    .filter_map(|r| created_task(&r.command)),
            )
            .collect()
    }
    /// Fill missing handles after migration seeds the original local mapping.
    /// Validate and save the complete envelope before use.
    pub fn register_handles(&mut self) -> io::Result<()> {
        for id in self.required_ids() {
            if !self.handles.contains_key(&id) {
                let next = self
                    .next_handle
                    .checked_add(1)
                    .ok_or_else(|| invalid("task handle limit reached"))?;
                self.handles.insert(id, self.next_handle);
                self.next_handle = next;
            }
        }
        Ok(())
    }
    pub fn handle(&self, id: Id) -> Option<u64> {
        self.handles.get(&id).copied()
    }
    pub fn identity(&self, handle: u64) -> Option<Id> {
        self.handles
            .iter()
            .find_map(|(id, h)| (*h == handle).then_some(*id))
    }

    /// Always rebuild from confirmed state, never from an optimistic view.
    fn projection(&self) -> (Snapshot, Vec<Id>) {
        let mut state = ServerState::new(self.confirmed.dataset, self.confirmed.generation)
            .expect("validated envelope identity");
        state.snapshot = self.confirmed.clone();
        state.snapshot.undo = None;
        let pending_creates: BTreeSet<_> = self.pending.iter().filter_map(created_task).collect();
        state.used_ids = self
            .handles
            .keys()
            .copied()
            .filter(|id| !pending_creates.contains(id))
            .chain(self.confirmed.tasks.iter().map(|t| t.id))
            .collect();
        let mut conflicts = Vec::new();
        for command in &self.pending {
            if matches!(command.action, Action::Queued { .. })
                && state.apply(command, self.server_time).is_err()
            {
                conflicts.push(command.id);
            }
        }
        (state.snapshot, conflicts)
    }
}

#[derive(Clone, Copy, Debug)]
struct Observation {
    clock: ClockSample,
    server_time: u64,
}
#[derive(Clone, Debug)]
struct TimerView {
    session: Session,
    started: u64,
    paused: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Online,
    Offline,
    NeedsAttention,
    Recovery,
    Durability,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientUndo {
    pub command: Id,
    pub task: u64,
    pub title: String,
    pub remaining: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatus {
    pub state: ConnectionState,
    pub message: Option<String>,
    pub pending: usize,
    pub focus_command: Option<Id>,
    pub likely_conflicts: usize,
    pub recovery_items: usize,
    pub undo: Option<ClientUndo>,
    pub last_focus_settlement: Option<Settlement>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settlement {
    pub command: Id,
    pub outcome: ReceiptOutcome,
}

/// The host serializes all access. Networking gets only an immutable request
/// and returns a response later; neither path borrows the engine during I/O.
#[derive(Debug)]
pub struct Client {
    path: PathBuf,
    envelope: Envelope,
    observation: Option<Observation>,
    online: bool,
    durability: Option<String>,
    timer: Option<TimerView>,
    projection: Snapshot,
    likely_conflicts: Vec<Id>,
}
impl Client {
    pub fn open(path: &Path, device: Id) -> io::Result<Self> {
        let envelope: Envelope = read_private(path, MAX_ENVELOPE_BYTES)?;
        envelope.validate()?;
        if device != envelope.device {
            return Err(invalid("credentials belong to another installation"));
        }
        // Re-sync the directory on every open, covering a prior post-rename
        // failure even if the process died before it could retain the warning.
        let durability = sync_parent(path).err().map(|e| e.to_string());
        let (projection, likely_conflicts) = envelope.projection();
        Ok(Self {
            path: path.into(),
            envelope,
            observation: None,
            online: false,
            durability,
            timer: None,
            projection,
            likely_conflicts,
        })
    }
    pub fn envelope(&self) -> &Envelope {
        &self.envelope
    }
    pub fn projected_snapshot(&self) -> Snapshot {
        self.projection.clone()
    }
    pub fn request(&self) -> Option<ExchangeRequest> {
        if self.durability.is_some() || self.envelope.suspension.is_some() {
            return None;
        }
        Some(ExchangeRequest {
            version: PROTOCOL_VERSION,
            checkpoint: Some(self.envelope.confirmed.checkpoint()),
            command: self.envelope.pending.first().cloned(),
        })
    }
    pub fn network_failed(&mut self) {
        self.online = false;
    }
    pub fn retry_durability(&mut self) -> io::Result<()> {
        sync_parent(&self.path)?;
        self.durability = None;
        Ok(())
    }
    fn can_edit(&self) -> io::Result<()> {
        if self.durability.is_some() {
            return Err(invalid("sync save is waiting for durable storage"));
        }
        if self
            .envelope
            .suspension
            .as_ref()
            .is_some_and(|s| s.recovery)
        {
            return Err(invalid(
                "this queue requires recovery; export into a new data directory",
            ));
        }
        if self.envelope.pending.len() >= MAX_PENDING {
            return Err(invalid("pending queue is full; input has not been saved"));
        }
        Ok(())
    }
    pub fn enqueue_queued(&mut self, action: QueuedAction) -> io::Result<Outcome<Id>> {
        self.can_edit()?;
        let command = Command::new(&self.envelope.confirmed, Action::Queued { action });
        if created_task(&command).is_some_and(|id| self.envelope.handles.contains_key(&id)) {
            return Err(invalid("task identity has already been used"));
        }
        let mut candidate = self.envelope.clone();
        candidate.pending.push(command.clone());
        candidate.register_handles()?;
        if candidate.projection().1.contains(&command.id) {
            return Err(invalid("change cannot be applied to the current queue"));
        }
        self.install(candidate)
            .map(|outcome| outcome.map(|_| command.id))
    }
    pub fn enqueue_focus(
        &mut self,
        action: FocusAction,
        clock: ClockSample,
    ) -> io::Result<Outcome<Id>> {
        self.can_edit()?;
        if !self.fresh(clock) || !self.envelope.pending.is_empty() {
            return Err(invalid("wait for sync before changing Now or the timer"));
        }
        let observed = self.observation.expect("fresh observation");
        let command = Command::new(
            &self.envelope.confirmed,
            Action::Focus {
                guard: FocusGuard::observed(&self.envelope.confirmed, observed.server_time),
                action,
            },
        );
        if created_task(&command).is_some_and(|id| self.envelope.handles.contains_key(&id)) {
            return Err(invalid("task identity has already been used"));
        }
        // Validate focus without changing its visible state. Undo needs the
        // private server record, so its hint is the local precondition.
        if let Action::Focus {
            action: FocusAction::Undo { completion },
            ..
        } = &command.action
        {
            if self.status(clock).undo.as_ref().map(|u| u.command) != Some(*completion) {
                return Err(invalid("completion is no longer undoable"));
            }
        } else {
            let mut state =
                ServerState::new(command.dataset, command.generation).map_err(protocol_error)?;
            state.snapshot = self.envelope.confirmed.clone();
            state.snapshot.undo = None;
            state.used_ids = self.envelope.handles.keys().copied().collect();
            state
                .apply(&command, self.server_now(clock))
                .map_err(protocol_error)?;
        }
        let mut candidate = self.envelope.clone();
        candidate.pending.push(command.clone());
        candidate.register_handles()?;
        self.install(candidate)
            .map(|outcome| outcome.map(|_| command.id))
    }
    pub fn accept(
        &mut self,
        request: &ExchangeRequest,
        response: ExchangeResponse,
        clock: ClockSample,
    ) -> io::Result<Outcome<Option<Settlement>>> {
        if self
            .envelope
            .suspension
            .as_ref()
            .is_some_and(|s| s.recovery)
        {
            return Err(invalid("response arrived after the queue entered recovery"));
        }
        response.snapshot.validate().map_err(protocol_error)?;
        if request.version != PROTOCOL_VERSION
            || response.version != PROTOCOL_VERSION
            || request.checkpoint != Some(self.envelope.confirmed.checkpoint())
            || response.device != self.envelope.device
            || response.snapshot.dataset != self.envelope.confirmed.dataset
            || response.snapshot.generation != self.envelope.confirmed.generation
            || response.snapshot.revision < self.envelope.confirmed.revision
            || response.server_time < self.envelope.server_time
            || (response.snapshot.revision == self.envelope.confirmed.revision
                && response.snapshot != self.envelope.confirmed)
        {
            return Err(invalid("sync response does not extend the confirmed queue"));
        }
        let mut candidate = self.envelope.clone();
        let settlement = match (&request.command, &response.receipt) {
            (None, None) => None,
            (Some(command), Some(receipt))
                if candidate.pending.first() == Some(command) && receipt.command == command.id =>
            {
                if let ReceiptOutcome::Applied { revision } = receipt.outcome {
                    if revision <= self.envelope.confirmed.revision
                        || revision > response.snapshot.revision
                    {
                        return Err(invalid("invalid receipt revision"));
                    }
                }
                let command = candidate.pending.remove(0);
                if command.is_focus() {
                    candidate.last_focus_settlement = Some(Settlement {
                        command: receipt.command,
                        outcome: receipt.outcome.clone(),
                    });
                }
                if let ReceiptOutcome::Rejected { reason } = &receipt.outcome {
                    candidate.recovery.push(RecoveryItem {
                        command,
                        reason: reason.clone(),
                    });
                }
                Some(Settlement {
                    command: receipt.command,
                    outcome: receipt.outcome.clone(),
                })
            }
            _ => {
                return Err(invalid(
                    "sync response is missing the requested command receipt",
                ))
            }
        };
        candidate.confirmed = response.snapshot;
        candidate.server_time = response.server_time;
        candidate.wall_time = clock.wall;
        candidate.register_handles()?;
        let outcome = self.install(candidate)?;
        self.observation = Some(Observation {
            clock,
            server_time: response.server_time,
        });
        self.online = self.durability.is_none();
        Ok(outcome.map(|_| settlement))
    }
    pub fn suspend(&mut self, message: &str, recovery: bool) -> io::Result<Outcome<()>> {
        self.online = false;
        let mut candidate = self.envelope.clone();
        // A later auth failure must never downgrade a history quarantine.
        let recovery = recovery || candidate.suspension.as_ref().is_some_and(|s| s.recovery);
        candidate.suspension = Some(Suspension {
            recovery,
            message: message.chars().take(512).collect(),
        });
        self.install(candidate)
    }
    pub fn resume_after_repair(&mut self) -> io::Result<Outcome<()>> {
        if self
            .envelope
            .suspension
            .as_ref()
            .is_some_and(|s| s.recovery)
        {
            return Err(invalid("history recovery requires a new data directory"));
        }
        let mut candidate = self.envelope.clone();
        candidate.suspension = None;
        self.install(candidate)
    }
    /// Explicit dismissal only; never called as part of reconciling a receipt.
    pub fn dismiss_recovery(&mut self, command: Id) -> io::Result<Outcome<()>> {
        if self
            .envelope
            .suspension
            .as_ref()
            .is_some_and(|s| s.recovery)
        {
            return Err(invalid(
                "preserve this quarantined queue and recover into a new directory",
            ));
        }
        let mut candidate = self.envelope.clone();
        candidate.recovery.retain(|r| r.command.id != command);
        self.install(candidate)
    }
    fn install(&mut self, candidate: Envelope) -> io::Result<Outcome<()>> {
        self.install_with(candidate, |path, value| {
            save_private(path, value, MAX_ENVELOPE_BYTES)
        })
    }
    fn install_with(
        &mut self,
        candidate: Envelope,
        save: impl FnOnce(&Path, &Envelope) -> Result<(), SaveError>,
    ) -> io::Result<Outcome<()>> {
        candidate.validate()?;
        if candidate == self.envelope {
            return Ok(Outcome::saved(()));
        }
        let warning = match save(&self.path, &candidate) {
            Ok(()) => {
                self.durability = None;
                None
            }
            Err(error) if error.is_committed() => {
                self.durability = Some(error.to_string());
                Some(DurabilityWarning::new(self.path.clone(), error.to_string()))
            }
            Err(error) => return Err(io::Error::new(error.kind(), error.to_string())),
        };
        (self.projection, self.likely_conflicts) = candidate.projection();
        self.envelope = candidate;
        Ok(Outcome { value: (), warning })
    }
    fn fresh(&self, clock: ClockSample) -> bool {
        self.online
            && self.durability.is_none()
            && self.envelope.suspension.is_none()
            && self.observation.is_some_and(|o| {
                clock
                    .boot
                    .checked_sub(o.clock.boot)
                    .is_some_and(|n| n <= FRESH_SECS * 1_000_000_000)
            })
    }
    pub fn server_now(&self, clock: ClockSample) -> u64 {
        match self.observation {
            Some(o) => o
                .server_time
                .saturating_add(clock.seconds_since(o.clock).unwrap_or(0)),
            None => self
                .envelope
                .server_time
                .saturating_add(clock.wall.saturating_sub(self.envelope.wall_time)),
        }
    }
    pub fn status(&self, clock: ClockSample) -> SyncStatus {
        let (state, message) = if let Some(message) = &self.durability {
            (ConnectionState::Durability, Some(message.clone()))
        } else if let Some(suspension) = &self.envelope.suspension {
            (
                if suspension.recovery {
                    ConnectionState::Recovery
                } else {
                    ConnectionState::NeedsAttention
                },
                Some(suspension.message.clone()),
            )
        } else {
            (
                if self.fresh(clock) {
                    ConnectionState::Online
                } else {
                    ConnectionState::Offline
                },
                None,
            )
        };
        let undo = self
            .envelope
            .confirmed
            .undo
            .as_ref()
            .filter(|hint| {
                self.fresh(clock)
                    && self.envelope.pending.is_empty()
                    && hint.until > self.server_now(clock)
            })
            .and_then(|hint| {
                Some(ClientUndo {
                    command: hint.command,
                    task: self.envelope.handle(hint.task)?,
                    title: hint.title.clone(),
                    remaining: hint.until - self.server_now(clock),
                })
            });
        SyncStatus {
            state,
            message,
            pending: self.envelope.pending.len(),
            focus_command: self
                .envelope
                .pending
                .iter()
                .find(|c| c.is_focus())
                .map(|c| c.id),
            likely_conflicts: self.likely_conflicts.len(),
            recovery_items: self.envelope.recovery.len(),
            undo,
            last_focus_settlement: self.envelope.last_focus_settlement.clone(),
        }
    }
    /// Constructs the legacy view directly, avoiding normalization's wall
    /// clock mutations. Stable anchors prevent a poll from counting as an edit.
    pub fn store(&mut self, clock: ClockSample) -> Store {
        let snapshot = self.projection.clone();
        let timer = snapshot.focus.session.as_ref().map(|session| {
            let elapsed = session.elapsed_at(self.server_now(clock));
            let keep = self.timer.as_ref().is_some_and(|old| {
                old.session == *session
                    && old
                        .paused
                        .unwrap_or(clock.wall)
                        .saturating_sub(old.started)
                        .abs_diff(elapsed)
                        <= 1
            });
            if !keep {
                self.timer = Some(TimerView {
                    session: session.clone(),
                    started: clock.wall.saturating_sub(elapsed),
                    paused: (!session.running).then_some(clock.wall),
                });
            }
            self.timer.clone().expect("timer exists")
        });
        if timer.is_none() {
            self.timer = None;
        }
        Store {
            next_id: self.envelope.next_handle,
            tasks: snapshot
                .tasks
                .iter()
                .map(|task| {
                    let timer = timer.as_ref().filter(|timer| timer.session.task == task.id);
                    Task {
                        id: self
                            .envelope
                            .handle(task.id)
                            .expect("validated task handle"),
                        title: task.title.clone(),
                        bucket: task.bucket,
                        tag: task.tag,
                        created_at: task.created_at,
                        started_at: timer.map(|t| t.started),
                        paused_at: timer.and_then(|t| t.paused),
                    }
                })
                .collect(),
        }
    }
}
fn created_task(command: &Command) -> Option<Id> {
    match &command.action {
        Action::Queued {
            action: QueuedAction::Create { task, .. },
        }
        | Action::Focus {
            action: FocusAction::Create { task, .. },
            ..
        } => Some(*task),
        _ => None,
    }
}
fn protocol_error(error: Rejection) -> io::Error {
    invalid(&error.to_string())
}

#[cfg(test)]
mod tests;
