#!/usr/bin/env bash
# Build and install Queue Focus for the current user. No root access required.
#
# The installed application is self-contained: binaries are copied into
# ~/.local/bin and integration files refer to those copies, not this checkout.
set -euo pipefail

readonly UUID="queue-focus@queuefocus.org"
readonly APPID="org.queuefocus.QueueFocus"
readonly MANUAL_ENABLE_STATUS=10

die() {
  echo "queue-focus install: $*" >&2
  exit 1
}

warn() {
  echo "queue-focus install: warning: $*" >&2
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command '$1' was not found${2:+ ($2)}"
}

data_home() {
  if [ -n "${XDG_DATA_HOME:-}" ] && [[ "$XDG_DATA_HOME" = /* ]]; then
    printf '%s\n' "$XDG_DATA_HOME"
  else
    printf '%s\n' "$HOME/.local/share"
  fi
}

quote_desktop_exec_arg() {
  local value=$1
  case "$value" in
    *$'\n'*|*$'\r'*) die "installation paths may not contain newlines" ;;
  esac
  python3 - "$value" <<'PY'
import sys

value = sys.argv[1]
escaped = []
for character in value:
    if character == "\\":
        escaped.append("\\\\\\\\")
    elif character in {'"', '`', '$'}:
        escaped.append("\\\\" + character)
    else:
        escaped.append(character)
print('"' + ''.join(escaped) + '"', end='')
PY
}

# D-Bus service files use key-file escaping, where \$ and \` are invalid even
# though desktop Exec fields require them. D-Bus does not invoke a shell, so
# those characters are already literal here.
quote_service_exec_arg() {
  local value=$1
  case "$value" in
    *$'\n'*|*$'\r'*) die "installation paths may not contain newlines" ;;
  esac
  python3 - "$value" <<'PY'
import sys

value = sys.argv[1]
escaped = []
for character in value:
    if character == "\\":
        escaped.append("\\\\\\\\")
    elif character == '"':
        escaped.append("\\\\\"")
    else:
        escaped.append(character)
print('"' + ''.join(escaped) + '"', end='')
PY
}

atomic_copy() {
  local source=$1 destination=$2 mode=$3 directory temporary
  directory=${destination%/*}
  temporary=$(mktemp "$directory/.${destination##*/}.new.XXXXXX")
  if install -m "$mode" "$source" "$temporary" && mv -f -- "$temporary" "$destination"; then
    return 0
  fi
  rm -f -- "$temporary"
  return 1
}

STAGING_ROOT=""
EXTENSION_STAGE=""
EXTENSION_BACKUP=""

cleanup() {
  local status=$?
  if [ -n "$STAGING_ROOT" ] && [ -d "$STAGING_ROOT" ]; then
    rm -rf -- "$STAGING_ROOT"
  fi
  if [ -n "$EXTENSION_STAGE" ] && [ -d "$EXTENSION_STAGE" ]; then
    rm -rf -- "$EXTENSION_STAGE"
  fi
  if [ -n "$EXTENSION_BACKUP" ] && { [ -e "$EXTENSION_BACKUP" ] || [ -L "$EXTENSION_BACKUP" ]; }; then
    if ! { [ -e "$EXT_DIR" ] || [ -L "$EXT_DIR" ]; }; then
      mv -- "$EXTENSION_BACKUP" "$EXT_DIR" || \
        warn "could not restore the previous extension from $EXTENSION_BACKUP"
    else
      warn "the previous extension was retained at $EXTENSION_BACKUP"
    fi
  fi
  return "$status"
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

replace_extension() {
  local prepared_extension=$1

  EXTENSION_STAGE=$(mktemp -d "$EXTENSION_PARENT/.${UUID}.new.XXXXXX")
  if ! cp -a -- "$prepared_extension/." "$EXTENSION_STAGE/"; then
    return 1
  fi

  if [ -e "$EXT_DIR" ] || [ -L "$EXT_DIR" ]; then
    EXTENSION_BACKUP=$(mktemp -d "$EXTENSION_PARENT/.${UUID}.old.XXXXXX")
    if ! rmdir "$EXTENSION_BACKUP"; then
      EXTENSION_BACKUP=""
      return 1
    fi
    if ! mv -- "$EXT_DIR" "$EXTENSION_BACKUP"; then
      EXTENSION_BACKUP=""
      return 1
    fi
  fi

  if ! mv -- "$EXTENSION_STAGE" "$EXT_DIR"; then
    if [ -n "$EXTENSION_BACKUP" ]; then
      if mv -- "$EXTENSION_BACKUP" "$EXT_DIR"; then
        EXTENSION_BACKUP=""
      fi
    fi
    return 1
  fi
  EXTENSION_STAGE=""

  if [ -n "$EXTENSION_BACKUP" ]; then
    rm -rf -- "$EXTENSION_BACKUP"
    EXTENSION_BACKUP=""
  fi
}

if [ "$#" -gt 1 ] || { [ "$#" -eq 1 ] && [ "$1" != "--uninstall" ]; }; then
  echo "usage: scripts/install-local.sh [--uninstall]" >&2
  exit 2
fi

: "${HOME:?HOME must be set}"
[[ "$HOME" = /* ]] || die "HOME must be an absolute path"
(( EUID != 0 )) || die "do not run this per-user installer as root or with sudo"

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly BUILD_BIN="$ROOT/target/release/queue-focus"
readonly DATA_HOME="$(data_home)"
readonly BIN_DIR="$HOME/.local/bin"
readonly INSTALLED_BIN="$BIN_DIR/queue-focus"
readonly SETUP_BIN="$BIN_DIR/queue-focus-setup"
readonly EXTENSION_PARENT="$DATA_HOME/gnome-shell/extensions"
readonly EXT_DIR="$EXTENSION_PARENT/$UUID"
readonly SERVICE_DIR="$DATA_HOME/dbus-1/services"
readonly SVC="$SERVICE_DIR/$APPID.service"
readonly APPLICATION_DIR="$DATA_HOME/applications"
readonly DESKTOP="$APPLICATION_DIR/$APPID.desktop"
readonly ICONS="$DATA_HOME/icons/hicolor"
readonly TASKS="$DATA_HOME/queue-focus/tasks.json"

refresh_caches() {
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPLICATION_DIR" 2>/dev/null || \
      warn "could not refresh the desktop application cache"
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || \
      warn "could not refresh the icon cache"
  fi
}

if [ "${1:-}" = "--uninstall" ]; then
  if [ -x "$SETUP_BIN" ]; then
    "$SETUP_BIN" disable || warn "could not update GNOME's extension settings"
  elif [ -x "$ROOT/scripts/queue-focus-setup" ]; then
    "$ROOT/scripts/queue-focus-setup" disable || warn "could not update GNOME's extension settings"
  fi
  if [ -x "$INSTALLED_BIN" ]; then
    "$INSTALLED_BIN" quit 2>/dev/null || true
  fi

  rm -rf -- "$EXT_DIR"
  rm -f -- "$SVC" "$DESKTOP" "$INSTALLED_BIN" "$SETUP_BIN" \
    "$ICONS/scalable/apps/$APPID.svg" \
    "$ICONS/symbolic/apps/$APPID-symbolic.svg"
  refresh_caches
  echo "uninstalled Queue Focus (tasks kept at $TASKS)"
  exit 0
fi

for required in \
  "$ROOT/scripts/cargo" \
  "$ROOT/scripts/queue-focus-setup" \
  "$ROOT/data/$APPID.desktop" \
  "$ROOT/data/icons/hicolor/scalable/apps/$APPID.svg" \
  "$ROOT/data/icons/hicolor/symbolic/apps/$APPID-symbolic.svg" \
  "$ROOT/extension/$UUID/metadata.json" \
  "$ROOT/extension/$UUID/schemas/org.gnome.shell.extensions.queue-focus.gschema.xml"; do
  [ -e "$required" ] || die "required project file is missing: $required"
done

for command in bash chmod cp glib-compile-schemas gnome-extensions gsettings \
  install mkdir mktemp mv python3 rm rmdir; do
  require_command "$command"
done

rust_output=$(PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH" rustc --version) || \
  die "Rust is unavailable; install Rust 1.80 or newer from https://rustup.rs"
rust_version=${rust_output#rustc }
rust_version=${rust_version%% *}
IFS=. read -r rust_major rust_minor _ <<<"$rust_version"
if ! [[ "$rust_major" =~ ^[0-9]+$ && "$rust_minor" =~ ^[0-9]+$ ]]; then
  die "could not parse Rust version: $rust_output"
fi
if (( rust_major < 1 || (rust_major == 1 && rust_minor < 80) )); then
  die "Rust 1.80 or newer is required (found $rust_version)"
fi

if ! PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH" cargo --version >/dev/null 2>&1; then
  die "Cargo is unavailable; install Rust 1.80 or newer from https://rustup.rs"
fi

cd "$ROOT"
"$ROOT/scripts/cargo" build --release -p queue-focus
[ -x "$BUILD_BIN" ] || die "the release build did not produce $BUILD_BIN"
if ! LD_BIND_NOW=1 "$BUILD_BIN" version >/dev/null; then
  die "the built application cannot run with the installed GTK/libadwaita runtime"
fi

STAGING_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/queue-focus-install.XXXXXX")
readonly STAGED_EXTENSION="$STAGING_ROOT/extension"
readonly STAGED_SERVICE="$STAGING_ROOT/$APPID.service"
readonly STAGED_DESKTOP="$STAGING_ROOT/$APPID.desktop"
mkdir -p "$STAGED_EXTENSION"
cp -a -- "$ROOT/extension/$UUID/." "$STAGED_EXTENSION/"
glib-compile-schemas --strict "$STAGED_EXTENSION/schemas"
python3 -m json.tool "$STAGED_EXTENSION/metadata.json" >/dev/null
if command -v node >/dev/null 2>&1; then
  node --check "$STAGED_EXTENSION/extension.js"
fi
bash -n "$ROOT/scripts/queue-focus-setup"

quoted_service_binary=$(quote_service_exec_arg "$INSTALLED_BIN")
quoted_desktop_binary=$(quote_desktop_exec_arg "$INSTALLED_BIN")
printf '[D-BUS Service]\nName=%s\nExec=%s service\n' \
  "$APPID" "$quoted_service_binary" >"$STAGED_SERVICE"
while IFS= read -r line || [ -n "$line" ]; do
  if [[ "$line" = Exec=* ]]; then
    printf 'Exec=%s toggle\n' "$quoted_desktop_binary"
  else
    printf '%s\n' "$line"
  fi
done <"$ROOT/data/$APPID.desktop" >"$STAGED_DESKTOP"
if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$STAGED_DESKTOP"
fi

install -d -m 0755 "$BIN_DIR" "$EXTENSION_PARENT" "$SERVICE_DIR" "$APPLICATION_DIR" \
  "$ICONS/scalable/apps" "$ICONS/symbolic/apps"

atomic_copy "$BUILD_BIN" "$INSTALLED_BIN" 0755
atomic_copy "$ROOT/scripts/queue-focus-setup" "$SETUP_BIN" 0755
atomic_copy "$STAGED_SERVICE" "$SVC" 0644
atomic_copy "$STAGED_DESKTOP" "$DESKTOP" 0644
atomic_copy "$ROOT/data/icons/hicolor/scalable/apps/$APPID.svg" \
  "$ICONS/scalable/apps/$APPID.svg" 0644
atomic_copy "$ROOT/data/icons/hicolor/symbolic/apps/$APPID-symbolic.svg" \
  "$ICONS/symbolic/apps/$APPID-symbolic.svg" 0644
replace_extension "$STAGED_EXTENSION"

refresh_caches

# Hand the bus name to the newly installed binary: the new CLI asks whatever
# service is running to quit (every version understands that), then starts
# the installed copy. The extension refreshes when the new service appears.
"$INSTALLED_BIN" restart 2>/dev/null || true
installed_version=$("$INSTALLED_BIN" version 2>/dev/null || printf 'queue-focus')
echo "installed $installed_version at $INSTALLED_BIN"

setup_status=0
"$SETUP_BIN" enable || setup_status=$?
printf -v quoted_setup_bin '%q' "$SETUP_BIN"
case "$setup_status" in
  0) ;;
  "$MANUAL_ENABLE_STATUS")
    warn "the app is installed, but the GNOME extension was not enabled"
    warn "after reviewing your enabled extensions, run: $quoted_setup_bin enable --allow-user-extensions"
    ;;
  *)
    die "the app is installed, but GNOME extension setup failed (status $setup_status); retry with: $quoted_setup_bin enable"
    ;;
esac

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) warn "$BIN_DIR is not on PATH; add it to use queue-focus from a terminal" ;;
esac

echo "note: log out and back in to load new extension code on Wayland"
