#!/usr/bin/env bash
# Run the shipped extension and real GNOME actors in an isolated headless shell.
set -euo pipefail
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
for tool in gnome-shell dbus-run-session gsettings glib-compile-schemas python3; do
  command -v "$tool" >/dev/null || { echo "missing test dependency: $tool" >&2; exit 1; }
done
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/qf-shell-test.XXXXXX")
trap 'rm -rf -- "$TEST_ROOT"' EXIT
export XDG_DATA_HOME="$TEST_ROOT/data" XDG_CONFIG_HOME="$TEST_ROOT/config"
export XDG_CACHE_HOME="$TEST_ROOT/cache" XDG_STATE_HOME="$TEST_ROOT/state"
export XDG_RUNTIME_DIR="$TEST_ROOT/runtime" GSETTINGS_BACKEND=keyfile
export XDG_DATA_DIRS=/usr/local/share:/usr/share
export GIO_USE_VFS=local LIBGL_ALWAYS_SOFTWARE=1 GNOME_SHELL_SESSION_MODE=user
export QF_SHELL_TEST_RESULT="$TEST_ROOT/result.json"
unset DISPLAY WAYLAND_DISPLAY SESSION_MANAGER GNOME_DESKTOP_SESSION_ID
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"
chmod 0700 "$XDG_RUNTIME_DIR"
UUID=queue-focus@queuefocus.org
DRIVER=qf-shell-test@local
EXTENSIONS="$XDG_DATA_HOME/gnome-shell/extensions"
mkdir -p "$EXTENSIONS/$DRIVER"
cp -a "$ROOT/extension/$UUID" "$EXTENSIONS/$UUID"
glib-compile-schemas --strict "$EXTENSIONS/$UUID/schemas"
cp "$ROOT/extension/test/shell-driver.js" "$EXTENSIONS/$DRIVER/extension.js"
python3 - "$EXTENSIONS/$UUID/metadata.json" "$EXTENSIONS/$DRIVER/metadata.json" <<'PY'
import json, sys
source = json.load(open(sys.argv[1]))
with open(sys.argv[2], 'w') as out:
    json.dump({'uuid': 'qf-shell-test@local', 'name': 'Queue Focus isolated test',
               'description': 'Test fixture and actor checks', 'shell-version': source['shell-version']}, out)
PY
# No activation directories: this bus cannot start the installed Queue Focus
# app (or any other installed session daemon) and cannot read the user's queue.
cat > "$TEST_ROOT/bus.conf" <<EOF_BUS
<busconfig><type>session</type><listen>unix:tmpdir=$XDG_RUNTIME_DIR</listen>
<auth>EXTERNAL</auth><policy context="default">
<allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/>
</policy></busconfig>
EOF_BUS
gsettings set org.gnome.shell enabled-extensions "['$DRIVER']"
gsettings set org.gnome.shell disable-user-extensions false
export QF_SHELL_TEST_LOG="$TEST_ROOT/shell.log"
# Set QF_SHELL_TEST_SHOTS to a directory to keep PNGs of the menu from the run.
if [ -n "${QF_SHELL_TEST_SHOTS:-}" ]; then mkdir -p -- "$QF_SHELL_TEST_SHOTS"; fi
dbus-run-session --config-file="$TEST_ROOT/bus.conf" -- bash -c '
  ulimit -c 0
  export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"
  gnome-shell --headless --wayland --no-x11 --virtual-monitor=1280x720 >"$QF_SHELL_TEST_LOG" 2>&1 &
  shell_pid=$!
  trap '\''kill "$shell_pid" 2>/dev/null || true; wait "$shell_pid" 2>/dev/null || true'\'' EXIT
  for ((attempt=0; attempt<600; attempt++)); do
    [ -f "$QF_SHELL_TEST_RESULT" ] && exit 0
    if ! kill -0 "$shell_pid" 2>/dev/null; then cat "$QF_SHELL_TEST_LOG"; exit 1; fi
    sleep 0.1
  done
  cat "$QF_SHELL_TEST_LOG"
  echo "GNOME test timed out" >&2
  exit 1
'
python3 - "$QF_SHELL_TEST_RESULT" "$QF_SHELL_TEST_LOG" <<'PY'
import json, sys
result = json.load(open(sys.argv[1]))
log = open(sys.argv[2]).read()
# A stale callback can log a disposed-actor error without failing its caller.
# Missing desktop daemons are expected on this bus; extension stack traces are not.
if any('/queue-focus@queuefocus.org/' in line and '.js:' in line for line in log.splitlines()):
    print(log, file=sys.stderr)
    raise SystemExit('GNOME logged an error from the shipped extension')
if not result['ok']:
    print(log, file=sys.stderr)
    raise SystemExit(result.get('stack', result['error']))
print('GNOME Shell integration passed: quick-add draft preservation, late reply, disable/re-enable, '
      'clock pill pausing without opening the menu, focus card, Side cards, undo, one task in Now, views')
PY
