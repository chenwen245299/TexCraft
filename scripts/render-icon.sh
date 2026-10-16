#!/bin/sh
# Renders assets/icon/texcraft.svg to assets/icon/texcraft-1024.png (transparent corners) with
# headless Chrome or Edge. Usage: scripts/render-icon.sh [SVG] [PNG]
set -eu
cd "$(dirname "$0")/.."
abs() { case "$1" in /*) echo "$1" ;; *) echo "$PWD/$1" ;; esac; }
SVG=$(abs "${1:-assets/icon/texcraft.svg}")
PNG=$(abs "${2:-assets/icon/texcraft-1024.png}")
for b in "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
         "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge" \
         "$(command -v chromium || true)" "$(command -v google-chrome || true)"; do
  [ -n "$b" ] && [ -x "$b" ] && BROWSER="$b" && break
done
[ -n "${BROWSER:-}" ] || { echo "needs Chrome, Edge or Chromium" >&2; exit 1; }
TMP=$(mktemp -d)
"$BROWSER" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
  --default-background-color=00000000 --window-size=1024,1024 \
  --user-data-dir="$TMP" --screenshot="$PNG" "file://$SVG" >/dev/null 2>&1
rm -rf "$TMP"
echo "wrote $PNG"
