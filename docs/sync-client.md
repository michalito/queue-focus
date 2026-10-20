# Sync setup and recovery

The sync utility and journal are under development. Native app integration is
the next checkpoint; released apps do not yet use the journal.

Build the utility with `cargo build --locked --release -p qf-sync`. Its path is
`target/release/qf-sync`. The server must already be initialized and have a
separate enrollment for each device; see [server operations](sync-server.md).

The default data directories are:

- macOS: `~/Library/Application Support/queue-focus`
- Linux: `~/.local/share/queue-focus`
- Either platform with an absolute `XDG_DATA_HOME`: `$XDG_DATA_HOME/queue-focus`

Quit Queue Focus, including its background/menu-bar service, before any
operation that changes these files. The utility and new apps use an exclusive
directory lock. Older app versions do not respect it and must be quit manually.

## Choose how to join

Create a private JSON file with mode 0600 containing `{"token":"YOUR_DEVICE_TOKEN"}`.
Use the token and device UUID from that machine's enrollment. Keep the token out
of command-line arguments and shell history. The utility saves its own private
credential file during setup; the temporary input file can then be removed.

Preview both queues first. Replace the example directory, domain, UUID and
token-file path with the values for this machine:

```sh
qf-sync preview "/path/to/queue-focus" https://sync.example.com DEVICE_UUID "/private/device-token.json"
```

On the first device, initialize the shared queue from its current local queue.
This preserves the selected Now task and the elapsed/paused timer captured at
setup:

```sh
qf-sync setup "/path/to/queue-focus" https://sync.example.com DEVICE_UUID "/private/device-token.json" initialize
```

On the second device, import its tasks:

```sh
qf-sync setup "/path/to/queue-focus" https://sync.example.com DEVICE_UUID "/private/device-token.json" import
```

Import appends every local task using new global identities. Local Now goes to
Next and does not replace the shared current task. Equal titles are separate
tasks. The explicit `shared` mode instead uses only the existing shared queue
and retains the original local tasks in a backup.

Setup saves an exact local backup and a setup record before contacting the
server. It saves command identities before mutations. If interrupted, keep
the directory intact and run:

```sh
qf-sync resume "/path/to/queue-focus"
```

Repeating the original setup command with the same choices also resumes it.
Do not delete the setup record to retry. A lost acknowledgement is retried
under the original command ID, preventing duplicate imports. Activation writes
the sync envelope before replacing `tasks.json` with a sentinel that old apps
refuse to parse. The backup remains after activation.

If setup cannot continue because the server was already initialized under a
different choice, or the combined queues exceed 2,000 tasks, export to a new
directory and review the queue before starting setup there.

## Inspect or repair

`status` reads the durable journal and can be used while the app is running:

```sh
qf-sync status "/path/to/queue-focus"
```

It shows pending and recoverable commands and any suspension. Current
connection status belongs to the running app.

After an operator rotates this device's token, quit the app and install the
replacement using another private token JSON file:

```sh
qf-sync credentials "/path/to/queue-focus" "/private/replacement-token.json"
```

Rotation must preserve the device UUID. Do not copy another machine's
credentials or sync envelope. The server checks the token's device before
executing commands.

After repairing a server/configuration problem, `qf-sync retry DIR` clears a
repairable suspension without removing pending commands. Restart the app to
retry. A history mismatch requires recovery into a new directory and cannot
be bypassed by `retry`.

Rejected edits remain as recoverable intent. After reviewing/exporting one,
`qf-sync dismiss DIR COMMAND_UUID` explicitly removes that recovery item.
It never removes a pending command whose outcome is unknown.

## Recover or leave sync

Export to a **new** directory:

```sh
qf-sync export "/path/to/queue-focus" "/path/to/recovered-queue"
```

The new directory contains standalone tasks/settings and
`sync-intents-export.json`, with pending/rejected intent and sync identities
but no credentials. The source installation remains intact. This also works
for an interrupted setup and when the server is permanently unavailable.
An interrupted export retains a marker and must not be opened as a completed
standalone queue; export again into a different new directory.

The exported task view includes replayable queued edits. An uncertain focus
command is recorded separately: its server outcome may still be unknown.
Review the intent record and the shared queue before importing into a recovered
dataset, especially after a server restore. Export is not an automatic merge.

Never disable sync by deleting credentials or copying an old `tasks.json`
over the active directory. Use the new standalone export or set up the new
directory against the recovered server.

For automated local tests only, `preview` and `setup` accept
`--allow-loopback-http` with a literal `127.0.0.1` or `::1` origin. Other
connections require HTTPS. Redirects are refused.
