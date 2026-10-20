# Ubuntu 24.04 compatibility implementation and validation

Completed 2026-10-20 against Queue Focus 0.5.1 / base commit `1420ab5`.

## Implemented

- Native API floors: GTK 4.14, libadwaita 1.5, GLib 2.80. Updated Debian metadata and README.
- Rust 1.92 workspace minimum inherited by both crates, checked by the installer; locked builds/tests.
- Shared layout with runtime-selected modern/legacy color palettes. Legacy work/personal accents use window selectors; selected-button colors follow theme changes, including pre-1.7 libadwaita.
- Preserved Board insertion-line geometry, including hovered and tagged rows, with rendered-pixel regression coverage.
- GNOME Shell 46 metadata support, retaining 48/49/50 and existing API compatibility branches.
- Native runtime preflight for normal and runtime-library-only builds; installer checks supported Shell versions and compiler availability before building.
- Explicit extension ES-module metadata so Ubuntu 24.04's Node 18 can run the JS tests and installer syntax checks.

## Validation results

All listed checks passed:

- `make test`: 53 core tests, 41 app unit tests, 21 flash checks, 15 connection tests, four runtime-preflight tests, installer integration tests, and versioning integration tests. The separate GTK test is intentionally ignored in the ordinary unit run.
- `make test-ui`: real GTK widgets on Ubuntu 24.04 with GTK 4.14.5/libadwaita 1.5.0; CSS diagnostics, tag/theme colors, selected-button fallback, hovered/unhovered drag-line pixels, placement/drop/focus/tooltip behavior, and drag feedback.
- The same GTK test binary on Ubuntu 26.04 with GTK 4.22.4/libadwaita 1.9.1, exercising the modern stylesheet as well as parsing the legacy palette.
- `make check`: Rust formatting, Clippy with warnings denied, JS syntax on Node 18, and Python/shell syntax.
- `make build`: optimized release with Rust 1.92.0 and the locked dependencies.
- Host `scripts/test-extension-shell.sh`: actual GNOME Shell 46.3.1, private bus/settings, menu/quick-add/reconnection/done/Undo/promotion/layout checks, and all six flash styles with animations enabled and disabled.
- Host real GJS D-Bus adapter integration test.
- A temporary end-to-end service probe using the release binary: D-Bus activation, add/promote, pause/resume, completion/Undo, Queue/Board/Settings/quick-add views, hide, settings persistence, task persistence after service restart, and private storage permissions. Repeated on the host's native Wayland/runtime with isolated data and without opening its temporary test windows.
- Host immediate symbol binding (`LD_BIND_NOW=1`) and native library resolution.
- Sol agent review: found two CSS cascade issues; both fixed and covered by the pixel test. Final re-review reported no actionable findings.

Build tools required a sudo password on the host. Compilation and Xvfb checks therefore used a disposable Ubuntu 24.04 container; modern GTK verification used a separate disposable Ubuntu 26.04 container. No host OS libraries or system build tools were upgraded.

## Installed result and limits

Ran the real installer in the build environment, with launch paths matching the user's account. Copied only its validated app binaries, extension, desktop/D-Bus integration, and icons into the host per-user installation, refreshed caches/D-Bus configuration, and ran the installed setup helper.

The running service executable is `~/.local/bin/queue-focus`. Its installed CLI successfully read the existing queue, and the Queue window was opened. SHA-256 comparisons confirmed that the existing `tasks.json` and `settings.json` contents were unchanged by installation and launch.

The Shell extension is enabled in the user's settings for the next session. **A Wayland logout/login is still required to discover and load the newly installed extension.** No logout or desktop restart was forced. Real desktop shortcut dispatch, fullscreen flash visibility, and interaction with the user's other extensions remain manual checks after login. GNOME Shell 48–50 were not re-run here; the modern GTK check does not substitute for those Shell environments.
