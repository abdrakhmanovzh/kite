#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
cargo build --locked --release

app_path="target/Kite.app"
mkdir -p "$app_path/Contents/MacOS" "$app_path/Contents/Resources"
cp target/release/kite "$app_path/Contents/MacOS/kite.new"
mv -f "$app_path/Contents/MacOS/kite.new" "$app_path/Contents/MacOS/kite"
cp assets/icon/Kite.icns "$app_path/Contents/Resources/Kite.icns"
cat > "$app_path/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Kite</string>
    <key>CFBundleDisplayName</key><string>Kite</string>
    <key>CFBundleIdentifier</key><string>local.kite</string>
    <key>CFBundleExecutable</key><string>kite</string>
    <key>CFBundleIconFile</key><string>Kite</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.social-networking</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Finder caches icons; touching the bundle makes it pick up a new one.
touch "$app_path"

printf 'Built %s\n' "$PWD/$app_path"
