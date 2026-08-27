#!/usr/bin/env bash
# Per-user install from this checkout (no root). Re-run to update.
#   scripts/install-local.sh              build + install + restart service
#   scripts/install-local.sh --uninstall
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UUID="queue-focus@queuefocus.org"
APPID="org.queuefocus.QueueFocus"
BIN="$ROOT/target/release/queue-focus"
SHARE="$HOME/.local/share"
EXT_DIR="$SHARE/gnome-shell/extensions/$UUID"
SVC="$SHARE/dbus-1/services/$APPID.service"
DESKTOP="$SHARE/applications/$APPID.desktop"
ICONS="$SHARE/icons/hicolor"

if [ "${1:-}" = "--uninstall" ]; then
  "$ROOT/scripts/queue-focus-setup" disable || true
  "$BIN" quit 2>/dev/null || true
  rm -rf "$EXT_DIR"
  rm -f "$SVC" "$DESKTOP" "$HOME/.local/bin/queue-focus" "$HOME/.local/bin/queue-focus-setup" \
        "$ICONS/scalable/apps/$APPID.svg" "$ICONS/symbolic/apps/$APPID-symbolic.svg"
  update-desktop-database "$SHARE/applications" 2>/dev/null || true
  echo "uninstalled (tasks kept in $SHARE/queue-focus)"
  exit 0
fi

cd "$ROOT"
scripts/cargo build --release -p queue-focus
glib-compile-schemas "extension/$UUID/schemas"

mkdir -p "$HOME/.local/bin" "$(dirname "$SVC")" "$(dirname "$DESKTOP")" "$(dirname "$EXT_DIR")" \
         "$ICONS/scalable/apps" "$ICONS/symbolic/apps"
ln -sf "$BIN" "$HOME/.local/bin/queue-focus"
ln -sf "$ROOT/scripts/queue-focus-setup" "$HOME/.local/bin/queue-focus-setup"
printf '[D-BUS Service]\nName=%s\nExec=%s service\n' "$APPID" "$BIN" > "$SVC"
sed "s|^Exec=.*|Exec=$BIN toggle|" "data/$APPID.desktop" > "$DESKTOP"
cp "data/icons/hicolor/scalable/apps/$APPID.svg" "$ICONS/scalable/apps/"
cp "data/icons/hicolor/symbolic/apps/$APPID-symbolic.svg" "$ICONS/symbolic/apps/"
rm -rf "$EXT_DIR" && cp -r "extension/$UUID" "$EXT_DIR"
update-desktop-database "$SHARE/applications" 2>/dev/null || true
gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || true

# Restart a running service so the new binary is used (the extension reconnects).
"$BIN" quit 2>/dev/null || true
echo "installed $("$BIN" version 2>/dev/null || echo queue-focus)"
"$ROOT/scripts/queue-focus-setup" enable
echo "note: extension code changes need a logout/login on Wayland"
