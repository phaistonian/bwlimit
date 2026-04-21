#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="BwLimit"
BUNDLE_DIR="$ROOT_DIR/dist/${APP_NAME}.app"
BIN_NAME="bwlimit"
ICON_PATH="$ROOT_DIR/macos/AppIcon.icns"

. "$HOME/.cargo/env"

cargo build --release --manifest-path "$ROOT_DIR/Cargo.toml"
swift "$ROOT_DIR/scripts/generate-macos-icon.swift" "$ICON_PATH"

rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/Contents/MacOS" "$BUNDLE_DIR/Contents/Resources"

cp "$ROOT_DIR/target/release/$BIN_NAME" "$BUNDLE_DIR/Contents/MacOS/$BIN_NAME"
cp "$ROOT_DIR/macos/Info.plist" "$BUNDLE_DIR/Contents/Info.plist"
cp "$ICON_PATH" "$BUNDLE_DIR/Contents/Resources/AppIcon.icns"

chmod +x "$BUNDLE_DIR/Contents/MacOS/$BIN_NAME"

codesign --force --deep --sign - "$BUNDLE_DIR" >/dev/null 2>&1 || true

echo "Built $BUNDLE_DIR"
