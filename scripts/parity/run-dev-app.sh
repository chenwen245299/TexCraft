#!/bin/sh
# Builds and starts a TexCraft for testing, isolated from your own settings and recent files
# (its HOME is /tmp/texcraft-dev/home), with the control channel on $TEXCRAFT_PORT (47900).
#
#   scripts/parity/run-dev-app.sh [FILE_OR_FOLDER] [extra texcraft options]
#   scripts/parity/tc.py state.inspect '{}'
#
# Stop it with: pkill -f /tmp/texcraft-dev/token
# It refuses to replace a running instance that has unsaved documents (TEXCRAFT_DEV_FORCE=1 does).
set -eu
cd "$(dirname "$0")/../.."
DEV=/tmp/texcraft-dev
PORT="${TEXCRAFT_PORT:-47900}"
mkdir -p "$DEV/home"
[ -f "$DEV/token" ] || (head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' > "$DEV/token")
chmod 600 "$DEV/token"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
# Never throw away unsaved work: a running instance with unsaved documents (someone may be using
# it) is left alone. TEXCRAFT_DEV_FORCE=1 restarts it anyway.
if [ "${TEXCRAFT_DEV_FORCE:-}" != 1 ] && pgrep -f "$DEV/token" >/dev/null 2>&1 &&
  python3 -I scripts/parity/tc.py state.inspect '{}' 2>/dev/null | grep -q '"dirty": true'; then
  echo "run-dev-app: the running TexCraft has unsaved documents; save them first" >&2
  echo "  (or TEXCRAFT_DEV_FORCE=1 to discard them)." >&2
  exit 1
fi
cargo build -q -p texcraft
pkill -f "$DEV/token" 2>/dev/null || true
# No automatic update checks unless a test manifest is given.
[ -n "${TEXCRAFT_UPDATE_URL:-}" ] || export TEXCRAFT_NO_UPDATES=1
# Detached (subshell + nohup) so it keeps running after this script's shell exits.
(HOME="$DEV/home" nohup "$TARGET_DIR/debug/texcraft" "$@" \
  --control "$PORT" --control-token-file "$DEV/token" --size 1388x824 > "$DEV/app.log" 2>&1 &)
sleep 4
cat "$DEV/app.log"
