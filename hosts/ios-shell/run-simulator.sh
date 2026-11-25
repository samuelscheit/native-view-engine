#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

APP_PKG="nve-studio-workspace"
BUILD_ONLY=0
HEADLESS=0
RELEASE=0
DEVICE_QUERY=""
SCREENSHOT_PATH=""
BUNDLE_ID="dev.nve.ios-shell"
IOS_MIN_VERSION="17.0"
RUST_TARGET="aarch64-apple-ios-sim"

usage() {
  cat <<'EOF'
Usage: ./hosts/ios-shell/run-simulator.sh [app-crate] [options]

Builds a standalone zero-Xcode-project iOS Simulator .app bundle embedding any
NVE Rust cdylib crate (default: nve-studio-workspace), then installs and
launches it in the iOS Simulator.

Positional arguments:
  app-crate               Cargo package name (e.g. nve-studio-workspace,
                          nve-example-counter). Defaults to nve-studio-workspace.

Options:
  --build-only            Build and sign hosts/ios-shell/build/NveIosShell.app
                          without booting, installing, or launching Simulator.
  --headless              Do not open the Simulator GUI app when launching.
  --device <udid-or-name> Target a specific iOS Simulator UDID or device name.
  --screenshot <out.png>  Wait ~2s after launch and capture a Simulator PNG.
  --release               Build the Rust crate in release profile.
  -h, --help              Show this help message.

Environment:
  RN_REACT_COMMON_DIR     Optional path to react-native/packages/react-native/ReactCommon
                          to compile and link React Native Yoga (NVE_WITH_RN_YOGA=1).
  NVE_ENABLE_CAMERA       Set to 1 to opt in to packages/modules/camera/apple and link
                          AVFoundation/CoreMedia/CoreVideo (default: 0).
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --build-only)
      BUILD_ONLY=1
      shift
      ;;
    --headless)
      HEADLESS=1
      shift
      ;;
    --release)
      RELEASE=1
      shift
      ;;
    --device)
      [[ $# -ge 2 ]] || { echo "error: --device requires an argument" >&2; exit 2; }
      DEVICE_QUERY="$2"
      shift 2
      ;;
    --screenshot)
      [[ $# -ge 2 ]] || { echo "error: --screenshot requires an argument" >&2; exit 2; }
      SCREENSHOT_PATH="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      APP_PKG="$1"
      shift
      ;;
  esac
done

APP_LIB="${APP_PKG//-/_}"
PROFILE_DIR="debug"
CARGO_PROFILE_ARGS=()
OPT_FLAG="-O0"
if [[ "$RELEASE" -eq 1 ]]; then
  PROFILE_DIR="release"
  CARGO_PROFILE_ARGS+=(--release)
  OPT_FLAG="-O2"
fi

BUILD_DIR="$SCRIPT_DIR/build"
OBJ_DIR="$BUILD_DIR/obj"
APP_BUNDLE="$BUILD_DIR/NveIosShell.app"
APP_BIN="$APP_BUNDLE/NveIosShell"

mkdir -p "$OBJ_DIR" "$APP_BUNDLE"

echo "==> [1/4] Building Rust crate '$APP_PKG' for $RUST_TARGET ($PROFILE_DIR)..."
if ! rustup target list --installed | grep -qx "$RUST_TARGET"; then
  echo "error: Rust target '$RUST_TARGET' is not installed. Run: rustup target add $RUST_TARGET" >&2
  exit 1
fi

(
  cd "$REPO_ROOT"
  cargo build -p "$APP_PKG" --lib --target "$RUST_TARGET" "${CARGO_PROFILE_ARGS[@]}"
)

CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
RUST_DYLIB="$CARGO_TARGET_DIR/$RUST_TARGET/$PROFILE_DIR/lib${APP_LIB}.dylib"
if [[ ! -f "$RUST_DYLIB" ]]; then
  echo "error: expected Rust cdylib not found at $RUST_DYLIB" >&2
  exit 1
fi

echo "==> [2/4] Compiling standalone iOS Simulator host binary..."
CLANGXX=(xcrun --sdk iphonesimulator clang++)
COMMON_FLAGS=(
  -std=c++20
  -target "arm64-apple-ios${IOS_MIN_VERSION}-simulator"
  "$OPT_FLAG"
  -g
  -Wall
  -Wextra
  -I"$REPO_ROOT/cxx/nve-fabric-bridge/include"
  -I"$REPO_ROOT/cxx/nve-fabric-bridge/src"
  -I"$REPO_ROOT/hosts/apple-common"
  -I"$REPO_ROOT/hosts/ios-shell"
)

YOGA_SOURCES=()
YOGA_TAG="standalone"
RN_DIR="${RN_REACT_COMMON_DIR:-${RN_COMMON_DIR:-}}"
if [[ -n "$RN_DIR" && -d "$RN_DIR/yoga/yoga" ]]; then
  echo "    Linking React Native Yoga from $RN_DIR/yoga"
  COMMON_FLAGS+=(-DNVE_WITH_RN_YOGA=1 -I"$RN_DIR/yoga")
  YOGA_TAG="rn-yoga"
  while IFS= read -r yoga_cpp; do
    YOGA_SOURCES+=("$yoga_cpp")
  done < <(find "$RN_DIR/yoga/yoga" -name '*.cpp' | sort)
fi

CAMERA_ENABLED="${NVE_ENABLE_CAMERA:-0}"
CAMERA_TAG="no-camera"
CAMERA_LINK_FLAGS=()
CAMERA_MM_SOURCES=()
if [[ "$CAMERA_ENABLED" == "1" ]]; then
  echo "    Enabling opt-in packages/modules/camera/apple"
  COMMON_FLAGS+=(-DNVE_ENABLE_CAMERA=1 -I"$REPO_ROOT/packages/modules/camera/apple")
  CAMERA_TAG="camera"
  CAMERA_MM_SOURCES+=("$REPO_ROOT/packages/modules/camera/apple/NveAppleCamera.mm")
  CAMERA_LINK_FLAGS+=(-framework AVFoundation -framework CoreMedia -framework CoreVideo)
fi

CONFIG_STAMP="$BUILD_DIR/.build-config"
CURRENT_CONFIG="${OPT_FLAG}|${IOS_MIN_VERSION}|${YOGA_TAG}|${RN_DIR}|${CAMERA_TAG}"
if [[ ! -f "$CONFIG_STAMP" ]] || [[ "$(cat "$CONFIG_STAMP")" != "$CURRENT_CONFIG" ]]; then
  rm -rf "$OBJ_DIR"
  mkdir -p "$OBJ_DIR"
  printf '%s' "$CURRENT_CONFIG" > "$CONFIG_STAMP"
fi

HEADER_DEPS=(
  "$REPO_ROOT/cxx/nve-fabric-bridge/include/nve_abi.generated.h"
  "$REPO_ROOT/cxx/nve-fabric-bridge/include/nve_scene_abi.generated.h"
  "$REPO_ROOT/cxx/nve-fabric-bridge/include/nve_draw_list.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/include/nve_host_runtime.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/include/detail/host_runtime_dylib.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/include/detail/host_runtime_reconciler.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/include/detail/host_runtime_session.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/include/nve_native_view.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/include/nve_pair_client.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/include/nve_rn_component.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/include/nve_value.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/command_decoder.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/command_encoder.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/event_ring_writer.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/mock_jsi_arena.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/mock_jsi_runtime.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/mock_jsi_runtime_helpers.hpp"
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/mock_jsi_recorder.hpp"
  "$REPO_ROOT/hosts/apple-common/NveAppleCanvasView.h"
  "$REPO_ROOT/hosts/apple-common/NveAppleImageCache.h"
  "$REPO_ROOT/hosts/apple-common/NveAppleValueBridge.h"
  "$REPO_ROOT/hosts/apple-common/NveAppleSupport.h"
  "$REPO_ROOT/hosts/apple-common/NveAppleFabricHost.h"
  "$REPO_ROOT/hosts/apple-common/NveAppleComponentHost.h"
  "$REPO_ROOT/hosts/apple-common/NveAppleComponentHostInternal.h"
  "$REPO_ROOT/hosts/apple-common/NveAppleComponentViewBase.h"
  "$REPO_ROOT/hosts/apple-common/NveAppleNativeViews.h"
  "$REPO_ROOT/hosts/apple-common/NveAppleSceneReconciler.h"
  "$REPO_ROOT/hosts/ios-shell/NveFabricHost.h"
  "$REPO_ROOT/hosts/ios-shell/RCTNveComponentView.h"
)
if [[ "$CAMERA_ENABLED" == "1" ]]; then
  HEADER_DEPS+=("$REPO_ROOT/packages/modules/camera/apple/NveAppleCamera.h")
fi

needs_rebuild() {
  local obj="$1"
  local src="$2"
  if [[ ! -f "$obj" ]] || [[ "$src" -nt "$obj" ]]; then
    return 0
  fi
  for hdr in "${HEADER_DEPS[@]}"; do
    if [[ -f "$hdr" && "$hdr" -nt "$obj" ]]; then
      return 0
    fi
  done
  return 1
}

OBJ_FILES=()

compile_unit() {
  local src="$1"
  local kind="$2" # cpp or mm
  local rel_key
  rel_key="$(printf '%s' "$src" | tr '/: ' '___')"
  local obj="$OBJ_DIR/${rel_key}.o"
  OBJ_FILES+=("$obj")
  if needs_rebuild "$obj" "$src"; then
    local extra_flags=()
    if [[ "$kind" == "mm" ]]; then
      extra_flags+=(-fobjc-arc)
    fi
    "${CLANGXX[@]}" "${COMMON_FLAGS[@]}" "${extra_flags[@]}" -c "$src" -o "$obj"
  fi
}

for src in \
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/shadow_tree.cpp" \
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/yoga_layout.cpp" \
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/flexbox_solver.cpp" \
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/scene_publisher.cpp" \
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/fabric_c_api.cpp" \
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/nve_rn_component.cpp" \
  "$REPO_ROOT/cxx/nve-fabric-bridge/src/pair_client.cpp"
do
  compile_unit "$src" cpp
done

for src in "${YOGA_SOURCES[@]+"${YOGA_SOURCES[@]}"}"; do
  compile_unit "$src" cpp
done

for src in \
  "$REPO_ROOT/hosts/apple-common/NveAppleCanvasView.mm" \
  "$REPO_ROOT/hosts/apple-common/NveAppleImageCache.mm" \
  "$REPO_ROOT/hosts/apple-common/NveAppleValueBridge.mm" \
  "$REPO_ROOT/hosts/apple-common/NveAppleSupport.mm" \
  "$REPO_ROOT/hosts/apple-common/NveAppleFabricHost.mm" \
  "$REPO_ROOT/hosts/apple-common/NveAppleComponentHost.mm" \
  "$REPO_ROOT/hosts/apple-common/NveAppleComponentHost+Session.mm" \
  "$REPO_ROOT/hosts/apple-common/NveAppleComponentViewBase.mm" \
  "$REPO_ROOT/hosts/apple-common/NveAppleNativeViews.mm" \
  "$REPO_ROOT/hosts/apple-common/NveAppleSceneReconciler.mm" \
  "$REPO_ROOT/hosts/ios-shell/NveFabricHost.mm" \
  "$REPO_ROOT/hosts/ios-shell/RCTNveComponentView.mm" \
  "$REPO_ROOT/hosts/ios-shell/main.mm"
do
  compile_unit "$src" mm
done

for src in "${CAMERA_MM_SOURCES[@]+"${CAMERA_MM_SOURCES[@]}"}"; do
  compile_unit "$src" mm
done

"${CLANGXX[@]}" \
  -std=c++20 \
  -fobjc-arc \
  -target "arm64-apple-ios${IOS_MIN_VERSION}-simulator" \
  "${OBJ_FILES[@]}" \
  "${CAMERA_LINK_FLAGS[@]+"${CAMERA_LINK_FLAGS[@]}"}" \
  -framework UIKit \
  -framework Foundation \
  -framework QuartzCore \
  -framework CoreGraphics \
  -framework WebKit \
  -o "$APP_BIN"

echo "==> [3/4] Assembling and ad-hoc signing $APP_BUNDLE..."
cp -f "$SCRIPT_DIR/Info.plist" "$APP_BUNDLE/Info.plist"
cp -f "$RUST_DYLIB" "$APP_BUNDLE/libnve_app.dylib"
chmod u+w "$APP_BUNDLE/libnve_app.dylib"
install_name_tool -id "@executable_path/libnve_app.dylib" "$APP_BUNDLE/libnve_app.dylib"
codesign --force --sign - "$APP_BUNDLE/libnve_app.dylib"
codesign --force --sign - "$APP_BUNDLE"

if [[ "$BUILD_ONLY" -eq 1 ]]; then
  echo "Built standalone iOS Simulator bundle: $APP_BUNDLE"
  exit 0
fi

echo "==> [4/4] Resolving iOS Simulator device, installing, and launching $BUNDLE_ID..."
read -r SIM_UDID SIM_NAME SIM_STATE < <(
  python3 - "$DEVICE_QUERY" <<'PY'
import json, subprocess, sys

query = sys.argv[1].strip()
raw = subprocess.check_output(
    ["xcrun", "simctl", "list", "devices", "available", "-j"], text=True
)
data = json.loads(raw)

candidates = []
for runtime, devices in data.get("devices", {}).items():
    if "iOS" not in runtime:
        continue
    for d in devices:
        if not d.get("isAvailable", False):
            continue
        candidates.append((runtime, d))

if not candidates:
    sys.stderr.write("error: no available iOS Simulator devices found\n")
    sys.exit(1)

candidates.sort(key=lambda item: item[0])

selected = None
if query:
    for _, d in candidates:
        if d.get("udid") == query or d.get("name") == query:
            selected = d
            break
    if selected is None:
        q_low = query.lower()
        for _, d in candidates:
            if q_low in d.get("name", "").lower():
                selected = d
                break
    if selected is None:
        sys.stderr.write(f"error: no available iOS Simulator matching '{query}'\n")
        sys.exit(1)
else:
    iphones = [(rt, d) for rt, d in candidates if "iPhone" in d.get("name", "")]
    pool = iphones if iphones else candidates
    for _, d in pool:
        if d.get("state") == "Booted":
            selected = d
            break
    if selected is None:
        selected = pool[-1][1]

print(selected["udid"], selected["name"].replace(" ", "_"), selected["state"])
PY
)

echo "    Simulator: ${SIM_NAME//_/ } ($SIM_UDID, state=$SIM_STATE)"
if [[ "$SIM_STATE" != "Booted" ]]; then
  xcrun simctl boot "$SIM_UDID"
fi
xcrun simctl bootstatus "$SIM_UDID" -b

if [[ "$HEADLESS" -eq 0 ]]; then
  DEV_DIR="$(xcode-select -p 2>/dev/null || true)"
  if [[ -n "$DEV_DIR" && -d "$DEV_DIR/Applications/Simulator.app" ]]; then
    open "$DEV_DIR/Applications/Simulator.app" --args -CurrentDeviceUDID "$SIM_UDID" || true
  elif [[ -n "$DEV_DIR" && -d "$DEV_DIR/../Applications/DeviceHub.app" ]]; then
    open "$DEV_DIR/../Applications/DeviceHub.app" || true
  else
    open -a Simulator --args -CurrentDeviceUDID "$SIM_UDID" 2>/dev/null || \
      echo "note: Simulator GUI app not found; continuing install/launch on booted device $SIM_UDID"
  fi
fi

xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
xcrun simctl install "$SIM_UDID" "$APP_BUNDLE"
xcrun simctl launch "$SIM_UDID" "$BUNDLE_ID"

if [[ -n "$SCREENSHOT_PATH" ]]; then
  sleep 2
  mkdir -p "$(dirname "$SCREENSHOT_PATH")"
  xcrun simctl io "$SIM_UDID" screenshot "$SCREENSHOT_PATH"
  echo "Saved Simulator screenshot to $SCREENSHOT_PATH"
fi
