#!/usr/bin/env bash
set -euo pipefail

# Unified local verification runner for native-view-engine.
# Builds all CMake/Swift artifacts inside a temporary directory and cleans up
# on exit so the git working tree remains clean.

usage() {
  cat <<'EOF'
Usage: scripts/check-all.sh [OPTIONS]

Options:
  --quick            Skip Criterion runtime benchmarks (still runs cargo bench --no-run)
                     and skip building the Web WASM bundle when pkg/ is absent
  --rn-dir <path>    Path to react-native/packages/react-native/ReactCommon
                     (overrides $RN_REACT_COMMON_DIR)
  -h, --help         Show this help message

Environment:
  RN_REACT_COMMON_DIR  Optional path to ReactCommon for Yoga + JSI C++ bridge builds
  NVE_CHROME_BIN       Optional Chrome/Chromium binary path for Web WASM E2E (CDP)
EOF
}

QUICK_MODE=0
RN_DIR="${RN_REACT_COMMON_DIR:-}"
RN_DIR_EXPLICIT=0

while [ $# -gt 0 ]; do
  case "$1" in
    --quick)
      QUICK_MODE=1
      shift
      ;;
    --rn-dir)
      if [ $# -lt 2 ]; then
        echo "error: --rn-dir requires a path argument" >&2
        exit 2
      fi
      RN_DIR="$2"
      RN_DIR_EXPLICIT=1
      shift 2
      ;;
    --rn-dir=*)
      RN_DIR="${1#--rn-dir=}"
      RN_DIR_EXPLICIT=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option '$1'" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ -n "$RN_DIR" ]; then
  if [ ! -d "$RN_DIR" ]; then
    if [ "$RN_DIR_EXPLICIT" -eq 1 ]; then
      echo "error: --rn-dir path does not exist: $RN_DIR" >&2
      exit 2
    else
      echo "warning: RN_REACT_COMMON_DIR does not exist, ignoring: $RN_DIR" >&2
      RN_DIR=""
    fi
  else
    RN_DIR="$(cd "$RN_DIR" && pwd)"
  fi
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OS_NAME="$(uname -s)"
BUILD_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nve-check-all.XXXXXX")"
FLUTTER_BUILD_DIR="$REPO_ROOT/hosts/flutter-plugin/build"
FLUTTER_BUILD_EXISTED=0
if [ -d "$FLUTTER_BUILD_DIR" ]; then
  FLUTTER_BUILD_EXISTED=1
fi

cleanup() {
  rm -rf "$BUILD_ROOT"
  if [ "$FLUTTER_BUILD_EXISTED" -eq 0 ] && [ -d "$FLUTTER_BUILD_DIR" ]; then
    rm -rf "$FLUTTER_BUILD_DIR"
  fi
}
trap cleanup EXIT

case "$OS_NAME" in
  Darwin)
    KITCHEN_SINK_DYLIB="$REPO_ROOT/target/debug/libnve_example_kitchen_sink.dylib"
    STUDIO_DYLIB="$REPO_ROOT/target/debug/libnve_studio_workspace.dylib"
    ;;
  Linux)
    KITCHEN_SINK_DYLIB="$REPO_ROOT/target/debug/libnve_example_kitchen_sink.so"
    STUDIO_DYLIB="$REPO_ROOT/target/debug/libnve_studio_workspace.so"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    KITCHEN_SINK_DYLIB="$REPO_ROOT/target/debug/nve_example_kitchen_sink.dll"
    STUDIO_DYLIB="$REPO_ROOT/target/debug/nve_studio_workspace.dll"
    ;;
  *)
    KITCHEN_SINK_DYLIB="$REPO_ROOT/target/debug/libnve_example_kitchen_sink.so"
    STUDIO_DYLIB="$REPO_ROOT/target/debug/libnve_studio_workspace.so"
    ;;
esac

CMAKE_GEN_ARGS=()
if command -v ninja >/dev/null 2>&1; then
  CMAKE_GEN_ARGS=(-G Ninja)
fi

CMAKE_RN_ARGS=()
if [ -n "$RN_DIR" ]; then
  CMAKE_RN_ARGS=("-DRN_REACT_COMMON_DIR=$RN_DIR")
fi

# Parallel indexed arrays for Bash 3.2 compatibility (macOS default /bin/bash).
SUITE_NAMES=()
SUITE_STATUSES=()
SUITE_DURATIONS=()
SUITE_NOTES=()
PASS_COUNT=0
SKIP_COUNT=0
FAIL_COUNT=0

banner() {
  printf '\n========================================================================\n'
  printf '  %s\n' "$1"
  printf '========================================================================\n'
}

record_result() {
  local name="$1"
  local status="$2"
  local duration="$3"
  local note="$4"
  SUITE_NAMES+=("$name")
  SUITE_STATUSES+=("$status")
  SUITE_DURATIONS+=("$duration")
  SUITE_NOTES+=("$note")
  case "$status" in
    PASS) PASS_COUNT=$((PASS_COUNT + 1)) ;;
    SKIP) SKIP_COUNT=$((SKIP_COUNT + 1)) ;;
    FAIL) FAIL_COUNT=$((FAIL_COUNT + 1)) ;;
  esac
}

run_suite() {
  local name="$1"
  local fn="$2"
  local note="${3:-}"
  banner "$name"
  local start_s="$SECONDS"
  local rc=0
  set +e
  (
    set -euo pipefail
    "$fn"
  )
  rc=$?
  set -e
  local elapsed=$((SECONDS - start_s))
  if [ "$rc" -eq 0 ]; then
    record_result "$name" "PASS" "${elapsed}s" "$note"
  else
    record_result "$name" "FAIL" "${elapsed}s" "exit code $rc"
  fi
}

skip_suite() {
  local name="$1"
  local reason="$2"
  banner "$name [SKIP]"
  printf 'Skipping %s: %s\n' "$name" "$reason"
  record_result "$name" "SKIP" "-" "$reason"
}

has_cmake() {
  command -v cmake >/dev/null 2>&1
}

has_wasm_target() {
  command -v rustup >/dev/null 2>&1 \
    && rustup target list --installed | grep -qx 'wasm32-unknown-unknown'
}

has_wasm_bindgen() {
  command -v "${WASM_BINDGEN:-wasm-bindgen}" >/dev/null 2>&1
}

has_node_websocket() {
  command -v node >/dev/null 2>&1 \
    && node -e 'process.exit(typeof WebSocket === "function" ? 0 : 1)' >/dev/null 2>&1
}

native_host_path() {
  local p="$1"
  case "$OS_NAME" in
    MINGW*|MSYS*|CYGWIN*)
      if command -v cygpath >/dev/null 2>&1; then
        cygpath -w "$p"
        return 0
      fi
      ;;
  esac
  printf '%s\n' "$p"
}

detect_chrome_bin() {
  local candidate
  for candidate in \
    "${NVE_CHROME_BIN:-}" \
    "${CHROME_BIN:-}" \
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
    "/Applications/Chromium.app/Contents/MacOS/Chromium" \
    "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary" \
    "/usr/bin/google-chrome" \
    "/usr/bin/google-chrome-stable" \
    "/usr/bin/chromium" \
    "/usr/bin/chromium-browser"; do
    if [ -n "$candidate" ] && [ -x "$candidate" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  local cmd
  for cmd in google-chrome google-chrome-stable chromium chromium-browser; do
    if command -v "$cmd" >/dev/null 2>&1; then
      command -v "$cmd"
      return 0
    fi
  done
  return 1
}

has_qt6() {
  if command -v qmake6 >/dev/null 2>&1; then
    return 0
  fi
  if command -v pkg-config >/dev/null 2>&1 && pkg-config --exists Qt6Widgets 2>/dev/null; then
    return 0
  fi
  if [ -n "${Qt6_DIR:-}" ] && [ -f "${Qt6_DIR}/Qt6Config.cmake" ]; then
    return 0
  fi
  local candidate
  for candidate in \
    /opt/homebrew/opt/qt/lib/cmake/Qt6/Qt6Config.cmake \
    /opt/homebrew/opt/qtbase/lib/cmake/Qt6/Qt6Config.cmake \
    /usr/local/opt/qt/lib/cmake/Qt6/Qt6Config.cmake \
    /usr/local/opt/qtbase/lib/cmake/Qt6/Qt6Config.cmake \
    /usr/lib/x86_64-linux-gnu/cmake/Qt6/Qt6Config.cmake \
    /usr/lib/aarch64-linux-gnu/cmake/Qt6/Qt6Config.cmake \
    /usr/lib64/cmake/Qt6/Qt6Config.cmake; do
    if [ -f "$candidate" ]; then
      return 0
    fi
  done
  return 1
}

is_openj9_java() {
  local java_bin="$1"
  "$java_bin" -version 2>&1 | grep -qi "OpenJ9"
}

detect_hotspot_java_home() {
  if [ -n "${JAVA_HOME:-}" ] && [ -x "${JAVA_HOME}/bin/java" ] && ! is_openj9_java "${JAVA_HOME}/bin/java"; then
    printf '%s\n' "$JAVA_HOME"
    return 0
  fi
  local candidate
  for candidate in \
    "/Applications/Android Studio.app/Contents/jbr/Contents/Home" \
    "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home" \
    "/Library/Java/JavaVirtualMachines/temurin-17.jdk/Contents/Home" \
    "/usr/lib/jvm/temurin-21-jdk-amd64" \
    "/usr/lib/jvm/temurin-21-jdk-arm64" \
    "/usr/lib/jvm/java-21-openjdk-amd64" \
    "/usr/lib/jvm/java-21-openjdk-arm64" \
    "/usr/lib/jvm/java-17-openjdk-amd64" \
    "/usr/lib/jvm/java-17-openjdk-arm64" \
    "/usr/lib/jvm/default-java"; do
    if [ -x "$candidate/bin/java" ] && ! is_openj9_java "$candidate/bin/java"; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  if [ "$OS_NAME" = "Darwin" ] && [ -x /usr/libexec/java_home ]; then
    local mac_java=""
    mac_java="$(/usr/libexec/java_home 2>/dev/null || true)"
    if [ -n "$mac_java" ] && [ -x "$mac_java/bin/java" ] && ! is_openj9_java "$mac_java/bin/java"; then
      printf '%s\n' "$mac_java"
      return 0
    fi
  fi
  if command -v java >/dev/null 2>&1; then
    local java_path=""
    java_path="$(command -v java)"
    local resolved=""
    if command -v python3 >/dev/null 2>&1; then
      resolved="$(python3 -c 'import os,sys; print(os.path.dirname(os.path.dirname(os.path.realpath(sys.argv[1]))))' "$java_path" 2>/dev/null || true)"
    elif command -v readlink >/dev/null 2>&1; then
      resolved="$(dirname "$(dirname "$(readlink -f "$java_path" 2>/dev/null || printf '%s' "$java_path")")")"
    fi
    if [ -n "$resolved" ] && [ -x "$resolved/bin/java" ] && ! is_openj9_java "$resolved/bin/java"; then
      printf '%s\n' "$resolved"
      return 0
    fi
  fi
  return 1
}

detect_android_sdk() {
  local candidate
  for candidate in \
    "${ANDROID_HOME:-}" \
    "${ANDROID_SDK_ROOT:-}" \
    "$HOME/Library/Android/sdk" \
    "$HOME/Android/Sdk" \
    "/usr/local/lib/android/sdk"; do
    if [ -n "$candidate" ] && [ -d "$candidate" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

detect_java_home() {
  if [ -n "${JAVA_HOME:-}" ] && [ -f "${JAVA_HOME}/include/jni.h" ]; then
    printf '%s\n' "$JAVA_HOME"
    return 0
  fi
  if [ "$OS_NAME" = "Darwin" ] && [ -x /usr/libexec/java_home ]; then
    local mac_java=""
    mac_java="$(/usr/libexec/java_home 2>/dev/null || true)"
    if [ -n "$mac_java" ] && [ -f "$mac_java/include/jni.h" ]; then
      printf '%s\n' "$mac_java"
      return 0
    fi
  fi
  if command -v javac >/dev/null 2>&1; then
    local javac_path=""
    javac_path="$(command -v javac)"
    local resolved=""
    if command -v python3 >/dev/null 2>&1; then
      resolved="$(python3 -c 'import os,sys; print(os.path.dirname(os.path.dirname(os.path.realpath(sys.argv[1]))))' "$javac_path" 2>/dev/null || true)"
    elif command -v readlink >/dev/null 2>&1; then
      resolved="$(dirname "$(dirname "$(readlink -f "$javac_path" 2>/dev/null || printf '%s' "$javac_path")")")"
    fi
    if [ -n "$resolved" ] && [ -f "$resolved/include/jni.h" ]; then
      printf '%s\n' "$resolved"
      return 0
    fi
  fi
  if [ -f /usr/lib/jvm/default-java/include/jni.h ]; then
    printf '%s\n' "/usr/lib/jvm/default-java"
    return 0
  fi
  return 1
}

# --- Suite definitions ------------------------------------------------------

suite_arch_invariants() {
  cd "$REPO_ROOT"
  cargo test --locked -p nve-cli --test arch_invariants
}

suite_rust_workspace() {
  cd "$REPO_ROOT"
  cargo build -p nve-example-kitchen-sink -p nve-studio-workspace
  cargo test --workspace -- --skip arch_inv::
  cargo bench --no-run
  if [ "$QUICK_MODE" -eq 0 ]; then
    cargo bench --workspace
  fi
}

suite_skia_desktop_shell() {
  cd "$REPO_ROOT"
  cargo test -p nve-backend-skia -p nve-skia-shell
  cargo run -p nve-skia-shell -- --help >/dev/null
}

suite_skia_gpu_check() {
  cd "$REPO_ROOT"
  cargo check -p nve-backend-skia --features "$SKIA_GPU_FEATURE"
}

suite_abi_codegen_parity() {
  cd "$REPO_ROOT"
  cargo run --locked -p nve-cli -- codegen abi
  cargo run --locked -p nve-cli -- codegen abi --check
}

suite_rust_rpc_hyper_matrix() {
  cd "$REPO_ROOT"
  cargo check --locked -p nve-http-hyper --no-default-features --all-targets
  cargo check --locked -p nve-http-hyper --no-default-features --features server --all-targets
  cargo check --locked -p nve-http-hyper --no-default-features --features client --all-targets
  cargo test --locked -p nve-http-hyper --no-default-features --doc
  cargo test --locked -p nve-http-hyper --no-default-features --features server --doc
  cargo test --locked -p nve-http-hyper --no-default-features --features client --doc
  RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --document-private-items \
    -p nve-hash -p nve-rpc -p nve-server -p nve-http-hyper \
    -p nve-query -p nve-http-web -p nve-query-dioxus \
    -p nve-example-issues-api -p nve-example-issues-client -p nve-example-issues-server
}

suite_rust_wasm() {
  cd "$REPO_ROOT"
  cargo check --locked -p nve-web-shell --target wasm32-unknown-unknown
  cargo check --locked -p nve-web-shell --no-default-features --features app-issues --target wasm32-unknown-unknown
  cargo check --locked -p nve-hash -p nve-rpc -p nve-server -p nve-query -p nve-http-web -p nve-query-dioxus -p nve-example-issues-api -p nve-example-issues-client --target wasm32-unknown-unknown
}

suite_rust_wasip2() {
  cd "$REPO_ROOT"
  cargo check --locked -p nve-hash -p nve-rpc -p nve-server -p nve-query -p nve-example-issues-api --target wasm32-wasip2
}

suite_web_wasm_e2e() {
  cd "$REPO_ROOT"
  if [ "$WEB_E2E_BUILD_BUNDLE" -eq 1 ]; then
    "$REPO_ROOT/hosts/web-shell/build.sh" --dev
  fi
  NVE_CHROME_BIN="$DETECTED_CHROME_BIN" \
    node "$REPO_ROOT/hosts/web-shell/e2e/run_e2e.mjs" --no-screenshot
}

suite_python_tests() {
  cd "$REPO_ROOT"
  python3 -B -m unittest discover -s scripts/tests -v
}

suite_cpp_fabric_bridge() {
  local build_dir="$BUILD_ROOT/fabric-bridge"
  cmake -S "$REPO_ROOT/cxx/nve-fabric-bridge" -B "$build_dir" \
    ${CMAKE_GEN_ARGS[@]+"${CMAKE_GEN_ARGS[@]}"} \
    -DCMAKE_BUILD_TYPE=Release \
    -DNVE_BUILD_BENCHMARKS=ON \
    ${CMAKE_RN_ARGS[@]+"${CMAKE_RN_ARGS[@]}"}
  cmake --build "$build_dir"
  ctest --test-dir "$build_dir" --output-on-failure
  if [ -x "$build_dir/nve_fabric_bridge_bench" ]; then
    "$build_dir/nve_fabric_bridge_bench" --iters 10
  elif [ -x "$build_dir/nve_fabric_bridge_bench.exe" ]; then
    "$build_dir/nve_fabric_bridge_bench.exe" --iters 10
  else
    echo "error: nve_fabric_bridge_bench binary not found in $build_dir" >&2
    return 1
  fi
}

suite_macos_appkit() {
  local build_dir="$BUILD_ROOT/macos-shell"
  cmake -S "$REPO_ROOT/hosts/macos-shell" -B "$build_dir" \
    ${CMAKE_GEN_ARGS[@]+"${CMAKE_GEN_ARGS[@]}"} \
    ${CMAKE_RN_ARGS[@]+"${CMAKE_RN_ARGS[@]}"}
  cmake --build "$build_dir"
  ctest --test-dir "$build_dir" --output-on-failure
  if [ ! -f "$KITCHEN_SINK_DYLIB" ]; then
    echo "error: expected kitchen-sink dylib at $KITCHEN_SINK_DYLIB" >&2
    return 1
  fi
  if [ ! -f "$STUDIO_DYLIB" ]; then
    echo "error: expected studio dylib at $STUDIO_DYLIB" >&2
    return 1
  fi
  NVE_TEST_KITCHEN_SINK_DYLIB="$KITCHEN_SINK_DYLIB" \
    NVE_TEST_STUDIO_DYLIB="$STUDIO_DYLIB" \
    "$build_dir/nve_macos_shell_test"
}

suite_ios_simulator_sdk() {
  cd "$REPO_ROOT"
  xcrun --sdk iphonesimulator clang++ \
    -std=c++20 -fobjc-arc -Wall -Wextra -Werror -fsyntax-only \
    -Icxx/nve-fabric-bridge/include \
    -Ihosts/apple-common \
    -Ihosts/ios-shell \
    hosts/apple-common/*.mm \
    hosts/ios-shell/*.mm
}

suite_windows_host() {
  local build_dir="$BUILD_ROOT/windows-shell"
  cmake -S "$REPO_ROOT/hosts/windows-shell" -B "$build_dir" \
    ${CMAKE_GEN_ARGS[@]+"${CMAKE_GEN_ARGS[@]}"} \
    ${CMAKE_RN_ARGS[@]+"${CMAKE_RN_ARGS[@]}"}
  cmake --build "$build_dir"
  if [ ! -f "$KITCHEN_SINK_DYLIB" ]; then
    echo "error: expected kitchen-sink dylib at $KITCHEN_SINK_DYLIB" >&2
    return 1
  fi
  if [ ! -f "$STUDIO_DYLIB" ]; then
    echo "error: expected studio dylib at $STUDIO_DYLIB" >&2
    return 1
  fi
  local win_ks win_studio
  win_ks="$(native_host_path "$KITCHEN_SINK_DYLIB")"
  win_studio="$(native_host_path "$STUDIO_DYLIB")"
  NVE_TEST_KITCHEN_SINK_DYLIB="$win_ks" \
    NVE_TEST_STUDIO_DYLIB="$win_studio" \
    ctest --test-dir "$build_dir" --output-on-failure
}

suite_qt6_host() {
  local build_dir="$BUILD_ROOT/qt-shell"
  cmake -S "$REPO_ROOT/hosts/qt-shell" -B "$build_dir" \
    ${CMAKE_GEN_ARGS[@]+"${CMAKE_GEN_ARGS[@]}"} \
    ${CMAKE_RN_ARGS[@]+"${CMAKE_RN_ARGS[@]}"}
  cmake --build "$build_dir"
  QT_QPA_PLATFORM=offscreen ctest --test-dir "$build_dir" --output-on-failure
  if [ ! -f "$KITCHEN_SINK_DYLIB" ]; then
    echo "error: expected kitchen-sink dylib at $KITCHEN_SINK_DYLIB" >&2
    return 1
  fi
  if [ ! -f "$STUDIO_DYLIB" ]; then
    echo "error: expected studio dylib at $STUDIO_DYLIB" >&2
    return 1
  fi
  local host_ks host_studio
  host_ks="$(native_host_path "$KITCHEN_SINK_DYLIB")"
  host_studio="$(native_host_path "$STUDIO_DYLIB")"

  local shell_test_bin=""
  if [ -x "$build_dir/nve_qt_shell_test" ]; then
    shell_test_bin="$build_dir/nve_qt_shell_test"
  elif [ -x "$build_dir/nve_qt_shell_test.exe" ]; then
    shell_test_bin="$build_dir/nve_qt_shell_test.exe"
  else
    echo "error: nve_qt_shell_test binary not found in $build_dir" >&2
    return 1
  fi

  QT_QPA_PLATFORM=offscreen \
    NVE_TEST_KITCHEN_SINK_DYLIB="$host_ks" \
    NVE_TEST_STUDIO_DYLIB="$host_studio" \
    "$shell_test_bin"

  local quick_test_bin=""
  if [ -x "$build_dir/nve_qt_quick_test" ]; then
    quick_test_bin="$build_dir/nve_qt_quick_test"
  elif [ -x "$build_dir/nve_qt_quick_test.exe" ]; then
    quick_test_bin="$build_dir/nve_qt_quick_test.exe"
  fi
  if [ -n "$quick_test_bin" ]; then
    QT_QPA_PLATFORM=offscreen \
      QT_QUICK_BACKEND=software \
      NVE_TEST_KITCHEN_SINK_DYLIB="$host_ks" \
      NVE_TEST_STUDIO_DYLIB="$host_studio" \
      "$quick_test_bin"
  fi
}

suite_gtk4_host() {
  local build_dir="$BUILD_ROOT/gtk-shell"
  cmake -S "$REPO_ROOT/hosts/gtk-shell" -B "$build_dir" \
    ${CMAKE_GEN_ARGS[@]+"${CMAKE_GEN_ARGS[@]}"} \
    ${CMAKE_RN_ARGS[@]+"${CMAKE_RN_ARGS[@]}"}
  cmake --build "$build_dir"

  local env_prefix=()
  if [ -f "$KITCHEN_SINK_DYLIB" ] && [ -f "$STUDIO_DYLIB" ]; then
    env_prefix=(env "NVE_TEST_KITCHEN_SINK_DYLIB=$KITCHEN_SINK_DYLIB" "NVE_TEST_STUDIO_DYLIB=$STUDIO_DYLIB")
  elif [ -f "$KITCHEN_SINK_DYLIB" ]; then
    env_prefix=(env "NVE_TEST_KITCHEN_SINK_DYLIB=$KITCHEN_SINK_DYLIB")
  fi

  if [ "$OS_NAME" = "Linux" ] && [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ] && command -v xvfb-run >/dev/null 2>&1; then
    ${env_prefix[@]+"${env_prefix[@]}"} xvfb-run -a ctest --test-dir "$build_dir" --output-on-failure
  else
    ${env_prefix[@]+"${env_prefix[@]}"} ctest --test-dir "$build_dir" --output-on-failure
  fi
}

suite_android_jni_host() {
  local build_dir="$BUILD_ROOT/android-shell"
  JAVA_HOME="$DETECTED_JAVA_HOME" cmake -S "$REPO_ROOT/hosts/android-shell" -B "$build_dir" \
    ${CMAKE_GEN_ARGS[@]+"${CMAKE_GEN_ARGS[@]}"} \
    ${CMAKE_RN_ARGS[@]+"${CMAKE_RN_ARGS[@]}"}
  cmake --build "$build_dir"
  ctest --test-dir "$build_dir" --output-on-failure
}

suite_android_compose_kit() {
  JAVA_HOME="$DETECTED_HOTSPOT_JAVA_HOME" \
    ANDROID_HOME="$DETECTED_ANDROID_SDK" \
    "$REPO_ROOT/hosts/compose-kit/run-tests.sh" testDebugUnitTest --console=plain
}

suite_swiftui_kit() {
  swift test \
    --package-path "$REPO_ROOT/hosts/swiftui-kit" \
    --scratch-path "$BUILD_ROOT/swiftui-kit"
}

suite_flutter_plugin() {
  cd "$REPO_ROOT/hosts/flutter-plugin"
  flutter pub get
  flutter analyze
  flutter test
  if [ "$FLUTTER_BUILD_EXISTED" -eq 0 ] && [ -d "$FLUTTER_BUILD_DIR" ]; then
    rm -rf "$FLUTTER_BUILD_DIR"
  fi
}

# --- Execution --------------------------------------------------------------

printf 'NVE Unified Verification Runner\n'
printf '  Repo root  : %s\n' "$REPO_ROOT"
printf '  Build root : %s\n' "$BUILD_ROOT"
printf '  OS         : %s\n' "$OS_NAME"
printf '  Quick mode : %s\n' "$([ "$QUICK_MODE" -eq 1 ] && printf 'yes (--quick)' || printf 'no (full benches)')"
if [ -n "$RN_DIR" ]; then
  printf '  RN Common  : %s\n' "$RN_DIR"
else
  printf '  RN Common  : standalone (RN_REACT_COMMON_DIR not set)\n'
fi

# 0. Mechanical architectural invariants (runs in both --quick and full modes)
if command -v cargo >/dev/null 2>&1; then
  run_suite "Arch invariants" suite_arch_invariants "cargo test --locked -p nve-cli --test arch_invariants"
else
  skip_suite "Arch invariants" "cargo not found"
fi

# 1. Rust workspace
if command -v cargo >/dev/null 2>&1; then
  rust_note="counter+studio cdylibs + test + bench --no-run"
  if [ "$QUICK_MODE" -eq 0 ]; then
    rust_note="counter+studio cdylibs + test + bench"
  fi
  run_suite "Rust workspace" suite_rust_workspace "$rust_note"
else
  skip_suite "Rust workspace" "cargo not found"
fi

# 1a. Skia desktop shell
if command -v cargo >/dev/null 2>&1; then
  run_suite "Skia desktop shell" suite_skia_desktop_shell "nve-backend-skia + nve-skia-shell + --help"
else
  skip_suite "Skia desktop shell" "cargo not found"
fi

# 1a2. Skia platform GPU feature check
SKIA_GPU_FEATURE=""
case "$OS_NAME" in
  Darwin) SKIA_GPU_FEATURE="skia-metal" ;;
  Linux) SKIA_GPU_FEATURE="skia-vulkan" ;;
  MINGW*|MSYS*|CYGWIN*) SKIA_GPU_FEATURE="skia-d3d" ;;
esac
if [ -n "$SKIA_GPU_FEATURE" ] && command -v cargo >/dev/null 2>&1; then
  run_suite "Skia GPU feature check" suite_skia_gpu_check "cargo check -p nve-backend-skia --features $SKIA_GPU_FEATURE"
elif [ -z "$SKIA_GPU_FEATURE" ]; then
  skip_suite "Skia GPU feature check" "no platform GPU feature mapped for $OS_NAME"
else
  skip_suite "Skia GPU feature check" "cargo not found"
fi

# 1b. ABI codegen generate + parity check (required before host builds/tests)
if command -v cargo >/dev/null 2>&1; then
  run_suite "ABI codegen parity" suite_abi_codegen_parity "codegen abi + --check (15/15)"
else
  skip_suite "ABI codegen parity" "cargo not found"
fi

# 1b2. Rust RPC/Hyper feature matrix & rustdoc check (native, independent of WASM target)
if command -v cargo >/dev/null 2>&1; then
  run_suite "Rust RPC/Hyper feature matrix & docs" suite_rust_rpc_hyper_matrix "nve-http-hyper features (--all-targets + --doc) + rustdoc -D warnings"
else
  skip_suite "Rust RPC/Hyper feature matrix & docs" "cargo not found"
fi

# 1c. Rust WASM check
if command -v cargo >/dev/null 2>&1 && has_wasm_target; then
  run_suite "Rust WASM check" suite_rust_wasm "wasm32-unknown-unknown (web-shell + hash/rpc/server/issues-api)"
else
  skip_suite "Rust WASM check" "wasm32-unknown-unknown target not installed"
fi

# 1c2. Rust WASI P2 check
if command -v cargo >/dev/null 2>&1 && rustup target list --installed 2>/dev/null | grep -q '^wasm32-wasip2$'; then
  run_suite "Rust WASI P2 check" suite_rust_wasip2 "wasm32-wasip2 (nve-hash/rpc/server/issues-api)"
else
  skip_suite "Rust WASI P2 check" "wasm32-wasip2 target not installed"
fi

# 1d. Web WASM E2E (headless Chrome CDP)
DETECTED_CHROME_BIN=""
WEB_E2E_BUILD_BUNDLE=0
# The E2E loads every build.sh bundle (pkg/studio, pkg/counter, pkg/dev); dev is built last.
WEB_PKG_MANIFEST="$REPO_ROOT/hosts/web-shell/pkg/dev/manifest.json"
if ! command -v node >/dev/null 2>&1; then
  skip_suite "Web WASM E2E (CDP)" "node not found"
elif ! has_node_websocket; then
  skip_suite "Web WASM E2E (CDP)" "Node 22+ (global WebSocket) required"
elif ! DETECTED_CHROME_BIN="$(detect_chrome_bin)"; then
  skip_suite "Web WASM E2E (CDP)" "Chrome/Chromium binary not found"
elif [ "$QUICK_MODE" -eq 0 ] && command -v cargo >/dev/null 2>&1 && has_wasm_target && has_wasm_bindgen; then
  WEB_E2E_BUILD_BUNDLE=1
  run_suite "Web WASM E2E (CDP)" suite_web_wasm_e2e "build.sh --dev + headless Chrome CDP"
elif [ -f "$WEB_PKG_MANIFEST" ]; then
  run_suite "Web WASM E2E (CDP)" suite_web_wasm_e2e "existing pkg/ + headless Chrome CDP"
elif [ "$QUICK_MODE" -eq 1 ]; then
  skip_suite "Web WASM E2E (CDP)" "hosts/web-shell/pkg/dev/manifest.json missing (run ./hosts/web-shell/build.sh --dev or omit --quick)"
else
  skip_suite "Web WASM E2E (CDP)" "wasm-bindgen or wasm32-unknown-unknown target not installed"
fi

# 1e. Python script tests (bench regression harness)
if command -v python3 >/dev/null 2>&1 && [ -d "$REPO_ROOT/scripts/tests" ]; then
  run_suite "Python script tests" suite_python_tests "unittest scripts/tests"
elif ! command -v python3 >/dev/null 2>&1; then
  skip_suite "Python script tests" "python3 not found"
else
  skip_suite "Python script tests" "scripts/tests directory not found"
fi

# 2. C++20 Fabric bridge
if has_cmake; then
  fabric_note="standalone + bench --iters 10"
  if [ -n "$RN_DIR" ]; then
    fabric_note="RN Yoga/JSI + bench --iters 10"
  fi
  run_suite "C++20 Fabric bridge" suite_cpp_fabric_bridge "$fabric_note"
else
  skip_suite "C++20 Fabric bridge" "cmake not found"
fi

# 3a. macOS AppKit host
if [ "$OS_NAME" = "Darwin" ] && has_cmake; then
  run_suite "macOS AppKit host" suite_macos_appkit "CTest + counter & studio dylibs"
elif [ "$OS_NAME" != "Darwin" ]; then
  skip_suite "macOS AppKit host" "requires macOS (Darwin)"
else
  skip_suite "macOS AppKit host" "cmake not found"
fi

# 3b. iOS Simulator SDK ObjC++ syntax check
if [ "$OS_NAME" = "Darwin" ] && command -v xcrun >/dev/null 2>&1 \
  && xcrun --sdk iphonesimulator --show-sdk-path >/dev/null 2>&1; then
  run_suite "iOS Simulator SDK" suite_ios_simulator_sdk "ObjC++ -Wall -Wextra -Werror"
elif [ "$OS_NAME" != "Darwin" ]; then
  skip_suite "iOS Simulator SDK" "requires macOS (Darwin)"
else
  skip_suite "iOS Simulator SDK" "iphonesimulator SDK not installed"
fi

# 3c. Windows host (MINGW / MSYS / Git Bash)
case "$OS_NAME" in
  MINGW*|MSYS*|CYGWIN*)
    if has_cmake; then
      run_suite "Windows host" suite_windows_host "CTest + counter & studio dylibs"
    else
      skip_suite "Windows host" "cmake not found"
    fi
    ;;
  *)
    skip_suite "Windows host" "requires Windows (MINGW/MSYS/Git Bash)"
    ;;
esac

# 4. Qt 6 host
if has_cmake && has_qt6; then
  run_suite "Qt 6 host" suite_qt6_host "offscreen CTest + Widgets & Quick dylibs"
elif ! has_cmake; then
  skip_suite "Qt 6 host" "cmake not found"
else
  skip_suite "Qt 6 host" "Qt6 / qmake6 not found"
fi

# 5. GTK4 host
if has_cmake && command -v pkg-config >/dev/null 2>&1 && pkg-config --exists gtk4; then
  run_suite "GTK4 host" suite_gtk4_host "CTest"
elif ! has_cmake; then
  skip_suite "GTK4 host" "cmake not found"
else
  skip_suite "GTK4 host" "gtk4 not found via pkg-config"
fi

# 6. Android JNI host
DETECTED_JAVA_HOME=""
if has_cmake && DETECTED_JAVA_HOME="$(detect_java_home)"; then
  run_suite "Android JNI host" suite_android_jni_host "android_jni_bridge_test + bridge_abi_test"
elif ! has_cmake; then
  skip_suite "Android JNI host" "cmake not found"
else
  skip_suite "Android JNI host" "JDK / JNI headers not found"
fi

# 6b. Android Compose kit (Gradle unit tests)
DETECTED_ANDROID_SDK=""
DETECTED_HOTSPOT_JAVA_HOME=""
if [ ! -x "$REPO_ROOT/hosts/compose-kit/run-tests.sh" ]; then
  skip_suite "Android Compose kit" "hosts/compose-kit/run-tests.sh not found"
elif ! DETECTED_ANDROID_SDK="$(detect_android_sdk)"; then
  skip_suite "Android Compose kit" "Android SDK not found"
elif ! DETECTED_HOTSPOT_JAVA_HOME="$(detect_hotspot_java_home)"; then
  skip_suite "Android Compose kit" "HotSpot/JBR JDK not found (OpenJ9 unsupported)"
else
  run_suite "Android Compose kit" suite_android_compose_kit "Gradle testDebugUnitTest"
fi

# 7. SwiftUI host kit
if command -v swift >/dev/null 2>&1; then
  run_suite "SwiftUI host kit" suite_swiftui_kit "swift test"
else
  skip_suite "SwiftUI host kit" "swift not found"
fi

# 8. Flutter host plugin
if command -v flutter >/dev/null 2>&1; then
  run_suite "Flutter host plugin" suite_flutter_plugin "flutter analyze + test"
else
  skip_suite "Flutter host plugin" "flutter not found"
fi

# --- Summary table ----------------------------------------------------------

printf '\n========================================================================\n'
printf '  VERIFICATION SUMMARY\n'
printf '========================================================================\n'
printf '%-24s  %-8s  %-7s  %s\n' "SUITE" "STATUS" "TIME" "DETAILS"
printf '%-24s  %-8s  %-7s  %s\n' "------------------------" "--------" "-------" "------------------------------"

idx=0
total="${#SUITE_NAMES[@]}"
while [ "$idx" -lt "$total" ]; do
  printf '%-24s  %-8s  %-7s  %s\n' \
    "${SUITE_NAMES[$idx]}" \
    "${SUITE_STATUSES[$idx]}" \
    "${SUITE_DURATIONS[$idx]}" \
    "${SUITE_NOTES[$idx]}"
  idx=$((idx + 1))
done

printf '%s\n' '------------------------------------------------------------------------'
printf 'Totals: %d PASS, %d SKIP, %d FAIL\n' "$PASS_COUNT" "$SKIP_COUNT" "$FAIL_COUNT"
printf '%s\n' '========================================================================'

if [ "$FAIL_COUNT" -gt 0 ]; then
  exit 1
fi
