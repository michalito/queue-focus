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

- [x] 1. `crates/qf-ffi` static library with UniFFI proc macros
- [x] 2. Engine object, records, enums, error enum. Per the design review,
      a time of day is an hour-and-minute record rather than an `HH:MM`
      string, and records are named so they never shadow Swift
- [x] 3. Constructor takes a directory; `tick` returns `TickResult` with the
      flash and every problem to report
- [x] 4. `uniffi.toml` (module `QfCore`) and a bindgen binary, in its own
      `crates/uniffi-bindgen` crate per the design review
- [x] 5. `scripts/build-mac-core.sh` and `make mac-core`: XCFramework and
      generated Swift inside the `macos/QfCore` package
- [x] 6. FFI tests through the exported API (20); `make test-core` and
      `make check-core` on either OS; `make test-mac-core` runs Swift tests
      of the bindings against the real library
- [x] Review: GPT phase review green (third pass)
- [x] Review: Fable approves the Phase 3 design (with changes, all taken)

## Phase 3. App scaffold and menu bar

- [x] 1. `macos/` Xcode project: accessory app, macOS 14, hardened runtime,
      Rust build phase (`RustCore`, rebuilds only on change, one
      architecture in Debug), unit and UI test targets, shared scheme
- [x] 2. Main-actor model owning the engine; 1 Hz tick in every run loop mode
- [x] 3. Launch: unreadable task file alerts and quits; unreadable settings
      warn; a second copy of the app quits; nothing opens at launch
- [x] 4. Status item: tag dot, title cut to a width in points, timer;
      left click popover, right click (or Control-click) pause
- [x] 5. Popover: Now card, view buttons, gear menu, add field (Return,
      Command-Return), Side cards, Done row with Undo, inline messages
- [x] 6. Launch at login toggle (gear menu and Settings)
- [x] 7. Milestone: 20 unit tests and 8 UI tests drive the real status item,
      popover and windows; a universal Release build launches under the
      hardened runtime. Queue, Board and Settings windows hold minimal real
      content until Phase 4
- [x] Review: GPT phase review green (fourth pass)
- [x] Review: Fable approves the Phase 4 design (with changes, all taken)

## Phase 4. Windows

- [x] 1. Queue window: three bands, every task key, tag tint, drags and drops
      (banner, headings, "empty" lines, the Later shelf open or closed)
- [x] 2. Board window: quadrants split three to two, drag and drop with the
      insertion line, Now panel and heading promote and ring, the current
      task can be dragged out of Now
- [x] 3. Settings: Reminder (with Try it and Flash now), Quiet, Appearance,
      Menu bar, Quick add, Keyboard, General
- [x] 4. Quick add panel: floating, non-activating, opened with ⌘N for now;
      Phase 6's global shortcut opens the same panel
- [x] 5. Shared row component with context menu, inline rename and tooltips
      only on cut titles
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

Deliberate differences from GNOME, so far:
- Queue and Board are two windows, not two pages of one; `q` and `b` open
  the other and keep this one open. Escape and ⌘W close a window; ⌘Q quits
  the app. ⌘1 and ⌘2 open Queue and Board, ⌘, Settings, ⌘N quick add.
- A task completed from a window can be undone from the popover.
- Settings has a Menu bar section (the title's width in points) where GNOME
  has Top bar; Mac-only settings live in UserDefaults.
- The windows' controls sit beside the add field, not in a toolbar: a
  toolbar rebuilt with every tick of the clock crashed AppKit's layout.

Known gaps, to close later:
- XCTest cannot type into the quick add panel: its accessibility does not
  report the field as focused, though AppKit has given it the keyboard.
  Opening and closing are tested; typing there is checked in Phase 6.
- SwiftUI's accessibility folds a list's only row into the list, so that row
  loses its identifier and VoiceOver reads the list's frame. Phase 7 audits
  accessibility.
- ⌘1 never reaches the app on the Mac these tests were written on: something
  outside the app takes it (the same menu item on ⌘3 fires at once, and ⌘2
  and ⌘N work). The UI tests go back to the Queue by its button; check ⌘1 by
  hand on a clean Mac in Phase 7.

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

### Phase 2, GPT (gpt-6.1-sol, xhigh)

First pass, not green:
1. The build read hardcoded `target/` paths, so a `CARGO_TARGET_DIR` could
   package an older library. It now asks `cargo metadata` where the output
   goes.
2. Recovering a poisoned lock kept a change made in memory but never saved,
   which the next change would save. A poisoned lock now reloads the engine
   from its files; while they cannot be read, changes are refused. The
   overflow GPT used to show it, resuming a clock with an absurd start time,
   now saturates.

Second pass, not green:
1. Recovery took unreadable settings as the defaults, which a later change
   would write over the user's own. That now counts as a failed recovery.
2. `scripts/set-version` knew only two of the four workspace packages. It now
   reads them from the workspace, and its test checks all four.

Third pass: green, no findings. GPT also reverted each fix in a copy and saw
its test fail.

### Phase 3 design, Fable

Approved with changes, all taken. Fable checked each against a scratch
project it built, tested and launched:
1. The generated Swift compiles straight into the app, which links the static
   XCFramework. A framework target would not launch in an ad-hoc signed
   Release build under the hardened runtime.
2. The project file is hand-written with synchronized folders and committed:
   user script sandboxing off, the Rust phase always run, `Info.plist` kept
   out of the resources, a shared scheme, and the version at project level.
3. A fresh checkout runs `make mac-core` before the first Xcode build: Xcode
   lists the generated files before any script runs.
4. Hosted unit tests run inside the app, so the app opens no engine, status
   item or timer when it finds itself under test.
5. No `withObservationTracking`: the model tells the status item after every
   change and tick, and a `.common` mode timer ticks on the main actor.
6. The popover mirrors GNOME's menu exactly (closing after an add, the Now
   card's heading, chip, PAUSED and trailing clock, the empty texts, the Side
   card's actions), and a completion's undo offer is made explicitly.
7. The title cap is in points, measured in the menu bar font; App Nap is off;
   a second copy of the app quits; controls carry accessibility identifiers.

### Phase 3, GPT (gpt-6.1-sol, xhigh)

First pass, not green:
1. A failed Undo dropped the offer, so it could not be tried again. The
   offer now goes only once the undo is done or refused.
2. The build stamp lived per Cargo target directory while every build
   writes one package. It now lives beside the package.
3. Windows could come back through state restoration. They are kept out of
   it. The scenario could not be reproduced here; the fix is defensive and a
   UI test guards it.

Second pass, not green: Xcode copied the XCFramework before the Rust build
ran, so the first universal Release build after a Debug one linked the old
arm64-only library. The app now links the library and its C module from
fixed paths that the Rust phase declares as outputs; Debug and Release
alternated from clean all build first time.

Third pass, not green: the gear menu and Settings kept separate copies of
Launch at Login. One shared, observable state serves both.

Fourth pass: green, no findings.

### Phase 4 design, Fable

Approved with changes, all taken:
1. The Queue window takes drags and drops too: rows, banner, headings, the
   "empty" lines and the Later shelf, open or closed.
2. Focus is never nil: the keys need a focused view to arrive at all.
   Focus comes back to the same task, or to whatever sits where it was, as
   GTK's `Focus` does, and the current task's panel is a focus stop only
   while it holds a task.
3. Drags record the task at pickup, with an item provider that never leaves
   the app; one mark at a time, cleared on exit or drop, under GTK's rules
   for lines and rings.
4. The Board's current task can be dragged out of Now.
5. Tooltips only on titles that were cut.
6. The Queue banner and the Board card are dressed as in GTK.
7. Rename covers the current task, opens the Later shelf, selects the old
   title, survives changes from elsewhere, and saves no blank title.
8. The windows and the quick add panel show the same message line as the
   popover.
9. The Board splits three to two.
10. Quick add is a non-activating panel an AppKit controller owns.
11. Flash now goes through the same path as a scheduled flash.
12. Accessibility identifiers on every row, list, heading and field.

### Phase 4, GPT (gpt-6.1-sol, xhigh)

First pass, not green:
1. Focus recovery could take the add field from someone typing in it: with
   the add field chosen, a task the keyboard had been on before going
   elsewhere sent focus back to the tasks, and the next `d` completed one.
   The rules moved into a pure `FocusKeeper` that tells a chosen add field
   from AppKit's fallback to it.
2. A click in the current task's rename field took focus to the panel and
   ended the rename. The panel's click leaves a rename alone.
3. Closing the Later shelf over a rename there left the rename open and
   focus nowhere. Closing the shelf ends the rename; a task moved to Later
   from the other window while renamed opens the shelf instead, and keeps
   the edit.
4. Settings had no message line, so a setting that could not be saved said
   nothing with only Settings open. It has the shared one.
5. The quiet hours pickers showed today's moment, so on a day the clocks
   change 02:30 showed as 03:00 and could be saved so. They now show the
   time in a zone that never changes its clocks.
6. A drop mark showed in both windows. Marks carry their window.

While fixing these: the app had two View menus (the standard one and its
own); its commands now sit in the standard one. Each fix has a test that
fails without it, checked by undoing the fix; the picker test reads the
screen, since the picker's accessibility value is a moment, not the text.
