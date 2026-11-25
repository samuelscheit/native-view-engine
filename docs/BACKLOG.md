# Backlog

Prioritized roadmap for `native-view-engine`, ordered from remaining platform verification and host hardening to core engine completeness and the flagship showcase.

## P0 — Verification & Platform Coverage

- [ ] **Live Windows verification on `samuel-pc`**
  - Run full MSVC + Windows SDK + `react-native-windows` 0.88 build and CTest suite.
  - Build and verify `nve_example_counter.dll` end-to-end against the live Windows host.

## P1 — Host & Runtime Hardening

- [ ] **Android JNI & Compose (`hosts/android-shell`, `hosts/compose-kit`)**
  - Swap `hosts/android-shell/NveAndroidSurfaceJni.cpp` snapshot transfer from JSON to a direct `ByteBuffer` binary buffer so Android stays zero-JSON end-to-end.
  - Add an Android Library Gradle module for `hosts/compose-kit` and `NveHostActivity.kt` CI tests.

- [ ] **Flutter & SwiftUI binary decoders**
  - Add direct binary `NveBinValue` decoding in `hosts/flutter-plugin` and `hosts/swiftui-kit`.

- [ ] **Standalone Flexbox (`cxx/nve-fabric-bridge`)**
  - Implement post-flex wrapping text re-measurement and true `Baseline` alignment in standalone `fabric_adapter.cpp`.

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
  - Integrate Dioxus signal table archiving into the relocatable state snapshot format (`crates/nve-engine/src/snapshot.rs`).
  - Add append-only WAL compaction for incremental state persistence between full snapshots.
  - Wire native host Frame-0 early-paint hook to replay the mapped `CommandBuffer` before runtime cold-boot completes.
  - Add WASM `wizer` pre-initialization pipeline for instant web snapshot hydration.

## Flagship / Vision

- [ ] **NVE Studio (Meta Workbench)**
  - Self-hosting live multi-host workbench rendering AppKit, WinUI, Qt 6, GTK4, iOS/Android Fabric, Flutter Canvas, and WASM surfaces side by side.
  - Live Taffy/Yoga layout switcher in Studio UI.
  - ~10 KB/s binary delta live pairing over QUIC/WebRTC.

- [ ] **Flagship Local-First Workspace & Messaging App**
  - Full Linear × Notion × Slack suite in one ~15 MB binary (`<50 ms` cold start, `<40 MB` RAM).
  - 100k-row virtualized 120fps views with real native OS text, IME, accessibility, and `<Island>` embeds.
  - Native-speed user plugins loaded via `nve_get_interface`.
