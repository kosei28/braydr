#!/bin/sh
# Builds target/release/Braydr.app. Notifications only work from a bundle.
set -eu
cd "$(dirname "$0")/.."
cargo build --release -p braydr-app -p braydr-mcp
app=target/release/Braydr.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/braydr target/release/braydr-mcp "$app/Contents/MacOS/"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Braydr</string>
    <key>CFBundleDisplayName</key><string>Braydr</string>
    <key>CFBundleIdentifier</key><string>dev.braydr.app</string>
    <key>CFBundleExecutable</key><string>braydr</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$app"
echo "$app"
