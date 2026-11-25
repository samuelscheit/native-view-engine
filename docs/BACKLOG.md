# Backlog

Prioritized roadmap for `native-view-engine`, ordered from remaining host hardening to core engine completeness and the flagship showcase.

## P1 — Host & Runtime Hardening

- [ ] **Standalone Flexbox (`cxx/nve-fabric-bridge`)**
  - Implement post-flex wrapping text re-measurement and true `Baseline` alignment in standalone `fabric_adapter.cpp`.

- [ ] **Bi-directional codegen (`crates/nve-codegen`)**
  - Support zero-copy `NveValue` typed props and event bindings.

## P2 — Core Engine & Platform Completeness

- [ ] **Hardware GPU Skia swapchains (`crates/nve-backend-skia`)**
  - Wire live Metal, Direct3D 12, and Vulkan GPU context/swapchain creation behind `--features skia` (building on the `cosmic-text` shaper and `SkiaCanvasRenderer` / `GpuSurface` scaffolding).

- [ ] **Instant-Boot Frame-0 `CommandBuffer` + `mmap` Zero-Copy State Snapshot**
  - Integrate Dioxus signal table archiving into the relocatable state snapshot format (`crates/nve-engine/src/snapshot.rs`).
  - Add append-only WAL compaction for incremental state persistence between full snapshots.
  - Wire `--snapshot` replay/save into `hosts/gtk-shell` and mobile shells (macOS, Qt 6, and Windows shells are wired).
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
