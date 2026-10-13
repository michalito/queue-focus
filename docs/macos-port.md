# macOS port: progress

One Rust engine, native shells, identical data files. This file tracks the plan
step by step and records each phase review. Tick a box only when the step is
done and its checks pass.

Reviews after every phase: GPT (gpt-6.1-sol, xhigh) reviews the finished phase
for correctness and a green build; Claude Fable reviews the design of the next
phase before it starts.

Linux suites run in a disposable Ubuntu 26.04 container with GNOME Shell 50,
because this Mac cannot build the GTK crate.

## Phase 0. Decisions

- [x] Minimum macOS 14
- [x] Developer ID distribution with notarization, no App Sandbox
- [x] Universal binary (arm64 and x86_64)
- [x] Hotkeys: Control Option Q queue, Control Option Shift Q quick add,
      Control Option B board, Control Option D complete current
- [x] Deliberate differences: right click pauses, fixed-width popover with
      tooltips, flash on the main display, configurable title width cap

## Phase 1. Lift the engine into the core crate

- [x] 1. Platform-aware data directory, a test per branch
- [x] 2. Elapsed formatters and the flash event in the core
- [x] 3. Core task state (persist, roll back, warn, revision, undo); 11 tests ported
- [x] 4. Core settings store (dirty, backoff, flush, tick); 6 tests ported
- [x] 5. Core reminder (tick, flash now, status); 13 tests ported
- [x] 6. Engine facade; D-Bus handler maps one to one. Added `make test-service`,
      which drives the real binary over D-Bus: it passes on the old handler and
      the new one, and fails on a deliberately broken one
- [x] 7. Full Linux suite green except `make test-extension-shell` (see notes);
      version 0.5.1 committed. Publishing is yours: merge, then `make install`
- [x] Review: GPT phase review green (third pass)
- [x] Review: Fable approves the Phase 2 design (with changes, all taken)

## Phase 2. FFI crate

- [ ] 1. `crates/qf-ffi` static library with UniFFI proc macros
- [ ] 2. Engine object, records, enums, `HH:MM` custom type, error enum
- [ ] 3. Constructor takes a directory; `tick` returns `TickResult`
- [ ] 4. `uniffi.toml` (module `QfCore`) and an in-crate bindgen binary
- [ ] 5. `scripts/build-mac-core.sh` and `make mac-core` (XCFramework)
- [ ] 6. FFI tests through the exported API; `make test-core`
- [ ] Review: GPT phase review green
- [ ] Review: Fable approves the Phase 3 design

## Phase 3. App scaffold and menu bar

- [ ] 1. `macos/` Xcode project: accessory app, macOS 14, hardened runtime,
      Rust build phase, unit and UI test targets
- [ ] 2. Main-actor model owning the engine; 1 Hz tick
- [ ] 3. Launch: unreadable task file alerts and quits; unreadable settings warn
- [ ] 4. Status item: tag dot, capped title, timer; left click popover, right click pause
- [ ] 5. Popover: Now card, view buttons, gear menu, add field, Side cards, Done row with Undo
- [ ] 6. Launch at login toggle
- [ ] 7. Milestone: usable daily from the menu bar
- [ ] Review: GPT phase review green
- [ ] Review: Fable approves the Phase 4 design

## Phase 4. Windows

- [ ] 1. Queue window: three bands, every shortcut, tag tint
- [ ] 2. Board window: four quadrants, drag and drop with insertion line, Now promote and ring
- [ ] 3. Settings scene: Flash, Quiet, Appearance, Menu bar, Adding, Shortcuts, General
- [ ] 4. Quick add panel
- [ ] 5. Shared row component with context menu
- [ ] Review: GPT phase review green
- [ ] Review: Fable approves the Phase 5 design

## Phase 5. Flash overlay

- [ ] 1. Click-through, non-activating panel above everything on the main screen
- [ ] 2. Six styles from the `flash.js` tables
- [ ] 3. The card: NOW, capped title, timer
- [ ] 4. Reduce Motion: still for 1.5 s
- [ ] 5. A new flash replaces a running one; panel released at the end
- [ ] 6. Hidden launch argument to render a style; Flash now uses the engine
- [ ] Review: GPT phase review green
- [ ] Review: Fable approves the Phase 6 design

## Phase 6. Hotkeys, notifications, automation

- [ ] 1. KeyboardShortcuts with the four actions and a recorder in Settings
- [ ] 2. Complete-current notification with Undo; fallback to the popover
- [ ] 3. Empty Now reports nothing to complete
- [ ] 4. One notification per durability or settings failure
- [ ] 5. Optional: URL scheme and App Intents
- [ ] Review: GPT phase review green
- [ ] Review: Fable approves the Phase 7 design

## Phase 7. Quality

- [ ] 1. Swift unit tests against a temporary engine
- [ ] 2. Parity tests: status title, truncation, hotkey collisions
- [ ] 3. UI tests: popover add, complete, undo; Board drag; settings to file
- [ ] 4. README audit and recorded differences
- [ ] 5. Accessibility: VoiceOver labels, Reduce Motion, Increase Contrast
- [ ] 6. Overlay on a second display and over full screen; crowded menu bar
- [ ] Review: GPT phase review green
- [ ] Review: Fable approves the Phase 8 design

## Phase 8. Release

- [ ] 1. `scripts/set-version` also sets the Xcode marketing version
- [ ] 2. macOS CI job
- [ ] 3. `scripts/release-mac.sh`: archive, export, notarize, staple, DMG
- [ ] 4. GitHub Release; Homebrew cask and Sparkle later
- [ ] 5. README macOS section and Source layout
- [ ] Review: GPT phase review green

## Notes

- `make test-extension-shell` fails in the container on `main` too: in the
  headless GNOME Shell 50 on arm64 the virtual pointer's x stays at 0, so the
  pill press misses. Phase 1 does not touch `extension/`, and that test runs
  the extension against a JavaScript stand-in, so no Rust change can reach it.
  Run it on a real GNOME machine before publishing.

## Review log

Findings from each review and what was done about them.

### Phase 1, GPT (gpt-6.1-sol, xhigh)

First pass, not green:
1. The first flash's wait was timed before the files loaded. `Engine::open`
   now takes a clock and reads it after loading; a test pins the order
   through the permissions loading repairs.
2. `make test-service` skipped `Show`, `Hide`, `Flash` and
   `DurabilityWarning`. It now calls the windows, waits for a real flash, and
   preloads a library that fails one directory sync. Removing either emission
   fails the test.
3. Real-clock tests could fail across a second boundary. They now bound the
   countdown by the seconds seen to pass.
4. `exec` skipped the test's cleanup trap. Fixed.
5. The docs called the whole core clock-free. Only the reminder is.

Second pass, not green: the failed-save check looked for the rolled-back
task in the wrong bucket. It looks everywhere now, and fails when the
rollback is removed.

Third pass: green, no findings.

### Phase 2 design, Fable

Approved with changes, all taken:
1. Records renamed so they never shadow Swift: `QueueTask`, `QueueSettings`,
   `TaskTag`, `QueueSnapshot`.
2. `TimeOfDay` is a record of hour and minute, not a string, and an invalid
   one is an `InvalidArgument`.
3. `tick` and `flashStatus` refuse an invalid local time instead of assuming
   noon.
4. Durability warnings and settings write failures share one channel: the
   problems `tick` and `flush` return.
5. `rename`, `shift`, `cycle_tag` and `move_before` belong to the core engine,
   and the Board's drop by row moved from the GTK crate into
   `Store::move_before`.
6. A `release-ffi` profile that unwinds panics and keeps symbols.
7. Build plumbing: a separate `uniffi-bindgen` crate, the Swift bindgen
   without `--xcframework`, `module.modulemap`, Swift written into the
   `macos/QfCore` package, `scripts/cargo` usable on macOS.
8. Mac-only presentation settings live in `UserDefaults`, never in the shared
   `settings.json`, which the GNOME app rewrites whole.
