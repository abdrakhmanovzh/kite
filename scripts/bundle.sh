#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
cargo build --locked --release

app_path="target/Telegram GPUI.app"
mkdir -p "$app_path/Contents/MacOS"
cp target/release/telegram-desktop "$app_path/Contents/MacOS/telegram-desktop.new"
mv -f "$app_path/Contents/MacOS/telegram-desktop.new" "$app_path/Contents/MacOS/telegram-desktop"
cat > "$app_path/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Telegram GPUI</string>
    <key>CFBundleDisplayName</key><string>Telegram GPUI</string>
    <key>CFBundleIdentifier</key><string>local.telegram-gpui</string>
    <key>CFBundleExecutable</key><string>telegram-desktop</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

printf 'Built %s\n' "$PWD/$app_path"
