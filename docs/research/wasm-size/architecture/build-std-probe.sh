#!/usr/bin/env bash
# Usage: build-std-probe.sh <features> <label> — nightly build-std + immediate-abort panics for nve-web-min.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
export CARGO_TARGET_DIR="$ROOT/target/wasm-size-bstd"
TC=${NIGHTLY:-nightly-2026-10-20}
(cd "$ROOT" && RUSTFLAGS="-Zunstable-options -Cpanic=immediate-abort" cargo +$TC rustc -q -p nve-web-min --lib --crate-type cdylib \
  --target wasm32-unknown-unknown --profile wasm-release --features "$1" \
  -Zbuild-std=std,panic_abort -Zpanic-immediate-abort --config 'profile.wasm-release.panic="immediate-abort"' 2>&1 | grep -E "^error" -A5 || true)
out=$(mktemp -d)
wasm-bindgen --target web --out-dir "$out" --no-typescript "$CARGO_TARGET_DIR/wasm32-unknown-unknown/wasm-release/nve_web_min.wasm" 2>/dev/null
"$(dirname "$0")/measure.sh" "$2" "$out/nve_web_min_bg.wasm" "$out/nve_web_min.js"
