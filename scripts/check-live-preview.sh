#!/bin/sh
# A missing TeX installation must fail this gate, never turn the preview contract into a skip.
set -eu
cd "$(dirname "$0")/.."
TEXCRAFT_REQUIRE_TEX=1 cargo test --locked -p texcraft-ui --test live_preview "$@"
# State-transition regressions (pause/abort/resume/configuration changes) live in the engine.
cargo test --locked -p texcraft-engine --lib "$@"
