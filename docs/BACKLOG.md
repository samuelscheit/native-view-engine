# Backlog

Prioritized roadmap for `native-view-engine`, ordered from remaining host hardening to core engine completeness and the flagship showcase.

## P1 — Host & Runtime Hardening

- [ ] **Dogfood Nitro-style hybrid generator on NVE's own core & scene ABI**
  - Generate `nve_abi.h` (fixing the stale Swift `CNveAbi/include/nve_abi.h` copy), `nve_scene_abi.h`, Dart `dart:ffi`, Android Java, and Web TS from NVE's unified `hybrid` codegen pipeline, delete legacy `abi_schema.rs`, and wire `cargo nve codegen {hybrid,abi}`.

## P2 — Core Engine & Platform Completeness

- [ ] **Direct3D 12 Skia GPU compile check on Windows (`crates/nve-backend-skia`)**
  - Verify `cargo check -p nve-backend-skia --features skia-d3d` on `samuel-pc` (Metal GPU and Vulkan compile/runtime are verified on macOS).

## Flagship / Vision

- [ ] **NVE Studio (Meta Workbench)**
  - Self-hosting live multi-host workbench rendering AppKit, WinUI, Qt 6, GTK4, iOS/Android Fabric, Flutter Canvas, and WASM surfaces side by side (live `NVD1` UDP/TCP/WebSocket binary delta transport at `5.18 KB/s` @ 60fps is landed in `crates/nve-interop/src/sync_transport.rs`).

- [ ] **Flagship Local-First Workspace & Messaging App**
  - Full Linear × Notion × Slack suite in one ~15 MB binary (`<50 ms` cold start, `<40 MB` RAM) with real native OS text, IME, accessibility, and `<Island>` embeds (100k-row virtualized 120fps stream, live Taffy/Yoga switcher, mobile portrait layout, and `nve_get_interface` plugin host are landed).
