# AGENTS.md — `native-view-engine`

Concise architecture invariants, engineering rules, and verified build/test commands for agents working in this repository.

## Architecture Invariants

- **Single binary IR & modular C ABI** (`crates/nve-abi`, `cxx/nve-fabric-bridge/include/nve_abi.h`, `crates/nve-engine`):
  - Every frame crosses the Rust/host boundary as one flat, batched binary `CommandBuffer` and `EventRing` (`submit_frame`). Never add per-prop FFI/JNI crossings or `folly::dynamic` serialization.
  - Expose app modules solely via `nve_get_interface(uint32_t)` (`NVE_IFACE_SURFACE=1`, `NVE_IFACE_LAYOUT=2`, `NVE_IFACE_COMPONENT=3`, `NVE_IFACE_DEV=4`), backed by the safe Rust `NveApp` trait and `export_nve_app!` macro. No monolithic vtables or legacy compatibility shims.
  - Keep `crates/nve-engine` framework-agnostic (zero dependency on `dioxus-core`).
- **Multi-backend & interop architecture**:
  - **Frontend**: `crates/nve-dioxus` — Dioxus 0.7 `WriteMutations` adapter emitting `CommandBuffer`s + Tier-1 RSX hot-patch.
  - **C++20 Fabric/Yoga/JSI bridge**: `cxx/nve-fabric-bridge` — zero-Hermes React Native Fabric/Yoga adapter with mock `jsi::Runtime` event recorder (`NVE_WITH_RN_YOGA`, `NVE_WITH_RN_JSI`, `NVE_WITH_RN_FABRIC`).
  - **Desktop Skia/winit**: `crates/nve-backend-skia` (`--features desktop` for winit/softbuffer, `--features skia` for `skia-safe`).
  - **Web WASM DOM**: `crates/nve-backend-web` + `hosts/web-shell` — decodes the same `CommandBuffer` onto browser DOM + CSS Flexbox.
  - **Headless**: `crates/nve-backend-headless` — pure-Rust Taffy layout + mutation recorder for sub-second tests.
  - **Bi-directional 5-target codegen**: `crates/nve-codegen` — `import_rn` (`*NativeComponent.{ts,js}` / `Native*.{ts,js}`) and `export` from one `NveExportSpec` to React Native Fabric, Flutter `dart:ffi`, SwiftUI, Jetpack Compose, and Web Components.
  - **Universal interop**: `crates/nve-interop` — `SurfaceOrchestrator`, `LayoutNegotiator`, `GestureArbitrator`, `NveChannelBus`, `RenderHandoff`, and opt-in `<Island>` specs.

## Engineering Rules

- **Strict DRY across platform hosts**: Put shared Apple UIKit/AppKit host logic (ABI loading, hot-swap, style/event/tree helpers) in `hosts/apple-common/` — never copy-paste between `hosts/ios-shell/` and `hosts/macos-shell/`.
- **Zero backwards compatibility**: Refactor cleanly and delete dead/legacy code, obsolete exports, and shims instead of preserving compatibility layers.
- **Root causes only**: Fix underlying invariants rather than patching symptoms.
- **Deep tests & live benchmarks**: Write edge-case, property, and fuzz tests (truncated/corrupt buffers, layout/style inheritance, hot-swap state preservation, ABI error paths) instead of smoke tests. Keep statistical benchmarks (`cargo bench`, `nve_fabric_bridge_bench`) green.
- **Keep files modular**: Split oversized files along clear subsystem boundaries.

## Orchestration & Git Workflow

- **Delegation**: Always use T3 `delegate_task` (never native subagents) for delegation; pass `isolation: "worktree"` for parallel work.
- **Never commit build or trace artifacts**: Keep `*.trace.json`, `*.perfetto.json`, `cxx/build*`, `hosts/*/build`, `hosts/swiftui-kit/.build`, `hosts/flutter-plugin/.dart_tool`, and compiled libraries/objects out of git.
- **Clean commits**: Inspect `git diff`, then commit only your files using a conventional commit message on stdin:
  ```bash
  git commit --only <paths> -F - <<'EOF'
  feat(scope): concise summary
  EOF
  ```

## Canonical Build, Test & Benchmark Commands

```bash
# Rust workspace tests & statistical benchmarks (plus CLI frame bench)
cargo test --workspace
cargo bench --workspace
cargo nve bench

# Build nve-example-counter cdylib (Tier-2 hot-swap & native host embedding)
cargo build -p nve-example-counter

# C++20 Fabric/Yoga/JSI bridge CTest & benchmark
# (omit -DRN_REACT_COMMON_DIR for standalone mode without RN Yoga/JSI)
cmake -S cxx/nve-fabric-bridge -B cxx/build -G Ninja \
  -DRN_REACT_COMMON_DIR=/path/to/react-native/packages/react-native/ReactCommon \
  && cmake --build cxx/build \
  && ctest --test-dir cxx/build --output-on-failure
./cxx/build/nve_fabric_bridge_bench

# macOS AppKit host CTest (optionally pass -DRN_REACT_COMMON_DIR=...)
cmake -S hosts/macos-shell -B hosts/macos-shell/build -G Ninja \
  && cmake --build hosts/macos-shell/build \
  && ctest --test-dir hosts/macos-shell/build --output-on-failure

# iOS Simulator SDK Objective-C++ syntax check
xcrun --sdk iphonesimulator clang++ -std=c++20 -fobjc-arc -fsyntax-only \
  -Icxx/nve-fabric-bridge/include -Ihosts/apple-common -Ihosts/ios-shell \
  hosts/ios-shell/NveFabricHost.mm hosts/ios-shell/RCTNveComponentView.mm

# SwiftUI host kit tests
swift test --package-path hosts/swiftui-kit

# Flutter plugin tests
(cd hosts/flutter-plugin && flutter test)

# Web WASM host unit tests, wasm32 compile check, and full JS/WASM bundle
cargo test --locked -p nve-backend-web -p nve-web-shell \
  && cargo check --locked -p nve-web-shell --target wasm32-unknown-unknown
./hosts/web-shell/build.sh
```

## Learned rules

- **Zero-overhead first**: For every boundary (FFI, events, interop, host bridges), research proven designs on the web and choose the zero-cost one, even when it is harder and takes longer. Never ship a quick solution that adds runtime overhead, such as serializing to JSON, wrapper layers or extra copies.
- **Delegate only real work**: Do small tasks yourself, such as a backlog or doc entry, instead of spawning a delegate. Give a delegate its own worktree only when it runs in parallel with others that edit the same files. Otherwise let it work in the shared checkout.
- **Merge as soon as done**: When a delegate's branch is finished and verified, merge it into `main` immediately, or have the delegate merge itself. Do not batch finished branches while waiting on others.
- **Document upstream issues**: Whenever an agent or delegate discovers an upstream bug, quirk, or footgun in a dependency or platform toolkit, document it concisely (version, symptom, and mitigation) in `docs/UPSTREAM.md`.
