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

`~/.local/share/queue-focus/tasks.json` (or `$XDG_DATA_HOME`). Plain JSON,
written atomically; safe to edit by hand or sync.

## Install / update

Rust ≥ 1.80 (`curl https://sh.rustup.rs | sh`). GTK dev packages are optional:
without them the build links straight against the runtime libraries every GNOME
system already has (`scripts/cargo` picks the right mode).

```
make install        # build, install into ~/.local, restart the service, enable the extension
make update         # git pull --ff-only (if this is a clone with a remote) + make install
make uninstall      # remove everything except your tasks
```

Re-run `make install` (or `make update`) whenever you want the latest version:
the service restarts with the new binary and the top-bar extension reconnects by
itself. Changes to the extension's own code load at the next login (Wayland).

Other targets: `make build`, `make test`, `make check` (fmt + clippy + JS syntax),
`make deb`, `make help`.

### System-wide .deb

```
cargo install cargo-deb
make deb                                       # → target/debian/queue-focus_*.deb
sudo apt install ./target/debian/queue-focus_*.deb
queue-focus-setup                              # once per user: enable the extension
```

If GNOME's global extension safety switch is active, setup leaves it active and
exits with an explanation. After checking your enabled extensions, explicitly
clear it with `queue-focus-setup enable --allow-user-extensions`.

Upgrading = build a new `.deb` and `apt install` it again.

## Layout

```
crates/qf-core      model + JSON storage, no GTK (unit tested)
crates/queue-focus  GTK4/libadwaita app, D-Bus service, CLI
extension/          GNOME Shell extension (top bar)
data/               .desktop, D-Bus service file
scripts/            cargo wrapper, per-user install, queue-focus-setup
Makefile            install / update / check / deb
```
