# Android Host Shell (`hosts/android-shell`)

`libnve_android_shell` is the JNI half of `dev.nve.host.NativeRuntime`: it binds Java/Kotlin hosts
to `libnve_runtime` through `nve::host::RuntimeSession`.
The standalone zero-Gradle APK (`android-app/`) renders the runtime scene on native Android Views.

| File | Contents |
|---|---|
| `NveAndroidRuntimeJni.cpp` | create / destroy (owner thread only), app attach via the runtime's `load_app_module` (hot reload, optional state carry; loaded in place on Android) / detach, `submitFrame`, viewport, tick, hit test, `scrollBy`, pairing |
| `NveAndroidSceneJni.cpp` | packed scene frame in the generated `NveJniFrame` layout (`long[]` header + cached `DirectByteBuffer` slabs: deltas incl. `moved_indices` / `dirty_resources`, `scroll_offsets`, tables with their sparse row index), per-call resource views |
| `NveAndroidEventsJni.cpp` | `NVEV` v2 `EventRing` (`EventRingWriter`; header, arena spans, framing flags, and UTF-8 validated) → 64-byte-aligned `NveEventBatch`, host text measurer and wake listener callbacks into Java (runtime threads attached once as daemons) |
| `android-app/` | `NveMainActivity` (lifecycle, pairing intent, keys), `NveSceneRenderer` (frame → Views), `NveViewStyle` (tables → View styles / TalkBack), `NveTextMeasurer` (`StaticLayout` measurement for runtime layout), `NveCanvasView` (`nve.res.draw_list` `FlatResource`) |

Contracts locked at compile time: the packed frame layout is generated once (`nve codegen abi`:
`NveJniFrame.java` + `nve_jni_frame.generated.hpp`), and CMake turns every `STATUS_*`, the
runtime schema hash, and the frame `LAYOUT_HASH` of the generated Java into unconditional
`static_assert`s (`nve_java_contract.generated.hpp`). Every JNI entry returns a non-negative
`STATUS_*` (values go through out arrays), checks and clears pending Java exceptions after each
JNI call that can raise, and releases the held scene first, so the runtime is never `Busy`
because of Java.

`NveSceneRenderer` is O(changed): a frame directly following the previous one in the same
`scene_epoch` touches only removed ids, content-dirty nodes (incl. inherited typography), parents
whose children changed, moved nodes (geometry), and the views bound to its `dirty_resources`
(indexed by resource id); a redelivered frame is a no-op; a new epoch drops every view first; any
other gap reconciles all live nodes. Runtime `scroll_offsets` drive scroll-to / restore. Inputs
honor `nve.interaction.disabled`, keep the caret when the app rewrites the value, repair lone
surrogates, and submit once per Enter. Renderer-caused focus / scroll / text callbacks are
suppressed while a frame is applied, and keys go to the focused View's node. Its unit suites run
under Robolectric in `hosts/compose-kit`.

## Standalone APK & Emulator Launcher (`run-emulator.sh`)

```bash
# Build + verify the signed APK only (the app crate defaults to nve-example-kitchen-sink)
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

The same JNI library builds for the desktop and runs under a real JVM (`-Xcheck:jni`, so any JNI
misuse aborts) against real `NVE2` frames from `nve-host-fixtures` (production `NodeStore` +
standard tables, image and canvas resources):

```bash
cmake -S hosts/android-shell -B /tmp/nve-android-host-test \
  && cmake --build /tmp/nve-android-host-test \
  && ctest --test-dir /tmp/nve-android-host-test --output-on-failure
```

- `android_scene_jni_test`: packed frames over every standard table, resources (URL, inline bytes,
  canvas draw list mirrored raw), deltas (edit, image swap, removal), validation of bad input,
  viewport, host text measurer (install / throwing / NaN / removal), hit test (miss and NaN are
  node 0), scroll clamping, `dirty_resources`, `SceneSemantics` (kinds, inherited typography),
  `RuntimeSurface` re-entrancy, `load_app_module` hot reload (v1 → v2, bad interface, non-ASCII
  missing library), `scene_epoch` across reload / pairing / detach, pairing replacing a running app
  (invalid URIs keep it), recycled and ephemeral `dirty_resources`, off-thread destroy refused.
- `android_events_jni_test`: every `EventRingWriter` payload (incl. key, pointer, layout, table
  events in the canonical field order) reaches a real attached app byte-exact; malformed rings
  never reach it; the app's re-render after the batch.
- `android_pair_jni_test`: loopback RFC 6455 pairing server, wake listener on a runtime thread,
  keyframe render, cross-thread `pairIsConnected`, URI validation, disconnect.
