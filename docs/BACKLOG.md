# Backlog

Prioritized roadmap for `native-view-engine`, ordered from immediate branch integration and audit hardening to platform completeness and the flagship showcase.

## P0 — Immediate Integration & Verification

- [x] **Land Wave 2 branches onto `main`**
  - [x] Qt 6 desktop host (`hosts/qt-shell`) and GTK4 desktop host (`hosts/gtk-shell`).
  - [x] Windows / React Native Windows (`react-native-windows`) host (`hosts/windows-shell`).
  - [x] Composable generic `LayoutEngine` abstraction + `nve-layout-yoga` backend.
  - [x] Zero-copy direct FFI `NveValue` bump arena and binary `EventRing`.

- [ ] **Live Windows verification on `samuel-pc`**
  - Run full MSVC + Windows SDK + `react-native-windows` 0.88 build and CTest suite.
  - Build and verify `nve_example_counter.dll` end-to-end against the live Windows host.

- [x] **Unified local check & multi-OS CI**
  - [x] Add `scripts/check-all.sh` covering Rust workspace tests, C++ CTest, Apple/Swift/Flutter/WASM checks, and benchmarks.
  - [x] Add `.github/workflows/ci.yml` with matrix jobs across `macos-latest`, `ubuntu-latest`, and `windows-latest`.

## P1 — Host & Runtime Hardening (from audit)

- [x] **Apple hosts (`hosts/apple-common`, `hosts/macos-shell`, `hosts/ios-shell`)**
  - [x] Reconcile removed and reordered subviews during Pass 3 mounting.
  - [x] Size `NSScrollView.documentView` and `UIScrollView.contentSize` to child bounds.
  - [x] Wire `TextInput` `TextChange` events back through `EventRing`.
  - [x] Tear down click/tap gesture recognizers when nodes become non-interactive.
  - [x] Verify live window mouse-click pass-through on leaf `NSTextField` inside `Pressable`.

- [ ] **Android JNI & Compose (`hosts/android-shell`, `hosts/compose-kit`)**
  - [x] Migrate `hosts/android-shell/NveJniBridge.cpp` onto `nve_host_runtime.hpp` for atomic state-preserving hot-swap.
  - [x] Add `nativeDestroyHost` teardown to release native runtime and surface resources cleanly.
  - [ ] Add an Android Library Gradle module for `hosts/compose-kit` and `NveHostActivity.kt` CI tests.

- [ ] **Flutter & SwiftUI binary decoders**
  - [x] Bounds-check all `EventRing` payload reads in `nve_flutter.dart`.
  - [x] Abort cleanly on truncated records in `NveSurfaceRepresentable.swift`.
  - [ ] Add direct binary `NveBinValue` decoding in both host decoders.

- [ ] **`HeadlessBackend` & standalone Flexbox**
  - [x] Add a cycle, duplicate-child, root, and NaN/non-finite scalar guard to `HeadlessBackend`.
  - [ ] Implement post-flex wrapping text re-measurement and true `Baseline` alignment in standalone `fabric_adapter.cpp`.

- [ ] **Bi-directional codegen (`crates/nve-codegen`)**
  - Emit `react-native-windows` Fabric registration alongside iOS and Android.
  - Support zero-copy `NveValue` typed props and event bindings.

## P2 — Core Engine & Platform Completeness

- [ ] **GPU-accelerated Skia & native font shaping**
  - Wire hardware-accelerated `skia-safe` backends (Metal, Direct3D, Vulkan) in `crates/nve-backend-skia`.
  - Integrate native system font discovery and shaping (`cosmic-text` / platform shapers).

- [ ] **Web WASM E2E suite**
  - Add `wasm-bindgen-test` coverage and a headless browser E2E test suite for `hosts/web-shell`.

- [ ] **Qt Quick GPU adapter**
  - Add a GPU-composited `QQuickItem` adapter alongside `QWidget` in `hosts/qt-shell`.

- [ ] **Continuous benchmark regression tracking**
  - Track Criterion (`cargo bench`) and `nve_fabric_bridge_bench --json` baselines to catch frame/layout/ABI regressions.

- [ ] **Instant-Boot Frame-0 `CommandBuffer` + `mmap` Zero-Copy State Snapshot**
  - [x] Land core `crates/nve-engine/src/snapshot.rs` prototype: aligned `NVE_SNAP` binary container with FNV-1a checksum, crash-safe atomic writer, zero-dependency POSIX/Win32 `mmap` reader (`wasm32` fallback), and pointer-free `RelocatableStateArena` / `RelocatableTableView` zero-copy state lookup.
  - [ ] Integrate Dioxus signal table archiving into the relocatable state snapshot format.
  - [ ] Add append-only WAL compaction for incremental state persistence between full snapshots.
  - [ ] Wire native host Frame-0 early-paint hook to replay the mapped `CommandBuffer` before runtime cold-boot completes.
  - [ ] Add WASM `wizer` pre-initialization pipeline for instant web snapshot hydration.

## Flagship / Vision

- [ ] **NVE Studio (Meta Workbench)**
  - [x] Initial flagship POC (`examples/studio-workspace`): docked toggleable NVE Studio workbench with live self-patching (Accent, Radius, Density, X-Ray HUD), bounded 64-frame deterministic time-travel scrubber, and Tier-1/Tier-2 hot-reload state preservation.
  - [ ] Self-hosting live multi-host workbench rendering AppKit, WinUI, Qt 6, GTK4, iOS/Android Fabric, and WASM surfaces side by side.
  - [ ] Live Taffy/Yoga layout switcher in Studio UI.
  - [ ] ~10 KB/s binary delta live pairing over QUIC/WebRTC.

- [ ] **Flagship Local-First Workspace & Messaging App**
  - [x] Initial Aether Workspace POC (`examples/studio-workspace`): Issue Board, Team Channels (`# engineering`, `# design-sync`), and Live Metrics stress views.
  - [ ] Full Linear × Notion × Slack suite in one ~15 MB binary (`<50 ms` cold start, `<40 MB` RAM).
  - [ ] 100k-row virtualized 120fps views with real native OS text, IME, accessibility, and `<Island>` embeds.
  - [ ] Instant zero-install Web WASM share links from the same binary IR.
  - [ ] Native-speed user plugins loaded via `nve_get_interface`.
