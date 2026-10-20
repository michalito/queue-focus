# Sync implementation progress

The protocol and scope are in [sync-design.md](sync-design.md). Implementation
starts after the Sol and Opus design reviews are green. Each code checkpoint
requires local validation and Sol review until no blocking findings remain.

## Design reviews

- Sol round 1: two blockers. Preserve installed state and command IDs after a
  post-rename directory-fsync failure; validate reorder anchor placement
  tokens, including moves within a bucket and away/back. Both addressed.
- Sol round 2: green.
- Opus round 1: two blockers. Detect forked server history even after a raw
  restore catches up past the client's revision; define explicit native
  pending/settled results so drafts and undo are handled honestly. Both
  addressed in the revised design.
- Sol round 3: green on the Opus amendments and the additional clock,
  recovery, migration and observation rules.
- Opus round 2: green. Carry forward explicit bootstrap checkpoint rules,
  clear pending messages for old extensions, and queued-undo refusal tests.

Decisions retained after critical review: an HTTP failure without a receipt
does not prove a command was never applied; keep its identity and offer
repair/export. Recreating a completed queued task under a new ID is not the
same undo operation, so queued completion undo remains deferred in sync mode.
Two-second polling remains the first implementation because the current task
is visible even when the app's windows are closed.

## Checkpoints

- [x] Protocol and deterministic reducer, tests, Sol green.
- [x] SQLite server, HTTP and operator tools, tests, Sol green.
- [x] Client journal, transport and migration utility, tests, Sol green.
- [ ] Shared engine and native app integration, tests, Sol green.
- [ ] Full Linux/macOS and end-to-end verification, final Sol green.

Checkpoint one: 23 new sync tests; all 157 core/golden/FFI tests and
`make check-core` passed. Sol code review round 1 returned green and
independently reran all 157 tests. Full Linux checks, tests, GTK widget and
D-Bus service integration passed. The initial installer fixture failed due
to macOS archive metadata files; removing those files fixed the test.

Checkpoint two: 12 server tests and one additional core restore test. All
170 core/golden/FFI/server tests and targeted Clippy checks passed. Sol code
review round 1 returned green and independently ran the server and core
restore tests. The deployment image built using Rust 1.92. Compose validation
and a container smoke test passed: non-root read-only operation, initialization,
authentication, command retry after restart, consistent backup, generation
rotation and SIGTERM shutdown. No VPS deployment has been performed.

Checkpoint 3a (client journal/projection) is green after two Sol rounds.
Round one found that malformed persisted command bodies were not rejected
at open. Pure command syntax validation now checks pending and recovery
intents without rejecting valid conflicts or expired requests. Eighteen
client tests cover journal replay, stable handles, acknowledgement/save
failure boundaries, focus/undo guards, timer continuity, locking, bounds and
corrupt files. Clippy and the existing core/FFI/server suites passed on macOS;
Linux also passed, including the final syntax-validation amendment.

Checkpoint 3b (HTTP worker and resumable setup/recovery utility) is green.
Eleven transport/migration tests cover HTTPS/redirect/body limits, credential
identity, worker acknowledgement and auth suspension, first/second-device
migration, lost acknowledgements, interrupted activation, changed local
source, backups and recovery exports. All 199 targeted core/golden/FFI/client/
server tests and Clippy passed on macOS and Linux. Rust 1.92 compilation of
the client and server passed. Sol found one nonblocking CLI argument panic;
the guard was fixed and re-reviewed green, with no outstanding findings.

The server now checks the client's persisted device UUID against its token
before any mutation, preventing a mistakenly copied credential from attributing
work to the other installation. The API and design docs include this header.

Checkpoint 4a (shared engine command boundary and host lifetime) is green
after Sol review and amendments. Native intents now produce durable queued
edits or Pending focus commands, with persisted settlement status. Queue and
status revisions are separate. The host locks the directory for its lifetime
and refuses partial setup without falling back to standalone mode. Eight new
tests include two native hosts sharing queue, Now and pause over HTTP.
All 207 targeted tests and Clippy passed on macOS and Linux.

Checkpoint 4b (macOS host and pending UI) is green after Sol review. The FFI
owns one worker outside recoverable engine state; reopening discards a stale
in-flight response and retries the durable journal. Swift receives typed sync
status and settlements, keeps pending drafts, uses server undo eligibility,
and reports pending shortcut results truthfully. The existing standalone
stale-undo test caught a regression that was fixed and re-reviewed.
All 209 targeted Rust tests and Clippy pass; the macOS app builds, 145 app
unit tests pass (one existing system-shortcut known issue), and all seven
Swift binding tests pass.

Checkpoint 4c (GNOME service, GTK and Shell) is green after Sol review.
Only the primary application opens the locked host. GTK and Shell show shared
status, retain pending drafts until a receipt, and use server undo. A two-device
native D-Bus/HTTP test verifies convergence, Now/pause, offline capture through
restart, reconnect without duplication and queued-undo refusal. The real GJS
adapter and headless GNOME 50 Shell tests pass, including durable status fallback
when the settlement event is missed. The headless fixture waits for the actual
virtual pointer position and enables animations despite software rendering so
both animation preferences are exercised.

Native recovery surfaces, packaging and final verification remain. Nothing has been activated in the user's app data or deployed to the VPS.
