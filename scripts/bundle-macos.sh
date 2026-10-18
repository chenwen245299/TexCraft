#!/bin/sh
# Builds TexCraft.app (release) in target/macos/. Usage: scripts/bundle-macos.sh
set -eu
cd "$(dirname "$0")/.."
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
# CI builds a universal binary first and sets TEXCRAFT_SKIP_BUILD=1.
if [ -z "${TEXCRAFT_SKIP_BUILD:-}" ]; then
  cargo build --release -p texcraft -p texcraft-cli
fi
APP="$TARGET_DIR/macos/TexCraft.app"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$TARGET_DIR/release/texcraft" "$APP/Contents/MacOS/texcraft"
cp "$TARGET_DIR/release/texcraft-cli" "$APP/Contents/MacOS/texcraft-cli"

# Icon: the app draws its own icon; turn it into an .icns.
ICONSET="$TARGET_DIR/macos/AppIcon.iconset"
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
"$TARGET_DIR/release/texcraft" --write-icon "$ICONSET/base.png"
for s in 16 32 128 256 512; do
  sips -z $s $s "$ICONSET/base.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z $d $d "$ICONSET/base.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
rm "$ICONSET/base.png"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET"

# The interface languages (crates/i18n): AppKit's own menu items and panels follow the first of
# the user's languages the bundle lists. The strings themselves are compiled in.
mkdir -p "$APP/Contents/Resources/en.lproj" "$APP/Contents/Resources/zh-Hans.lproj"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>TexCraft</string>
  <key>CFBundleDisplayName</key><string>TexCraft</string>
  <key>CFBundleIdentifier</key><string>dev.texcraft.TexCraft</string>
  <key>CFBundleExecutable</key><string>texcraft</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string><string>zh-Hans</string></array>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
</dict>
</plist>
PLIST
echo "Built $APP"
