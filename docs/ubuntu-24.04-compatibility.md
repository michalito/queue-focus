# Queue Focus on Ubuntu 24.04: compatibility plan

Investigated 2026-10-20 at commit `1420ab5` (Queue Focus 0.5.1).

## Recommendation

Backport the application's styling and declare the older supported runtime; keep the existing Rust binding versions and extension implementation. The evidence supports a contained compatibility change, rather than an operating-system upgrade or a UI rewrite. A complete application build and installation remain unverified because the machine has no Rust toolchain or C compiler.

The extension is more compatible than its current metadata implies: with only `46` added in a disposable copy, the existing real GNOME Shell integration suite passed on this machine. Additional temporary checks passed notifications with an Undo action and all six flash styles with animations enabled and disabled. See the [extension audit and reproduction procedure](gnome46-extension-research.md).

## Observed machine and project requirements

Read-only package queries and runtime introspection found:

- Ubuntu 24.04.5 LTS; GNOME Shell 46.3.1; Wayland.
- GTK 4.14.5, libadwaita 1.5.0, GLib 2.80.0.
- No `rustc`, `cargo`, `make`, C compiler, `pkg-config`, `Xvfb`, or `xvfb-run` on the current PATH. Rust is also absent from the default `~/.cargo/bin` location.
- Node, Python, GJS, GNOME extension tools, and schema compilation tools are available.
- Local APT metadata offers GTK development files 4.14.5, libadwaita development files 1.5.0, and Rust 1.75.0. This is the machine's configured package metadata, not a claim about every possible Ubuntu repository.

The project currently opts into GTK 4.16 (`crates/queue-focus/Cargo.toml:19`), libadwaita 1.6 (`:20`), and GIO/GLib 2.80 (`:21`). Its Debian metadata repeats the higher native library floors (`:34–35`). The extension metadata lists only Shell 48, 49, and 50. README and installer advertise Rust 1.80, but exact locked crate manifests contradict that minimum.

## Required project changes

### 1. Lower the native API feature gates and package requirements

In `crates/queue-focus/Cargo.toml`, change GTK feature `v4_16` to `v4_14` and libadwaita feature `v1_6` to `v1_5`. Keep the binding crate families `gtk4 = 0.11`, `libadwaita = 0.9`, and `gio = 0.22`; these Rust crate version numbers do not require corresponding GNOME release numbers. GIO's `v2_80` already matches the installed GLib and need not be lowered.

Change the Debian requirements to `libgtk-4-1 (>= 4.14)`, `libadwaita-1-0 (>= 1.5)`, and the Shell recommendation to `gnome-shell (>= 46)`. README should distinguish the tested supported Shell majors `46, 48, 49, 50` from that packaging lower bound; this investigation does not establish GNOME 47 support.

An audit of the actual widget classes and calls found no demonstrated need for a newer widget or interaction API. `AdwAlertDialog` and its methods used here are available in 1.5, and `GtkCssProvider.load_from_string` is available since 4.12. Existing Queue, Board, Settings, drag/drop, and alert UI should be retained. This is a source/API audit, not a substitute for compiling with the lowered gates. [AdwAlertDialog API](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.AlertDialog.html), [GtkCssProvider.load_from_string API](https://docs.gtk.org/gtk4/method.CssProvider.load_from_string.html), [exact GTK binding manifest](https://docs.rs/crate/gtk4/0.11.4/source/Cargo.toml), [exact libadwaita binding manifest](https://docs.rs/crate/libadwaita/0.9.2/source/Cargo.toml)

### 2. Supply styling that GTK 4.14/libadwaita 1.5 can render

This is the main substantive backport. `crates/queue-focus/src/style.css` relies throughout on CSS custom properties, `var(...)`, and `color-mix(...)`; the work/personal rules also override accent variables on the window. GTK only gained CSS custom properties in 4.16. The installed 4.14 parser does not accept the stylesheet's `color-mix` expressions either. [GTK CSS reference](https://docs.gtk.org/gtk4/css-properties.html), [GTK 4.14.5 color parser source](https://raw.githubusercontent.com/GNOME/gtk/4.14.5/gtk/gtkcsscolorvalue.c)

**Recommended implementation:** retain the current modern stylesheet and add a legacy color/style variant selected in `ui::load_css()` when GTK is below 4.16 **or** libadwaita below 1.6. Share layout-only rules if that keeps the change readable; avoid two manually duplicated full stylesheets. This preserves modern behavior while making the older color handling explicit. A single legacy-compatible stylesheet is also viable, but would change the styling path for every supported runtime and require broader visual regression checking.

The legacy variant must:

- Use named colors such as `@window_fg_color`, `@headerbar_bg_color`, `@card_bg_color`, `@card_fg_color`, `@card_shade_color`, `@destructive_color`, and GTK's `mix()` / `alpha()` functions. These named colors were verified in the installed libadwaita theme. [libadwaita 1.5 named colors](https://raw.githubusercontent.com/GNOME/libadwaita/1.5.0/doc/named-colors.md)
- Preserve blue work, orange personal, and untagged accents. Current window classes are already applied in `ui.rs:1630–1636`. Named colors lack CSS variables' per-widget cascading: implement the tag-dependent declarations with scoped selectors or deliberately updated app palette rules, rather than assuming `@define-color` can be set inside a window selector.
- Preserve light/dark/System theme behavior. In particular, the current `--active-toggle-bg-color` and `--active-toggle-fg-color` references (`style.css:60–61`, `:540–541`) have no equivalent installed `@active_toggle_*` names. Those CSS variables are documented as **since libadwaita 1.7**, so the app already needs explicit fallbacks if its claimed 1.6 minimum is to be accurate. Define app-owned selected-button colors for light/dark mode (and refresh on theme changes), or provide appropriate CSS fallbacks in the modern variant. [Active Toggle Colors](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/css-variables.html#active-toggle-colors)
- Preserve the tint/foreground distinction, disabled and paused states, focus outlines, and drag indicators. A successful CSS parse alone cannot establish equivalent contrast or appearance.

### 3. Admit GNOME Shell 46 in extension metadata

Add `"46"` to `extension/queue-focus@queuefocus.org/metadata.json`'s `shell-version`, keeping `48`, `49`, and `50`. Existing orientation and compositor feature detection already works on 46. No notification API shim or separate legacy extension was demonstrated necessary. Update the compatibility comments and README. Keep GNOME's normal version validation enabled. [GNOME metadata documentation](https://gjs.guide/extensions/overview/anatomy.html#shell-version)

The real-Shell integration suite and extra flash/notification checks passed in temporary copies, with no changes to production JavaScript. This is stronger evidence than comparing API names alone, but does not establish full installed application integration. [Detailed local test evidence](gnome46-extension-research.md)

### 4. Correct and check the actual build prerequisites

The downloaded manifests for the exact locked `gtk4 0.11.4`, `gio 0.22.8`, and `glib 0.22.8` all declare `rust-version = "1.92"`. The README/installer's Rust 1.80 check is insufficient. Keep the existing dependency versions and use Rust **1.92 or newer**; downgrading the entire Rust GTK binding stack is unnecessary for the older native runtime. [gtk4 manifest archive](https://static.crates.io/crates/gtk4/gtk4-0.11.4.crate), [gio manifest archive](https://static.crates.io/crates/gio/gio-0.22.8.crate), [glib manifest archive](https://static.crates.io/crates/glib/glib-0.22.8.crate), [Cargo rust-version documentation](https://doc.rust-lang.org/cargo/reference/rust-version.html)

Add a workspace `rust-version` and inherit it in both member packages. Make installer checks and README agree with the tested minimum; keep the lockfile in builds (`--locked`) for reproducibility. This establishes a lower bound from the locked dependency manifests; only a build of the complete lockfile proves the whole project's minimum toolchain.

Add an early runtime prerequisite check for GNOME, GTK, and libadwaita, including the runtime-library-only build path in `scripts/devlibs-env.sh`. That path deliberately bypasses `pkg-config` and therefore its version checks. Checking `queue-focus version` under `LD_BIND_NOW=1` is useful for symbol linkage, but does not validate CSS support or exercise the app's UI.

Detect a missing C compiler before starting the build and report actionable prerequisites. Do not make GTK development packages mandatory unless removing the runtime-only build path intentionally. Python can query the installed native library versions via `ctypes` without needing development packages or opening an application window.

## Machine setup after the patch

For a source build, this machine needs `build-essential` (including `make` and a C compiler) and a Rust toolchain at least 1.92. Rustup is the project's documented approach; the currently configured APT Rust 1.75 package is too old. The clean development setup also includes `pkg-config`, `libgtk-4-dev`, and `libadwaita-1-dev`, available at the appropriate native runtime versions from this machine's repositories. [Official Rust installation](https://www.rust-lang.org/tools/install)

`Xvfb`/`xvfb-run` are additional test prerequisites for `make test-ui`, not runtime requirements. The real headless GNOME Shell test already runs here without them. A prebuilt compatible binary could remove the user's Rust/compiler requirement; it would still need the CSS and Shell metadata changes.

## Verification completed and limits

Completed without installing software or changing the live queue/extension/settings:

1. Read package versions and queried the actual GTK/libadwaita runtime versions.
2. Loaded the shipped stylesheet with the real GTK 4.14.5 parser: **66 parsing diagnostics** (multiple diagnostics can arise from one declaration).
3. Converted the modern color syntax in a temporary `/tmp` probe, removing the per-window custom-property definitions: **zero parsing diagnostics**. This demonstrates viable parser syntax only; it does not implement task accents, selected-button colors, or visual parity.
4. Queried the real theme's symbolic colors: all named colors listed above exist; both `active_toggle_*` names are absent.
5. Verified the exact locked GTK/GIO/GLib crate manifest Rust minimums from crates.io archives.
6. Ran the existing extension suite and extra flash/notification checks in a disposable GNOME 46 session: **passed**. The fixture uses private settings, XDG directories, and D-Bus, not the Rust application.

Before calling the port ready:

1. Build with the lowered native feature gates, the locked dependencies, and Rust 1.92+; run the Rust, extension, installer/version, and GJS D-Bus suites plus `make check`.
2. Add CSS parsing checks that fail on diagnostics, and check palette resolution so nonexistent named colors cannot silently pass a syntax-only test.
3. Run real GTK tests and inspect Queue, Board, Settings, quick-add, work/personal/untagged colors, System/light/dark changes, selected buttons, focus, pause, and drag feedback on GTK 4.14/libadwaita 1.5.
4. Re-run GNOME 46 shell tests with the metadata committed, retaining new notification/flash coverage; test existing 48–50 targets as regression environments.
5. Install for the user and verify D-Bus activation, status, add/promote/pause/complete/Undo, persistence after service restart, and app-extension state synchronization with disposable tasks or isolated data.
6. Check physical shortcuts, real notification action dispatch, fullscreen flash visibility, and the Ubuntu theme/scaling in the actual desktop session. After loading new extension code on Wayland, log out/in as documented; do not restart the user's whole desktop during automated verification.

No production code has been changed by this research. Only research documentation was added.
