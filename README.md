# Queue Focus

Queue Focus is a task queue for GNOME. It keeps one task visible in the top bar and stores the rest in four ordered buckets.

A task has a title, a bucket, and an optional `work` or `personal` tag. There are no projects, dates, priorities, or completion history. Marking a task done deletes it.

Queue Focus supports GNOME Shell 48, 49, and 50.

## The four buckets

1. Now contains active tasks. The first task in Now is the current task. It appears in the top bar and has a timer you can pause.

2. Side contains work that can continue beside the current task. A build, download, or request waiting for a reply can go here.

3. Next is an ordered queue. If completing the current task leaves Now empty, the first task in Next moves to Now.

4. Later is the backlog.

Promoting a task puts it first in Now. The previous current task stays in Now behind it. A task starts a new timer whenever it becomes current, and any pause is cleared.

## What you can open

### Top bar

The GNOME Shell extension shows the current title and elapsed time. While the timer is paused the time stops and a `⏸` follows it. Its dot uses the tag of the current task: blue for work, orange for personal, and grey when untagged.

The app window also changes its accent to match the tag of the current task.

Open the top bar menu to add a task, complete the current task, or promote a task from Now, Side, or Next. The menu shows up to eight Next tasks and the number of Later tasks. It also has buttons for the Queue and Board views.

The extension starts the app service when needed. It reconnects after an install, update, or service restart. A save warning appears as a GNOME notification.

### Queue view

The Queue view is a narrow window in three fixed bands.

1. The current task sits at the top in its own banner: the `Now` heading, its tag, a timer, the title, a done button, and a menu. When Now is empty the banner reads `empty — promote one ↑`.

2. Below it, one scrolling card holds Side and then Next. Anything else in Now — what is queued behind the current task — is listed at the top of Next.

3. Later is a shelf pinned to the bottom of the window. It starts collapsed and scrolls once open. Its rows can be promoted or moved to Next directly.

### Board view

The Board view shows the same tasks in four columns. Use it when you need to move several tasks.

### Quick add window

The quick add window is a small entry that opens without the main window. Use `Super+Shift+Q` from the GNOME desktop or overview. Press Enter to add to Next. Press `Ctrl+Enter` to add to Now. Press `Escape` to close it.

All views use the same service and task file. A change in one view appears in the others.

## Install for the current user

Run this from the repository root:

```sh
make install
```

The command asks for the app version. Press Enter to keep the version shown in brackets. The installer then builds the latest code in the current working tree and installs it for the current user.

Do not use `sudo` with `make install`.

The install contains these files:

1. `~/.local/bin/queue-focus`

2. `~/.local/bin/queue-focus-setup`

3. The GNOME Shell extension under `$XDG_DATA_HOME/gnome-shell/extensions`

4. The D Bus service under `$XDG_DATA_HOME/dbus-1/services`

5. The desktop entry under `$XDG_DATA_HOME/applications`

6. The app icons under `$XDG_DATA_HOME/icons`

If `XDG_DATA_HOME` is not an absolute path, the installer uses `~/.local/share`.

The installed app does not refer back to the repository. You can move the repository or run `make clean` after installation.

The installer builds and checks staged files before replacing an installed extension. It validates the GNOME schema and extension metadata. It also checks the JavaScript when Node.js is available. Existing extension files are restored if replacement fails.

On Wayland, log out and back in after the first install or after extension code changes. App binary updates do not need a logout. The extension reconnects when the new service starts.

If `~/.local/bin` is not in `PATH`, GNOME can still open the app. Add that directory to `PATH` if you want to use `queue-focus` and `queue-focus-setup` in a terminal.

### Requirements

The local install needs:

1. GNOME Shell 48, 49, or 50

2. Rust and Cargo 1.80 or newer

3. GTK 4.16 or newer and libadwaita 1.6 or newer at runtime

4. `make`, Bash, Python 3.11 or newer, a C compiler, and common Unix file tools

5. `glib-compile-schemas`, `gsettings`, and `gnome-extensions`

6. Internet access when Cargo needs to download a dependency

Install Rust from [rustup.rs](https://rustup.rs/) if it is not present.

GTK development packages are optional. `scripts/cargo` uses them through `pkg-config` when they are installed. Otherwise it links against the GTK and libadwaita runtime libraries on the system. Use `scripts/cargo` or the Make targets so this choice is made for you.

Node.js is optional for installation and required by `make check`.

## Update and uninstall

To pull the current branch and install it:

```sh
make update
```

If an `origin` remote exists, the pull uses fast forward mode. If the branch has diverged or local changes block the pull, resolve the Git state and run the command again. If there is no `origin` remote, the command installs the current working tree.

To reinstall the current working tree without pulling:

```sh
make install
```

To remove the current user install:

```sh
make uninstall
```

Uninstall removes the app, extension, desktop entry, service file, and icons. It keeps the task file.

## Choose the version

Queue Focus has two version numbers.

The app version is a semantic version such as `0.2.0`. Cargo, the binary, and the Debian package use it.

The GNOME extension revision is a positive integer. GNOME uses it to tell extension builds apart.

### Interactive use

These commands ask for the app version when `VERSION` is not given:

```sh
make version
make install
make update
```

The prompt looks like this:

```text
Queue Focus version [0.1.0]:
```

Press Enter to keep the current version. Invalid input is rejected and the prompt is shown again.

### Explicit use

Pass the version on the Make command line when no prompt is wanted:

```sh
make version VERSION=0.2.0
make install VERSION=0.2.0
make update VERSION=0.2.0
```

Ambient environment variables named `VERSION` or `EXTENSION_VERSION` are ignored. This prevents an unrelated shell setting from changing project files. Put each value after the Make target as shown above.

In a noninteractive job such as CI, pass `VERSION` explicitly. The command stops with an error if it cannot read an answer.

Pre release and build information are supported:

```sh
make version VERSION=0.2.0-rc.1+build.5
```

When the app version changes, the GNOME extension revision increases by one. Set it yourself when needed:

```sh
make version VERSION=0.2.0 EXTENSION_VERSION=4
```

The extension revision must be from 1 through 2147483647. It must increase when the app version changes. It cannot go backwards when the app version stays the same.

The version command updates `Cargo.toml`, the local workspace entries in `Cargo.lock`, and the extension `metadata.json` as one operation. If one replacement fails, it restores the old files. It does not change dependency versions, create a Git tag, or create a commit.

Check the installed binary version with:

```sh
queue-focus version
```

## Global shortcuts

The GNOME extension owns four global shortcuts.

1. `Super+Q` shows or hides the Queue view.

2. `Super+Shift+Q` opens the quick add window.

3. `Super+Alt+Q` opens the Board view.

4. `Super+Ctrl+Q` completes the current task. GNOME shows the result on screen.

Show the current shortcut values with:

```sh
queue-focus-setup keys
```

Change one shortcut with:

```sh
queue-focus-setup key toggle-queue '<Super>space'
queue-focus-setup key quick-add '<Super><Shift>space'
queue-focus-setup key show-board '<Super><Alt>space'
queue-focus-setup key complete-current '<Super><Control>space'
```

The valid names are `toggle-queue`, `quick-add`, `show-board`, and `complete-current`.

Disable the extension with:

```sh
queue-focus-setup disable
```

Enable it again with:

```sh
queue-focus-setup enable
```

GNOME has a global safety switch that can disable every user extension. Queue Focus does not clear it without an explicit command. Review the extensions enabled for your account, then run this if you want to clear the switch:

```sh
queue-focus-setup enable --allow-user-extensions
```

## Add tasks

An entry adds to Next by default. Use markers anywhere in the text to set a tag or bucket.

```text
fix the login bug #w
!ship version 0.2.0
call mum #p @later
wait for CI @side
```

The accepted tag markers are:

1. `#w` and `#work` select work.

2. `#p` and `#personal` select personal.

The accepted bucket markers are `@now`, `@next`, `@later`, and `@side`. Their short forms are `@n`, `@x`, `@l`, and `@s`.

Recognised markers are removed from the title. Text that is not a valid marker stays in the title, so `#123` is kept. If more than one `#` or `@` marker is present, the last recognised marker of that kind wins. `!now` and a bare `!` select Now wherever they appear, and a leading `!` on the title does the same.

In the main window, Enter adds to Next and `Ctrl+Enter` adds to Now. `Escape` clears a nonempty entry. Press it again to return focus to the task list. The entry's placeholder shows the marker syntax rather than the `Ctrl+Enter` shortcut, which still works.

## Use the app window

### Keyboard

Task keys work when an entry is not being edited.

The task keys act on the focused task. In the Queue view the current task's banner is the first focus stop, so with nothing else focused these keys act on the current task.

1. `j` and `k` move focus down and up across visible sections.

2. `J` and `K` move the focused task down and up within its bucket.

3. `Enter` makes the focused task current. The current task is already current, so its banner ignores it.

4. `d`, `x`, and `Delete` mark the focused task done. A task that is not current is deleted. Completing the current task also pulls from Next when Now becomes empty.

5. `1`, `2`, `3`, and `4` move the task to Now, Next, Later, and Side.

6. `t` cycles the tag through no tag, work, personal, and no tag.

7. `p` pauses or resumes the current task's timer. The stored time is kept, so resuming continues where it stopped.

8. `r` and `F2` rename the task in place. The title becomes an entry. Enter saves the new title. Escape or moving focus away cancels it. An empty title is not saved.

9. `l` expands or collapses the Later shelf in the Queue view.

10. `n`, `/`, and `a` focus the add entry.

11. `?` opens the keyboard shortcut list in the header bar.

12. `q` and `b` open the Queue and Board views.

13. `Ctrl+1` and `Ctrl+2` open the Queue and Board views.

14. `Escape`, `Ctrl+W`, and `Ctrl+Q` hide the window.

### Mouse

Double click a task to make it current. Drag a task to a new position or bucket. Drop it on a bucket heading to place it at the end of that bucket. This also works with an empty bucket or the collapsed Later shelf. Drop a task on the current task's banner to make it current.

Queue rows carry a button that makes the task current and a menu button. Later rows add a `→ next` button. Board rows have the menu alone because the columns are narrower. Every row's menu can make the task current, cycle its tag, mark it done, move it to another bucket, rename it, or delete it.

The current task's banner has its own done button and menu. Click its tag to cycle it, and click the timer to pause or resume.

Closing an app window hides it. The service continues running so the top bar and global shortcuts keep working.

## Command line use

Most commands start the D Bus service when it is not running.

```sh
queue-focus
queue-focus toggle
queue-focus queue
queue-focus show
queue-focus board
queue-focus add
queue-focus add "fix login #w @next"
queue-focus done
queue-focus status
queue-focus hide
queue-focus service
queue-focus quit
queue-focus version
queue-focus --version
queue-focus help
queue-focus -h
queue-focus --help
```

`queue-focus` with no command and `queue-focus toggle` both show or hide the Queue view.

`queue-focus queue` and `queue-focus show` open the Queue view. `queue-focus board` opens the Board view.

`queue-focus add` opens the quick add window. When text follows `add`, the task is added without opening a window. The same marker syntax is accepted.

`queue-focus done` completes the current task. It prints the deleted title, or `nothing in Now`.

`queue-focus status` prints a compact JSON snapshot with `current`, `now`, `side`, `next`, and `later` fields. Each task in it carries its id, title, tag, start time, and pause time. This is a view of the current state, not the storage file format.

`queue-focus hide` hides every app window. `queue-focus quit` stops the service. The next app or extension request starts it again.

`queue-focus service` starts the service without opening a window. It is mainly used by D Bus activation and `make run`.

## Task data

Tasks are stored at:

```text
$XDG_DATA_HOME/queue-focus/tasks.json
```

The default path is:

```text
~/.local/share/queue-focus/tasks.json
```

The file is readable JSON. It contains the next task id and an ordered list of tasks. Each task has an id, title, bucket, creation time, optional tag, optional start time, and optional pause time. A file written by an older version loads unchanged.

The app creates the data directory with mode `0700` and the file with mode `0600`. It sets those modes when it loads an older file. It rejects a task file that is a symbolic link.

Each save writes and syncs a temporary file, replaces the task file atomically, and syncs its directory. A failure before replacement rolls the change back in memory. A directory sync failure after replacement keeps the committed change and reports a warning. The app shows the warning in a dialog, the extension shows a GNOME notification, and a command line change writes it to standard error.

Malformed JSON is not replaced with an empty queue. The service refuses to start and prints an error so the file can be repaired.

Stop the service before editing or syncing the task file:

```sh
queue-focus quit
```

The next Queue Focus command reloads the file. Editing or replacing it while the service is running can be overwritten by the next task change.

## D Bus interface

Queue Focus is a session D Bus service.

```text
Bus name: org.queuefocus.QueueFocus
Object path: /org/queuefocus/QueueFocus
Interface: org.queuefocus.QueueFocus1
```

The service exports these methods:

1. `GetState()` returns the same JSON snapshot as `queue-focus status`.

2. `Add(text, bucket)` parses the add markers, creates a task, and returns its id. Valid bucket values are `now`, `next`, `later`, and `side`, with the short forms `n`, `x`, `l`, and `s`. An unknown value defaults to Next. A bucket marker in the text takes precedence.

3. `CompleteCurrent()` deletes the current task and returns a Boolean.

4. `Promote(id)` moves a task to the front of Now.

5. `Remove(id)` deletes a task.

6. `Move(id, bucket, index)` moves a task to a zero based position. It accepts the same bucket values as `Add`. A negative index or an index past the bucket length places it at the end.

7. `SetTag(id, tag)` accepts `work`, `personal`, `w`, or `p`. An empty string clears the tag.

8. `Show(view)` accepts `queue`, `board`, `add`, or `toggle`. Any other value opens the Queue view.

9. `Hide()` hides all app windows.

The `Changed(json)` signal is emitted after a saved task change. The `DurabilityWarning(message)` signal is emitted when the new task file was installed but its directory could not be synced.

Invalid arguments use `org.queuefocus.Error.InvalidArgs`. Save failures use `org.queuefocus.Error.Persistence`.

The service is activated on demand through its D Bus service file. A client does not need to start it first.

## Developer commands

Run these commands from the repository root.

```sh
make help
make build
make run
make check
make test
make test-install
make test-version
make deb
make clean
```

`make help` lists the Make targets.

`make build` compiles the GNOME schema and builds the release binary at `target/release/queue-focus`.

`make run` builds the app and runs the service in the foreground. Stop an installed service first so the development process can own the D Bus name:

```sh
queue-focus quit
make run
```

`make check` checks Rust formatting, runs Clippy for all targets with warnings denied, checks the extension JavaScript with Node.js, checks the version script, and checks shell files with the `.sh` suffix.

`make test` runs the Rust workspace tests, the local installer integration tests, and the version integration tests. The integration tests use temporary homes and project copies. They do not install into the developer account.

`make test-install` runs only the installer tests. `make test-version` runs only the version tests.

`make clean` removes Cargo build output and the compiled GNOME schema in the source tree.

For a direct Cargo command, use the repository wrapper:

```sh
scripts/cargo test --workspace
scripts/cargo build --release -p queue-focus
```

### Source layout

```text
crates/qf-core
crates/queue-focus
extension/queue-focus@queuefocus.org
data
scripts
Makefile
```

`crates/qf-core` contains the task model, add parser, JSON store, and unit tests. It has no GTK dependency.

`crates/queue-focus` contains the GTK and libadwaita app, D Bus service, command line commands, state handling, and app styles.

`extension/queue-focus@queuefocus.org` contains the GNOME Shell extension, its GSettings schema, metadata, and styles.

`data` contains the desktop entry, system D Bus service file, and icons.

`scripts` contains the Cargo wrapper, local installer, setup helper, version command, and integration tests.

`Makefile` provides the supported build, test, version, install, and package commands.

## Debian package

Install `cargo-deb`, build the package, and install it:

```sh
cargo install cargo-deb
make deb
sudo apt install ./target/debian/queue-focus_*.deb
queue-focus-setup
```

The package is written to `target/debian`. It installs the binary and integration files under `/usr`.

Run `queue-focus-setup` once for each user who wants the extension and shortcuts. Build and install a package with a newer app version to upgrade it. Removing the package does not remove task files from user home directories.

## Common problems

### The command is not found

Add `~/.local/bin` to `PATH`, then open a new terminal. The installer prints this warning when needed.

### The extension is installed but not visible

Log out and back in. GNOME Shell on Wayland does not load new extension code into the current session. Check its state with:

```sh
gnome-extensions info queue-focus@queuefocus.org
```

### GNOME says user extensions are disabled

Review the enabled extensions, then run:

```sh
queue-focus-setup enable --allow-user-extensions
```

### A shortcut does not work

Check the stored value and look for a conflict with another GNOME shortcut:

```sh
queue-focus-setup keys
```

Set a different key with `queue-focus-setup key`.

### The service refuses to start

Run the service in a terminal to see the error:

```sh
queue-focus service
```

If the task file contains malformed JSON, back it up and repair it. Queue Focus will not overwrite it.

### The app cannot load GTK or libadwaita

Confirm that GTK 4.16 or newer and libadwaita 1.6 or newer are installed. The installer tests the built binary before copying it.
