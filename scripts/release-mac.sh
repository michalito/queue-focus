#!/usr/bin/env bash
# Build the macOS app for release: a universal archive signed with Developer
# ID, notarized and stapled, in a signed, notarized, stapled DMG, with its
# checksum and debug symbols, in dist/. It publishes nothing.
#
#   scripts/release-mac.sh            # needs DEVELOPER_TEAM and notary credentials
#   scripts/release-mac.sh --dry-run  # signs ad hoc; no notarization
#
# DEVELOPER_TEAM is the ten-character team id of the "Developer ID
# Application" certificate in the login keychain. Notarization uses
# NOTARY_PROFILE (a `xcrun notarytool store-credentials` profile) or an App
# Store Connect API key: NOTARY_KEY (path to the .p8), NOTARY_KEY_ID and
# NOTARY_ISSUER. Nothing secret is passed on a command line or printed.
#
# The dry run takes the same path with an ad hoc signature, so it checks the
# archive, the binary, the DMG and the checks themselves without the
# certificate; Gatekeeper rejects what it makes, and it says so.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly DIST="$ROOT/dist"
readonly APP_NAME="Queue Focus"
readonly PROJECT="$ROOT/macos/QueueFocus.xcodeproj"

dry_run=false
case "${1:-}" in
  "") ;;
  --dry-run) dry_run=true ;;
  *) echo "usage: $0 [--dry-run]" >&2; exit 2 ;;
esac

say() { printf '\n==> %s\n' "$*"; }
fail() { echo "release-mac: $*" >&2; exit 1; }

version=$(python3 -c 'import tomllib,sys; print(tomllib.load(open(sys.argv[1],"rb"))["workspace"]["package"]["version"])' "$ROOT/Cargo.toml")
readonly version
# The app's version: the semantic one without its build metadata.
readonly app_version="${version%%+*}"
readonly name="QueueFocus-$app_version"

# A release is made from a clean tree at its tag; a dry run only warns.
check_tree() {
  local problem=""
  if [ -n "$(git -C "$ROOT" status --porcelain)" ]; then
    problem="the working tree has uncommitted changes"
  elif [ "$(git -C "$ROOT" tag --points-at HEAD | grep -Fxc "v$version")" != 1 ]; then
    problem="HEAD is not tagged v$version"
  fi
  if [ -n "$problem" ]; then
    if $dry_run; then echo "warning: $problem (a release would stop here)" >&2; else fail "$problem"; fi
  fi
}

# The one Developer ID Application identity of the team, by its hash.
signing_identity() {
  if $dry_run; then
    echo "-"
    return
  fi
  [[ "${DEVELOPER_TEAM:-}" =~ ^[A-Z0-9]{10}$ ]] || fail "set DEVELOPER_TEAM to the Developer ID team id"
  local matches
  matches=$(security find-identity -v -p codesigning |
    grep "\"Developer ID Application: .* ($DEVELOPER_TEAM)\"" | awk '{print $2}' || true)
  [ "$(printf '%s\n' "$matches" | grep -c .)" = 1 ] ||
    fail "expected one Developer ID Application identity for team $DEVELOPER_TEAM in the keychain"
  echo "$matches"
}

notary() {
  if [ -n "${NOTARY_PROFILE:-}" ]; then
    xcrun notarytool "$@" --keychain-profile "$NOTARY_PROFILE"
  elif [ -n "${NOTARY_KEY:-}" ] && [ -n "${NOTARY_KEY_ID:-}" ] && [ -n "${NOTARY_ISSUER:-}" ]; then
    xcrun notarytool "$@" --key "$NOTARY_KEY" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER"
  else
    fail "set NOTARY_PROFILE, or NOTARY_KEY, NOTARY_KEY_ID and NOTARY_ISSUER"
  fi
}

# Submit a file for notarization and wait; show Apple's log if it is refused.
notarize() {
  local file=$1 result id status
  result=$(notary submit "$file" --wait --timeout 30m --output-format json)
  id=$(python3 -c 'import json,sys; print(json.loads(sys.stdin.read())["id"])' <<<"$result")
  status=$(python3 -c 'import json,sys; print(json.loads(sys.stdin.read())["status"])' <<<"$result")
  if [ "$status" != "Accepted" ]; then
    notary log "$id" >&2 || true
    fail "notarization of $(basename "$file") came back $status"
  fi
}

# Attach the ticket; the service can take a moment to publish it.
staple() {
  local file=$1 attempt
  for attempt in 1 2 3 4 5; do
    if xcrun stapler staple "$file" >/dev/null; then
      xcrun stapler validate "$file" >/dev/null && return
    fi
    sleep $((attempt * 10))
  done
  fail "could not staple $(basename "$file")"
}

check_tree
identity=$(signing_identity)
readonly identity
mkdir -p "$DIST"
rm -rf "$DIST/$name".* "$DIST/QueueFocus.xcarchive" "$DIST/stage"

say "Building the engine for Apple silicon and Intel"
"$ROOT/scripts/build-mac-core.sh" --archs "arm64 x86_64"
[ "$(lipo -archs "$ROOT/macos/QfCore/lib/libqf_ffi.a")" = "x86_64 arm64" ] ||
  fail "the engine library is not universal"

say "Archiving $APP_NAME $app_version"
sign_settings=(CODE_SIGN_STYLE=Manual "CODE_SIGN_IDENTITY=$identity")
$dry_run || sign_settings+=("DEVELOPMENT_TEAM=$DEVELOPER_TEAM" "OTHER_CODE_SIGN_FLAGS=--timestamp")
# Always an archive: a plain Release build adds the debugger's entitlement,
# which notarization refuses.
xcodebuild archive -project "$PROJECT" -scheme QueueFocus -configuration Release \
  -destination "generic/platform=macOS" \
  -derivedDataPath "$ROOT/target/xcode-release" -archivePath "$DIST/QueueFocus.xcarchive" \
  -onlyUsePackageVersionsFromResolvedFile -skipPackageUpdates \
  ARCHS="arm64 x86_64" ONLY_ACTIVE_ARCH=NO "${sign_settings[@]}" -quiet
mkdir -p "$DIST/stage"
ditto "$DIST/QueueFocus.xcarchive/Products/Applications/$APP_NAME.app" "$DIST/stage/$APP_NAME.app"
readonly app="$DIST/stage/$APP_NAME.app"

say "Checking the app"
codesign --verify --deep --strict "$app"
details=$(codesign -d --verbose=2 "$app" 2>&1)
grep -q "flags=.*runtime" <<<"$details" || fail "the app is not signed for the hardened runtime"
if ! $dry_run; then
  grep -q "^Timestamp=" <<<"$details" || fail "the signature has no secure timestamp"
  grep -qx "TeamIdentifier=$DEVELOPER_TEAM" <<<"$details" || fail "the signature is not team $DEVELOPER_TEAM's"
fi
entitlements=$(codesign -d --entitlements - --xml "$app" 2>/dev/null || true)
python3 - "$entitlements" <<'PY' || fail "the app has entitlements; it should have none"
import plistlib, sys
text = sys.argv[1].strip()
assert not text or plistlib.loads(text.encode()) == {}
PY
executable="$app/Contents/MacOS/$APP_NAME"
[ "$(lipo -archs "$executable")" = "x86_64 arm64" ] || fail "the app is not universal"
plist() { /usr/libexec/PlistBuddy -c "Print :$1" "$app/Contents/Info.plist"; }
[ "$(plist CFBundleShortVersionString)" = "$app_version" ] || fail "the app's version is not $app_version"
[ "$(plist LSMinimumSystemVersion)" = "14.0" ] || fail "the app's minimum macOS is not 14.0"

if ! $dry_run; then
  say "Notarizing the app"
  ditto -c -k --keepParent "$app" "$DIST/$name.zip"
  notarize "$DIST/$name.zip"
  rm "$DIST/$name.zip"
  staple "$app"
fi

say "Making the disk image"
ln -s /Applications "$DIST/stage/Applications"
hdiutil create -volname "$APP_NAME" -srcfolder "$DIST/stage" -fs HFS+ -format UDZO -ov \
  "$DIST/$name.dmg" -quiet
dmg_signing=(--sign "$identity")
$dry_run || dmg_signing+=(--timestamp)
codesign "${dmg_signing[@]}" "$DIST/$name.dmg"
if ! $dry_run; then
  say "Notarizing the disk image"
  notarize "$DIST/$name.dmg"
  staple "$DIST/$name.dmg"
fi

# Gatekeeper's verdict, whole: spctl exits 0 when it accepts, 3 when it
# rejects, and with another code when it could not judge.
assess() {
  assessed=0
  assessment=$(spctl --assess -vv "$@" 2>&1) || assessed=$?
}
# Accepted, and as notarized Developer ID.
notarized() {
  [ "$assessed" = 0 ] && grep -q "^source=Notarized Developer ID$" <<<"$assessment"
}

say "Asking Gatekeeper"
if $dry_run; then
  # An ad hoc signature is not Developer ID: Gatekeeper must say no.
  assess --type execute "$app"
  [ "$assessed" = 3 ] || fail "Gatekeeper did not reject the ad hoc app (spctl exited $assessed): $assessment"
  echo "rejected, as an ad hoc signature should be"
else
  assess --type execute "$app"
  notarized || fail "Gatekeeper does not take the app as notarized Developer ID: $assessment"
  assess --type open --context context:primary-signature "$DIST/$name.dmg"
  notarized || fail "Gatekeeper does not take the disk image: $assessment"
fi

say "Writing the checksum, the debug symbols and the build record"
(cd "$DIST/QueueFocus.xcarchive/dSYMs" && ditto -c -k --keepParent "$APP_NAME.app.dSYM" "$DIST/$name.dSYM.zip")
(cd "$DIST" && shasum -a 256 "$name.dmg" >"$name.dmg.sha256")
{
  echo "version: $app_version (build $(plist CFBundleVersion))"
  echo "commit: $(git -C "$ROOT" rev-parse HEAD)"
  echo "xcode: $(plist DTXcode) ($(plist DTXcodeBuild)), sdk: $(plist DTSDKName)"
  echo "signed: $($dry_run && echo "ad hoc (dry run)" || echo "Developer ID, team $DEVELOPER_TEAM, notarized")"
  cat "$DIST/$name.dmg.sha256"
} >"$DIST/$name.build-info.txt"
rm -rf "$DIST/stage"

say "Done"
ls -1 "$DIST" | sed 's/^/  dist\//'
if $dry_run; then echo "dry run: nothing here is for publishing"; fi
