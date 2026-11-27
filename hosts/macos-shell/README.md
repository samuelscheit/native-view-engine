# macOS Native AppKit Host Shell (`hosts/macos-shell`)

Native macOS AppKit host shell and embeddable `react-native-macos` component view backed by `cxx/nve-fabric-bridge` (with real React Native Yoga Flexbox layout when `RN_REACT_COMMON_DIR` is supplied).

## Components

1. **Standalone Host Controller (`NveMacFabricHost.h` / `NveMacFabricHost.mm`):**
   Loads and hot-swaps Tier-2 Rust app `.dylib` bundles (`dlopen` / `nve_get_interface` / `NveDevApi::snapshot_state` / `restore_state`) and applies Tier-1 JSON hot patches without rebuilding the macOS host.
2. **Embeddable AppKit / RN-macOS View (`RCTNveMacComponentView.h` / `RCTNveMacComponentView.mm`):**
   Subclasses `RCTViewComponentView` under `-DNVE_WITH_RN_FABRIC=1` (or `NSView` standalone), uses flipped top-left coordinates to match Yoga/Fabric layout 1:1, reconciles live AppKit `NSView` / `NSTextField` / `NSScrollView` subviews, handles click gestures, and forwards outbound events from `NveComponentApi::drain_emitted_events`.
3. **Runnable Desktop Host (`main.mm`):**
   Launches a native macOS `NSWindow` hosting `RCTNveMacComponentView` with either `--dylib <path/to/libapp.dylib>` or an interactive built-in demo.

## Build & Test

```bash
# Build with real React Native Yoga + JSI:
cmake -S hosts/macos-shell -B hosts/macos-shell/build -G Ninja \
  -DRN_REACT_COMMON_DIR=/path/to/react-native/packages/react-native/ReactCommon
cmake --build hosts/macos-shell/build
ctest --test-dir hosts/macos-shell/build --output-on-failure

# Launch native AppKit host window:
./hosts/macos-shell/build/nve_macos_shell
# Or load a compiled Rust NVE cdylib:
./hosts/macos-shell/build/nve_macos_shell --dylib target/debug/libnve_studio_workspace.dylib
```
