#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
OUT_DIR="${SCRIPT_DIR}/pkg"

PROFILE="wasm-release"
CARGO_PROFILE_ARGS=(--profile wasm-release)
for arg in "$@"; do
  case "$arg" in
    --dev|--debug)
      PROFILE="debug"
      CARGO_PROFILE_ARGS=()
      ;;
    --release)
      PROFILE="wasm-release"
      CARGO_PROFILE_ARGS=(--profile wasm-release)
      ;;
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

WASM_BINDGEN_BIN="${WASM_BINDGEN:-wasm-bindgen}"

if ! rustup target list --installed | grep -qx "wasm32-unknown-unknown"; then
  echo "Installing rustup target wasm32-unknown-unknown..."
  rustup target add wasm32-unknown-unknown
fi

if ! command -v "${WASM_BINDGEN_BIN}" >/dev/null 2>&1; then
  echo "Error: wasm-bindgen CLI ('${WASM_BINDGEN_BIN}') not found." >&2
  echo "Install with: cargo install wasm-bindgen-cli" >&2
  exit 1
fi

LOCKED_VER=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/"/, "", $3); print $3; exit}' "${REPO_ROOT}/Cargo.lock" || true)
CLI_VER=$("${WASM_BINDGEN_BIN}" --version | awk '{print $2}')
if [[ -n "${LOCKED_VER}" && "${CLI_VER}" != "${LOCKED_VER}" ]]; then
  echo "Error: wasm-bindgen CLI version (${CLI_VER}) != Cargo.lock wasm-bindgen (${LOCKED_VER})." >&2
  echo "Fix with: cargo install -f wasm-bindgen-cli --version ${LOCKED_VER}" >&2
  exit 1
fi

echo "==> Building nve-web-shell (${PROFILE}, wasm32-unknown-unknown)"
(
  cd "${REPO_ROOT}"
  CARGO_TARGET_DIR="${TARGET_DIR}" cargo rustc --locked \
    -p nve-web-shell \
    --lib \
    --crate-type cdylib \
    --target wasm32-unknown-unknown \
    "${CARGO_PROFILE_ARGS[@]}"
)

WASM_INPUT="${TARGET_DIR}/wasm32-unknown-unknown/${PROFILE}/nve_web_shell.wasm"
mkdir -p "${OUT_DIR}"

echo "==> Running ${WASM_BINDGEN_BIN} --target web -> ${OUT_DIR}"
"${WASM_BINDGEN_BIN}" \
  --target web \
  --out-dir "${OUT_DIR}" \
  --no-typescript \
  "${WASM_INPUT}"

WASM_OUT="${OUT_DIR}/nve_web_shell_bg.wasm"
WIZER_BIN="${WIZER:-wizer}"
if command -v "${WIZER_BIN}" >/dev/null 2>&1; then
  PRE_WIZER_BYTES=$(wc -c < "${WASM_OUT}" | tr -d ' ')
  WIZER_TMP="${WASM_OUT}.wizer.tmp"
  echo "==> Running ${WIZER_BIN} --allow-wasi=false to pre-initialize Frame-0 VDOM (${PRE_WIZER_BYTES} bytes)"
  if "${WIZER_BIN}" --allow-wasi=false "${WASM_OUT}" -o "${WIZER_TMP}"; then
    mv "${WIZER_TMP}" "${WASM_OUT}"
    echo "==> Wizer pre-initialization succeeded (wizer.initialize baked into linear memory)"
  else
    rm -f "${WIZER_TMP}"
    echo "Warning: ${WIZER_BIN} pre-initialization failed; keeping standard wasm-bindgen output" >&2
  fi
else
  echo "==> Info: '${WIZER_BIN}' not found on PATH; skipping build-time Frame-0 pre-initialization (install with: cargo install wizer --all-features)"
fi

WASM_OPT_BIN="${WASM_OPT:-wasm-opt}"
if [[ "${PROFILE}" != "debug" ]]; then
  if command -v "${WASM_OPT_BIN}" >/dev/null 2>&1; then
    PRE_OPT_BYTES=$(wc -c < "${WASM_OUT}" | tr -d ' ')
    WASM_OPT_TMP="${WASM_OUT}.opt.tmp"
    echo "==> Running ${WASM_OPT_BIN} -Oz (${PRE_OPT_BYTES} bytes)"
    if "${WASM_OPT_BIN}" -Oz "${WASM_OUT}" -o "${WASM_OPT_TMP}"; then
      mv "${WASM_OPT_TMP}" "${WASM_OUT}"
      POST_OPT_BYTES=$(wc -c < "${WASM_OUT}" | tr -d ' ')
      echo "==> ${WASM_OPT_BIN} -Oz reduced WASM: ${PRE_OPT_BYTES} -> ${POST_OPT_BYTES} bytes"
    else
      rm -f "${WASM_OPT_TMP}"
      echo "Warning: ${WASM_OPT_BIN} -Oz failed; keeping unoptimized wasm-bindgen output" >&2
    fi
  else
    echo "==> Info: '${WASM_OPT_BIN}' not found on PATH; skipping wasm-opt -Oz (install binaryen for further size reduction)"
  fi
fi

RAW_BYTES=$(wc -c < "${WASM_OUT}" | tr -d ' ')
RAW_KIB=$(awk -v b="${RAW_BYTES}" 'BEGIN { printf "%.1f", b / 1024.0 }')
echo "==> Built ${WASM_OUT}: ${RAW_BYTES} bytes (${RAW_KIB} KiB)"
GZIP_BYTES=""
if command -v gzip >/dev/null 2>&1; then
  GZIP_BYTES=$(gzip -9 -c "${WASM_OUT}" | wc -c | tr -d ' ')
  GZIP_KIB=$(awk -v b="${GZIP_BYTES}" 'BEGIN { printf "%.1f", b / 1024.0 }')
  echo "==> Gzipped size: ${GZIP_BYTES} bytes (${GZIP_KIB} KiB)"
fi

if [[ "${PROFILE}" != "debug" ]]; then
  RAW_BUDGET_BYTES="${WASM_RAW_BUDGET_BYTES:-1048576}"
  GZIP_BUDGET_BYTES="${WASM_GZIP_BUDGET_BYTES:-358400}"
  RAW_STATUS="PASS"
  if (( RAW_BYTES > RAW_BUDGET_BYTES )); then
    RAW_STATUS="OVER BUDGET"
  fi
  echo "==> Size budget (raw):  ${RAW_BYTES} / ${RAW_BUDGET_BYTES} bytes [${RAW_STATUS}]"
  if [[ -n "${GZIP_BYTES}" ]]; then
    GZIP_STATUS="PASS"
    if (( GZIP_BYTES > GZIP_BUDGET_BYTES )); then
      GZIP_STATUS="OVER BUDGET"
    fi
    echo "==> Size budget (gzip): ${GZIP_BYTES} / ${GZIP_BUDGET_BYTES} bytes [${GZIP_STATUS}]"
  fi
fi
echo "==> Serve locally: python3 -m http.server 8080 -d hosts/web-shell"
