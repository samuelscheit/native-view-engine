# Upstream Issues (`docs/UPSTREAM.md`)

Known bugs, quirks, and footguns discovered in upstream dependencies or platform toolkits, along with our local mitigations. Whenever an agent or delegate hits a real upstream issue, append a concise entry here with exact version/file/workaround details.

---

### React Native Yoga (`yogacore`)

- **`StyleValuePool` float-to-int overflow UB on large scalars**
  - **Observed**: Passing finite floats with large magnitude (`|v| > 1e7f`) into Yoga style setters triggers signed integer overflow UB inside Yoga's `StyleValuePool` under UBSan.
  - **Mitigation**: `cxx/nve-fabric-bridge/src/command_decoder.hpp` (`is_valid_scalar`) rejects non-finite values and scalars with `|v| > 1e7f` before they reach Yoga.
- **Detached `YGNodeRef` returns `NaN` layout metrics**
  - **Observed**: Reading `YGNodeLayoutGet{Left,Top,Width,Height}` on a node that was never laid out or was detached returns `NaN`, which triggers AppKit's `_NSViewValidateGeometry` `SIGTRAP` if forwarded into `NSView.frame`.
  - **Mitigation**: `cxx/nve-fabric-bridge/src/fabric_adapter.cpp` sanitizes non-finite layout outputs to `0.0f`, and `hosts/apple-common/NveAppleComponentHost.mm` guards `frame` geometry.

### Dioxus (`dioxus-core 0.7.10`)

- **Nested `Text` wrapper/leaf split for inline literals**
  - **Observed**: `text { style: "...", "literal" }` emits a styled parent `Text` element wrapping an unstyled child `Text` leaf node, so host shells that don't inherit text styles render invisible or unstyled leaves.
  - **Mitigation**: `crates/nve-dioxus/src/lib.rs` and `cxx/nve-fabric-bridge/src/fabric_adapter.cpp` propagate `text_color` and `font_size` from styled parent `Text` nodes down to unstyled child `Text` leaves; native hosts mount wrapper `Text` nodes with children as containers and leaf `Text` nodes as native text labels.
- **Event attribute `"on"` prefix mismatch**
  - **Observed**: `WriteMutations` delivers event attributes prefixed with `"on"` (e.g., `"onclick"`), while `VirtualDom::runtime().handle_event` expects the unprefixed name (`"click"`).
  - **Mitigation**: `crates/nve-dioxus/src/lib.rs` strips the leading `"on"` via `attr.name.get(2..)`.

### Rust `std` on `wasm32-unknown-unknown`

- **`std::time::Instant::now()` panics at runtime**
  - **Observed**: Calling `std::time::Instant::now()` inside `DioxusSurface::render_frame` causes an immediate runtime panic when booted in browser WASM (`hosts/web-shell`).
  - **Mitigation**: Target-gated `FrameTimer` in `crates/nve-dioxus/src/lib.rs` (real `Instant` on native, no-op on `wasm32`).

### Xcode 27 / CoreSimulator

- **`Simulator.app` replaced by `DeviceHub.app`**
  - **Observed**: In Xcode 27, `/Applications/Xcode.app/Contents/Developer/Applications/Simulator.app` is absent (`open -a Simulator` fails with `Unable to find application named 'Simulator'`); the Simulator GUI lives at `/Applications/Xcode.app/Contents/Applications/DeviceHub.app`.
  - **Mitigation**: `hosts/ios-shell/run-simulator.sh` checks `$DEV_DIR/Applications/Simulator.app` first, falls back to `$DEV_DIR/../Applications/DeviceHub.app`, and keeps GUI launch non-fatal so `xcrun simctl install` and `launch` always succeed.

### GTK 4 (`gtk4 4.24.2`)

- **`gtk_fixed_get_child_position` returns `(0, 0)` on unmapped widgets**
  - **Observed**: In GTK 4.24.2, `gtk_fixed_get_child_position` delegates to `gtk_widget_compute_point`, which returns `(0, 0)` for unmapped/offscreen widgets (such as headless CTest fixtures).
  - **Mitigation**: Read child translation from `gtk_fixed_get_child_transform` + `gsk_transform_to_translate` (see `hosts/gtk-shell/tests/gtk_shell_test.cpp`).

### Eclipse OpenJ9 (`openj9 0.59.0` / OpenJDK 25.0.3) + Android SDK `d8` (`build-tools 36.0.0`)

- **Scavenger GC assertion crash (`ScavengerRootScanner.hpp:105`) during `d8` dexing**
  - **Observed**: When Eclipse OpenJ9 0.59.0 is the default `java` on `PATH` on macOS arm64, Android `build-tools/36.0.0/d8` crashes inside `ForkJoinPool` (`com/android/tools/r8/internal/n30.<init>(I)V`) with `Misaligned object` / `j9mm.479 ** ASSERTION FAILED ** at .../openj9/runtime/gc_glue_java/ScavengerRootScanner.hpp:105` and hangs for ~70s writing multi-GB core/javacore/snap dumps.
  - **Mitigation**: `hosts/android-shell/run-emulator.sh` detects OpenJ9 and automatically switches `JAVA_HOME`/`PATH` to an installed HotSpot/JBR JDK (e.g., Android Studio JBR or Adoptium Temurin) before invoking `javac`/`d8`/`apksigner`, falling back to `-JXgcpolicy:optthruput -JXdump:none` if no HotSpot JDK is installed.

### iOS 27 Simulator SDK (`iPhoneSimulator27.0.sdk`)

- **Fatal `SIGTRAP` at launch without `UIScene` lifecycle adoption**
  - **Observed**: Apps linked against `iPhoneSimulator27.0.sdk` that still create a `UIWindow` in `UIApplicationDelegate` without `UIApplicationSceneManifest` / `UIWindowSceneDelegate` trap immediately on launch inside `UIKitCore`'s `___UIApplicationEvaluateRuntimeIssueForNoSceneLifecycleAdoption_block_invoke` (`EXC_BREAKPOINT` / `SIGTRAP`).
  - **Mitigation**: `hosts/ios-shell/Info.plist` declares `UIApplicationSceneManifest` (`NveIosSceneDelegate`) and `hosts/ios-shell/main.mm` creates the `UIWindow` via `initWithWindowScene:` inside `UIWindowSceneDelegate`'s `scene:willConnectToSession:options:`.
