#!/usr/bin/env bash
# Build the engine for the macOS app into the macos/QfCore Swift package: a
# universal static library wrapped in an XCFramework, and the QfCore Swift
# module generated from that very library. The Swift checks at first use that
# it matches the library, so both always come from one run of this script.
#
# Only the crates the app needs are built, by name: the GTK app cannot build
# on macOS, so never the whole workspace.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

die() {
  echo "build-mac-core: $*" >&2
  exit 1
}

[ "$(uname -s)" = Darwin ] || die "builds for macOS, on macOS"
for tool in lipo xcodebuild; do
  command -v "$tool" >/dev/null || die "missing $tool (install the Xcode command line tools)"
done

export MACOSX_DEPLOYMENT_TARGET=14.0
readonly CARGO=scripts/cargo
readonly PROFILE=release-ffi
readonly TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
readonly PACKAGE=macos/QfCore
readonly WORK=target/mac-core

if command -v rustup >/dev/null 2>&1 || [ -x "${CARGO_HOME:-$HOME/.cargo}/bin/rustup" ]; then
  PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH" rustup target add "${TARGETS[@]}" >/dev/null
fi

libraries=()
for target in "${TARGETS[@]}"; do
  "$CARGO" build --locked -p qf-ffi --lib --profile "$PROFILE" --target "$target"
  libraries+=("target/$target/$PROFILE/libqf_ffi.a")
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

rm -rf "$PACKAGE/QfCoreFFI.xcframework"
xcodebuild -create-xcframework -library "$WORK/libqf_ffi.a" -headers "$WORK/headers" \
  -output "$PACKAGE/QfCoreFFI.xcframework" >/dev/null
mkdir -p "$PACKAGE/Sources/QfCore"
cp "$WORK/swift/QfCore.swift" "$PACKAGE/Sources/QfCore/QfCore.swift"

echo "built $PACKAGE: QfCoreFFI.xcframework ($(lipo -archs "$WORK/libqf_ffi.a")) and Sources/QfCore/QfCore.swift"
