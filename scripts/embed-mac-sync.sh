#!/usr/bin/env bash
# Xcode runs this after the Rust target and before signing the enclosing app.
set -euo pipefail
source_binary="$SRCROOT/QfCore/bin/qf-sync"
destination="$TARGET_BUILD_DIR/$CONTENTS_FOLDER_PATH/Helpers/qf-sync"
mkdir -p "${destination%/*}"
install -m 0755 "$source_binary" "$destination"
if [ "${CODE_SIGNING_ALLOWED:-YES}" != NO ]; then
  identity="${EXPANDED_CODE_SIGN_IDENTITY:--}"
  [ -n "$identity" ] || identity=-
  signing=(--force --sign "$identity" --options runtime)
  [ "$identity" = - ] || signing+=(--timestamp)
  codesign "${signing[@]}" "$destination"
fi
