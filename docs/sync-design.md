# Shared queue sync

Queue Focus will synchronize tasks, bucket order, Now and its timer between the
macOS and GNOME apps through one private Rust service on the user's Dokploy VPS.
Both apps retain their native interfaces and share the implementation in Rust.
This is an opt-in feature; installations without sync retain their current
behavior and file format.

## Scope and user behavior

- Queued tasks can be created, renamed, tagged, reordered, moved between
  Next/Later/Side, completed or deleted offline. Changes become visible only
  after the local durable save succeeds.
- Changing Now requires a recent connection and no unacknowledged changes.
  This includes promotion, completion/removal/moving of Now, explicit add into
  Now, pause/resume and undo. These operations are submitted asynchronously;
  Now and its timer change only on server confirmation.
- A running timer continues displaying during an outage and through sleep.
  Offline pause/resume and switching Now are unavailable in this version.
- In sync mode an ordinary add uses the configured queued bucket, including
  when Now is empty. It never implicitly acquires Now. Explicit `!`/`@now`
  remains an online action. If the configured default is Now, offline add is
  refused with an explanation; text is retained.
- Theme, shortcuts, reminder schedule and quiet hours remain device-local.
- Status distinguishes Disabled, Connecting, Synced, Offline (saved changes),
  Waiting for confirmation, Needs attention and Recovery required.
- One failed/conflicting edit does not silently disappear. Recovery items show
  its task/title, attempted action and reason, with copy/export and dismiss.
  Applying a conflicting edit again is a new explicit user action.
  Version one shows the count and status in the apps; detailed export and
  dismiss are available in the `qf-sync` utility.
- UI operations must not report a completion, clear an add draft, offer undo,
  or animate a successful focus switch while its result is still uncertain.
  A dedicated pending result is distinct from a rejected operation.

## Components

1. `qf-core::sync`: versioned protocol, deterministic state transitions, client
   journal/reconciliation and conversion to the existing `Store` view. It has
   no network I/O or toolkit dependencies. Conversion constructs the view
   directly; it never deserializes through `Store::normalize` or calls local
   mutation helpers, which read the wall clock and can reset timers.
2. `qf-sync`: bounded HTTP transport, background worker and a setup/status/
   recovery command-line utility, usable on macOS and Linux.
3. `qf-sync-server`: HTTP service with one SQLite database and one serialized
   write transaction per command. It uses the same deterministic reducer.
4. Engine integration: every task mutation enters an explicit command boundary.
   Existing GTK closures must be replaced or refused in sync mode; no untracked
   mutation path may remain. FFI and D-Bus keep stable local numeric handles.
5. Platform integration: workers exchange immutable messages; results are
   applied on the host's existing serialized engine boundary. Network I/O
   never holds the engine mutex, GTK borrow or main actor.

Start with polling every two seconds while connected, immediate work after a
local edit, and retries with bounded exponential backoff to 30 seconds after
failure. Foreground/wake requests a fresh exchange. Only one exchange is in
flight per client. SSE and database replication are deferred.
The menu-bar current task is always visible, so the initial two-second polling
interval applies even with the windows closed.

## Identity and versions

All wire task IDs, command IDs, device IDs and session IDs are random UUIDs.
An installation assigns stable, never-reused local `u64` handles and persists
the mapping, including deleted tasks needed for undo or recovery. Equal task
titles or legacy numeric IDs do not imply equal tasks across installations.

A dataset has a permanent UUID and a restore-generation UUID. A snapshot has
a durable monotonically increasing revision within that generation. These
are distinct from the local UI revision and protocol/schema version.
Each revision also has a SHA-256 history fingerprint over the previous hash,
command identity and canonical resulting state (not just the command ID,
since the same request replayed after rollback can produce different timer
anchors). The genesis fingerprint includes dataset and generation identity.
The server retains the fingerprint at every revision. Each exchange carries
the client's confirmed dataset/generation/revision/hash; the server verifies
that checkpoint before returning state or executing commands. Missing or
different ancestry requires recovery, even when the current revision is
higher. A raw volume restore followed by new writes cannot hide the rollback.

Each task carries separate title, tag and placement version tokens. Tokens
are the UUID of the accepted command that last changed the corresponding
field. A newly created task's tokens are its create command ID. This permits
offline create → rename → rename without guessing future server revisions or
rewriting a command that might already have been delivered.

The shared focus slot has a version token even when empty. Each entry into
Now creates a new session; A → B → A cannot be mistaken for the original A
session. Pausing/resuming changes the focus version while keeping the session.

## State and commands

The wire state contains ordered live tasks, their field versions and the
focus slot. Server-only state also retains used task IDs/tombstones and the
last undo record. Deleted task IDs can never be reused by a new create.
An explicit validated undo is the only resurrection operation.

Commands have protocol version, dataset and generation IDs, command UUID,
and a typed body. Device identity comes from authentication, not an untrusted
payload. Bodies are narrow intentions:

- Create with explicit UUID, title, tag and queued bucket.
- Rename and SetTag with the expected field token. These may update the text
  or tag of Now offline; they do not change its identity or timer.
- MoveBefore with expected placement token, queued destination and optional
  stable neighbor ID and expected neighbor placement token. The source must
  also be queued. A missing/moved neighbor (including movement within the same
  bucket or away and back) is a conflict, not an append.
- DeleteQueued with expected title token and a still-queued check. A harmless
  intervening retag or queued reorder does not prevent completion. If the task
  became Now on another device, reject rather than delete the shared current
  task.
- Online focus commands: Promote, CreateNow, CompleteNow, RemoveNow,
  MoveNowToQueue, SetPaused and UndoCompletion.

Convenience UI actions are resolved before enqueueing. CycleTag becomes
SetTag with a concrete value; Shift becomes MoveBefore with a concrete
neighbor; CompleteCurrent captures the observed task and session IDs.
Commands never mean “whatever is current when this arrives.”

Queued completion uses DeleteQueued in version one. Undo for offline queued
completion is deferred; the UI must not offer an unavailable undo. Standalone
mode keeps its existing behavior.

Every focus command carries the observed snapshot revision, focus version,
session/task IDs where applicable, and a short server-time expiry. Require a
snapshot received within five seconds, no pending journal entries and no
other pending focus command before issuing it. The expiry is ten seconds
after that snapshot's server timestamp. The server refuses stale revisions,
wrong focus/session, expired commands, or deadlines beyond its allowed window.
An unrelated intervening edit can conservatively reject a focus action;
the user retries deliberately after seeing the refreshed queue.

CompleteNow atomically deletes the observed task and promotes the current
server head of Next. Its revision guard ensures that this is the queue the
user observed. RemoveNow and MoveNowToQueue leave Now empty, matching existing
behavior. Promote displaces the current task to the front of Next.

An online completion records enough state for an eight-second undo, referenced
by its command UUID. Undo requires the exact post-completion server revision
and focus version. Any later accepted mutation invalidates it. Completion
receipts never authorize an unconditional restoration of an old snapshot.
Responses expose undo eligibility and its absolute server expiry. A delayed
completion receipt never starts a new eight-second UI window.
Undo creates a new revision and fresh version tokens for restored/changed
fields; it never restores old concurrency tokens or the old revision. Restoring
Now creates a new session ID while preserving its previous elapsed/running
state. This prevents stale edits and controls from matching after undo.

## Timer

A session contains task UUID, session UUID, accumulated elapsed seconds,
running/paused state and a server timestamp anchor. The server samples its
time once per accepted transition. Start/resume/pause never read a client
timestamp. The reducer takes time as an argument and never reads the clock.
The server persists its last observed timestamp and clamps subsequent samples
to at least that value, including across restarts.

Clients estimate server time from a timestamped response and bounded request
round trip, then render locally against a boot clock that includes suspend:
`CLOCK_BOOTTIME` on Linux and `CLOCK_MONOTONIC_RAW` on macOS. Do not use Rust
`Instant`, which can exclude sleep. Freshness and response timing use this
clock too. No per-second timer uploads occur. Sleep advances an unpaused
display; wall-clock changes do not alter the elapsed estimate. A process restart
while offline can only estimate elapsed from its persisted wall-clock anchor;
show the offline status and correct it on the next server response.
On reconnect, the server snapshot wins.
The display can be corrected, but never combines two devices' elapsed times.
Reconciliation itself never starts a new session or resets a timer.
For legacy renderers, project a wall-clock `started_at`/`paused_at` from this
estimate. Preserve that projection until semantic timer state changes or its
wall/boot-clock offset drifts by more than one second. Identical polls do not
bump the local view revision or broadcast `Changed`; status notifications are
separate from queue revision changes.

## Journal and reconciliation

Each client saves a versioned private JSON envelope in `sync-state.json`:
dataset identity, confirmed snapshot, ordered pending immutable commands,
stable numeric handle mapping, and recoverable rejected changes. The displayed
view is derived from the confirmed snapshot plus replayable queued commands.
A pending focus command is never optimistically applied.

Use the existing temp-file, file fsync, rename and directory fsync mechanism.
Confirmed snapshot replacement, acknowledgement, journal removal and recovery
creation form one local save. A save failure retains the previous durable
state only when it occurs before rename. After rename, the installed envelope
is adopted in memory with its original command IDs and a distinct durability
warning; transport is suspended until directory fsync succeeds. Never report
an ordinary rejected edit or enqueue a replacement command after this outcome.
This rule applies to both enqueue and acknowledgement saves. The same command
ID is retried after a lost acknowledgement. A response
must never be applied by replaying commands over an already optimistic view.

Process commands in journal order. Field tokens make dependencies explicit:
if a predecessor is rejected, later commands requiring its token become
conflicts when their own definitive receipts arrive, never silently retargeted
or rebased to the latest token.
Missing create targets likewise fail. Independent later edits can continue.
Rejecting an edit preserves its payload locally; resolving it creates a new
command ID against the state the user currently sees.
Projection skips commands whose preconditions fail and marks them as likely
conflicts without removing them. Only a definitive receipt creates a recovery
item. Whenever the journal is non-empty, an exchange submits its head command,
so an already accepted operation is not projected a second time without its
receipt. Creates check unique IDs, edits check field tokens, deletes check
existence/title and moves check placement tokens; replay never duplicates an
effect. All commands, including focus requests, are persisted before sending.

While a focus result is unknown, further focus controls are disabled. Queued
edits may still be durably captured behind it; they cannot acquire Now and
will be checked against the eventual confirmed state. A timeout is not proof
of rejection. Keep the exact request until a definitive receipt is obtained.
Retries after expiry retrieve an existing receipt, or atomically record an
expired rejection if it never committed. Expiry is checked after receipt lookup.

No accepted command is silently dropped due to capacity. Bound request/response
sizes and pending journal growth; if full, refuse the new edit before reporting
it saved, retaining the user's input. Initial limits are 2,000 live tasks,
1,000 pending commands, 4 MiB per HTTP body, and 32 MiB per local envelope.
Validate serialized authoritative snapshot size before committing each command;
reserve response overhead so any accepted dataset can always be fetched.
Recovery capacity uses the envelope bound; when exhausted, stop reconciliation
and retry the same command to refetch its receipt after capacity is freed,
rather than depend on an unsaved response surviving in memory.

## HTTP and server persistence

One authenticated `POST /v1/exchange` sends either no command (fetch) or one
immutable command. It returns the current snapshot, server timestamp and,
when submitted, a definitive receipt with applied/rejected status and reason.
Use an explicit protocol version and reject unsupported versions.
The client includes its persisted device UUID in `X-Queue-Focus-Device`.
The server checks that the token belongs to that UUID before executing any
command, so accidentally using the other machine's token cannot misattribute
a command before the client sees the response.
Only authenticated setup discovery may omit a confirmed checkpoint, and only
for a read without a command. It returns the current checkpoint (the genesis
checkpoint for an uninitialized dataset). Setup persists that checkpoint
before its first mutation. An activated client never omits its checkpoint.

SQLite stores dataset state, enrolled device token hashes, command receipts
and tombstones. Use WAL with `PRAGMA synchronous=FULL`; acknowledged state,
receipt and revision fingerprint must survive a restart. A transaction
verifies identity/generation/history checkpoint, checks for an
existing receipt, validates/reduces the command, then stores the new state and
receipt atomically. Reusing a command ID with different content or a different
device is rejected. Rejections are also durable receipts. A duplicate command
returns its original outcome together with the current snapshot.

Keep command receipts and tombstone IDs indefinitely initially. Receipts need
command identity/hash and result, not permanent copies of deleted task titles.
The short-lived undo record is removed/invalidated on the next mutation.
No client uploads a whole snapshot to overwrite the server.

Validate bounds, identifiers, uniqueness, bucket invariants and version
compatibility before applying a remote snapshot or command. HTTP timeouts,
response limits and disabled cross-origin redirects prevent unbounded work or
credential forwarding. HTTPS is required except explicit loopback development.
Authentication/configuration/protocol 4xx failures suspend exchange with
Needs attention and an actionable repair/export path. Do not discard the
head: a prior attempt may have committed before its response was lost. A
4xx without a receipt does not prove the command never applied. After token
or server configuration repair, retry the same command identity.

## Native result and observation contract

Every mutation has three logical outcomes: Applied, Pending(command ID), or
Rejected. Existing success values are returned only for a saved queued edit
or a confirmed standalone edit. A shared focus submission returns a distinct
typed Pending result. It must never masquerade as an ordinary successful
completion or as a failed edit safe to resubmit under a new ID.

The existing Rust/FFI `Result` signatures can represent this with a dedicated
`Pending { command_id }` error variant; native adapters explicitly distinguish
it from rejection. The pending command and its eventual settlement also appear
in sync status. Pending commands cannot be resubmitted by repeated button
presses while the original is unresolved.

D-Bus focus methods return `org.queuefocus.Error.Pending` with the command ID.
Old extensions therefore retain drafts and offer no undo while a request is
pending. `GetState`/`Changed` JSON gains a `sync` object with status, pending
focus command and eligible server `undo: {task_id, command_id, until}`.
A `CommandSettled` signal reports a durably processed receipt; FFI drains
equivalent settlement events on tick. Events are conveniences, not durable
truth: snapshots/status retain the pending state and undo eligibility.

New Swift and extension code offer sync undo only from the server metadata,
never merely because Complete returned successfully. Queued completions in
sync mode do not offer undo. They preserve the existing offer logic in
standalone mode. GTK replaces all Store mutation closures with explicit engine
calls. A pending add retains and disables that draft until settlement; after
confirmation it clears only if its text still matches the submitted draft.
A lost transient event is reconciled from pending status/receipt tracking.
App Intents either await confirmation with a bounded wait or explicitly say
“Sent, waiting for confirmation”; they never claim an unconfirmed completion.

Linux initializes the lock and worker only for the primary GApplication
instance, not every command-line forwarding process. Startup during setup
reports the resumable setup state without an extension restart loop. The FFI
host owns one worker independently of engine panic recovery; reopening the
engine must not spawn a second worker or discard an in-flight identity.

## Setup, migration and credentials

Ship a cross-platform `qf-sync` utility for setup and recovery before native
settings integration. Setup requires the app to be stopped and holds an
exclusive data-directory lock. Normal app startup holds the same lock in sync
mode so there cannot be two journal writers. Never modify a live app's files
from the utility.

Setup backs up the original tasks file and constructs the new envelope before
activating sync. A version marker in the legacy tasks file prevents older
binaries from interpreting sync installations as empty or overwriting them;
old builds must fail parsing it. After activation the legacy file is not a
second writable copy.

Activation is a resumable state machine. Before contacting the server, persist
`sync-setup.json` with the original backup identity, device and task IDs, exact
import commands, destination and chosen migration mode. Write credentials
privately. Existing apps must be stopped before setup begins; the new app and
utility share a directory lock. App startup refuses normal operation whenever
a setup record exists and directs the user to resume it. After idempotent remote
initialization/import and envelope installation, replace `tasks.json` with
`{"tasks":"Sync is enabled; use a sync-capable Queue Focus build"}`; its wrong
field type is rejected by old builds. Finally remove the setup record and fsync
the directory. A crash at any boundary resumes the same generated IDs and
commands, never regenerates imports or silently falls back to standalone.

First device: explicitly initialize an empty dataset from its existing queue,
preserving its chosen Now and elapsed/paused timer. Server initialization is
authenticated, one-time, atomic and idempotent.

Second device: preview its local queue versus the shared queue. Joining alone
must never discard local tasks. Import assigns new global IDs and appends its
queued tasks in order; its old Now is imported into Next, preserving its title
but not replacing the selected shared session. Do not deduplicate by title.
An explicit “use shared queue only” option retains a local backup. Import
operations are durable/idempotent, so interrupted setup can resume safely.

The server has no public signup. An operator CLI initializes the dataset and
issues individually revocable device tokens. Only token hashes are persisted
server-side. Client credentials live in a separate private 0600 configuration
file, excluded from diagnostic exports and logs; directory permissions are
0700. The VPS is trusted with plaintext task titles. End-to-end encryption,
multi-user accounts, browser UI and automatic pairing are outside this version.
The envelope is bound to its enrolled device UUID. Token rotation preserves
that UUID; adding another machine enrolls a new device and never copies
credentials/envelope files. Update the README's standalone file-copy guidance
to explicitly exclude sync installations.

## Recovery and deployment

One container instance, a persistent Docker named volume, HTTPS through
Dokploy, and health endpoints without task data. Container restarts must never
initialize a fresh dataset over an existing database. Deploying another
instance must not create a second independent authoritative queue.

Use SQLite's consistent backup API and off-VPS storage. Dokploy volume backups
may copy a completed backup artifact, never the live database/WAL files.
Restoring uses an
operator command that changes the restore generation before serving clients.
Clients seeing a different dataset/generation or a revision rollback stop
uploads and retain their snapshot/journal for export and explicit recovery.
Do not silently replace a newer local confirmed state with the restored one.
History fingerprint verification also detects restoration that bypassed the
operator procedure. Restoring an old client envelope cannot reuse random
command IDs.

Protocol/schema version checks protect old clients. Unknown or malformed local
state fails closed without replacing data. Turning sync off requires explicit
export to standalone mode; deleting configuration is not a safe downgrade.
Recovery exits through an explicit export to a new standalone directory
(with a separate copy of pending/recovery intentions), followed by setup/join
and import preview. It never overwrites the quarantined source installation.
This path also works when the server database is permanently lost.

## Implementation checkpoints and review gates

1. **Protocol and reducer.** IDs, validation, field conflicts, queue invariants,
   focus guards/session identity, expiry, deterministic timer and undo. Unit
   and two-client scenario tests. Sol review until no blocking findings.
2. **Server.** SQLite transactions/receipts, HTTP/auth, initialization, revoke,
   backup/restore generation, container and operator instructions. Integration
   tests including response loss and restart. Sol review until green.
3. **Client persistence and transport.** Atomic journal, replay/recovery, worker,
   setup/import/locking/legacy guard, status/export utility. Crash/retry and
   actual two-client/server tests. Sol review until green.
4. **Engine and native apps.** Route every mutation, apply remote updates and
   status, async focus confirmation, pending UI/draft preservation, timer
   projection, GNOME/D-Bus and Swift integration. Sol review until green.
5. **Delivery.** Full local Linux/macOS checks, end-to-end outage and migration
   tests, docs and deployment smoke test. Final Sol review until green.

Before code, Sol reviews this complete design; its findings are resolved,
then Opus reviews the revised design. Material design changes go through the
same critical review before their dependent implementation.

## Acceptance scenarios

- Two devices create tasks with overlapping old numeric IDs: both survive.
- Create → rename → rename offline survives restart and uploads in order.
- Concurrent rename/tag changes merge; competing renames retain rejected text.
- Delete versus offline edit never resurrects a task.
- Reorder before a missing/moved anchor is reported, not silently appended.
  Cover moves within the same bucket and away/back as well.
- Both devices complete A; retries never complete successor B.
- A → B → A invalidates an old pause even though its task ID matches.
- A request commits, its response is lost, both client and server restart:
  exactly one effect, and the client eventually observes the receipt.
- An undelivered focus request expires and cannot surprise the user later.
- No UI success/undo/draft clearing before confirmed focus completion.
- Offline ordinary add with empty Now stays queued; explicit Now is refused.
- Server snapshot, journal and recovery survive each local write failure point.
  A post-rename directory-fsync failure keeps the installed command IDs and
  suspends transport until durability can be established.
- Sleep, wall-clock jumps and timezone changes do not reset the shared timer.
- Restore generation prevents automatic replay into an older dataset.
- A raw restore followed by a different branch reaching a higher revision is
  detected through the client's historical fingerprint.
- Undo never restores old tokens/session identity; stale pre-completion
  commands remain stale.
- Polling identical state does not invalidate undo or rebuild native views.
- Interrupted setup/import resumes without task loss or duplicate import.
- Bad token, revoked token, malformed/oversized body, incompatible protocol,
  duplicate UUID with different payload and corrupt local envelope fail safely.
- Standalone installations retain their existing behavior and tests.

## References

- [SQLite online backup](https://www.sqlite.org/backup.html)
- [Dokploy volume backups](https://docs.dokploy.com/docs/core/volume-backups)
- [Axum documentation](https://docs.rs/axum/latest/axum/)
- [Apple continuous clock](https://developer.apple.com/documentation/kernel/1646199-mach_continuous_time)
- [Linux clocks](https://man7.org/linux/man-pages/man3/clock_gettime.3.html)
