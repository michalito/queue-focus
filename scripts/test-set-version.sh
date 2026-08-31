#!/usr/bin/env bash
# Isolated integration coverage for scripts/set-version.
set -euo pipefail

readonly SOURCE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/queue-focus-version-test.XXXXXX")
readonly TEST_ROOT
readonly FIXTURE="$TEST_ROOT/repository"

cleanup() {
  rm -rf -- "$TEST_ROOT"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

fail() {
  echo "version test: $*" >&2
  exit 1
}

mkdir -p "$FIXTURE/scripts" \
  "$FIXTURE/extension/queue-focus@queuefocus.org"
cp "$SOURCE_ROOT/Cargo.toml" "$SOURCE_ROOT/Cargo.lock" "$FIXTURE/"
cp "$SOURCE_ROOT/Makefile" "$FIXTURE/"
cp "$SOURCE_ROOT/scripts/set-version" "$FIXTURE/scripts/"
cp "$SOURCE_ROOT/extension/queue-focus@queuefocus.org/metadata.json" \
  "$FIXTURE/extension/queue-focus@queuefocus.org/"
chmod 0755 "$FIXTURE/scripts/set-version"
cat >"$FIXTURE/scripts/install-local.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
: >"$QF_TEST_INSTALL_MARKER"
EOF
chmod 0755 "$FIXTURE/scripts/install-local.sh"

read -r original_version original_extension_version < <(
  python3 - "$FIXTURE" <<'PY'
import json
from pathlib import Path
import sys
import tomllib

root = Path(sys.argv[1])
cargo = tomllib.loads((root / "Cargo.toml").read_text())
metadata = json.loads(
    (root / "extension/queue-focus@queuefocus.org/metadata.json").read_text()
)
print(cargo["workspace"]["package"]["version"], metadata["version"])
PY
)

first_version="987.$$.1"
[ "$first_version" != "$original_version" ] || first_version="987.$$.2"
first_extension_version=$((original_extension_version + 1))
readonly INSTALL_MARKER="$TEST_ROOT/install-ran"
printf '%s\n' "$first_version" | QF_TEST_INSTALL_MARKER="$INSTALL_MARKER" \
  make --no-print-directory -s -C "$FIXTURE" install >/dev/null 2>&1
[ -f "$INSTALL_MARKER" ] || fail "make install did not run after setting VERSION"

python3 - "$FIXTURE" "$first_version" "$first_extension_version" <<'PY'
import json
from pathlib import Path
import sys
import tomllib

root = Path(sys.argv[1])
expected_version = sys.argv[2]
expected_extension_version = int(sys.argv[3])
cargo = tomllib.loads((root / "Cargo.toml").read_text())
lock = tomllib.loads((root / "Cargo.lock").read_text())
metadata = json.loads(
    (root / "extension/queue-focus@queuefocus.org/metadata.json").read_text()
)
assert cargo["workspace"]["package"]["version"] == expected_version
workspace = {
    package["name"]: package["version"]
    for package in lock["package"]
    if package["name"] in {"qf-core", "queue-focus"} and "source" not in package
}
assert workspace == {
    "qf-core": expected_version,
    "queue-focus": expected_version,
}
assert metadata["version"] == expected_extension_version
PY

# Reapplying the same semantic version is idempotent and does not consume a new
# GNOME extension revision.
same_output=$("$FIXTURE/scripts/set-version" "$first_version")
grep -Fq "version files already up to date" <<<"$same_output" || \
  fail "same-version update was not idempotent"

snapshot() {
  cp "$FIXTURE/Cargo.toml" "$TEST_ROOT/Cargo.toml.snapshot"
  cp "$FIXTURE/Cargo.lock" "$TEST_ROOT/Cargo.lock.snapshot"
  cp "$FIXTURE/extension/queue-focus@queuefocus.org/metadata.json" \
    "$TEST_ROOT/metadata.json.snapshot"
}

assert_snapshot() {
  cmp -s "$FIXTURE/Cargo.toml" "$TEST_ROOT/Cargo.toml.snapshot" || \
    fail "Cargo.toml changed after a rejected version"
  cmp -s "$FIXTURE/Cargo.lock" "$TEST_ROOT/Cargo.lock.snapshot" || \
    fail "Cargo.lock changed after a rejected version"
  cmp -s "$FIXTURE/extension/queue-focus@queuefocus.org/metadata.json" \
    "$TEST_ROOT/metadata.json.snapshot" || \
    fail "extension metadata changed after a rejected version"
}

# Invalid SemVer and a non-increasing explicit extension revision both fail
# before any file changes.
snapshot
printf '\n' | VERSION="999.0.0" EXTENSION_VERSION=99 \
  make --no-print-directory -s -C "$FIXTURE" maybe-version >/dev/null 2>&1
assert_snapshot

reprompt_output=$(printf '01.2.3\n\n' | \
  make --no-print-directory -s -C "$FIXTURE" version 2>&1)
grep -Fq "Invalid version:" <<<"$reprompt_output" || \
  fail "interactive versioning did not explain and retry invalid input"
assert_snapshot

if make --no-print-directory -s -C "$FIXTURE" maybe-version \
    EXTENSION_VERSION=99 </dev/null >/dev/null 2>&1; then
  fail "non-interactive prompt unexpectedly succeeded"
fi
assert_snapshot

if "$FIXTURE/scripts/set-version" "01.2.3" >/dev/null 2>&1; then
  fail "invalid semantic version unexpectedly succeeded"
fi
assert_snapshot

second_version="988.$$.0-rc.1+build.5"
snapshot
if "$FIXTURE/scripts/set-version" "$second_version" \
    --extension-version "$first_extension_version" >/dev/null 2>&1; then
  fail "non-increasing extension revision unexpectedly succeeded"
fi
assert_snapshot

# An explicit increasing extension revision is accepted alongside prerelease
# and build SemVer identifiers.
second_extension_version=$((first_extension_version + 9))
"$FIXTURE/scripts/set-version" "$second_version" \
  --extension-version "$second_extension_version" >/dev/null
python3 - "$FIXTURE" "$second_version" "$second_extension_version" <<'PY'
import json
from pathlib import Path
import sys
import tomllib

root = Path(sys.argv[1])
version = sys.argv[2]
extension_version = int(sys.argv[3])
assert tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"] == version
lock = tomllib.loads((root / "Cargo.lock").read_text())
assert all(
    package["version"] == version
    for package in lock["package"]
    if package["name"] in {"qf-core", "queue-focus"} and "source" not in package
)
metadata = json.loads(
    (root / "extension/queue-focus@queuefocus.org/metadata.json").read_text()
)
assert metadata["version"] == extension_version
PY

echo "versioning integration tests passed"
