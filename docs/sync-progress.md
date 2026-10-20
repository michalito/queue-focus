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
- [ ] SQLite server, HTTP and operator tools, tests, Sol green.
- [ ] Client journal, transport and migration utility, tests, Sol green.
- [ ] Shared engine and native app integration, tests, Sol green.
- [ ] Full Linux/macOS and end-to-end verification, final Sol green.

Checkpoint one: 23 new sync tests; all 157 core/golden/FFI tests and
`make check-core` passed. Sol code review round 1 returned green and
independently reran all 157 tests. Broader Linux validation is running.

Checkpoint two is in progress.
