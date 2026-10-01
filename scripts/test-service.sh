#!/usr/bin/env bash
# Drive the real service binary over D-Bus and from the command line, on a
# private bus, display and data directory. Never touches the user's queue.
#
# It waits for one real flash on a one-minute interval, so it takes a little
# over a minute.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ "${QF_SERVICE_TEST_INNER:-}" != 1 ]; then
  for tool in xvfb-run dbus-run-session gdbus python3 cc; do
    command -v "$tool" >/dev/null || { echo "missing test dependency: $tool" >&2; exit 1; }
  done
  scripts/cargo build -p queue-focus
  TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/qf-service-test.XXXXXX")
  trap 'rm -rf -- "$TEST_ROOT"' EXIT
  mkdir -p "$TEST_ROOT/runtime" && chmod 0700 "$TEST_ROOT/runtime"
  # No activation directories: this bus cannot start an installed service.
  cat >"$TEST_ROOT/bus.conf" <<EOF
<busconfig><type>session</type><listen>unix:tmpdir=$TEST_ROOT/runtime</listen>
<auth>EXTERNAL</auth><policy context="default">
<allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/>
</policy></busconfig>
EOF
  # Fails fsync on a directory while the named file exists, so the service
  # commits a change it cannot make crash-safe.
  cat >"$TEST_ROOT/fail-dir-sync.c" <<'EOF'
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <unistd.h>

int fsync(int fd) {
    static int (*real_fsync)(int);
    if (!real_fsync) real_fsync = (int (*)(int))dlsym(RTLD_NEXT, "fsync");
    const char *flag = getenv("QF_FAIL_DIR_SYNC");
    struct stat st;
    if (flag && access(flag, F_OK) == 0 && fstat(fd, &st) == 0 && S_ISDIR(st.st_mode)) {
        errno = EIO;
        return -1;
    }
    return real_fsync(fd);
}
EOF
  cc -shared -fPIC -o "$TEST_ROOT/fail-dir-sync.so" "$TEST_ROOT/fail-dir-sync.c" -ldl
  export QF_SERVICE_TEST_INNER=1 TEST_ROOT
  export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
  export HOME="$TEST_ROOT/home" XDG_DATA_HOME="$TEST_ROOT/data"
  export XDG_CONFIG_HOME="$TEST_ROOT/config" XDG_CACHE_HOME="$TEST_ROOT/cache"
  export XDG_RUNTIME_DIR="$TEST_ROOT/runtime"
  export GIO_USE_VFS=local GTK_A11Y=none GDK_BACKEND=x11 GSK_RENDERER=cairo
  export GSETTINGS_BACKEND=memory
  mkdir -p "$HOME"
  # Not exec: the cleanup trap above has to run once the inner run is over.
  status=0
  xvfb-run -a dbus-run-session --config-file="$TEST_ROOT/bus.conf" -- "$0" || status=$?
  exit "$status"
fi

readonly BIN="$ROOT/target/debug/queue-focus"
readonly DATA="$XDG_DATA_HOME/queue-focus"
readonly NAME=org.queuefocus.QueueFocus
readonly OBJECT=/org/queuefocus/QueueFocus
readonly IFACE=org.queuefocus.QueueFocus1
readonly SIGNALS="$TEST_ROOT/signals.log"
readonly FAIL_DIR_SYNC="$TEST_ROOT/fail-dir-sync"

fail() {
  echo "service test: $*" >&2
  exit 1
}

call() {
  gdbus call --session --dest "$NAME" --object-path "$OBJECT" --method "$IFACE.$1" "${@:2}"
}

# `expect_reply METHOD EXPECTED ARGS...`: the call succeeds with this reply.
expect_reply() {
  local method=$1 expected=$2 got
  got=$(call "$method" "${@:3}" 2>&1) || fail "$method ${*:3} failed: $got"
  [ "$got" = "$expected" ] || fail "$method ${*:3}: expected $expected, got $got"
}

# `expect_error METHOD ERROR MESSAGE ARGS...`: the call fails with this error.
expect_error() {
  local method=$1 name=$2 message=$3 got
  if got=$(call "$method" "${@:4}" 2>&1); then
    fail "$method ${*:4} should have failed, replied $got"
  fi
  case $got in
    *"GDBus.Error:$name: $message"*) ;;
    *) fail "$method ${*:4}: expected $name: $message, got $got" ;;
  esac
}

# `state EXPRESSION`: evaluate a Python expression over GetState's JSON `s`.
state() {
  local json
  json=$(call GetState | python3 -c 'import ast, sys; print(ast.literal_eval(sys.stdin.read())[0])')
  python3 -c 'import json, sys; s = json.loads(sys.argv[1]); print(eval(sys.argv[2]))' "$json" "$1"
}

expect_state() {
  local got
  got=$(state "$1")
  [ "$got" = "$2" ] || fail "state $1: expected $2, got $got"
}

# `wait_for SECONDS DESCRIPTION COMMAND...`: poll until the command succeeds.
wait_for() {
  local tries=$(($1 * 10)) description=$2
  shift 2
  for _ in $(seq "$tries"); do
    "$@" && return 0
    sleep 0.1
  done
  fail "timed out: $description"
}

service_running() {
  gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.NameHasOwner "$NAME" 2>/dev/null | grep -q true
}

signal_count() {
  grep -c "$IFACE.$1 " "$SIGNALS" || true
}

signal_seen() {
  [ "$(signal_count "$1")" -gt 0 ]
}

# ---- the service refuses a task file it cannot read --------------------------

mkdir -p "$DATA"
printf '{ not a queue' >"$DATA/tasks.json"
if output=$("$BIN" service 2>&1); then
  fail "the service started over a malformed task file"
fi
case $output in
  *"refusing to start to protect the task file"*) ;;
  *) fail "unexpected refusal: $output" ;;
esac
[ "$(cat "$DATA/tasks.json")" = '{ not a queue' ] || fail "the malformed task file was changed"
rm "$DATA/tasks.json"

# ---- start it, with a settings file it cannot read ---------------------------

printf '[]' >"$DATA/settings.json"
LD_PRELOAD="$TEST_ROOT/fail-dir-sync.so" QF_FAIL_DIR_SYNC="$FAIL_DIR_SYNC" \
  "$BIN" service >"$TEST_ROOT/service.log" 2>&1 &
service_pid=$!
trap 'kill "$service_pid" 2>/dev/null || true' EXIT
wait_for 10 "the service owns its name" service_running
grep -q "using the default settings" "$TEST_ROOT/service.log" ||
  fail "an unreadable settings file was not reported"
gdbus monitor --session --dest "$NAME" --object-path "$OBJECT" >"$SIGNALS" 2>&1 &
monitor_pid=$!
trap 'kill "$service_pid" "$monitor_pid" 2>/dev/null || true' EXIT
sleep 0.5

expect_state 's["current"]' None

# ---- the windows ------------------------------------------------------------------

for view in queue board settings add toggle nonsense; do
  expect_reply Show '()' "$view"
done
expect_reply Hide '()'

# ---- adding -------------------------------------------------------------------

expect_reply Add '(uint64 1,)' 'fix login #w' ''
expect_reply Add '(uint64 2,)' '!ship it' ''
expect_reply Add '(uint64 3,)' 'call mum' 'later'
expect_reply Add '(uint64 4,)' 'marker wins @side' 'later'
expect_error Add org.queuefocus.Error.InvalidArgs 'empty title' '#w @next' ''
expect_error Add org.queuefocus.Error.InvalidArgs 'task text is too long' \
  "$(python3 -c 'print("x" * 4097)')" ''
expect_state '[t["id"] for t in s["next"]]' '[1]'
expect_state 's["next"][0]["tag"]' 'work'
expect_state 's["current"]["title"]' 'ship it'
expect_state '[t["id"] for t in s["later"]]' '[3]'
expect_state '[t["id"] for t in s["side"]]' '[4]'
wait_for 10 "Changed is broadcast" signal_seen Changed

# ---- completing and undoing ---------------------------------------------------

expect_reply CompleteCurrent "(uint64 2, 'ship it')"
expect_state 's["current"]["id"]' 1
expect_reply UndoComplete '(false,)' 'uint64 1'
expect_reply UndoComplete '(true,)' 'uint64 2'
expect_state 's["current"]["id"]' 2
expect_state '[t["id"] for t in s["next"]]' '[1]'
expect_reply UndoComplete '(false,)' 'uint64 2'
expect_reply Complete '()' 'uint64 4'
expect_error Complete org.queuefocus.Error.InvalidArgs 'no such task' 'uint64 99'
expect_reply UndoComplete '(true,)' 'uint64 4'
expect_state '[t["id"] for t in s["side"]]' '[4]'

# ---- moving, tagging, pausing -------------------------------------------------

expect_reply Move '()' 'uint64 1' 'later' 'int32 -1'
expect_state '[t["id"] for t in s["later"]]' '[3, 1]'
expect_reply Move '()' 'uint64 1' 'l' 'int32 0'
expect_state '[t["id"] for t in s["later"]]' '[1, 3]'
expect_error Move org.queuefocus.Error.InvalidArgs 'bad bucket' 'uint64 1' 'bogus' 'int32 0'
expect_error Move org.queuefocus.Error.InvalidArgs 'no such task' 'uint64 99' 'next' 'int32 0'
expect_reply Promote '()' 'uint64 3'
expect_state 's["current"]["id"]' 3
expect_state '[t["id"] for t in s["next"]]' '[2]'
expect_error Promote org.queuefocus.Error.InvalidArgs 'no such task' 'uint64 99'
expect_reply SetTag '()' 'uint64 3' 'p'
expect_state 's["current"]["tag"]' 'personal'
expect_reply SetTag '()' 'uint64 3' ''
expect_state 's["current"]["tag"]' None
expect_error SetTag org.queuefocus.Error.InvalidArgs 'bad tag' 'uint64 3' 'x'
expect_reply TogglePause '(true,)'
expect_state 's["current"]["paused_at"] is not None' True
expect_reply TogglePause '(true,)'
expect_state 's["current"]["paused_at"]' None
expect_reply Remove '()' 'uint64 4'
expect_error Remove org.queuefocus.Error.InvalidArgs 'no such task' 'uint64 4'

# ---- settings -----------------------------------------------------------------

# A one-minute interval: the first flash is due a minute from here.
settings=$(call SetSettings '{"interval_min": 1, "default_bucket": "side"}')
case $settings in
  *'"interval_min":1'*'"default_bucket":"side"'*) ;;
  *) fail "SetSettings replied $settings" ;;
esac
expect_error SetSettings org.queuefocus.Error.InvalidArgs 'unknown setting: nope' '{"nope": 1}'
expect_error SetSettings org.queuefocus.Error.InvalidArgs 'settings patch is too long' \
  "$(python3 -c 'print("{" + " " * 4096 + "}")')"
case $(call GetSettings) in
  *'"interval_min":1'*) ;;
  *) fail "GetSettings lost the change" ;;
esac
wait_for 10 "SettingsChanged is broadcast" signal_seen SettingsChanged
settings_written() {
  python3 -c 'import json, sys; sys.exit(json.load(open(sys.argv[1]))["interval_min"] != 1)' \
    "$DATA/settings.json" 2>/dev/null
}
wait_for 10 "the settings reach the file" settings_written
# The chosen bucket is where an unmarked task goes.
expect_reply Add '(uint64 5,)' 'beside it' ''
expect_state '[t["id"] for t in s["side"]]' '[5]'

# ---- the command line -----------------------------------------------------------

[ "$("$BIN" add 'from the cli @next')" = 'added #6' ] || fail "queue-focus add"
[ "$("$BIN" done)" = 'done: call mum' ] || fail "queue-focus done"
expect_state 's["current"]["id"]' 2
status=$("$BIN" status)
python3 -c 'import json, sys; assert json.loads(sys.argv[1])["current"]["id"] == 2' "$status" ||
  fail "queue-focus status printed $status"

# ---- a change that commits but may not survive a crash --------------------------

changed=$(signal_count Changed)
touch "$FAIL_DIR_SYNC"
expect_reply Add '(uint64 7,)' 'not yet crash-safe' 'later'
rm "$FAIL_DIR_SYNC"
wait_for 10 "DurabilityWarning is broadcast" signal_seen DurabilityWarning
grep "$IFACE.DurabilityWarning " "$SIGNALS" | grep -q "could not make the change crash-safe" ||
  fail "the durability warning does not say what happened"
[ "$(signal_count Changed)" -gt "$changed" ] || fail "a committed change was not broadcast"
expect_state '[t["id"] for t in s["later"]]' '[1, 7]'

# ---- a change that cannot be saved ----------------------------------------------

mv "$DATA/tasks.json" "$TEST_ROOT/tasks.json.saved"
mkdir "$DATA/tasks.json"
expect_error Add org.queuefocus.Error.Persistence 'could not save' 'lost' ''
expect_state '"lost" in [t["title"] for t in s["next"]]' False
rmdir "$DATA/tasks.json"
mv "$TEST_ROOT/tasks.json.saved" "$DATA/tasks.json"

# ---- the reminder -----------------------------------------------------------------

wait_for 75 "a flash is broadcast" signal_seen Flash
flash=$(grep "$IFACE.Flash " "$SIGNALS" | head -n 1)
python3 - "$flash" <<'PY' || fail "unexpected flash: $flash"
import ast, json, re, sys
line = sys.argv[1]
payload = json.loads(ast.literal_eval(line[line.index("("):])[0])
assert set(payload) == {"style", "intensity", "palette", "title", "timer"}, payload
assert payload["style"] in {"wash", "wash2", "edges", "edgesSoft", "topbar", "topbarBeam"}, payload
assert payload["intensity"] == "normal", payload
assert payload["palette"] == "blue", payload
assert payload["title"] == "ship it", payload
assert re.fullmatch(r"\d+m", payload["timer"]), payload
PY

# ---- stopping -------------------------------------------------------------------

"$BIN" quit
wait_for 10 "the service exits" bash -c "! kill -0 $service_pid 2>/dev/null"
signal_seen Stopping || fail "Stopping was not broadcast"
python3 - "$DATA/tasks.json" <<'PY' || fail "the task file does not hold the queue"
import json, sys
stored = json.load(open(sys.argv[1]))
titles = sorted(t["title"] for t in stored["tasks"])
assert titles == ["beside it", "fix login", "from the cli", "not yet crash-safe", "ship it"], titles
PY
echo "service integration passed: every D-Bus method, its errors and signals, the windows," \
  "a flash, the command line, a durability warning, a failed save, and shutdown"
