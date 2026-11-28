# shellcheck shell=bash
# Toolchain detection helpers and `suite_*` definitions for `scripts/check-all.sh` (sourced; it
# owns `REPO_ROOT`, `BUILD_ROOT`, `OS_NAME`, `CMAKE_GEN_ARGS`, the dylib paths, and `run_suite`).

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
  cargo test --locked -p nve-arch-invariants
}

suite_ast_grep() {
  cd "$REPO_ROOT"
  ast-grep test && ast-grep scan
}

suite_cargo_deny() {
  cd "$REPO_ROOT"
  cargo deny check
}

suite_rust_clippy() {
  cd "$REPO_ROOT"
  cargo clippy --locked --workspace --all-targets -- -D warnings
  cargo clippy --locked -p nve-ui --features spatial-focus --all-targets -- -D warnings
  if has_wasm_target; then
    cargo clippy --locked -p nve-web-shell -p nve-docs-site --target wasm32-unknown-unknown -- -D warnings
  fi
}

suite_rust_workspace() {
  cd "$REPO_ROOT"
  cargo build -p nve-example-kitchen-sink -p nve-studio-workspace
  cargo test --workspace --exclude nve-arch-invariants
  cargo test --locked -p nve-ui --features spatial-focus --test spatial_focus_tests
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
  cargo clippy --locked -p nve-backend-skia --features skia -- -D warnings
  cargo clippy --locked -p nve-backend-skia --features "$SKIA_GPU_FEATURE" -- -D warnings
  cargo clippy --locked -p nve-skia-shell --features "$SKIA_GPU_FEATURE" -- -D warnings
}

suite_abi_codegen_parity() {
  cd "$REPO_ROOT"
  cargo run --locked -p nve-cli -- codegen abi
  cargo run --locked -p nve-cli -- codegen abi --check
}

suite_rust_abi_release() {
  cd "$REPO_ROOT"
  cargo test --locked --release -p nve-abi --lib id::tests
}

suite_rust_abi_32bit() {
  cd "$REPO_ROOT"
  cargo check --locked -p nve-abi --target i686-linux-android
  cargo check --locked -p nve-abi --target armv7-linux-androideabi
}

suite_rust_rpc_hyper_matrix() {
  cd "$REPO_ROOT"
  cargo check --locked -p nve-http-hyper --no-default-features --all-targets
  cargo check --locked -p nve-http-hyper --no-default-features --features server --all-targets
  cargo check --locked -p nve-http-hyper --no-default-features --features client --all-targets
  cargo test --locked -p nve-http-hyper --no-default-features --doc
  cargo test --locked -p nve-http-hyper --no-default-features --features server --doc
  cargo test --locked -p nve-http-hyper --no-default-features --features client --doc
  # Web backend feature-gate matrix: `ssr`-only (server) and `hydrate`-only (client) builds.
  cargo check --locked -p nve-backend-web --no-default-features --features ssr --all-targets
  cargo check --locked -p nve-backend-web --no-default-features --features hydrate --all-targets
  RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --document-private-items \
    -p nve-hash -p nve-rpc -p nve-server -p nve-http-hyper \
    -p nve-query -p nve-http-web -p nve-query-dioxus \
    -p nve-example-issues-api -p nve-example-issues-client -p nve-example-issues-server
}

suite_rust_wasm() {
  cd "$REPO_ROOT"
  cargo check --locked -p nve-web-shell --target wasm32-unknown-unknown
  cargo check --locked -p nve-web-shell --no-default-features --features app-issues --target wasm32-unknown-unknown
  cargo check --locked -p nve-docs-site --target wasm32-unknown-unknown
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

# Header-only native-view codecs + the camera module's C++ codec (and, on macOS, its AppKit
# adapter) under AddressSanitizer / UBSan.
suite_native_modules() {
  local san=(-DCMAKE_BUILD_TYPE=Debug)
  case "$OS_NAME" in
    MINGW*|MSYS*|CYGWIN*) ;;
    *)
      san+=("-DCMAKE_CXX_FLAGS=-fsanitize=address,undefined -fno-sanitize-recover=all"
        "-DCMAKE_OBJCXX_FLAGS=-fsanitize=address,undefined -fno-sanitize-recover=all"
        "-DCMAKE_EXE_LINKER_FLAGS=-fsanitize=address,undefined")
      ;;
  esac
  local mods=(native-views/cxx camera/cxx) mod dir
  [ "$OS_NAME" = "Darwin" ] && mods+=(camera/apple)
  for mod in "${mods[@]}"; do
    dir="$BUILD_ROOT/modules/$mod"
    cmake -S "$REPO_ROOT/packages/modules/$mod" -B "$dir" ${CMAKE_GEN_ARGS[@]+"${CMAKE_GEN_ARGS[@]}"} \
      -DNVE_HOST_CXX_BUILD_TESTS=OFF "${san[@]}"
    cmake --build "$dir"
    ctest --test-dir "$dir" --output-on-failure
  done
}

suite_macos_appkit() {
  local build_dir="$BUILD_ROOT/macos-shell"
  cmake -S "$REPO_ROOT/hosts/macos-shell" -B "$build_dir" \
    ${CMAKE_GEN_ARGS[@]+"${CMAKE_GEN_ARGS[@]}"}
  cmake --build "$build_dir"
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
    ctest --test-dir "$build_dir" --output-on-failure
}

suite_ios_simulator_sdk() {
  cd "$REPO_ROOT"
  xcrun --sdk iphonesimulator clang++ \
    -std=c++20 -fobjc-arc -Wall -Wextra -Werror -fsyntax-only \
    -Ipackages/core/host-cxx/include \
    -Ihosts/apple-common \
    -Ihosts/ios-shell \
    hosts/apple-common/*.mm \
    hosts/ios-shell/*.mm
}

# Configures + builds a desktop C++ host (`hosts/<name>`) and runs its CTest with the live
# kitchen-sink `cdylib` test required (built here; never skipped silently). Extra args prefix
# ctest (e.g. `xvfb-run -a`).
run_desktop_host_ctest() {
  local name="$1"
  shift
  local build_dir="$BUILD_ROOT/$name"
  cmake -S "$REPO_ROOT/hosts/$name" -B "$build_dir" \
    ${CMAKE_GEN_ARGS[@]+"${CMAKE_GEN_ARGS[@]}"}
  cmake --build "$build_dir"
  (cd "$REPO_ROOT" && cargo build --locked -p nve-example-kitchen-sink)
  if [ ! -f "$KITCHEN_SINK_DYLIB" ]; then
    echo "error: expected kitchen-sink dylib at $KITCHEN_SINK_DYLIB" >&2
    return 1
  fi
  NVE_TEST_KITCHEN_SINK_DYLIB="$(native_host_path "$KITCHEN_SINK_DYLIB")" \
    ${1+"$@"} ctest --test-dir "$build_dir" --output-on-failure
}

suite_windows_host() {
  run_desktop_host_ctest windows-shell
}

suite_qt6_host() {
  # `qt_shell_test` / `qt_quick_test` set their own offscreen platform.
  run_desktop_host_ctest qt-shell
}

suite_gtk4_host() {
  if [ "$OS_NAME" = "Linux" ] && [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ] && command -v xvfb-run >/dev/null 2>&1; then
    run_desktop_host_ctest gtk-shell xvfb-run -a
  else
    run_desktop_host_ctest gtk-shell
  fi
}

suite_android_jni_host() {
  local build_dir="$BUILD_ROOT/android-shell"
  JAVA_HOME="$DETECTED_JAVA_HOME" cmake -S "$REPO_ROOT/hosts/android-shell" -B "$build_dir" \
    ${CMAKE_GEN_ARGS[@]+"${CMAKE_GEN_ARGS[@]}"}
  cmake --build "$build_dir"
  JAVA_HOME="$DETECTED_JAVA_HOME" ctest --test-dir "$build_dir" --output-on-failure
}

suite_android_compose_kit() {
  JAVA_HOME="$DETECTED_HOTSPOT_JAVA_HOME" \
    ANDROID_HOME="$DETECTED_ANDROID_SDK" \
    "$REPO_ROOT/hosts/compose-kit/run-tests.sh" testDebugUnitTest --console=plain
}

# The live runtime tests (`NveLiveRuntimeTests`) need libnve_runtime + the host fixture app.
suite_swiftui_kit() {
  host_runtime_ffi_libs || return 1
  swift test \
    --package-path "$REPO_ROOT/hosts/swiftui-kit" \
    --scratch-path "$BUILD_ROOT/swiftui-kit" \
    -Xswiftc -warnings-as-errors
}

# Live runtime widget tests need libnve_runtime + the kitchen sink; the export fixture's generated
# widget (gitignored `test/generated/`) is analyzed against the plugin API, then removed.
suite_flutter_plugin() {
  local generated="$REPO_ROOT/hosts/flutter-plugin/test/generated"
  host_runtime_ffi_libs || return 1
  rm -rf "$BUILD_ROOT/flutter-export" "$generated"
  cargo run --locked -q -p nve-cli -- codegen export "$REPO_ROOT/hosts/flutter-plugin/test/fixtures/export_spec.json" \
    --target flutter --out "$BUILD_ROOT/flutter-export" || return 1
  mkdir -p "$generated"
  cp "$BUILD_ROOT"/flutter-export/flutter/*.generated.dart "$generated/"
  local status=0
  # `&&`, not `set -e`: errexit is ignored inside a subshell that is the left side of `||`.
  (cd "$REPO_ROOT/hosts/flutter-plugin" && flutter pub get && flutter analyze --fatal-infos && flutter test) || status=1
  rm -rf "$generated"
  if [ "$FLUTTER_BUILD_EXISTED" -eq 0 ] && [ -d "$FLUTTER_BUILD_DIR" ]; then
    rm -rf "$FLUTTER_BUILD_DIR"
  fi
  return "$status"
}
