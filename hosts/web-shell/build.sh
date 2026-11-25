#!/usr/bin/env bash
# Build the nve-web-shell browser bundle into hosts/web-shell/pkg.
#
# Release (default) is a size pipeline tuned for the *brotli* bytes users download, measured in
# docs/research/wasm-size/toolchain.md:
#   1. pinned nightly + -Zbuild-std (std rebuilt with optimize_for_size) + -Cpanic=immediate-abort,
#      -Zfmt-debug=shallow, -Zlocation-detail=none
#   2. wasm-bindgen (names kept for step 3; `target_features` kept so externref glue is used)
#   3. snip functions that are unreachable on wasm32-unknown-unknown (std backtrace symbolization)
#      and sort functions by size (function order alone is worth ~1 KB brotli / ~22 KB gzip)
#   4. wasm-opt with a compression-friendly pass list (NOT -Oz: -Oz grows the brotli size)
#   5. minify the JS glue, precompress .br/.gz next to every shipped file
# --dev keeps the fast stable debug build without post-processing.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
OUT_DIR="${SCRIPT_DIR}/pkg"

PROFILE="wasm-release"
for arg in "$@"; do
  case "$arg" in
    --dev|--debug) PROFILE="debug" ;;
    --release) PROFILE="wasm-release" ;;
    *)
      echo "Usage: $0 [--release | --dev]" >&2
      exit 1
      ;;
  esac
done

TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
if [[ "${TARGET_DIR}" != /* ]]; then
  TARGET_DIR="${REPO_ROOT}/${TARGET_DIR}"
fi
TARGET="wasm32-unknown-unknown"
WASM_BINDGEN_BIN="${WASM_BINDGEN:-wasm-bindgen}"
WASM_OPT_BIN="${WASM_OPT:-wasm-opt}"

require() {
  command -v "$1" >/dev/null 2>&1 || { echo "Error: '$1' not found. $2" >&2; exit 1; }
}
require "${WASM_BINDGEN_BIN}" "Install with: cargo install wasm-bindgen-cli"

LOCKED_VER=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/"/, "", $3); print $3; exit}' "${REPO_ROOT}/Cargo.lock" || true)
CLI_VER=$("${WASM_BINDGEN_BIN}" --version | awk '{print $2}')
if [[ -n "${LOCKED_VER}" && "${CLI_VER}" != "${LOCKED_VER}" ]]; then
  echo "Error: wasm-bindgen CLI version (${CLI_VER}) != Cargo.lock wasm-bindgen (${LOCKED_VER})." >&2
  echo "Fix with: cargo install -f wasm-bindgen-cli --version ${LOCKED_VER}" >&2
  exit 1
fi

if [[ "${PROFILE}" == "debug" ]]; then
  rustup target list --installed | grep -qx "${TARGET}" || rustup target add "${TARGET}"
  echo "==> Building nve-web-shell (debug, stable, ${TARGET})"
  (cd "${REPO_ROOT}" && CARGO_TARGET_DIR="${TARGET_DIR}" cargo rustc --locked \
    -p nve-web-shell --lib --crate-type cdylib --target "${TARGET}")
  rm -rf "${OUT_DIR}"
  "${WASM_BINDGEN_BIN}" --target web --out-dir "${OUT_DIR}" --no-typescript \
    "${TARGET_DIR}/${TARGET}/debug/nve_web_shell.wasm"
  echo "==> Built ${OUT_DIR} (debug)"
  exit 0
fi

require "${WASM_OPT_BIN}" "Install binaryen (brew install binaryen)."
require wasm-tools "Install with: cargo install wasm-tools"
require brotli "Install with: brew install brotli"

# --- 1. Compile: pinned nightly, std rebuilt for size, panics compiled to `unreachable`. ---------
# -Cpanic=immediate-abort removes every panic message/location/formatting path (incl. std's).
# -Zfmt-debug=shallow keeps `{:?}` printing type/variant names (used by visible UI strings) while
# dropping recursive field formatting; -Zlocation-detail=none drops file/line/column constants.
WASM_TOOLCHAIN="${NVE_WASM_TOOLCHAIN:-nightly-2026-10-20}"
if ! rustup run "${WASM_TOOLCHAIN}" rustc --version >/dev/null 2>&1; then
  rustup toolchain install "${WASM_TOOLCHAIN}" --profile minimal --component rust-src
fi
rustup component list --toolchain "${WASM_TOOLCHAIN}" --installed | grep -q '^rust-src' \
  || rustup component add rust-src --toolchain "${WASM_TOOLCHAIN}"

echo "==> Building nve-web-shell (wasm-release, ${WASM_TOOLCHAIN}, -Zbuild-std, ${TARGET})"
(
  cd "${REPO_ROOT}"
  CARGO_TARGET_DIR="${TARGET_DIR}" \
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="-Zunstable-options -Cpanic=immediate-abort -Zfmt-debug=shallow -Zlocation-detail=none" \
  cargo "+${WASM_TOOLCHAIN}" rustc --locked \
    -p nve-web-shell --lib --crate-type cdylib --target "${TARGET}" --profile wasm-release \
    -Zbuild-std=std,panic_abort -Zbuild-std-features=optimize_for_size
)
WASM_INPUT="${TARGET_DIR}/${TARGET}/wasm-release/nve_web_shell.wasm"

# --- 2. Bindings. The name section is kept for step 3 and dropped by wasm-opt in step 4. ---------
rm -rf "${OUT_DIR}"
"${WASM_BINDGEN_BIN}" --target web --out-dir "${OUT_DIR}" --no-typescript \
  --remove-producers-section "${WASM_INPUT}"
WASM_OUT="${OUT_DIR}/nve_web_shell_bg.wasm"
JS_OUT="${OUT_DIR}/nve_web_shell.js"

# --- 3. Snip code that can never run on wasm32-unknown-unknown; order functions by size. ---------
# std::backtrace is `unsupported` on this target (capture never yields frames), but anyhow's
# Debug impl keeps Backtrace's Display -> rustc_demangle alive: ~17 KB raw / ~6 KB brotli.
# Step 4 must not run --reorder-functions (or -O2+/-Os/-Oz, which include it) after this.
python3 "${SCRIPT_DIR}/tools/wat-snip.py" "${WASM_OUT}" "${WASM_OUT}" \
  'rustc_demangle\[' 'std\[[0-9a-f]+\]::backtrace'

# --- 4. wasm-opt tuned for brotli size. -----------------------------------------------------------
# -Oz/-Os/-O2+ shrink raw bytes but grow brotli bytes (inlining, local-cse, memory-packing,
# coalesce/simplify-locals and reorder-functions all hurt compressibility of LLVM -Oz output).
# This list only runs passes that measured as brotli-positive.
WASM_OPT_PASSES=(
  --strip-debug --strip-producers --strip-target-features
  --duplicate-function-elimination --remove-unused-module-elements
  --vacuum --merge-blocks --dce --remove-unused-brs --optimize-instructions --precompute
  --reorder-locals --converge
)
"${WASM_OPT_BIN}" "${WASM_OPT_PASSES[@]}" "${WASM_OUT}" -o "${WASM_OUT}.opt"
mv "${WASM_OUT}.opt" "${WASM_OUT}"

# --- 5. Minify JS glue + precompress. --------------------------------------------------------------
if command -v npx >/dev/null 2>&1; then
  while IFS= read -r js; do
    npx -y terser@5 "${js}" --module --compress passes=3 --mangle --toplevel -o "${js}.min"
    mv "${js}.min" "${js}"
  done < <(find "${OUT_DIR}" -name '*.js')
else
  echo "==> Info: npx not found; shipping unminified JS glue (~2.7 KB brotli more)"
fi
while IFS= read -r f; do
  brotli -q 11 -f -k "${f}"
  gzip -9 -f -k "${f}"
done < <(find "${OUT_DIR}" -type f \( -name '*.wasm' -o -name '*.js' \))

bytes() { wc -c < "$1" | tr -d ' '; }
WASM_BR=$(bytes "${WASM_OUT}.br")
JS_BR=0
while IFS= read -r f; do JS_BR=$((JS_BR + $(bytes "${f}"))); done < <(find "${OUT_DIR}" -name '*.js.br')
TOTAL_BR=$((WASM_BR + JS_BR))
echo "==> wasm: $(bytes "${WASM_OUT}") raw, $(bytes "${WASM_OUT}.gz") gzip, ${WASM_BR} brotli"
echo "==> js:   $(bytes "${JS_OUT}") raw (entry), ${JS_BR} brotli (all JS)"
BUDGET="${WASM_BROTLI_BUDGET_BYTES:-245760}"
STATUS="PASS"; (( TOTAL_BR > BUDGET )) && STATUS="OVER BUDGET"
echo "==> Size budget (wasm+js brotli): ${TOTAL_BR} / ${BUDGET} bytes [${STATUS}]"
echo "==> Serve locally: python3 -m http.server 8080 -d hosts/web-shell"
