# macOS Native AppKit Host Shell (`hosts/macos-shell`)

Native macOS AppKit host shell and embeddable `react-native-macos` component view over
**`libnve_runtime`** (`nve::runtime` from `cxx/cmake/NveRuntime.cmake`). The runtime hosts the app
(`nve_get_interface`), reconciles, lays out (native `NSString` text measurement), and pairs; the
shared `hosts/apple-common` host realizes its zero-copy scene as native views.

## Components

1. **Embeddable AppKit / RN-macOS view (`RCTNveMacComponentView`)**: subclasses
   `RCTViewComponentView` under `-DNVE_WITH_RN_FABRIC=1` (else `NSView`), flipped top-left layout,
   realizes `NSTextField` / `NSSecureTextField` / `NveAppleImageView` / `NSScrollView` / container
   views per node from the generated core tables, forwards clicks, text (IME-safe), focus, submit,
   and scroll input, and emits outbound app events.
2. **Multi-window manager (`NveMacWindowManager`)**: one runtime surface per window, modal blocking,
   close vetoes, cascading close.
3. **Runnable desktop host (`main.mm`)**: `--dylib <path>` (Tier-2 reload keeps state), the built-in
   demo app (`apple-common/NveAppleDemoApp.hpp`), `--pair-connect <nve://pair?...> [--pair-token]`,
   and instant boot (`--snapshot`, `--no-snapshot`, `--reset-state`, `--schema`; env
   `NVE_NO_SNAPSHOT`, `NVE_RESET_STATE`, `NVE_SNAPSHOT_DIR`) via `nve::host::SnapshotFile`.

## Build & Test

```bash
cargo run --locked -p nve-cli -- codegen abi
cmake -S hosts/macos-shell -B hosts/macos-shell/build -G Ninja
cmake --build hosts/macos-shell/build        # builds libnve_runtime via cargo on demand
ctest --test-dir hosts/macos-shell/build --output-on-failure

# Real Rust apps (as scripts/check-all.sh runs them):
NVE_TEST_KITCHEN_SINK_DYLIB=target/debug/libnve_example_kitchen_sink.dylib \
NVE_TEST_STUDIO_DYLIB=target/debug/libnve_studio_workspace.dylib \
  hosts/macos-shell/build/nve_macos_shell_test

./hosts/macos-shell/build/nve_macos_shell --dylib target/debug/libmy_app.dylib
```