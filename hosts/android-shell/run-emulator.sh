#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BUILD_DIR="$SCRIPT_DIR/build"

APP_PKG="nve-example-counter"
BUILD_ONLY=0
AVD_NAME=""
SCREENSHOT_PATH=""

usage() {
  cat <<'EOF'
Usage: ./hosts/android-shell/run-emulator.sh [nve-studio-workspace|nve-example-counter] [--build-only] [--avd <name>] [--screenshot <out.png>]

Cross-compiles an NVE Rust app (`aarch64-linux-android`) + `libnve_android_shell.so` (`arm64-v8a`),
packages and debug-signs a zero-Gradle APK (`hosts/android-shell/build/nve-android-shell.apk`),
and optionally installs + launches it on an Android Emulator.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --build-only)
      BUILD_ONLY=1
      shift
      ;;
    --avd)
      if [[ $# -lt 2 ]]; then
        echo "error: --avd requires a value" >&2
        exit 2
      fi
      AVD_NAME="$2"
      shift 2
      ;;
    --screenshot)
      if [[ $# -lt 2 ]]; then
        echo "error: --screenshot requires an output path" >&2
        exit 2
      fi
      SCREENSHOT_PATH="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "error: unknown flag: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      APP_PKG="$1"
      shift
      ;;
  esac
done

# Auto-detect Android SDK root without hardcoded personal paths.
if [[ -n "${ANDROID_HOME:-}" ]]; then
  SDK_ROOT="$ANDROID_HOME"
elif [[ -n "${ANDROID_SDK_ROOT:-}" ]]; then
  SDK_ROOT="$ANDROID_SDK_ROOT"
elif [[ -d "$HOME/Library/Android/sdk" ]]; then
  SDK_ROOT="$HOME/Library/Android/sdk"
elif [[ -d "$HOME/Android/Sdk" ]]; then
  SDK_ROOT="$HOME/Android/Sdk"
else
  echo "error: Android SDK not found; set ANDROID_HOME or install Android SDK." >&2
  exit 1
fi

latest_dir() {
  local base="$1"
  local pattern="$2"
  local found=""
  if [[ -d "$base" ]]; then
    found="$(find "$base" -mindepth 1 -maxdepth 1 -type d -name "$pattern" | sort -V | tail -n 1)"
  fi
  printf '%s' "$found"
}

NDK_DIR="$(latest_dir "$SDK_ROOT/ndk" "*")"
BUILD_TOOLS_DIR="$(latest_dir "$SDK_ROOT/build-tools" "*")"
PLATFORM_DIR="$(latest_dir "$SDK_ROOT/platforms" "android-*")"

if [[ -z "$NDK_DIR" || ! -d "$NDK_DIR" ]]; then
  echo "error: Android NDK not found under $SDK_ROOT/ndk" >&2
  exit 1
fi
if [[ -z "$BUILD_TOOLS_DIR" || ! -d "$BUILD_TOOLS_DIR" ]]; then
  echo "error: Android build-tools not found under $SDK_ROOT/build-tools" >&2
  exit 1
fi
if [[ -z "$PLATFORM_DIR" || ! -f "$PLATFORM_DIR/android.jar" ]]; then
  echo "error: Android platform android.jar not found under $SDK_ROOT/platforms" >&2
  exit 1
fi

PREBUILT_DIR="$(find "$NDK_DIR/toolchains/llvm/prebuilt" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
if [[ -z "$PREBUILT_DIR" || ! -d "$PREBUILT_DIR" ]]; then
  echo "error: NDK LLVM prebuilt toolchain directory not found under $NDK_DIR" >&2
  exit 1
fi

NDK_CLANG_AA64="$PREBUILT_DIR/bin/aarch64-linux-android24-clang"
NDK_AR="$PREBUILT_DIR/bin/llvm-ar"
LIBCXX_SHARED="$PREBUILT_DIR/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so"
ANDROID_JAR="$PLATFORM_DIR/android.jar"
AAPT2="$BUILD_TOOLS_DIR/aapt2"
D8="$BUILD_TOOLS_DIR/d8"
ZIPALIGN="$BUILD_TOOLS_DIR/zipalign"
APKSIGNER="$BUILD_TOOLS_DIR/apksigner"

for tool in "$NDK_CLANG_AA64" "$AAPT2" "$D8" "$ZIPALIGN" "$APKSIGNER"; do
  if [[ ! -x "$tool" ]]; then
    echo "error: required Android SDK/NDK tool missing or not executable: $tool" >&2
    exit 1
  fi
done
if [[ ! -f "$LIBCXX_SHARED" ]]; then
  echo "error: libc++_shared.so not found at $LIBCXX_SHARED" >&2
  exit 1
fi

# Eclipse OpenJ9 (e.g. 0.59.0 on macOS arm64) hits an internal Scavenger GC assertion
# (`ScavengerRootScanner.hpp:105`, "Misaligned object in thread ForkJoinPool") when
# running Android build-tools `d8`. Prefer a HotSpot/JBR JDK when available; otherwise
# disable Scavenger GC and core dumps on `d8` as a fallback.
D8_EXTRA_JAVA_OPTS=()
is_openj9_java() {
  local java_bin="$1"
  [[ -x "$java_bin" ]] && "$java_bin" -version 2>&1 | grep -qi "OpenJ9"
}

if command -v java >/dev/null 2>&1 && is_openj9_java "$(command -v java)"; then
  HOTSPOT_JAVA_HOME=""
  JAVA_CANDIDATES=()
  if [[ -n "${JAVA_HOME:-}" ]]; then
    JAVA_CANDIDATES+=("$JAVA_HOME")
  fi
  JAVA_CANDIDATES+=(
    "/Applications/Android Studio.app/Contents/jbr/Contents/Home"
  )
  for jvm_root in "/Library/Java/JavaVirtualMachines" "$HOME/Library/Java/JavaVirtualMachines"; do
    if [[ -d "$jvm_root" ]]; then
      while IFS= read -r home_dir; do
        JAVA_CANDIDATES+=("$home_dir")
      done < <(find "$jvm_root" -mindepth 3 -maxdepth 3 -type d -path '*/Contents/Home' 2>/dev/null | sort -rV)
    fi
  done
  if [[ -d "/usr/lib/jvm" ]]; then
    while IFS= read -r home_dir; do
      JAVA_CANDIDATES+=("$home_dir")
    done < <(find "/usr/lib/jvm" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -rV)
  fi

  for cand in "${JAVA_CANDIDATES[@]}"; do
    if [[ -x "$cand/bin/java" && -x "$cand/bin/javac" && -x "$cand/bin/keytool" ]] && ! is_openj9_java "$cand/bin/java"; then
      HOTSPOT_JAVA_HOME="$cand"
      break
    fi
  done

  if [[ -n "$HOTSPOT_JAVA_HOME" ]]; then
    export JAVA_HOME="$HOTSPOT_JAVA_HOME"
    export PATH="$HOTSPOT_JAVA_HOME/bin:$PATH"
    echo "==> Detected OpenJ9 default JVM; switching to HotSpot JDK for d8/apksigner: $HOTSPOT_JAVA_HOME"
  else
    echo "==> Warning: OpenJ9 JVM detected and no HotSpot JDK found; passing -JXgcpolicy:optthruput -JXdump:none to d8" >&2
    D8_EXTRA_JAVA_OPTS+=(-JXgcpolicy:optthruput -JXdump:none)
  fi
fi

for cmd in cargo cmake javac keytool zip; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: required host tool not found in PATH: $cmd" >&2
    exit 1
  fi
done

echo "==> Android SDK:        $SDK_ROOT"
echo "==> Android NDK:        $(basename "$NDK_DIR")"
echo "==> Build-tools:        $(basename "$BUILD_TOOLS_DIR")"
echo "==> Platform:           $(basename "$PLATFORM_DIR")"
echo "==> App crate:          $APP_PKG"

# 1. Cross-compile Rust app crate to aarch64-linux-android.
APP_LIB_NAME="lib${APP_PKG//-/_}.so"
echo "==> [1/5] Cross-compiling Rust crate '$APP_PKG' (aarch64-linux-android)..."
(
  cd "$REPO_ROOT"
  CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$NDK_CLANG_AA64" \
  CC_aarch64_linux_android="$NDK_CLANG_AA64" \
  AR_aarch64_linux_android="$NDK_AR" \
  cargo build -p "$APP_PKG" --target aarch64-linux-android
)

APP_SO="$REPO_ROOT/target/aarch64-linux-android/debug/$APP_LIB_NAME"
if [[ ! -f "$APP_SO" ]]; then
  echo "error: expected Rust cdylib not found at $APP_SO" >&2
  exit 1
fi

# 2. Cross-compile libnve_android_shell.so (arm64-v8a, android-24) using NDK CMake toolchain.
CMAKE_BUILD_DIR="$BUILD_DIR/cmake-arm64"
CMAKE_GEN_ARGS=()
if command -v ninja >/dev/null 2>&1; then
  CMAKE_GEN_ARGS=(-G Ninja)
fi

echo "==> [2/5] Building libnve_android_shell.so (arm64-v8a, android-24)..."
cmake -S "$SCRIPT_DIR" -B "$CMAKE_BUILD_DIR" "${CMAKE_GEN_ARGS[@]}" \
  -DCMAKE_TOOLCHAIN_FILE="$NDK_DIR/build/cmake/android.toolchain.cmake" \
  -DANDROID_ABI=arm64-v8a \
  -DANDROID_PLATFORM=android-24 \
  -DANDROID_STL=c++_shared \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_CXX_FLAGS="-Wall -Wextra"
cmake --build "$CMAKE_BUILD_DIR" --target nve_android_shell

SHELL_SO="$CMAKE_BUILD_DIR/libnve_android_shell.so"
if [[ ! -f "$SHELL_SO" ]]; then
  echo "error: expected libnve_android_shell.so not found at $SHELL_SO" >&2
  exit 1
fi

# 3. Compile pure-Java NveMainActivity against android.jar and dex via d8.
CLASSES_DIR="$BUILD_DIR/classes"
DEX_DIR="$BUILD_DIR/dex"
rm -rf "$CLASSES_DIR" "$DEX_DIR"
mkdir -p "$CLASSES_DIR" "$DEX_DIR"

echo "==> [3/5] Compiling NveCoreAbi.java + NveSceneBuffer.java + NveCanvasView.java + NveMainActivity.java (javac --release 11) and dexing (d8)..."
javac --release 11 -Xlint:deprecation -cp "$ANDROID_JAR" -d "$CLASSES_DIR" \
  "$SCRIPT_DIR/android-app/src/dev/nve/android/NveCoreAbi.java" \
  "$SCRIPT_DIR/android-app/src/dev/nve/android/NveSceneBuffer.java" \
  "$SCRIPT_DIR/android-app/src/dev/nve/android/NveCanvasView.java" \
  "$SCRIPT_DIR/android-app/src/dev/nve/android/NveMainActivity.java"

CLASS_FILES=()
while IFS= read -r -d '' f; do
  CLASS_FILES+=("$f")
done < <(find "$CLASSES_DIR" -type f -name '*.class' -print0)

"$D8" "${D8_EXTRA_JAVA_OPTS[@]+"${D8_EXTRA_JAVA_OPTS[@]}"}" --min-api 24 --lib "$ANDROID_JAR" --output "$DEX_DIR" "${CLASS_FILES[@]}"

# 4. Package APK via aapt2 + zip + zipalign.
STAGE_DIR="$BUILD_DIR/apk-stage"
UNSIGNED_APK="$BUILD_DIR/unsigned-unaligned.apk"
ALIGNED_APK="$BUILD_DIR/unsigned-aligned.apk"
SIGNED_APK="$BUILD_DIR/nve-android-shell.apk"

rm -rf "$STAGE_DIR" "$UNSIGNED_APK" "$ALIGNED_APK" "$SIGNED_APK"
mkdir -p "$STAGE_DIR/lib/arm64-v8a"

echo "==> [4/5] Packaging APK with aapt2, zip, and zipalign..."
"$AAPT2" link \
  -I "$ANDROID_JAR" \
  --manifest "$SCRIPT_DIR/android-app/AndroidManifest.xml" \
  --min-sdk-version 24 \
  --target-sdk-version 34 \
  -o "$UNSIGNED_APK"

cp "$DEX_DIR/classes.dex" "$STAGE_DIR/classes.dex"
cp "$SHELL_SO" "$STAGE_DIR/lib/arm64-v8a/libnve_android_shell.so"
cp "$APP_SO" "$STAGE_DIR/lib/arm64-v8a/libnve_app.so"
cp "$LIBCXX_SHARED" "$STAGE_DIR/lib/arm64-v8a/libc++_shared.so"

(
  cd "$STAGE_DIR"
  zip -q "$UNSIGNED_APK" classes.dex
  zip -q -0 -r "$UNSIGNED_APK" lib
)

"$ZIPALIGN" -p -f 4 "$UNSIGNED_APK" "$ALIGNED_APK"

# 5. Sign with cached debug keystore and verify via apksigner.
KEYSTORE="$BUILD_DIR/debug.keystore"
if [[ ! -f "$KEYSTORE" ]]; then
  echo "==> Generating cached debug keystore at $KEYSTORE..."
  keytool -genkeypair -noprompt \
    -keystore "$KEYSTORE" \
    -storepass android \
    -alias androiddebugkey \
    -keypass android \
    -keyalg RSA \
    -keysize 2048 \
    -validity 10000 \
    -dname "CN=Android Debug,O=Android,C=US" >/dev/null 2>&1
fi

echo "==> [5/5] Signing and verifying APK..."
"$APKSIGNER" sign \
  --ks "$KEYSTORE" \
  --ks-key-alias androiddebugkey \
  --ks-pass pass:android \
  --key-pass pass:android \
  --out "$SIGNED_APK" \
  "$ALIGNED_APK"

"$APKSIGNER" verify --print-certs "$SIGNED_APK"
echo "==> Signed APK ready: $SIGNED_APK"

if [[ "$BUILD_ONLY" -eq 1 ]]; then
  exit 0
fi

# Launch on connected Android device or Android Emulator.
ADB_BIN="$SDK_ROOT/platform-tools/adb"
if [[ ! -x "$ADB_BIN" ]]; then
  ADB_BIN="$(command -v adb || true)"
fi
if [[ -z "$ADB_BIN" || ! -x "$ADB_BIN" ]]; then
  echo "error: adb not found under $SDK_ROOT/platform-tools or PATH" >&2
  exit 1
fi

EMULATOR_BIN="$SDK_ROOT/emulator/emulator"

online_device() {
  "$ADB_BIN" devices | awk 'NR>1 && $2=="device" {print $1; exit}'
}

DEVICE_SERIAL="$(online_device)"
if [[ -z "$DEVICE_SERIAL" ]]; then
  if [[ ! -x "$EMULATOR_BIN" ]]; then
    echo "error: no Android device online and emulator binary not found at $EMULATOR_BIN" >&2
    exit 1
  fi
  if [[ -z "$AVD_NAME" ]]; then
    AVDS="$("$EMULATOR_BIN" -list-avds || true)"
    if printf '%s\n' "$AVDS" | grep -qx "Pixel_10"; then
      AVD_NAME="Pixel_10"
    else
      AVD_NAME="$(printf '%s\n' "$AVDS" | head -n 1)"
    fi
  fi
  if [[ -z "$AVD_NAME" ]]; then
    echo "error: no Android Virtual Devices (AVDs) found; create one or pass --avd <name>" >&2
    exit 1
  fi

  echo "==> Starting Android Emulator AVD '$AVD_NAME'..."
  "$EMULATOR_BIN" -avd "$AVD_NAME" -no-snapshot-load >/dev/null 2>&1 &
  "$ADB_BIN" wait-for-device

  echo "==> Waiting for Android sys.boot_completed=1..."
  BOOT_WAIT=0
  until [[ "$("$ADB_BIN" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]]; do
    BOOT_WAIT=$((BOOT_WAIT + 1))
    if [[ "$BOOT_WAIT" -gt 120 ]]; then
      echo "error: timed out waiting for Android emulator boot" >&2
      exit 1
    fi
    sleep 1
  done
  DEVICE_SERIAL="$(online_device)"
fi

echo "==> Installing APK onto $DEVICE_SERIAL..."
"$ADB_BIN" -s "$DEVICE_SERIAL" install -r "$SIGNED_APK"

echo "==> Launching dev.nve.android/.NveMainActivity..."
"$ADB_BIN" -s "$DEVICE_SERIAL" shell am start -W -n dev.nve.android/.NveMainActivity

if [[ -n "$SCREENSHOT_PATH" ]]; then
  sleep 2
  mkdir -p "$(dirname "$SCREENSHOT_PATH")"
  "$ADB_BIN" -s "$DEVICE_SERIAL" exec-out screencap -p > "$SCREENSHOT_PATH"
  echo "==> Saved screenshot: $SCREENSHOT_PATH"
fi
