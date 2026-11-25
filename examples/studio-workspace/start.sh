#!/usr/bin/env bash
# Launch all locally available NVE hosts side-by-side in parallel.
# Usage:
#   ./examples/studio-workspace/start.sh                 # launches studio-workspace on all hosts
#   ./examples/studio-workspace/start.sh --counter       # launches counter example instead
#   ./examples/studio-workspace/start.sh --no-web        # skip web WASM dev server
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

APP_PKG="nve-studio-workspace"
APP_LIB="nve_studio_workspace"
WEB_APP="studio"
START_WEB=1
WEB_PORT="${NVE_WEB_PORT:-8080}"

for arg in "$@"; do
  case "$arg" in
    --counter)
      APP_PKG="nve-example-counter"
      APP_LIB="nve_example_counter"
      WEB_APP="counter"
      ;;
    --studio)
      APP_PKG="nve-studio-workspace"
      APP_LIB="nve_studio_workspace"
      WEB_APP="studio"
      ;;
    --no-web)
      START_WEB=0
      ;;
    -h|--help)
      echo "Usage: $0 [--studio|--counter] [--no-web]"
      exit 0
      ;;
    *)
      echo "Unknown option: $arg" >&2
      exit 2
      ;;
  esac
done

case "$(uname -s)" in
  Darwin) DYLIB_PATH="$ROOT_DIR/target/debug/lib${APP_LIB}.dylib" ;;
  Linux)  DYLIB_PATH="$ROOT_DIR/target/debug/lib${APP_LIB}.so" ;;
  *)      DYLIB_PATH="$ROOT_DIR/target/debug/${APP_LIB}.dll" ;;
esac

echo "==> Building $APP_PKG cdylib + Skia desktop shell..."
cargo build -p "$APP_PKG"
cargo build -p nve-skia-shell

PIDS=()
cleanup() {
  echo ""
  echo "==> Stopping all launched NVE hosts..."
  for pid in "${PIDS[@]:-}"; do
    kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
}
trap cleanup INT TERM EXIT

launch() {
  local label="$1"
  shift
  echo "==> Starting [$label]: $*"
  "$@" &
  PIDS+=("$!")
}

# 1. Native macOS AppKit host
if [[ "$(uname -s)" == "Darwin" ]]; then
  if [[ ! -x "$ROOT_DIR/hosts/macos-shell/build/nve_macos_shell" ]]; then
    echo "==> Building hosts/macos-shell..."
    cmake -S hosts/macos-shell -B hosts/macos-shell/build -G Ninja \
      ${RN_REACT_COMMON_DIR:+-DRN_REACT_COMMON_DIR="$RN_REACT_COMMON_DIR"}
    cmake --build hosts/macos-shell/build --target nve_macos_shell
  fi
  launch "macOS AppKit" "$ROOT_DIR/hosts/macos-shell/build/nve_macos_shell" --dylib "$DYLIB_PATH"
fi

# 2. Native Qt 6 host (when Qt6 is installed)
if command -v qmake6 >/dev/null 2>&1 || pkg-config --exists Qt6Widgets 2>/dev/null; then
  if [[ ! -x "$ROOT_DIR/hosts/qt-shell/build/nve_qt_shell" ]]; then
    echo "==> Building hosts/qt-shell..."
    cmake -S hosts/qt-shell -B hosts/qt-shell/build -G Ninja \
      ${RN_REACT_COMMON_DIR:+-DRN_REACT_COMMON_DIR="$RN_REACT_COMMON_DIR"}
    cmake --build hosts/qt-shell/build --target nve_qt_shell
  fi
  launch "Qt 6" "$ROOT_DIR/hosts/qt-shell/build/nve_qt_shell" --dylib "$DYLIB_PATH" --width 960 --height 640
else
  echo "==> Skipping [Qt 6] (Qt6 not installed)"
fi

# 3. Native GTK4 host (when gtk4 is installed)
if pkg-config --exists gtk4 2>/dev/null; then
  if [[ ! -x "$ROOT_DIR/hosts/gtk-shell/build/nve_gtk_shell" ]]; then
    echo "==> Building hosts/gtk-shell..."
    cmake -S hosts/gtk-shell -B hosts/gtk-shell/build -G Ninja \
      ${RN_REACT_COMMON_DIR:+-DRN_REACT_COMMON_DIR="$RN_REACT_COMMON_DIR"}
    cmake --build hosts/gtk-shell/build --target nve_gtk_shell
  fi
  launch "GTK4" "$ROOT_DIR/hosts/gtk-shell/build/nve_gtk_shell" --dylib "$DYLIB_PATH" --width 960 --height 640
else
  echo "==> Skipping [GTK4] (gtk4 not installed via pkg-config)"
fi

# 4. Cross-platform Skia / winit Desktop Shell
launch "Skia Desktop" "$ROOT_DIR/target/debug/nve-skia-shell" "$DYLIB_PATH" --no-ws

# 5. Web WASM host
if [[ "$START_WEB" -eq 1 ]]; then
  if [[ ! -f "$ROOT_DIR/hosts/web-shell/pkg/${WEB_APP}/manifest.json" ]]; then
    echo "==> Building hosts/web-shell WASM bundle..."
    "$ROOT_DIR/hosts/web-shell/build.sh"
  fi
  launch "Web WASM (http://127.0.0.1:${WEB_PORT}/?app=${WEB_APP})" \
    python3 -m http.server "$WEB_PORT" --bind 127.0.0.1 --directory "$ROOT_DIR/hosts/web-shell"
fi

# 6. Optional mobile simulators (when standalone launcher scripts are present)
if [[ -x "$ROOT_DIR/hosts/ios-shell/run-simulator.sh" && "${NVE_START_IOS:-0}" == "1" ]]; then
  launch "iOS Simulator" "$ROOT_DIR/hosts/ios-shell/run-simulator.sh" "$APP_PKG"
fi
if [[ -x "$ROOT_DIR/hosts/android-shell/run-emulator.sh" && "${NVE_START_ANDROID:-0}" == "1" ]]; then
  launch "Android Emulator" "$ROOT_DIR/hosts/android-shell/run-emulator.sh" "$APP_PKG"
fi

echo ""
echo "All ${#PIDS[@]} hosts running! Press Ctrl-C to stop them all."
wait
