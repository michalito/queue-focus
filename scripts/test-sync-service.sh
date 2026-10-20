#!/usr/bin/env bash
# Real native hosts, disposable databases and buses; no user's data is opened.
set -euo pipefail
readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
for tool in xvfb-run dbus-daemon gdbus python3; do
  command -v "$tool" >/dev/null || { echo "missing test dependency: $tool" >&2; exit 1; }
done
scripts/cargo build --locked -p queue-focus -p qf-sync -p qf-sync-server
xvfb-run -a python3 scripts/test-sync-service.py
