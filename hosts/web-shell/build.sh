#!/usr/bin/env bash
# Build the nve-web-shell browser bundles into hosts/web-shell/pkg/<bundle>/.
#
# Each bundle is published as pkg/<bundle>/<content-hash>/ (JS glue, wasm, wasm-bindgen snippets,
# precompressed .br/.gz) plus pkg/<bundle>/manifest.json naming the entry files in it. Everything
# under the hash directory is immutable; only index.html and the manifests change between builds
# (see README.md "Caching").
#
#   studio        `--no-default-features --features app-studio`        production Studio Workspace
#   kitchen-sink  `--no-default-features --features app-kitchen-sink`  production Kitchen Sink
#   dev           default features: both apps, NVP1 hot reload, pairing, popups, scene view, probes
#
# index.html loads pkg/<app> for `?app=<app>` and pkg/dev for `?dev`, `?ws=<url>`, `?pair=...`
# (or an explicit `?bundle=<name>`). Usage: build.sh [--release | --dev] [studio|kitchen-sink|dev ...]
# (default: all three bundles).
#
# Release (default) is a size pipeline tuned for the *brotli* bytes users download, measured in
# docs/research/wasm-size/toolchain.md:
#   1. pinned nightly + -Zbuild-std (std rebuilt with optimize_for_size), panic = "abort" with
#      messages and locations kept: production panics print `[NVE Web Panic] <message> at file:line`
#   2. wasm-bindgen (names kept for step 3; `target_features` kept so externref glue is used)
#   3. sort functions by size (function order alone is worth ~1 KB brotli / ~22 KB gzip)
#   4. wasm-opt with a compression-friendly pass list (NOT -Oz: -Oz grows the brotli size)
#   5. minify the JS glue, precompress .br/.gz next to every shipped file
# --dev keeps the fast stable debug build without post-processing (what `nve dev` and the E2E use).
# Both profiles publish through the same content-hashed layout.
# Each release bundle is checked against its own brotli budget (wasm + JS); over budget fails.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PKG_ROOT="${SCRIPT_DIR}/pkg"

usage() { echo "Usage: $0 [--release | --dev] [studio|kitchen-sink|issues|dev ...]" >&2; exit 1; }

PROFILE="wasm-release"
BUNDLES=()
for arg in "$@"; do
  case "$arg" in
    --dev|--debug) PROFILE="debug" ;;
    --release) PROFILE="wasm-release" ;;
    studio|kitchen-sink|issues|dev) BUNDLES+=("$arg") ;;
    *) usage ;;
  esac
done
(( ${#BUNDLES[@]} )) || BUNDLES=(studio kitchen-sink dev)

# Cargo feature selection per bundle.
bundle_features() {
  case "$1" in
    studio) echo "--no-default-features --features app-studio" ;;
    kitchen-sink) echo "--no-default-features --features app-kitchen-sink" ;;
    issues)
      if [[ "${NVE_ISSUES_PROBES:-0}" == "1" ]]; then
        echo "--no-default-features --features app-issues,issues-probes"
      else
        echo "--no-default-features --features app-issues"
      fi
      ;;
    dev) echo "" ;;
  esac
}

# Brotli (wasm + JS) budgets, ~3% above measured sizes.
bundle_budget() {
  case "$1" in
    studio) echo "${WASM_BROTLI_BUDGET_STUDIO:-316416}" ;;
    kitchen-sink) echo "${WASM_BROTLI_BUDGET_KITCHEN_SINK:-315392}" ;;
    issues) echo "${WASM_BROTLI_BUDGET_ISSUES:-255000}" ;;
    dev) echo "${WASM_BROTLI_BUDGET_DEV:-485376}" ;;
  esac
}

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

# A fresh staging directory next to pkg/<bundle> (same filesystem, so publishing is a rename).
new_stage_dir() {
  mkdir -p "${PKG_ROOT}"
  rm -rf "${PKG_ROOT}/.$1.staging"
  mkdir "${PKG_ROOT}/.$1.staging"
  echo "${PKG_ROOT}/.$1.staging"
}

# Publishes a finished staging directory as pkg/<bundle>/<hash>/ + pkg/<bundle>/manifest.json.
# The hash covers every shipped file's path and bytes (not the derived .br/.gz), so any change to
# the glue, the wasm or a snippet yields a new directory. Sets PUBLISHED_HASH.
publish_bundle() {
  local stage_dir="$1" out_dir="$2"
  PUBLISHED_HASH=$(cd "${stage_dir}" \
    && find . -type f ! -name '*.br' ! -name '*.gz' -print0 | LC_ALL=C sort -z \
    | xargs -0 shasum -a 256 | shasum -a 256 | cut -c1-16)
  rm -rf "${out_dir}"
  mkdir -p "${out_dir}"
  mv "${stage_dir}" "${out_dir}/${PUBLISHED_HASH}"
  printf '{"js":"%s/nve_web_shell.js","wasm":"%s/nve_web_shell_bg.wasm"}\n' \
    "${PUBLISHED_HASH}" "${PUBLISHED_HASH}" > "${out_dir}/manifest.json"
}

# Ensure generated TypeScript ABI/Wire bindings exist on a fresh checkout.
if [[ ! -f "${SCRIPT_DIR}/nve_core_abi.generated.ts" \
   || ! -f "${SCRIPT_DIR}/nve_scene_view.generated.ts" \
   || ! -f "${SCRIPT_DIR}/nve_wire_codecs.generated.ts" ]]; then
  echo "==> Generating NVE ABI + Wire Codec bindings (nve-cli codegen abi)"
  (cd "${REPO_ROOT}" && CARGO_TARGET_DIR="${TARGET_DIR}" cargo run --locked -p nve-cli -- codegen abi)
fi

if [[ "${PROFILE}" == "debug" ]]; then
  rustup target list --installed | grep -qx "${TARGET}" || rustup target add "${TARGET}"
  for bundle in "${BUNDLES[@]}"; do
    out_dir="${PKG_ROOT}/${bundle}"
    echo "==> Building nve-web-shell [${bundle}] (debug, stable, ${TARGET})"
    # shellcheck disable=SC2046 # feature flags are intentionally word-split
    (cd "${REPO_ROOT}" && CARGO_TARGET_DIR="${TARGET_DIR}" cargo rustc --locked \
      -p nve-web-shell $(bundle_features "${bundle}") --lib --crate-type cdylib --target "${TARGET}")
    stage_dir="$(new_stage_dir "${bundle}")"
    "${WASM_BINDGEN_BIN}" --target web --out-dir "${stage_dir}" --no-typescript \
      "${TARGET_DIR}/${TARGET}/debug/nve_web_shell.wasm"
    publish_bundle "${stage_dir}" "${out_dir}"
    echo "==> Built ${out_dir}/${PUBLISHED_HASH} (debug)"
  done
  exit 0
fi

require "${WASM_OPT_BIN}" "Install binaryen (brew install binaryen)."
require wasm-tools "Install with: cargo install wasm-tools"
require brotli "Install with: brew install brotli"

# --- 1. Compile: pinned nightly, std rebuilt for size (optimize_for_size), panic = "abort". ------
# Panic messages, `{:?}` formatting and file:line locations are kept on purpose: a production panic
# must be diagnosable from the console (`[NVE Web Panic] ...`). Stripping them with
# -Cpanic=immediate-abort -Zfmt-debug=shallow -Zlocation-detail=none saved ~23 KB brotli; see
# docs/research/wasm-size/toolchain.md for the decision.
WASM_TOOLCHAIN="${NVE_WASM_TOOLCHAIN:-nightly-2026-10-20}"
if ! rustup run "${WASM_TOOLCHAIN}" rustc --version >/dev/null 2>&1; then
  rustup toolchain install "${WASM_TOOLCHAIN}" --profile minimal --component rust-src
fi
rustup component list --toolchain "${WASM_TOOLCHAIN}" --installed | grep -q '^rust-src' \
  || rustup component add rust-src --toolchain "${WASM_TOOLCHAIN}"

bytes() { wc -c < "$1" | tr -d ' '; }

build_release_bundle() {
  local bundle="$1" out_dir="${PKG_ROOT}/$1"
  local WASM_INPUT WASM_OUT JS_OUT WASM_BR JS_BR TOTAL_BR BUDGET STATUS stage_dir
  echo "==> Building nve-web-shell [${bundle}] (wasm-release, ${WASM_TOOLCHAIN}, -Zbuild-std, ${TARGET})"
  # shellcheck disable=SC2046 # feature flags are intentionally word-split
  (
    cd "${REPO_ROOT}"
    CARGO_TARGET_DIR="${TARGET_DIR}" \
    cargo "+${WASM_TOOLCHAIN}" rustc --locked \
      -p nve-web-shell $(bundle_features "${bundle}") --lib --crate-type cdylib --target "${TARGET}" \
      --profile wasm-release -Zbuild-std=std,panic_abort -Zbuild-std-features=optimize_for_size
  )
  WASM_INPUT="${TARGET_DIR}/${TARGET}/wasm-release/nve_web_shell.wasm"

  # --- 2. Bindings. The name section is kept for step 3 and dropped by wasm-opt in step 4. ---------
  stage_dir="$(new_stage_dir "${bundle}")"
  "${WASM_BINDGEN_BIN}" --target web --out-dir "${stage_dir}" --no-typescript \
    --remove-producers-section "${WASM_INPUT}"
  WASM_OUT="${stage_dir}/nve_web_shell_bg.wasm"
  JS_OUT="${stage_dir}/nve_web_shell.js"

  # --- 3. Order functions by size. -------------------------------------------------------------------
  # Step 4 must not run --reorder-functions (or -O2+/-Os/-Oz, which include it) after this.
  python3 "${SCRIPT_DIR}/tools/wasm-sort-funcs.py" "${WASM_OUT}" "${WASM_OUT}"

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
    done < <(find "${stage_dir}" -name '*.js')
  else
    echo "==> Info: npx not found; shipping unminified JS glue (~2.7 KB brotli more)"
  fi
  while IFS= read -r f; do
    brotli -q 11 -f -k "${f}"
    gzip -9 -f -k "${f}"
  done < <(find "${stage_dir}" -type f \( -name '*.wasm' -o -name '*.js' \))

  publish_bundle "${stage_dir}" "${out_dir}"
  WASM_OUT="${out_dir}/${PUBLISHED_HASH}/nve_web_shell_bg.wasm"
  JS_OUT="${out_dir}/${PUBLISHED_HASH}/nve_web_shell.js"
  WASM_BR=$(bytes "${WASM_OUT}.br")
  JS_BR=0
  while IFS= read -r f; do JS_BR=$((JS_BR + $(bytes "${f}"))); done < <(find "${out_dir}" -name '*.js.br')
  TOTAL_BR=$((WASM_BR + JS_BR))
  echo "==> wasm: $(bytes "${WASM_OUT}") raw, $(bytes "${WASM_OUT}.gz") gzip, ${WASM_BR} brotli"
  echo "==> js:   $(bytes "${JS_OUT}") raw (entry), ${JS_BR} brotli (all JS)"
  BUDGET=$(bundle_budget "${bundle}")
  STATUS="PASS"; (( TOTAL_BR > BUDGET )) && STATUS="WARN: OVER BUDGET" && OVER_BUDGET+=("${bundle}")
  echo "==> [${bundle}] published ${out_dir}/${PUBLISHED_HASH} (manifest.json)"
  echo "==> [${bundle}] size budget (wasm+js brotli): ${TOTAL_BR} / ${BUDGET} bytes [${STATUS}]"
}

OVER_BUDGET=()
for bundle in "${BUNDLES[@]}"; do
  build_release_bundle "${bundle}"
done
echo "==> Serve locally: python3 -m http.server 8080 -d hosts/web-shell"
if (( ${#OVER_BUDGET[@]} )); then
  echo "Warning: over brotli budget (set WASM_BUDGET_STRICT=1 to fail): ${OVER_BUDGET[*]}" >&2
  if [[ "${WASM_BUDGET_STRICT:-0}" == "1" ]]; then
    exit 1
  fi
fi
