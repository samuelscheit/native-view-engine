# Android Prebuilt Host Shell (`hosts/android-shell`)

Built once against React Native Android Prefab (`ReactAndroid::fabricjni` + `nve_fabric_bridge`), or packaged as a standalone zero-Gradle native Android APK via `run-emulator.sh`.

- **Tier 1 (<50 ms, Emulator & physical Android):** WebSocket `TemplatePatch` applied in-place.
- **Tier 2 (150–400 ms, Emulator & physical Android):** `nve dev` compiles only the app crate for `aarch64-linux-android` (`lib<app>.so`), pushes it via `adb push` to the app's `filesDir`, and calls `NveMainActivity.nativeLoadDylib(handle, path)` — zero Gradle rebuilds.

## Standalone Zero-Gradle APK & Emulator Launcher (`run-emulator.sh`)

Build a signed `arm64-v8a` Android APK (`dev.nve.android`) directly from `$ANDROID_HOME` (`javac` + `d8` + `aapt2` + `zipalign` + `apksigner`, no Gradle/Maven downloads) and launch it on an Android Emulator or connected device:

```bash
# Build + verify signed APK only (defaults to nve-example-counter)
./hosts/android-shell/run-emulator.sh nve-example-counter --build-only

# Build and launch nve-studio-workspace on Pixel_10 (or first available AVD)
./hosts/android-shell/run-emulator.sh nve-studio-workspace

# Specify an AVD and capture a screenshot after launch
./hosts/android-shell/run-emulator.sh nve-example-counter --avd Pixel_10 --screenshot /tmp/nve-android.png
```

### What `run-emulator.sh` Does

1. Auto-detects `ANDROID_HOME` (or `$HOME/Library/Android/sdk` / `$HOME/Android/Sdk`), latest installed NDK, `build-tools`, and `platforms/android-*`.
2. Cross-compiles the selected Rust app crate (`nve-example-counter` or `nve-studio-workspace`) for `aarch64-linux-android` using the NDK Clang linker.
3. Cross-compiles `libnve_android_shell.so` (`arm64-v8a`, `android-24`, `c++_shared`) with `NveAndroidSurfaceJni.cpp` backed by `nve::NativeHostSession`.
4. Compiles `android-app/src/dev/nve/android/NveMainActivity.java` against `android.jar`, dexes via `d8`, packages `AndroidManifest.xml`, `classes.dex`, and `lib/arm64-v8a/{libnve_android_shell.so,libnve_app.so,libc++_shared.so}`, aligns with `zipalign`, and debug-signs + verifies `hosts/android-shell/build/nve-android-shell.apk` via `apksigner`.
5. Unless `--build-only` is passed, boots the requested/default AVD if no `adb` device is online, installs the APK (`adb install -r`), launches `dev.nve.android/.NveMainActivity`, and optionally captures a PNG screenshot.

## Host-Side JNI Bridge Tests (CTest)

```bash
cmake -S hosts/android-shell -B /tmp/nve-android-host-test -G Ninja \
  && cmake --build /tmp/nve-android-host-test \
  && ctest --test-dir /tmp/nve-android-host-test --output-on-failure
```
