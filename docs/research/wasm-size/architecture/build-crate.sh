#!/usr/bin/env bash
# Usage: build-crate.sh <package> <lib_name> <label> [--opt] [extra cargo args...]
# Builds <package> as cdylib for wasm32 with profile wasm-release, runs wasm-bindgen --target web,
# optionally wasm-opt -Oz, and prints sizes. Output dir is printed (mktemp).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
pkg=$1; lib=$2; label=$3; shift 3
opt=0; if [[ "${1:-}" == "--opt" ]]; then opt=1; shift; fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/wasm-size}"
(cd "$ROOT" && cargo rustc --locked -q -p "$pkg" --lib --crate-type cdylib --target wasm32-unknown-unknown --profile wasm-release "$@")
out=$(mktemp -d)
wasm-bindgen --target web --out-dir "$out" --no-typescript "$CARGO_TARGET_DIR/wasm32-unknown-unknown/wasm-release/$lib.wasm" 2>/dev/null
w="$out/${lib}_bg.wasm"
if [[ $opt == 1 ]]; then wasm-opt -Oz --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals --enable-reference-types --enable-multivalue "$w" -o "$w.o" && mv "$w.o" "$w"; label="$label +Oz"; fi
"$(dirname "$0")/measure.sh" "$label" "$w" "$out/$lib.js"
echo "  out=$out"
