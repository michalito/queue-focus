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
- [x] Review: GPT phase review green (third pass)
- [x] Review: Fable approves the Phase 5 design (with changes, all taken)

## Phase 5. Flash overlay

- [x] 1. Click-through, non-activating panel above everything on the main screen
- [x] 2. Six styles from the `flash.js` tables
- [x] 3. The card: NOW, capped title, timer
- [x] 4. Reduce Motion: still for 1.5 s
- [x] 5. A new flash replaces a running one; panel released at the end
- [x] 6. Hidden launch argument to render a style; Flash now uses the engine
- [x] Review: GPT phase review green (second pass)
- [x] Review: Fable approves the Phase 6 design (with changes, all taken)

## Phase 6. Hotkeys, notifications, automation

- [x] 1. KeyboardShortcuts with the four actions and a recorder in Settings
- [x] 2. Complete-current notification with Undo; fallback to the popover
- [x] 3. Empty Now reports nothing to complete
- [x] 4. One notification per durability or settings failure
- [x] 5. Optional: URL scheme (add, show) and App Intents
- [x] Review: GPT phase review green (fourth pass)
- [x] Review: Fable approves the Phase 7 design (with changes, all taken)

## Phase 7. Quality

- [x] 1. Swift unit tests against a temporary engine (golden files too)
- [x] 2. Parity tests: status title, truncation, hotkey collisions
- [x] 3. UI tests: popover add, complete, undo; Board drag; settings to file
- [x] 4. README audit and recorded differences (table below)
- [x] 5. Accessibility: VoiceOver labels, Reduce Motion, Increase Contrast
- [x] 6. Overlay on a second display and over full screen; crowded menu bar
      (by hand: see the hand checks below)
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
- The flash is drawn on the screen with the menu bar, as GNOME draws it on
  the primary monitor; the top bar styles colour the menu bar (or the notch
  row, which is taller). Reduce Motion holds it still for 1.5 s, as GNOME
  does with animations off.
- The global shortcuts are Control-Option with GNOME's letters (Super is
  taken on the Mac); they are changed in Settings, not with
  `queue-focus-setup`.
- A Done notification goes once the queue changes, since its Undo could no
  longer work; GNOME's go with the extension's next action. Banners show
  even while the app is in front. With notifications off, or before the
  user has answered the first request, the popover opens with its Undo row
  instead.
- `queuefocus://add?text=…` (`&now=1`) and `queuefocus://show?view=…`
  (queue, board, add) do what the D-Bus `Add` and `Show` do. There is no
  link to complete a task: any web page can open a link. Shortcuts has Add
  a Task, Complete the Current Task and Get the Current Task.
- `-notifications off` keeps the app from asking for notifications; the UI
  tests pass it, and move the global shortcuts to Control-Option-Shift
  with J, L, K and U through the launch arguments.
- The tag chip's blue is GNOME's next blue down (#1C71D8), not its accent:
  white on the accent is 3.8 to 1, under the 4.5 small text needs. The
  Board's Later rows step back by a grey title, not by fading the whole row,
  which faded the chip below 3 to 1. With Increase Contrast both tag
  colours darken further and the Later titles are not greyed; with Differentiate Without Color the menu bar
  names the tag's letter beside its dot; with Reduce Transparency the flash
  card is solid. The paused timer in the windows carries the `❚❚` glyph, as
  the menu bar's does, rather than a fainter colour alone.
- Settings is a window of its own that scrolls, rather than a page in the
  Queue's window; it adds the menu bar title width, the global shortcuts and
  Launch at login.
- The Queue and Board windows have no full-screen mode (SwiftUI gives its
  `Window` scenes none).
- `-displayOptions increaseContrast,reduceTransparency,…` stands in for the
  system's accessibility display settings, for the UI tests and for
  checking by eye; nothing changes the user's System Settings.
- `-flashPreview <style>` (`all` for the six in turn), with
  `-flashIntensity` and `-flashPalette`, draws a flash with a sample task
  two seconds after launch, for checking a style by eye.

Known gaps, to close later:
- ⌃⌥D is also Rectangle's default for "First Third", and two apps can hold
  one hot key without either knowing, so both act. Settings says to change
  a shortcut that does nothing.
- ⌘1 and ⌘2 are system shortcuts on the Mac these tests were written on
  (desktop switching, most likely), so ⌘1 never reaches the app there; a
  test reports it, and fails on a build machine if it ever happens on a
  clean Mac. The UI tests go back to the Queue by its button.
- The accessibility audit lets through only what is the system's, each
  matched narrowly: a window's hosting root, the Touch Bar and its items, a
  text field's completions window, a named slider's thumb, a window's title
  bar (its title, and content scrolled under it), text a scroll view cuts
  off (Settings is audited part by part as it scrolls, so each line is
  measured whole), and text outside every window. Grey text in the app uses
  a grey that reads at 4.5 to 1 in either appearance, where the system's
  secondary grey does not. Every issue is printed; on another macOS than
  27 they are reported, not failed.

Hand checks, which a test cannot make here:
- ⌘1 and ⌘2 on a Mac where the system does not hold them.
- The real Done notification and its Undo button, once notifications are
  allowed.
- The flash over a full-screen app, and on a Mac with a second display (it
  is drawn on the screen with the menu bar).
- The status item with a crowded menu bar (macOS hides items that do not
  fit; the title width setting is the defence).
- A VoiceOver walk through the popover, the Queue, the Board and Settings.
- Shortcuts and Spotlight listing Add a Task, Complete the Current Task and
  Get the Current Task.

### README, section by section

| README section | On the Mac | Covered by |
| --- | --- | --- |
| The four buckets | The same: the shared engine. | QfCore EngineTests, QueueModelTests |
| Top bar | A status item: dot in the tag's colour, the title cut to a width set in Settings, the clock with `❚❚` while paused. Right-click pauses. The popover is fixed-width, with tooltips; Done offers Undo for eight seconds; the global shortcut's completion is a notification with Undo, or the popover. Save problems are notifications and the message line. No service to start or quit. | StatusTitleTests, MenuBarUITests, NoticesTests, HotkeyUITests |
| Queue view | The same three bands; its own window. | WindowUITests, AccessibilityUITests |
| Board view | The same four quadrants, drags, rings and lines; its own window. | WindowUITests, DragStateTests |
| Settings view | Its own window (⌘,), scrolling; adds the menu bar title width, the global shortcuts and Launch at login. | SettingsUITests, HotkeyUITests |
| Quick add window | A floating panel: ⌃⌥⇧Q or ⌘N; Return adds, ⌘Return adds as current, Escape closes. | QuickAddTests, QuickAddUITests |
| Install, update, uninstall, choose the version | Not yet: Phase 8 (a notarized app). | — |
| Global shortcuts | ⌃⌥Q, ⌃⌥⇧Q, ⌃⌥B, ⌃⌥D, changed in Settings rather than with `queue-focus-setup`; no Accessibility permission. | HotkeyTests, HotkeyUITests |
| The flash reminder | The same six styles, envelopes and card, on the screen with the menu bar; the top bar styles colour the menu bar; Reduce Motion holds it still for 1.5 s, as GNOME does with animations off. | FlashPlanTests, FlashStageTests, FlashControllerTests, FlashUITests |
| Add tasks | The same markers: the shared engine. ⌘Return where GNOME has Ctrl+Enter. | QueueModelTests, WindowUITests |
| Keyboard | The same task keys in the Queue and the Board; ⌘1 and ⌘2 where GNOME has Ctrl+1 and Ctrl+2; ⌘, for Settings; Escape and ⌘W close a window, ⌘Q quits. Settings is a window of its own, so `q` and `b` do not lead from it to the Queue and the Board; ⌘1 and ⌘2 do. | PlacementTests, WindowUITests |
| Mouse | The same: double-click promotes, drags and drops, tooltips only on cut titles. | WindowUITests |
| Command line use | No command; `queuefocus://add` and `queuefocus://show` links, and Shortcuts actions (`shortcuts run` for the current task). | AppURLTests, IntentsTests, HotkeyUITests |
| Task data | The same file, format and modes, in `~/Library/Application Support/queue-focus`; a task file that cannot be read is an alert, and the app quits. | golden files, EngineTests |
| Settings data | The same file and keys; the Mac-only settings (title width, shortcuts) live in the app's defaults, the login item with the system. | golden files, QueueModelTests |
| D Bus interface, Debian package | GNOME only. | — |
| Developer commands, Source layout | Already list the Mac's make targets and directories; Phase 8 adds the release target and corrects the Xcode version. | — |
| Common problems | "A shortcut does not work": another app may hold it, change it in Settings. | — |

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

Second pass, not green:
1. A task moved between lists still shown, or made current, would leave the
   keyboard in the add field if AppKit dropped it there. It does not: SwiftUI
   hands focus to the task's new row. The Queue's key test already moved and
   promoted a task and kept using the keys; a Board test now does the same
   through Side, Later and Now, and checks nothing reached the add field.
   The keeper says why it relies on this rather than guess at intent from
   the mouse, which would take the field from VoiceOver.
2. A rename begun while the add field had the keyboard kept that choice, so
   closing the shelf over it left the keyboard in the add field. Starting a
   rename clears it.

Third pass: green. The answer to the first finding was accepted; no
further findings.

### Phase 5 design, Fable

Approved with changes, all taken. Two scratch AppKit probes backed them: a
borderless non-activating panel at the screen saver's level stays the size
of the screen, never activates the app or takes the keyboard, and sits over
the menu bar; the menu bar here is 30 points while the status bar's
thickness says 22; Core Animation's ease-in-ease-out is not Clutter's quad;
and a transaction's completion fires at once for a panel not yet on screen.
1. The plan follows flash.js exactly: the edges dip to 0.5 (not wash2's
   0.45), the card takes the intensity's scale, every step eases with
   Clutter's quad curve, the soft edges breathe, the beam, glow and timer
   keep flash.js's guards, and the title is bounded as `safeTitle` bounds
   it, by code point.
2. The stage is built where its run ends, put on screen, and only then
   played; the end of a superseded flash does nothing; no animation
   delegate holds the controller.
3. The stage is flipped, so the plan keeps GNOME's coordinates; it sits one
   layer below the view's own, since AppKit sets the flip of a layer a view
   hosts. (The first try hosted it directly; the top bar came out at the
   bottom of the screen, and a test now pins the orientation in the panel.)
4. The menu bar's height comes from the screen, or the notch, or the main
   menu; the status bar's thickness only as a last resort.
5. The panel never hides on deactivation, adds no fade of its own, stays
   out of the Window menu, takes the screen's frame on every flash, and is
   not drawn with no screen at all.
6. The card is SwiftUI, drawn once into a picture at the screen's scale.
7. The Reduce Motion hold is cancelled when a flash replaces it.
8. The preview goes through the normal launch; the flash is an
   accessibility group, `flash-overlay`, labelled with what it says.
9. The layer tree and its animations are tested without a window, and the
   controller with a stand-in surface and clock.

Validation: plan tests ported from the GNOME flash tests case by case;
layer tests that read the animations and render the stage off screen (the
top bar at the top, the beam below it, each glow strip fading inwards, the
card upright and sliding down); a panel test for orientation, accessibility
and level; controller tests for replacing, holding still and clearing; UI
tests that the preview and Flash now draw a flash that goes, and that the
keyboard stays where it was. Each style was looked at by capturing the
flash's own window, never the screen.

### Phase 5, GPT (gpt-6.1-sol, xhigh)

First pass, not green:
1. Where two glow strips overlap, Clutter dims each on its own as the glow
   fades, while Core Animation dimmed the pair as a group, so the corners
   came out darker mid-fade (0.45 against 0.56 at half way). Each part now
   fades on its own and the layer only moves, which also spares an
   offscreen pass. (Turning group opacity off was tried first; the off
   screen renderer ignores that flag, so the test could not see it.)
2. The ease was the usual cubic stand-in for Clutter's quadratic, 1.5%
   off at the middle of each step. Each step is now split at its middle
   into the two quadratics, which cubics draw exactly; the test samples
   every step against Clutter's formula.
3. `safeTitle` folded U+0085, which JavaScript's `\s` leaves alone.

Second pass: green, no findings. It sampled every easing step against
Clutter's formula and checked `safeTitle` against JavaScript's white space
for every Unicode scalar.

### Phase 6 design, Fable

Approved with changes, all taken. Spikes behind them: KeyboardShortcuts
3.1.0 builds for macOS 14 under Swift 6; two processes can register the
same hot key and both are told; the shortcut store can be overridden from
the launch arguments without touching the user's defaults.
1. Completing the current task says whether it was done, Now was empty, or
   the save failed; a failure is a note with the reason, not "Nothing in
   Now".
2. Nothing waits on the permission prompt: the first time, the popover
   offers the undo and the request is for next time.
3. The notification delegate and the Done category with its Undo are set
   up at launch; a Done note from a previous run is withdrawn at launch and
   at quit, since the engine's undo record does not outlive the app.
4. Windows open from AppKit through SwiftUI's own actions, lent by a view
   kept in the status item; a shortcut brings the app in front with
   `activate(ignoringOtherApps:)`, since plain `activate()` only asks.
5. UI tests never ask for notifications and never press the real
   shortcuts; no UI test records a shortcut into the user's defaults.
6. A completion while the popover is open is offered in its row only.
7. A recorder refuses a shortcut another of the four already has.
8. The model reports problems to a closure; notifications stay outside it.
9. Links add and show only.
10. Hosted unit tests never touch the shortcut store or Carbon.
11. Undo from a note clears the popover's offer, and says "Nothing to undo"
    or why it failed.
12. Package versions come from the committed Package.resolved only.

Found while building it:
- A link that starts the app arrives before the queue is open; it was
  dropped, and is now held until the queue opens.
- The quick add field could miss the keyboard the first time the panel
  opened, before SwiftUI had built it. The panel is laid out first and the
  field focused at once, then again once SwiftUI settles. A hosted test
  now types into the panel five times over; with the old code it fails.
- XCTest's function keys never match a hot key, so the test shortcuts use
  letters; XCTest opens every link in a fresh copy of the app, so the link
  test covers a link that starts the app, and the unit tests the rest.

### Phase 6, GPT (gpt-6.1-sol, xhigh)

First pass, not green:
1. A Done note could outlive its undo: the queue could change, or a second
   completion overtake the first, while the permission was looked up. The
   completion's revision is taken before the wait and checked after it;
   tests hold the answer back and change the queue, or answer two
   completions out of order.
2. Undo from a note that could not be saved withdrew the note first, so
   nothing was left to try again with once the popover's eight seconds were
   up. The note now stays, saying why, with its Undo; tests break and then
   repair the task file.
3. A link that could not add said nothing with notifications off; it now
   opens the popover with the reason on its message line.
4. A link's failure was read after the next link had cleared it; the
   reason is taken at once, by a `LinkFollower` the tests drive.
5. A note Notification Center would not take still counted as shown; the
   notifier now says whether it was, and the popover opens if not.
6. The Settings test checked one label; it now reads all four recorders'
   keys, and refusing another action's key is a pure rule with its own
   test.

Each of 1, 2, 3 and 5 has a test that fails with its fix undone. Also: a UI
test that followed one which quits and restarts the app sometimes clicked
the menu bar while it was laying its items out again; tests now wait for
the status item to stay put after launch.

Second pass, not green: six ways for answers that arrive late to mislead,
and one wrong reason.
1. A retry note could be posted after the queue had moved on.
2. An older completion's note, answered last, could replace the newer one
   and then be withdrawn with it.
3. A late Undo on an older note withdrew the newer note.
4. A failed undo with no note to show lost its Undo once the popover's eight
   seconds were up.
5. Two failures answered out of order left the older reason on the message
   line.
6. A failed completion's reason could be cleared by a request in between.
7. Add a Task with blank text gave whatever reason an earlier failure left.
Notices now handles one request at a time, in the order they came, and
whatever waited checks the queue again before it says anything; an Undo
withdraws only its own note; the fallback renews the popover's offer and
shows the reason it kept; the intent says blank text is nothing to add.
Each has a test that fails with its fix undone; the stand-in Notification
Center holds back permission answers and posts to make the races happen.

Third pass, not green: two fallbacks still fell short. A completion whose
permission answer, or refused note, took longer than eight seconds opened
the popover after its offer had run out; it is renewed as the popover
opens. A stale Undo with no note to say so said nothing; it now says so on
the popover's message line. Each has a test that fails without its fix.

Fourth pass: green, no findings.

### Phase 7 design, Fable

Approved with changes, all taken. Spikes behind them: an out-of-process
accessibility dump confirmed SwiftUI drops a group's only element;
`performAccessibilityAudit` ran against SwiftUI on macOS 27 and its noise
was catalogued; the system's symbolic hot keys can be read; SwiftUI's
`Window` scenes have no full-screen mode.
1. Golden task and settings files in the core, read byte for byte by the
   core and through the Swift bindings, instead of a GTK-written fixture
   that would only repeat the shared serializer.
2. The default shortcuts as plain Carbon numbers, so tests never touch the
   user's stored shortcuts; a structural test, and comparisons with the
   system's shortcuts that fail on a build machine and report elsewhere.
3. Each bucket's heading in the bucket's group, so a bucket with one task
   keeps its row.
4. The audit gates on contrast and descriptions only, with the noise named;
   a table checks every icon-only control's label.
5. One `DisplayOptions` seam for Reduce Motion, Increase Contrast, Reduce
   Transparency and Differentiate Without Color, with its launch argument.
6. Full screen and a second display are hand checks.
7. No tests that cannot fail.
8. No hosted test touches the user's defaults.
9. Machine-dependent tests go through `TestEnvironment`; the test plan keeps
   no screenshots or recordings.

Found while building it:
- XCTest kept screen recordings and screenshots of failing UI tests in the
  result bundles, which can show anything else on the screen. The bundles
  were deleted unopened; the committed test plan now keeps none
  (`uiTestingScreenshotsLifetime`, `systemAttachmentLifetime` and
  `userAttachmentLifetime` all `keepNever`), the Makefile runs every test
  through it with `-collect-test-diagnostics never`, and the audit fails the
  test itself rather than letting a failed audit attach pictures. A failing
  UI test was run to prove no media is kept.
- The task menu read "More" to VoiceOver: a SwiftUI menu takes no
  accessibility label, only a title. It is now titled "Task menu" and drawn
  as its symbol.
- The add fields had no names, the sliders none for VoiceOver.
- Settings grew taller than a laptop's screen; it now scrolls.

### Phase 7, GPT (gpt-6.1-sol, xhigh)

First pass, not green:
1. Settings' explanatory lines were let off the contrast audit by
   identifier, though they say things worth reading. They, the flash
   status and the Board's Later titles now use a grey that reads at 4.5 to
   1 in either appearance; the exemption is gone, and Settings is audited
   in light and dark, part by part as it scrolls. That turned up the
   shortcut list: a key such as `l` is one stroke too thin to measure in
   grey or alone, so each line is now one text, the key in semibold.
2. Fading the Board's Later rows faded their tag chips below 3 to 1. Only
   the title steps back now; the audit seeds tagged Later tasks.
3. The audit's filters were broader than their reasons. Each now matches
   only its system piece: the hosting root by its window's frame, the
   completions window by name, a slider's thumb inside a named slider, the
   title bar band, text cut off by a scroll view.
4. The gear menu had the task menu's old problem: titled now. The label
   table covers the popover's gear, a Side card's buttons (shown under the
   pointer; the card also offers both as actions, since hidden buttons are
   hidden from VoiceOver too) and the message line, now one group.
5. The system-shortcut tests called Carbon's reader from parallel tests;
   they run on the main actor.
6. The pause test could fail at a second boundary; its clock starts after
   the engine stamps the task.
7. The display options were read before the hosted tests' early return;
   they are read once the app is past it.
8. Quick add now has the display options in its environment.
9. Two rows of the README table were stale.

Second pass, not green: four smaller things.
1. The title-bar exemption covered popovers and any text in the band; it
   now takes only a window's own title text.
2. Text a scroll view cut off was let through with no promise it would be
   measured: horizontally cut text is no longer let through at all, and
   text cut off at the top or bottom of its own scroll view is set aside,
   with the test failing unless each such line was seen whole, and so
   measured, in some pass.
3. The pause test could still meet a second boundary: the model reads the
   clock it is given on a tick, so it ticks before the first reading.
4. The light Settings audit followed the Mac's appearance; it is now light.

Third pass, not green: set-aside text was matched by its words, so two
chips reading "P" could stand in for each other. A surface audited once
may now set nothing aside, and across Settings' passes a line is matched
by its identifier, or by its words and where they start across the line.

Fourth pass: green, no findings, pending the test runs.

For a while neither the unit nor the UI tests could run: an
authentication prompt on the screen (macOS's coreautha) kept XCTest's
runner from connecting, until the user answered it. Then the unit tests
passed (122, one known issue), and the UI tests all but one (44 of 45):
the light Settings audit, light at last since the second pass, found grey
the dark run never showed.
1. LabeledContent draws its content in the system's secondary grey, 3.9
   to 1 here: the interval's minutes, the title width's points, the "and"
   between the quiet hours and the keyboard lines. They take the readable
   grey.
2. Two wrapped notes were failed by the audit at 5.8 and 6.2 to 1, as
   measured from a picture of the Settings window alone; a wrapped note
   beside them, of the same grey, passed. Text the audit fails is now
   measured from its own pixels, and let through, logged, only at 4.5 to 1
   or more. With the readable grey made too faint (white 0.56), every grey
   line in Settings fails, these two included.
3. The popover's audit once failed reading its own bookkeeping: the clock
   changed between listing a scroll view's text and reading one. Each pass
   now reads the app from one snapshot.

