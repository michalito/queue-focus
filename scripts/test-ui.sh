#!/usr/bin/env bash
# Exercise the real GTK wiring without desktop services or user data.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
mkdir -p "$test_dir/home" "$test_dir/config" "$test_dir/data" "$test_dir/cache" "$test_dir/runtime"
chmod 700 "$test_dir/runtime"
cat > "$test_dir/bus.conf" <<CONF
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=$test_dir</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONF
# Keep the toolchain available after isolating HOME.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export HOME="$test_dir/home" XDG_CONFIG_HOME="$test_dir/config"
export XDG_DATA_HOME="$test_dir/data" XDG_CACHE_HOME="$test_dir/cache"
export XDG_RUNTIME_DIR="$test_dir/runtime"
export GIO_USE_VFS=local GTK_A11Y=none GDK_BACKEND=x11 GSK_RENDERER=cairo
export GSETTINGS_BACKEND=memory
xvfb-run -a dbus-run-session --config-file="$test_dir/bus.conf" -- \
  scripts/cargo test -p queue-focus ui::gtk_tests -- --ignored --test-threads=1
