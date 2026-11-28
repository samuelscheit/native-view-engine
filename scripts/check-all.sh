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
  -h, --help         Show this help message

Environment:
  NVE_CHROME_BIN       Optional Chrome/Chromium binary path for Web WASM E2E (CDP)
EOF
}

QUICK_MODE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --quick)
      QUICK_MODE=1
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

# shellcheck source=scripts/check-suites.sh
source "$REPO_ROOT/scripts/check-suites.sh"

# --- Execution --------------------------------------------------------------

printf 'NVE Unified Verification Runner\n'
printf '  Repo root  : %s\n' "$REPO_ROOT"
printf '  Build root : %s\n' "$BUILD_ROOT"
printf '  OS         : %s\n' "$OS_NAME"
printf '  Quick mode : %s\n' "$([ "$QUICK_MODE" -eq 1 ] && printf 'yes (--quick)' || printf 'no (full benches)')"

# 0. Mechanical architectural invariants (runs in both --quick and full modes)
if command -v cargo >/dev/null 2>&1; then
  run_suite "Arch invariants" suite_arch_invariants "cargo test --locked -p nve-arch-invariants"
else
  skip_suite "Arch invariants" "cargo not found"
fi

# 0b. Polyglot AST lints (ast-grep)
if command -v ast-grep >/dev/null 2>&1; then
  run_suite "ast-grep lints" suite_ast_grep "ast-grep test && ast-grep scan"
else
  skip_suite "ast-grep lints" "ast-grep not found"
fi

# 0b1. ABI codegen generate + parity check: the host runtimes and every native host build against
# the generated (gitignored) host ABI / table / XXH64 vector files.
if command -v cargo >/dev/null 2>&1; then
  run_suite "ABI codegen parity" suite_abi_codegen_parity "codegen abi + --check (24 host files)"
else
  skip_suite "ABI codegen parity" "cargo not found"
fi

# 0b2. Static host runtimes (packages/core/host-*): --quick runs C/C++ and TS; full runs all five.
# shellcheck source=scripts/host-runtimes.sh
source "$REPO_ROOT/scripts/host-runtimes.sh"
host_runtime_suites=(cxx ts)
[ "$QUICK_MODE" -eq 0 ] && host_runtime_suites=(cxx swift java dart ts)
for host_rt in "${host_runtime_suites[@]}"; do
  if host_runtime_has_tools "$host_rt"; then
    run_suite "Host runtime $host_rt" "suite_host_$host_rt" "packages/core/host-$host_rt"
  else
    host_rt_missing="$(host_runtime_missing_tool "$host_rt")"
    if [ "${NVE_HOST_RUNTIMES_OPTIONAL:-0}" = "1" ]; then
      skip_suite "Host runtime $host_rt" "$host_rt_missing not found (NVE_HOST_RUNTIMES_OPTIONAL=1)"
    else
      banner "Host runtime $host_rt [MISSING TOOLCHAIN]"
      printf 'error: %s not found; install it or set NVE_HOST_RUNTIMES_OPTIONAL=1\n' "$host_rt_missing" >&2
      record_result "Host runtime $host_rt" "FAIL" "-" "$host_rt_missing not found"
    fi
  fi
done

# 0c. Dependency & license audit (cargo-deny)
if command -v cargo-deny >/dev/null 2>&1; then
  run_suite "cargo-deny audit" suite_cargo_deny "cargo deny check"
else
  skip_suite "cargo-deny audit" "cargo-deny not found"
fi

# 0d. Rust Clippy workspace lints
if command -v cargo >/dev/null 2>&1; then
  run_suite "Rust Clippy" suite_rust_clippy "--all-targets + wasm32"
else
  skip_suite "Rust Clippy" "cargo not found"
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
  run_suite "Skia GPU feature check" suite_skia_gpu_check "clippy skia + $SKIA_GPU_FEATURE"
elif [ -z "$SKIA_GPU_FEATURE" ]; then
  skip_suite "Skia GPU feature check" "no platform GPU feature mapped for $OS_NAME"
else
  skip_suite "Skia GPU feature check" "cargo not found"
fi

# 1b2. Rust RPC/Hyper feature matrix & rustdoc check (native, independent of WASM target)
if command -v cargo >/dev/null 2>&1; then
  run_suite "Rust RPC/Hyper feature matrix & docs" suite_rust_rpc_hyper_matrix "nve-http-hyper + nve-backend-web feature matrices + rustdoc -D warnings"
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

# Native hosts: every one is on `host-cxx` / `packages/core/host-*` + `libnve_runtime`.
# Windows: the retained model / controller / window-manager suite runs everywhere; on Windows
# (MINGW / MSYS / Git Bash) it adds the Direct2D, DirectWrite, WIC and live-window suite.
if has_cmake; then
  run_suite "Windows host" suite_windows_host "CTest (+ Direct2D / window suite on Windows) + kitchen sink"
else
  skip_suite "Windows host" "cmake not found"
fi
if has_cmake && has_qt6; then
  run_suite "Qt 6 host" suite_qt6_host "offscreen CTest (Widgets + Quick) + kitchen sink"
elif ! has_cmake; then
  skip_suite "Qt 6 host" "cmake not found"
else
  skip_suite "Qt 6 host" "Qt6 / qmake6 not found"
fi
if has_cmake && command -v pkg-config >/dev/null 2>&1 && pkg-config --exists gtk4; then
  run_suite "GTK4 host" suite_gtk4_host "CTest + kitchen sink"
elif ! has_cmake; then
  skip_suite "GTK4 host" "cmake not found"
else
  skip_suite "GTK4 host" "gtk4 not found via pkg-config"
fi

if has_cmake; then
  run_suite "Native modules C++" suite_native_modules "native-views + camera CTest (ASan + UBSan)"
else
  skip_suite "Native modules C++" "cmake not found"
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

# 6. Android JNI host
DETECTED_JAVA_HOME=""
if has_cmake && DETECTED_JAVA_HOME="$(detect_java_home)"; then
  run_suite "Android JNI host" suite_android_jni_host "android_{scene,events,pair}_jni_test (desktop JVM + libnve_runtime)"
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
