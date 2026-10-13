#!/usr/bin/env bash
# Builds dist/XDAW.app (unsigned) and dist/XDAW-macos-<arch>.zip from a release build.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --locked -p xdaw-ui -p xdaw-engine --bins
arch="$(uname -m)"
app="dist/XDAW.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/xdaw "$app/Contents/MacOS/XDAW"
cp target/release/xdaw-headless target/release/xdaw-make-demo "$app/Contents/MacOS/"
version="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="xdaw-ui"))')"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>XDAW</string>
  <key>CFBundleDisplayName</key><string>XDAW</string>
  <key>CFBundleIdentifier</key><string>io.github.xdaw.app</string>
  <key>CFBundleExecutable</key><string>XDAW</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>XDAW records audio from your input device when a track is armed.</string>
  <key>CFBundleDocumentTypes</key>
  <array><dict>
    <key>CFBundleTypeName</key><string>XDAW Set</string>
    <key>CFBundleTypeExtensions</key><array><string>xdaw</string></array>
    <key>CFBundleTypeRole</key><string>Editor</string>
  </dict></array>
</dict>
</plist>
PLIST
# Ad-hoc signature so the bundle launches locally on Apple Silicon.
codesign --force --deep --sign - "$app" >/dev/null 2>&1 || echo "codesign unavailable; bundle left unsigned"
(cd dist && rm -f "XDAW-macos-${arch}.zip" && ditto -c -k --keepParent XDAW.app "XDAW-macos-${arch}.zip")
echo "built $app and dist/XDAW-macos-${arch}.zip (version ${version}, ${arch})"
