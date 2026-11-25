#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
OUT_DIR="${SCRIPT_DIR}/pkg"

PROFILE="release"
CARGO_PROFILE_FLAG="--release"
for arg in "$@"; do
  case "$arg" in
    --dev|--debug)
      PROFILE="debug"
      CARGO_PROFILE_FLAG=""
      ;;
    --release)
      PROFILE="release"
      CARGO_PROFILE_FLAG="--release"
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
    ${CARGO_PROFILE_FLAG}
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

RAW_BYTES=$(wc -c < "${WASM_OUT}" | tr -d ' ')
echo "==> Built ${WASM_OUT} (${RAW_BYTES} bytes)"
if command -v gzip >/dev/null 2>&1; then
  GZIP_BYTES=$(gzip -c "${WASM_OUT}" | wc -c | tr -d ' ')
  echo "==> Gzipped size: ${GZIP_BYTES} bytes"
fi
echo "==> Serve locally: python3 -m http.server 8080 -d hosts/web-shell"
