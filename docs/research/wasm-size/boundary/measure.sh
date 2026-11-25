#!/usr/bin/env bash
# Usage: measure.sh <label> <wasm> [js...]  -> one line: raw/gzip-9/brotli-q11 for wasm, js and total brotli.
set -euo pipefail
label=$1; wasm=$2; shift 2
sz() { wc -c < "$1" | tr -d ' '; }
gz() { gzip -9 -c "$1" | wc -c | tr -d ' '; }
br() { brotli -q 11 -c "$1" | wc -c | tr -d ' '; }
wr=$(sz "$wasm"); wg=$(gz "$wasm"); wb=$(br "$wasm")
jr=0; jg=0; jb=0
for js in "$@"; do jr=$((jr+$(sz "$js"))); jg=$((jg+$(gz "$js"))); jb=$((jb+$(br "$js"))); done
printf '%-40s wasm %8d raw %7d gz %7d br | js %6d raw %6d gz %6d br | total br %7d\n' "$label" "$wr" "$wg" "$wb" "$jr" "$jg" "$jb" $((wb+jb))
