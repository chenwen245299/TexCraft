#!/bin/sh
# Packs TexCraft.app into a disk image people install from: open it, drag TexCraft onto the
# Applications shortcut beside it. Usage: scripts/make-dmg.sh path/to/TexCraft.app out.dmg
# (run scripts/bundle-macos.sh first). release.yml runs it after signing the app.
set -eu
APP="$1"
OUT="$2"
[ -d "$APP/Contents/MacOS" ] || { echo "make-dmg: $APP is not an app bundle" >&2; exit 1; }

STAGE=$(mktemp -d "${TMPDIR:-/tmp}/texcraft-dmg.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT INT TERM
# ditto keeps the bundle's signature, symlinks and extended attributes intact.
ditto "$APP" "$STAGE/TexCraft.app"
ln -s /Applications "$STAGE/Applications"

rm -f "$OUT"
hdiutil create -quiet -volname TexCraft -srcfolder "$STAGE" -fs HFS+ -format UDZO \
  -imagekey zlib-level=9 -ov "$OUT"
hdiutil verify -quiet "$OUT"
echo "Built $OUT"
