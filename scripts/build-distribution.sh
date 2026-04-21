#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="BwLimit"
VERSION="$(perl -ne 'print "$1\n" if /^version = "(.*)"/' "$ROOT_DIR/Cargo.toml" | head -n1)"
APP_BUNDLE="$ROOT_DIR/dist/${APP_NAME}.app"
ZIP_PATH="$ROOT_DIR/dist/${APP_NAME}-macOS-v${VERSION}.zip"

. "$HOME/.cargo/env"

"$ROOT_DIR/scripts/package-macos-app.sh"

rm -f "$ZIP_PATH"
ditto -c -k --sequesterRsrc --keepParent "$APP_BUNDLE" "$ZIP_PATH"

echo "Built $ZIP_PATH"
