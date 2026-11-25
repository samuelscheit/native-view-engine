# Backlog

Prioritized roadmap for `native-view-engine`, ordered from remaining host hardening to core engine completeness and the flagship showcase.

## P1 — Host & Runtime Hardening

- [ ] **Bi-directional codegen (`crates/nve-codegen`)**
  - Support zero-copy `NveValue` typed props and event bindings.

## P2 — Core Engine & Platform Completeness

- [ ] **Direct3D 12 Skia GPU compile check on Windows (`crates/nve-backend-skia`)**
  - Verify `cargo check -p nve-backend-skia --features skia-d3d` on `samuel-pc` (Metal GPU and Vulkan compile/runtime are verified on macOS).

## Flagship / Vision

- [ ] **NVE Studio (Meta Workbench)**
  - Self-hosting live multi-host workbench rendering AppKit, WinUI, Qt 6, GTK4, iOS/Android Fabric, Flutter Canvas, and WASM surfaces side by side.
  - Live Taffy/Yoga layout switcher in Studio UI.
  - ~10 KB/s binary delta live pairing over QUIC/WebRTC.

- [ ] **Flagship Local-First Workspace & Messaging App**
  - Full Linear × Notion × Slack suite in one ~15 MB binary (`<50 ms` cold start, `<40 MB` RAM).
  - 100k-row virtualized 120fps views with real native OS text, IME, accessibility, and `<Island>` embeds.
  - Native-speed user plugins loaded via `nve_get_interface`.
