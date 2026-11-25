#!/usr/bin/env bash
# Usage: measure.sh <label> [extra cargo args...]
# Builds nve-web-shell for wasm32 (wasm-release), runs wasm-bindgen --target web,
# then wasm-opt -Oz, and prints raw/gzip-9/brotli-11 for wasm and JS glue.
# Env: SYMBOLS=1 keeps a symbol build next to it (for twiggy), TARGET_DIR overrides.
set -euo pipefail
LABEL="$1"; shift || true
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
TD="${TARGET_DIR:-$ROOT/target/wasm-size}"
OUT="$ROOT/target/wasm-size-out/$LABEL"
mkdir -p "$OUT"
cd "$ROOT"
STRIP=symbols; [[ "${SYMBOLS:-0}" == 1 ]] && STRIP=none
CARGO_PROFILE_WASM_RELEASE_STRIP=$STRIP CARGO_TARGET_DIR="$TD" cargo rustc -q \
  -p nve-web-shell --lib --crate-type cdylib --target wasm32-unknown-unknown --profile wasm-release "$@"
IN="$TD/wasm32-unknown-unknown/wasm-release/nve_web_shell.wasm"
KD=(); [[ "${SYMBOLS:-0}" == 1 ]] && KD=(--keep-debug)
wasm-bindgen --target web "${KD[@]}" --out-dir "$OUT" --no-typescript "$IN" 2>/dev/null
W="$OUT/nve_web_shell_bg.wasm"; J="$OUT/nve_web_shell.js"
wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue "$W" -o "$OUT/opt.wasm" 2>/dev/null || cp "$W" "$OUT/opt.wasm"
sz(){ local f=$1; printf "%d/%d/%d" "$(wc -c <"$f")" "$(gzip -9 -c "$f"|wc -c)" "$(brotli -q 11 -c "$f"|wc -c)"; }
jb=$(brotli -q 11 -c "$J"|wc -c); wb=$(brotli -q 11 -c "$W"|wc -c); ob=$(brotli -q 11 -c "$OUT/opt.wasm"|wc -c)
printf "%-28s wasm %s | wasm-opt -Oz %s | js %s | total-br %d (opt %d)\n" "$LABEL" "$(sz "$W")" "$(sz "$OUT/opt.wasm")" "$(sz "$J")" $((wb+jb)) $((ob+jb))
