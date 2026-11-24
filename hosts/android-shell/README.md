# Android Prebuilt Host Shell (`hosts/android-shell`)

Built once against React Native Android Prefab (`ReactAndroid::fabricjni` + `nve_fabric_bridge`).

- **Tier 1 (<50 ms, Emulator & physical Android):** WebSocket `TemplatePatch` applied in-place.
- **Tier 2 (150–400 ms, Emulator & physical Android):** `nve dev` compiles only the app crate for `aarch64-linux-android` (`lib<app>.so`), pushes it via `adb push` to the app's `filesDir`, and calls `NveNativeBridge.nativeHotSwapApp(handle, path)` — zero Gradle rebuilds.
