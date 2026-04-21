#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="BwLimit"
INSTALL_DIR="/Applications"
APP_BUNDLE="$ROOT_DIR/dist/${APP_NAME}.app"
INSTALL_BUNDLE="$INSTALL_DIR/${APP_NAME}.app"

"$ROOT_DIR/scripts/package-macos-app.sh"

osascript -e 'tell application id "com.gp.bwlimit" to quit' >/dev/null 2>&1 || true
pkill -x bwlimit >/dev/null 2>&1 || true
sleep 1

mkdir -p "$INSTALL_BUNDLE"
rsync -a --delete "$APP_BUNDLE/" "$INSTALL_BUNDLE/"

xattr -dr com.apple.quarantine "$INSTALL_BUNDLE" >/dev/null 2>&1 || true

echo "Installed $INSTALL_BUNDLE"
open "$INSTALL_BUNDLE"
