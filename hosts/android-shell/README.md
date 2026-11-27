# Android Host Shell (`hosts/android-shell`)

`libnve_android_shell` is the JNI half of `dev.nve.host.NativeRuntime`: it binds Java/Kotlin hosts
to `libnve_runtime` through `nve::host::RuntimeSession` — no `nve-fabric-bridge`.
The standalone zero-Gradle APK (`android-app/`) renders the runtime scene on native Android Views.

| File | Contents |
|---|---|
| `NveAndroidRuntimeJni.cpp` | create/destroy, app attach (`dlopen` + `nve_get_interface`, hot reload) / detach, `submitFrame`, viewport, tick, hit test, `scrollBy`, pairing |
| `NveAndroidSceneJni.cpp` | packed scene frame for `RuntimeSceneReader` (`long[]` header incl. `scene_epoch` + cached `DirectByteBuffer` slabs incl. `dirty_resources`), per-call resource views |
| `NveAndroidEventsJni.cpp` | `EventRing` (`nve_abi::encode_events`, all 10 payload tags) → `NveDirectEvent` batch, host text measurer and wake listener callbacks into Java |
| `android-app/` | `NveMainActivity` (lifecycle, pairing intent, keys), `NveSceneRenderer` (frame → Views), `NveViewStyle` (tables → View styles / TalkBack), `NveTextMeasurer` (`StaticLayout` measurement for runtime layout), `NveCanvasView` (`NVE_DRAW1`) |

Contracts locked at compile time: CMake reads the generated `NveRuntimeAbi.RUNTIME_SCHEMA_HASH` /
`STATUS_*` and the packed-frame `H_*` / `S_*` layout of `RuntimeSceneReader` from the Java sources,
and `NveAndroidSurface.hpp` `static_assert`s them against the generated C header. Every JNI entry
returns a non-negative `STATUS_*` (values go through out arrays). Every call that can write
releases the held scene first, so the runtime is never `Busy` because of Java.

`NveSceneRenderer` applies a frame incrementally only when it directly follows the previous one
in the same `scene_epoch`; otherwise it re-reconciles everything. Image / canvas views rebind when
their resource id changes or appears in `dirty_resources`. Renderer-caused focus / scroll / text
callbacks are suppressed while a frame is applied, and keys go to the focused View's node.

## Standalone APK & Emulator Launcher (`run-emulator.sh`)

```bash
# Build + verify the signed APK only (defaults to nve-example-kitchen-sink)
./hosts/android-shell/run-emulator.sh nve-example-kitchen-sink --build-only

# Build and launch on Pixel_10 (or the first AVD), with a screenshot
./hosts/android-shell/run-emulator.sh nve-example-kitchen-sink --avd Pixel_10 --screenshot /tmp/nve-android.png
```

1. Cross-compiles the app crate for `aarch64-linux-android` (NDK Clang).
2. Builds `libnve_android_shell.so` and the `libnve_runtime` staticlib (`-DNVE_RUNTIME_CARGO_TARGET=aarch64-linux-android`).
3. Compiles `dev.nve.host` (with its generated `dev.nve.host.generated` ABI / tables) and the Activity against `android.jar`, dexes via `d8`, packages, aligns, and signs `build/nve-android-shell.apk`.
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
  viewport, host text measurer (install / throwing / NaN / removal), hit test (miss and NaN are
  node 0), scroll clamping, `dirty_resources`, `SceneSemantics` (kinds, inherited typography),
  `RuntimeSurface` re-entrancy, app hot reload (v1 → v2, bad interface, non-ASCII missing
  library), detach.
- `android_events_jni_test`: every `EventRingWriter` payload (incl. key, pointer, layout, table
  events in the canonical field order) reaches a real attached app byte-exact; malformed rings
  never reach it; the app's re-render after the batch.
- `android_pair_jni_test`: loopback RFC 6455 pairing server, wake listener on a runtime thread,
  keyframe render, cross-thread `pairIsConnected`, URI validation, disconnect.
