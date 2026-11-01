#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Building release binary..."
cargo build --release

APP_DIR="target/release/TetherControl.app"
CONTENTS_DIR="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"

rm -rf "${APP_DIR}"
mkdir -p "${MACOS_DIR}"

cp "target/release/tether-control" "${MACOS_DIR}/tether-control"

cat > "${CONTENTS_DIR}/Info.plist" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>TetherControl</string>
    <key>CFBundleDisplayName</key>
    <string>TetherControl</string>
    <key>CFBundleIdentifier</key>
    <string>com.tethercontrol.host</string>
    <key>CFBundleVersion</key>
    <string>0.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleExecutable</key>
    <string>tether-control</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

echo "==> Ad-hoc signing ${APP_DIR} so macOS Accessibility permissions bind cleanly..."
codesign --force --deep --sign - "${APP_DIR}"

echo "==> Built ${APP_DIR}"
echo "You can launch it with: open \"${APP_DIR}\" or move it to /Applications."

