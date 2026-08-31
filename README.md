# Queue Focus

A minimal focus queue for Ubuntu/GNOME. Tasks are just titles, arranged in four
buckets. The head of **Now** is the one thing you're doing; it lives in the GNOME
top bar with a running timer. Done tasks are deleted — there is no history.

| Bucket | Meaning |
|--------|---------|
| **Now** | What you're doing. The first item is *the* current task (timed, shown in the top bar). |
| **Side** | Things running in parallel to the current task (builds, waiting on someone, a slow download). |
| **Next** | Ordered queue. Finishing the current task pulls the head of Next into Now. |
| **Later** | Backlog. |

Each task can carry a `work` or `personal` tag. Both live in the same queue; the
window accent and the top-bar dot follow the tag of the current task.

## Three zoom levels

1. **Top bar** (GNOME Shell extension) – current task + elapsed time. Click for a
   popup with quick-add, *Done*, and one-click promotion of Side/Next items.
2. **Queue** (`Super+Q`) – a narrow panel: Now, Side, Next, and a collapsed Later.
3. **Board** (`Super+Alt+Q`) – four columns for the occasional big reshuffle.

Plus **quick add** (`Super+Shift+Q`): a floating entry, type, Enter, gone — and
`Super+Ctrl+Q` completes the current task with an on-screen confirmation.

The shortcuts belong to the extension; see them with `queue-focus-setup keys` and
change one with `queue-focus-setup key toggle-queue '<Super>space'`.

## Quick-add syntax

```
fix the login bug #w          → Next, tagged work
!ship v0.1                    → straight to Now (becomes current)
call mum #p @later            → Later, tagged personal
wait for CI @side             → Side
```
`#w`/`#work`, `#p`/`#personal`; `@now @next @later @side`; `!` prefix = Now.
`Ctrl+Enter` in any entry also sends to Now. A `#` that isn't a tag (`#123`) is kept.

## Keyboard (in the window)

| Key | Action |
|-----|--------|
| `j` / `k` | move focus down / up (across sections) |
| `J` / `K` | move task down / up within its bucket |
| `Enter` | make focused task current |
| `d`, `x`, `Delete` | done → delete (current task pulls the next one) |
| `1` `2` `3` `4` | move to Now / Next / Later / Side |
| `t` | cycle tag: none → work → personal |
| `r`, `F2` | rename |
| `l` | expand/collapse Later (queue view) |
| `n`, `/`, `a` | focus the add entry |
| `q` / `b`, `Ctrl+1` / `Ctrl+2` | queue / board view |
| `Esc`, `Ctrl+W` | hide window |

Mouse: drag rows between/within lists (or onto a section header, e.g. the collapsed
Later); hover a row for its buttons; double-click to make current; `⋮` menu for
move/rename/delete.

## CLI

```
queue-focus toggle|queue|board|hide      windows
queue-focus add "text"                   add (quick-add syntax)
queue-focus add                          open the floating quick-add
queue-focus done                         complete the current task
queue-focus status                       JSON snapshot
queue-focus quit                         stop the background service
queue-focus version
```

## D-Bus

Bus `org.queuefocus.QueueFocus`, object `/org/queuefocus/QueueFocus`, interface
`org.queuefocus.QueueFocus1`: `GetState`, `Add(text, bucket)`, `CompleteCurrent`,
`Promote(id)`, `Remove(id)`, `Move(id, bucket, index)`, `SetTag(id, tag)`,
`Show(view)`, `Hide`; signal `Changed(json)`. The service is D-Bus activatable,
so the extension starts it on demand.

## Data

`$XDG_DATA_HOME/queue-focus/tasks.json` (default:
`~/.local/share/queue-focus/tasks.json`). Plain JSON, written atomically; safe
to edit by hand or sync.

## Install / update

Prerequisites: GNOME Shell 48–50, Rust ≥ 1.80
(`curl https://sh.rustup.rs | sh`), Python 3, and GLib's
`glib-compile-schemas`. GTK development packages are optional: without them the
build links directly against the GTK 4.16+/libadwaita 1.6+ runtime libraries
already provided by a supported GNOME system (`scripts/cargo` selects the
appropriate mode).

```
make install        # ask for a version, then build and install into ~/.local
make update         # git pull --ff-only, ask for a version, then install
make uninstall      # remove everything except your tasks
```

`make install` is a self-contained per-user installation: it copies the binary
and setup helper into `~/.local/bin`, then installs the D-Bus service, desktop
entry, icons, and extension under `$XDG_DATA_HOME` (default:
`~/.local/share`). It builds and validates everything before replacing existing
files, so moving the checkout or running `make clean` afterward does not break
the installed app. No `sudo` is used.

Re-run `make install` (or `make update`) whenever you want the latest version.
The service restarts with the new binary and the top-bar extension reconnects by
itself. Changes to the extension's own code load at the next login on Wayland.
If `~/.local/bin` is not on `PATH`, installation still works from GNOME, but the
installer prints the command-line setup warning.

### Choosing the version number

Installation always uses the latest code in the working tree. When `VERSION` is
omitted, the version-aware commands prompt you and show the current version as
the default; press Enter to keep it:

```
make version
make install
make update          # pulls first, then prompts
```

For a non-interactive command, or simply to provide the answer up front, pass
the semantic version explicitly:

```
make version VERSION=0.2.0
```

This validates and atomically updates the workspace version in `Cargo.toml`,
both workspace packages in `Cargo.lock`, and GNOME's separate integer extension
revision. The extension revision increments automatically whenever the semantic
version changes. To choose it explicitly:

```
make version VERSION=0.2.0 EXTENSION_VERSION=4
```

You can combine versioning with either installation workflow:

```
make install VERSION=0.2.0   # version the current working tree, then install it
make update VERSION=0.2.0    # pull the latest code, version it, then install it
```

Pre-release and build versions such as `0.2.0-rc.1+build.5` are supported. The
version command does not change dependency versions, create a Git tag, or make a
commit. Non-interactive environments must pass `VERSION` explicitly rather than
waiting for input. `queue-focus version` and the `.deb` version are derived from
the Cargo workspace version automatically.

If GNOME's global extension safety switch is active, the app is still installed
successfully and the switch is left unchanged. After reviewing your enabled
extensions, explicitly clear it with:

```
queue-focus-setup enable --allow-user-extensions
```

Other targets: `make build`, `make test`, `make check` (fmt, clippy, JS, Python,
and shell syntax), `make test-install`, `make test-version`, `make deb`, and
`make help`.

### System-wide .deb

```
cargo install cargo-deb
make deb                                       # → target/debian/queue-focus_*.deb
sudo apt install ./target/debian/queue-focus_*.deb
queue-focus-setup                              # once per user: enable the extension
```

Upgrading = build a new `.deb` and `apt install` it again.

## Layout

```
crates/qf-core      model + JSON storage, no GTK (unit tested)
crates/queue-focus  GTK4/libadwaita app, D-Bus service, CLI
extension/          GNOME Shell extension (top bar)
data/               .desktop, D-Bus service file
scripts/            build, install, setup, versioning, and integration-test tooling
Makefile            install / update / version / check / deb
```
