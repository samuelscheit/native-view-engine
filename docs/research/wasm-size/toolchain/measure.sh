#!/usr/bin/env bash
# Toolchain-track size harness (docs/research/wasm-size/toolchain.md): build nve-web-shell once
# with a given toolchain/flag set, then post-process and report shipped sizes
# (wasm raw/gzip -9/brotli -q 11, JS raw/gzip/brotli, total brotli) into target/wasm-size-out/results.txt.
#
#   measure.sh build NAME [cargo args...]   env: TC (toolchain, default stable), RF (wasm32 rustflags),
#                                            STRIP (profile strip, default none), PROFILE_ENV (extra env)
#   measure.sh post  NAME LABEL "BINDGEN_FLAGS" "WASM_OPT_FLAGS"
#   measure.sh final NAME                   (same post-processing as hosts/web-shell/build.sh)
#   measure.sh size  FILE.wasm FILE.js [LABEL]
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
OUT="${MEASURE_OUT:-${REPO}/target/wasm-size-out}"
mkdir -p "$OUT"

size_row() { # wasm js label
  local w="$1" j="$2" label="$3"
  local wr wg wb jr jg jb
  wr=$(wc -c <"$w" | tr -d ' ')
  wg=$(gzip -9 -c "$w" | wc -c | tr -d ' ')
  wb=$(brotli -q 11 -c "$w" | wc -c | tr -d ' ')
  jr=$(wc -c <"$j" | tr -d ' ')
  jg=$(gzip -9 -c "$j" | wc -c | tr -d ' ')
  jb=$(brotli -q 11 -c "$j" | wc -c | tr -d ' ')
  printf '%-48s wasm %8d raw %7d gz %7d br | js %6d raw %6d gz %6d br | total br %7d\n' \
    "$label" "$wr" "$wg" "$wb" "$jr" "$jg" "$jb" $((wb + jb)) | tee -a "$OUT/results.txt"
}

cmd="$1"; shift
case "$cmd" in
build)
  name="$1"; shift
  tc="${TC:-stable}"
  td="${REPO}/target/wsz-${name}"
  (cd "$REPO" && env RUSTC_WRAPPER= \
    CARGO_PROFILE_WASM_RELEASE_STRIP="${STRIP:-none}" \
    CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="${RF:-}" \
    CARGO_TARGET_DIR="$td" \
    ${PROFILE_ENV:-} \
    cargo "+$tc" rustc --locked -p nve-web-shell --lib --crate-type cdylib \
    --target wasm32-unknown-unknown --profile wasm-release "$@")
  cp "$td/wasm32-unknown-unknown/wasm-release/nve_web_shell.wasm" "$OUT/${name}.raw.wasm"
  ls -l "$OUT/${name}.raw.wasm"
  ;;
post)
  name="$1" label="$2" bg="$3" opt="$4"
  d=$(mktemp -d)
  mkdir "$d/in" && cp "$OUT/${name}.raw.wasm" "$d/in/nve_web_shell.wasm"
  # shellcheck disable=SC2086
  wasm-bindgen --target web --out-dir "$d" --no-typescript $bg "$d/in/nve_web_shell.wasm" 2>"$d/bg.log" || { tail -5 "$d/bg.log"; exit 1; }
  w="$d/nve_web_shell_bg.wasm"
  if [[ -n "$opt" ]]; then
    # shellcheck disable=SC2086
    if ! wasm-opt $opt "$w" -o "$w.o" >"$d/opt.log" 2>&1; then
      echo "wasm-opt FAILED ($name/$label):"; grep -m3 -E 'error|Fatal' "$d/opt.log"; tail -1 "$d/opt.log"; exit 1
    fi
    mv "$w.o" "$w"
  fi
  size_row "$w" "$d/nve_web_shell.js" "$name/$label"
  mkdir -p "$OUT/final/$name-$label" && cp "$w" "$d/nve_web_shell.js" "$OUT/final/$name-$label/"
  rm -rf "$d"
  ;;
final)
  # Same post-processing as hosts/web-shell/build.sh: bindgen (names kept) -> snip -> tuned
  # wasm-opt -> terser; JS size includes the snippets/ files.
  name="$1"
  d=$(mktemp -d)
  mkdir "$d/in" && cp "$OUT/${name}.raw.wasm" "$d/in/nve_web_shell.wasm"
  wasm-bindgen --target web --out-dir "$d/o" --no-typescript --remove-producers-section "$d/in/nve_web_shell.wasm" 2>/dev/null
  w="$d/o/nve_web_shell_bg.wasm"
  python3 "$REPO/hosts/web-shell/tools/wat-snip.py" "$w" "$w" 'rustc_demangle\[' 'std\[[0-9a-f]+\]::backtrace' 2>/dev/null
  wasm-opt --strip-debug --strip-producers --strip-target-features --duplicate-function-elimination \
    --remove-unused-module-elements --vacuum --merge-blocks --dce --remove-unused-brs \
    --optimize-instructions --precompute --reorder-locals --converge "$w" -o "$w.o" && mv "$w.o" "$w"
  find "$d/o" -name '*.js' | while read -r js; do npx -y terser@5 "$js" --module --compress passes=3 --mangle --toplevel -o "$js"; done
  cat $(find "$d/o" -name '*.js' | sort) > "$d/all.js"
  size_row "$w" "$d/all.js" "$name/final"
  rm -rf "$d"
  ;;
size)
  size_row "$1" "$2" "${3:-$1}"
  ;;
esac
