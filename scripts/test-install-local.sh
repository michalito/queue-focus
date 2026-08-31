#!/usr/bin/env bash
# Isolated integration coverage for the per-user installer. This uses a fixture
# checkout and fake platform commands, so it never touches the caller's GNOME
# settings or home directory.
set -euo pipefail

readonly SOURCE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/queue-focus install test.XXXXXX")
readonly TEST_ROOT

cleanup() {
  rm -rf -- "$TEST_ROOT"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

fail() {
  echo "installer test: $*" >&2
  exit 1
}

assert_file() {
  [ -f "$1" ] || fail "expected file: $1"
}

assert_absent() {
  [ ! -e "$1" ] && [ ! -L "$1" ] || fail "expected path to be absent: $1"
}

assert_line() {
  grep -Fqx -- "$2" "$1" || fail "expected '$2' in $1"
}

readonly FIXTURE="$TEST_ROOT/checkout with spaces"
readonly TEST_HOME="$TEST_ROOT/home with spaces "'$cash `tick` "quote" back\slash'
readonly TEST_DATA="$TEST_ROOT/xdg data"
readonly MOCK_BIN="$TEST_ROOT/mock bin"
readonly COMMAND_LOG="$TEST_ROOT/commands.log"
mkdir -p "$FIXTURE/scripts" "$TEST_HOME" "$TEST_DATA" "$MOCK_BIN"
cp -a "$SOURCE_ROOT/data" "$SOURCE_ROOT/extension" "$FIXTURE/"
cp "$SOURCE_ROOT/scripts/install-local.sh" "$SOURCE_ROOT/scripts/queue-focus-setup" \
  "$FIXTURE/scripts/"
chmod 0755 "$FIXTURE/scripts/install-local.sh" "$FIXTURE/scripts/queue-focus-setup"

# The fixture's Cargo wrapper produces a tiny stand-in executable. The real
# build is covered separately by `make build`; this test focuses on install
# semantics and remains quick.
cat >"$FIXTURE/scripts/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" = "--version" ]; then
  echo "cargo 1.80.0 (installer test)"
  exit 0
fi
if [ "${QF_TEST_BUILD_FAIL:-0}" = "1" ]; then
  echo "intentional build failure" >&2
  exit 42
fi
mkdir -p target/release
cat >target/release/queue-focus <<'APP'
#!/usr/bin/env bash
[ -z "${QF_TEST_APP_LOG:-}" ] || printf '%s\n' "$*" >>"$QF_TEST_APP_LOG"
case "${1:-}" in
  version) echo "queue-focus 0.1.0-test" ;;
  quit) exit 0 ;;
  *) exit 0 ;;
esac
APP
chmod 0755 target/release/queue-focus
EOF
chmod 0755 "$FIXTURE/scripts/cargo"
ln -s "$FIXTURE/scripts/cargo" "$MOCK_BIN/cargo"

cat >"$MOCK_BIN/rustc" <<'EOF'
#!/usr/bin/env bash
echo "rustc 1.80.0 (installer test)"
EOF

cat >"$MOCK_BIN/gsettings" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'gsettings' >>"$QF_TEST_COMMAND_LOG"
printf ' %q' "$@" >>"$QF_TEST_COMMAND_LOG"
printf '\n' >>"$QF_TEST_COMMAND_LOG"
if [ "${1:-}" = "--schemadir" ]; then
  shift 2
fi
case "${1:-}:${2:-}:${3:-}" in
  get:org.gnome.shell:disable-user-extensions)
    [ "${QF_TEST_SAFETY_SWITCH:-0}" = "1" ] && echo true || echo false
    ;;
  get:org.gnome.shell:enabled-extensions|get:org.gnome.shell:disabled-extensions)
    echo "@as []"
    ;;
  get:org.gnome.shell.extensions.queue-focus:*)
    echo "['<Super>q']"
    ;;
  set:*) ;;
  *) echo "unexpected gsettings call: $*" >&2; exit 2 ;;
esac
EOF

cat >"$MOCK_BIN/gnome-extensions" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'gnome-extensions' >>"$QF_TEST_COMMAND_LOG"
printf ' %q' "$@" >>"$QF_TEST_COMMAND_LOG"
printf '\n' >>"$QF_TEST_COMMAND_LOG"
[ "${1:-}" != "info" ]
EOF

cat >"$MOCK_BIN/mv" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [ "${QF_TEST_FAIL_EXTENSION_SWAP:-0}" = "1" ] \
    && [[ " $* " = *".queue-focus@queuefocus.org.new."* ]]; then
  echo "intentional extension swap failure" >&2
  exit 43
fi
exec /usr/bin/mv "$@"
EOF

for command in update-desktop-database gtk-update-icon-cache; do
  cat >"$MOCK_BIN/$command" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s' "${0##*/}" >>"$QF_TEST_COMMAND_LOG"
printf ' %q' "$@" >>"$QF_TEST_COMMAND_LOG"
printf '\n' >>"$QF_TEST_COMMAND_LOG"
EOF
done
chmod 0755 "$MOCK_BIN"/*

run_installer() {
  env \
    HOME="$TEST_HOME" \
    XDG_DATA_HOME="$TEST_DATA" \
    CARGO_HOME="$TEST_ROOT/cargo home" \
    QF_TEST_COMMAND_LOG="$COMMAND_LOG" \
    PATH="$MOCK_BIN:$PATH" \
    "$FIXTURE/scripts/install-local.sh" "$@"
}

install_output=$(run_installer 2>&1)
readonly INSTALLED_BIN="$TEST_HOME/.local/bin/queue-focus"
readonly INSTALLED_SETUP="$TEST_HOME/.local/bin/queue-focus-setup"
readonly SERVICE="$TEST_DATA/dbus-1/services/org.queuefocus.QueueFocus.service"
readonly DESKTOP="$TEST_DATA/applications/org.queuefocus.QueueFocus.desktop"
readonly INSTALLED_EXTENSION="$TEST_DATA/gnome-shell/extensions/queue-focus@queuefocus.org"
SERVICE_EXEC_PATH=${INSTALLED_BIN//\\/\\\\\\\\}
SERVICE_EXEC_PATH=${SERVICE_EXEC_PATH//\"/\\\\\"}
DESKTOP_EXEC_PATH=${SERVICE_EXEC_PATH//\$/\\\\$}
DESKTOP_EXEC_PATH=${DESKTOP_EXEC_PATH//\`/\\\\\`}
readonly SERVICE_EXEC_PATH DESKTOP_EXEC_PATH

assert_file "$INSTALLED_BIN"
assert_file "$INSTALLED_SETUP"
[ ! -L "$INSTALLED_BIN" ] || fail "installed binary must not be a symlink"
[ ! -L "$INSTALLED_SETUP" ] || fail "installed setup helper must not be a symlink"
[ "$(stat -c '%a' "$INSTALLED_BIN")" = "755" ] || fail "installed binary mode is not 755"
assert_line "$SERVICE" "Exec=\"$SERVICE_EXEC_PATH\" service"
assert_line "$DESKTOP" "Exec=\"$DESKTOP_EXEC_PATH\" toggle"
assert_file "$INSTALLED_EXTENSION/schemas/gschemas.compiled"
grep -Fq "installed queue-focus 0.1.0-test" <<<"$install_output" || \
  fail "installer did not report the installed version"
if grep -Fq "$FIXTURE" "$SERVICE" "$DESKTOP"; then
  fail "installed launch metadata still refers to the checkout"
fi

# Ask a fresh session bus to activate the service. The stand-in executable does
# not claim the bus name, so the call itself fails; its log proves that D-Bus
# parsed and launched an Exec path containing spaces correctly.
if command -v dbus-run-session >/dev/null 2>&1 \
    && command -v gdbus >/dev/null 2>&1 \
    && command -v timeout >/dev/null 2>&1; then
  readonly APP_LOG="$TEST_ROOT/app.log"
  activation_output=$(env XDG_DATA_HOME="$TEST_DATA" QF_TEST_APP_LOG="$APP_LOG" dbus-run-session -- \
    timeout 2s gdbus call --session \
      --dest org.freedesktop.DBus \
      --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.StartServiceByName \
      org.queuefocus.QueueFocus 0 2>&1 || true)
  [ -f "$APP_LOG" ] || fail "D-Bus did not launch the quoted Exec path: $activation_output"
  assert_line "$APP_LOG" "service"
fi

# Moving the checkout and deleting its build output must not break either
# installed executable.
readonly MOVED_FIXTURE="$FIXTURE moved"
mv "$FIXTURE" "$MOVED_FIXTURE"
[ "$("$INSTALLED_BIN" version)" = "queue-focus 0.1.0-test" ] || \
  fail "installed binary depends on the checkout"
env HOME="$TEST_HOME" XDG_DATA_HOME="$TEST_DATA" QF_TEST_COMMAND_LOG="$COMMAND_LOG" \
  PATH="$MOCK_BIN:$PATH" "$INSTALLED_SETUP" keys >/dev/null
mv "$MOVED_FIXTURE" "$FIXTURE"
rm -f "$FIXTURE/target/release/queue-focus"
[ "$("$INSTALLED_BIN" version)" = "queue-focus 0.1.0-test" ] || \
  fail "removing the build output broke the installed binary"

# A failed extension swap restores the previous directory. A successful
# reinstall then replaces the extension as a unit without stale files.
touch "$INSTALLED_EXTENSION/stale-file"
if QF_TEST_FAIL_EXTENSION_SWAP=1 run_installer >/dev/null 2>&1; then
  fail "intentional extension swap failure unexpectedly succeeded"
fi
assert_file "$INSTALLED_EXTENSION/stale-file"
run_installer >/dev/null 2>&1
assert_absent "$INSTALLED_EXTENSION/stale-file"

# Uninstall removes integration files but deliberately preserves task data.
mkdir -p "$TEST_DATA/queue-focus"
printf '{"kept":true}\n' >"$TEST_DATA/queue-focus/tasks.json"
run_installer --uninstall >/dev/null 2>&1
assert_absent "$INSTALLED_BIN"
assert_absent "$INSTALLED_SETUP"
assert_absent "$SERVICE"
assert_absent "$DESKTOP"
assert_absent "$INSTALLED_EXTENSION"
assert_file "$TEST_DATA/queue-focus/tasks.json"

# A build failure happens before any destination is mutated.
readonly FAILED_HOME="$TEST_ROOT/failed home"
readonly FAILED_DATA="$TEST_ROOT/failed data"
if env HOME="$FAILED_HOME" XDG_DATA_HOME="$FAILED_DATA" CARGO_HOME="$TEST_ROOT/cargo home" \
    QF_TEST_BUILD_FAIL=1 QF_TEST_COMMAND_LOG="$COMMAND_LOG" PATH="$MOCK_BIN:$PATH" \
    "$FIXTURE/scripts/install-local.sh" >/dev/null 2>&1; then
  fail "intentional build failure unexpectedly succeeded"
fi
assert_absent "$FAILED_HOME/.local/bin/queue-focus"
assert_absent "$FAILED_DATA/dbus-1/services/org.queuefocus.QueueFocus.service"

# GNOME's safety switch is a successful app installation with an explicit
# manual-action warning, not an ambiguous partial-install failure.
readonly SAFETY_HOME="$TEST_ROOT/safety home"
readonly SAFETY_DATA="$TEST_ROOT/safety data"
safety_output=$(env HOME="$SAFETY_HOME" XDG_DATA_HOME="$SAFETY_DATA" \
  CARGO_HOME="$TEST_ROOT/cargo home" QF_TEST_SAFETY_SWITCH=1 \
  QF_TEST_COMMAND_LOG="$COMMAND_LOG" PATH="$MOCK_BIN:$PATH" \
  "$FIXTURE/scripts/install-local.sh" 2>&1)
assert_file "$SAFETY_HOME/.local/bin/queue-focus"
grep -Fq "app is installed, but the GNOME extension was not enabled" <<<"$safety_output" || \
  fail "safety-switch outcome was not explained"
env HOME="$SAFETY_HOME" XDG_DATA_HOME="$SAFETY_DATA" QF_TEST_SAFETY_SWITCH=1 \
  QF_TEST_COMMAND_LOG="$COMMAND_LOG" PATH="$MOCK_BIN:$PATH" \
  "$SAFETY_HOME/.local/bin/queue-focus-setup" enable --allow-user-extensions >/dev/null
grep -Fq "set org.gnome.shell disable-user-extensions false" "$COMMAND_LOG" || \
  fail "explicit safety-switch override was not applied"
env HOME="$SAFETY_HOME" XDG_DATA_HOME="$SAFETY_DATA" QF_TEST_COMMAND_LOG="$COMMAND_LOG" \
  PATH="$MOCK_BIN:$PATH" "$FIXTURE/scripts/install-local.sh" --uninstall >/dev/null 2>&1

echo "installer integration tests passed"
