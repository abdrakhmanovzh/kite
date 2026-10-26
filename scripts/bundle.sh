#!/bin/sh
# Builds target/Kite.app (release), signs it ad hoc, and with --dmg also
# packs target/Kite.dmg: the app next to an Applications shortcut, ready to
# drag and drop. Set KITE_API_ID and KITE_API_HASH to build in Telegram API
# credentials, so users only log in.
set -eu

cd "$(dirname "$0")/.."
cargo build --locked --release

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app_path="target/Kite.app"
rm -rf "$app_path"
mkdir -p "$app_path/Contents/MacOS" "$app_path/Contents/Resources"
cp target/release/kite "$app_path/Contents/MacOS/kite"
cp assets/icon/Kite.icns "$app_path/Contents/Resources/Kite.icns"
cat > "$app_path/Contents/Info.plist" <<PLIST
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
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.social-networking</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad hoc signature: no Apple Developer ID, so on first launch macOS asks
# once (System Settings → Privacy & Security → Open Anyway).
codesign --force --deep --sign - "$app_path"
printf 'Built %s\n' "$PWD/$app_path"

if [ "${1:-}" = "--dmg" ]; then
    stage=$(mktemp -d)
    trap 'rm -rf "$stage"' EXIT
    cp -R "$app_path" "$stage/"
    ln -s /Applications "$stage/Applications"
    rm -f target/Kite.dmg
    hdiutil create -volname Kite -srcfolder "$stage" -format UDZO -ov target/Kite.dmg >/dev/null
    printf 'Packed %s\n' "$PWD/target/Kite.dmg"
fi
