#!/usr/bin/env bash
# Build the engine for the macOS app into the macos/QfCore Swift package: a
# universal static library wrapped in an XCFramework, and the QfCore Swift
# module generated from that very library. The Swift checks at first use that
# it matches the library, so both always come from one run of this script.
#
# The app links the same library from lib/ and its C module from include/,
# at paths that never change with the architectures built. Xcode copies an
# XCFramework when it plans a build, before this script runs, so linking the
# XCFramework there could link the library from the build before.
#
# Only the crates the app needs are built, by name: the GTK app cannot build
# on macOS, so never the whole workspace.
#
#   --archs "arm64 x86_64"  the Apple architectures to build (default: both);
#                           Xcode passes $ARCHS so a Debug build makes one
#   --if-changed            do nothing when the Rust inputs and the
#                           architectures match the last build
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

die() {
  echo "build-mac-core: $*" >&2
  exit 1
}

archs="arm64 x86_64"
if_changed=false
while [ $# -gt 0 ]; do
  case $1 in
    --archs) archs=${2:?--archs needs a value}; shift 2 ;;
    --if-changed) if_changed=true; shift ;;
    *) die "unknown option: $1" ;;
  esac
done

[ "$(uname -s)" = Darwin ] || die "builds for macOS, on macOS"
for tool in lipo xcodebuild python3; do
  command -v "$tool" >/dev/null || die "missing $tool (install the Xcode command line tools)"
done

export MACOSX_DEPLOYMENT_TARGET=14.0
readonly CARGO=scripts/cargo
readonly PROFILE=release-ffi
TARGETS=()
for arch in $archs; do
  case $arch in
    arm64) TARGETS+=(aarch64-apple-darwin) ;;
    x86_64) TARGETS+=(x86_64-apple-darwin) ;;
    *) die "unsupported architecture: $arch" ;;
  esac
done
[ ${#TARGETS[@]} -gt 0 ] || die "no architectures to build"
readonly TARGETS
readonly PACKAGE=macos/QfCore
# Wherever Cargo puts its output: CARGO_TARGET_DIR or a config file can move
# it, and reading a hardcoded path would package an old library.
TARGET_DIR=$("$CARGO" metadata --format-version 1 --no-deps |
  python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
readonly TARGET_DIR
readonly WORK="$TARGET_DIR/mac-core"
# Beside what it vouches for: every build writes the same package, whichever
# Cargo target directory it was made in.
readonly STAMP="$PACKAGE/QfCoreFFI.inputs"

# Everything the library and its bindings are made from, and the targets.
inputs() {
  {
    printf '%s\n' "${TARGETS[@]}"
    find crates/qf-core crates/qf-ffi crates/uniffi-bindgen -type f \
      \( -name '*.rs' -o -name '*.toml' \) -print | LC_ALL=C sort | xargs shasum
    shasum Cargo.toml Cargo.lock scripts/build-mac-core.sh scripts/cargo
  } | shasum | cut -d' ' -f1
}
readonly INPUTS=$(inputs)
if $if_changed && [ -f "$STAMP" ] && [ "$(cat "$STAMP")" = "$INPUTS" ] &&
  [ -d "$PACKAGE/QfCoreFFI.xcframework" ] && [ -f "$PACKAGE/lib/libqf_ffi.a" ] &&
  [ -f "$PACKAGE/include/module.modulemap" ] && [ -f "$PACKAGE/Sources/QfCore/QfCore.swift" ]; then
  echo "build-mac-core: up to date"
  exit 0
fi
rm -f "$STAMP"

if command -v rustup >/dev/null 2>&1 || [ -x "${CARGO_HOME:-$HOME/.cargo}/bin/rustup" ]; then
  PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH" rustup target add "${TARGETS[@]}" >/dev/null
fi

libraries=()
for target in "${TARGETS[@]}"; do
  "$CARGO" build --locked -p qf-ffi --lib --profile "$PROFILE" --target "$target"
  libraries+=("$TARGET_DIR/$target/$PROFILE/libqf_ffi.a")
done

rm -rf "$WORK"
mkdir -p "$WORK/headers" "$WORK/swift"
lipo -create "${libraries[@]}" -output "$WORK/libqf_ffi.a"

bindgen() {
  "$CARGO" run --locked --quiet -p uniffi-bindgen --bin uniffi-bindgen-swift -- \
    "$WORK/libqf_ffi.a" "$@" --metadata-no-deps
}
bindgen "$WORK/swift" --swift-sources
bindgen "$WORK/headers" --headers --modulemap \
  --module-name QfCoreFFI --modulemap-filename module.modulemap

rm -rf "$PACKAGE/QfCoreFFI.xcframework" "$PACKAGE/lib" "$PACKAGE/include"
xcodebuild -create-xcframework -library "$WORK/libqf_ffi.a" -headers "$WORK/headers" \
  -output "$PACKAGE/QfCoreFFI.xcframework" >/dev/null
mkdir -p "$PACKAGE/lib" "$PACKAGE/include" "$PACKAGE/Sources/QfCore"
cp "$WORK/libqf_ffi.a" "$PACKAGE/lib/"
cp "$WORK/headers/QfCoreFFI.h" "$WORK/headers/module.modulemap" "$PACKAGE/include/"
cp "$WORK/swift/QfCore.swift" "$PACKAGE/Sources/QfCore/QfCore.swift"

echo "$INPUTS" >"$STAMP"
echo "built $PACKAGE: QfCoreFFI.xcframework ($(lipo -archs "$WORK/libqf_ffi.a")) and Sources/QfCore/QfCore.swift"
