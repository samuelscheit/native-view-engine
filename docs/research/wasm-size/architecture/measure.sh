#!/usr/bin/env bash
# Usage: measure.sh <label> <final.wasm> [glue.js]
# Prints: label wasm_raw wasm_gz9 wasm_br11 js_raw js_gz9 js_br11 total_br
set -euo pipefail
label=$1; wasm=$2; js=${3:-}
sz() { wc -c < "$1" | tr -d ' '; }
gz() { gzip -9 -c "$1" | wc -c | tr -d ' '; }
br() { brotli -q 11 -c "$1" | wc -c | tr -d ' '; }
wr=$(sz "$wasm"); wg=$(gz "$wasm"); wb=$(br "$wasm")
if [[ -n "$js" ]]; then jr=$(sz "$js"); jg=$(gz "$js"); jb=$(br "$js"); else jr=0; jg=0; jb=0; fi
printf '%-40s wasm %8d raw %8d gz %8d br | js %6d raw %6d gz %6d br | total_br %8d\n' "$label" "$wr" "$wg" "$wb" "$jr" "$jg" "$jb" $((wb+jb))
