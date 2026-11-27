# Android Host Shell (`hosts/android-shell`)

`libnve_android_shell` is the JNI half of `dev.nve.host.NativeRuntime`: it binds Java/Kotlin hosts
to `libnve_runtime` (runtime ABI v2) through `nve::host::RuntimeSession` — no `nve-fabric-bridge`.
The standalone zero-Gradle APK (`android-app/`) renders the runtime scene on native Android Views.

| File | Contents |
|---|---|
| `NveAndroidRuntimeJni.cpp` | create/destroy, app attach (`dlopen` + `nve_get_interface`, hot reload) / detach, `submitFrame`, viewport, tick, hit test, `scrollBy`, pairing |
| `NveAndroidSceneJni.cpp` | packed scene frame for `RuntimeSceneReader` (`long[]` header + cached `DirectByteBuffer` slabs), resource bytes |
| `NveAndroidEventsJni.cpp` | `EventRing` → `NveDirectEvent` batch, host text measurer and wake listener callbacks into Java |
| `android-app/` | `NveMainActivity` (lifecycle, pairing intent, keys), `NveSceneRenderer` (frame → Views), `NveViewStyle` (tables → View styles / TalkBack), `NveTextMeasurer` (`StaticLayout` measurement for runtime layout), `NveCanvasView` (`NVE_DRAW1`) |

Contracts locked at compile time: CMake reads `RuntimeSceneReader.SCHEMA`, the packed-frame
`H_*` / `S_*` strides, and `NativeRuntime.STATUS_*` from the Java sources and
`NveAndroidSurface.hpp` `static_assert`s them against the generated C header. Every call that
can write releases the held scene first, so the runtime is never `Busy` because of Java.

## Standalone APK & Emulator Launcher (`run-emulator.sh`)

```bash
# Build + verify the signed APK only (defaults to nve-example-kitchen-sink)
./hosts/android-shell/run-emulator.sh nve-example-kitchen-sink --build-only

# Build and launch on Pixel_10 (or the first AVD), with a screenshot
./hosts/android-shell/run-emulator.sh nve-example-kitchen-sink --avd Pixel_10 --screenshot /tmp/nve-android.png
```

1. Cross-compiles the app crate for `aarch64-linux-android` (NDK Clang).
2. Builds `libnve_android_shell.so` and the `libnve_runtime` staticlib (`-DNVE_RUNTIME_CARGO_TARGET=aarch64-linux-android`).
3. Compiles `dev.nve.host`, the generated `NveCoreAbi` / `NveTables`, and the Activity against `android.jar`, dexes via `d8`, packages, aligns, and signs `build/nve-android-shell.apk`.
4. Unless `--build-only`, boots an AVD, installs, and launches `dev.nve.android/.NveMainActivity`.

Pairing: launch with `--es pair_connect ws://host:port/nve-wire --es pair_token <token>` or an
`nve://pair?endpoint=...&token=...` link (TLS transports need `cert_sha256` in the URI).

## End-to-end JNI suites (CTest, desktop JVM)

The same JNI library builds for the desktop and runs under a real JVM against real `NVE2` frames
from `nve-host-fixtures` (production `NodeStore` + standard tables, image and canvas resources):

```bash
cmake -S hosts/android-shell -B /tmp/nve-android-host-test \
  && cmake --build /tmp/nve-android-host-test \
  && ctest --test-dir /tmp/nve-android-host-test --output-on-failure
```

- `android_scene_jni_test`: packed frames over every standard table, resources (URL, inline bytes,
  canvas draw list mirrored raw), deltas (edit, image swap, removal), validation of bad input,
  viewport, host text measurer (install / fallback / removal), hit test, scroll clamping, app hot
  reload (v1 → v2, bad interface, missing library), detach.
- `android_events_jni_test`: every `EventRingWriter` payload reaches a real attached app byte-exact;
  malformed rings never reach it; the app's re-render after the batch.
- `android_pair_jni_test`: loopback RFC 6455 pairing server, wake listener on a runtime thread,
  keyframe render, cross-thread `pairIsConnected`, URI validation, disconnect.
