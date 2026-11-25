# Backlog

Prioritized roadmap for `native-view-engine`, ordered from remaining platform verification to the flagship showcase.

## P2 — Core Engine & Platform Completeness

- [ ] **Direct3D 12 Skia GPU & Windows Native Host Compile/Runtime Verification (`crates/nve-backend-skia`, `hosts/windows-shell`)**
  - Verify `cargo check -p nve-backend-skia --features skia-d3d` and the Windows `#ifdef _WIN32` WIC/Direct2D `Image` + `NVE_DRAW1` `SkiaCanvas` + modularized `NveWinD2D{Renderer,Media}.cpp` paths on `samuel-pc` once online (Metal GPU, Vulkan compile, and cross-platform Windows visual-tree/bridge CTests are verified on macOS).

## Flagship / Vision

- [ ] **NVE Studio (Meta Workbench)**
  - Self-hosting multi-host workbench rendering AppKit, WinUI, Qt 6, GTK4, iOS/Android Fabric, Flutter Canvas, SwiftUI, Skia, and WASM surfaces side by side.
  - *Landed*: Bidirectional `NVT1`/`NVD1` binary delta transport (`InputEvents = 11`, `5.18 KB/s` @ 60fps under 15% loss + 20% reorder), `nve pair` / `nve dev --pair` daemon with live `SyncTelemetry` pushing, zero-cdylib `--pair-connect` thin clients across **Skia Desktop**, **Web Browser (`?pair=ws://...`)**, **macOS AppKit**, **Qt 6 (Widgets + Quick)**, and **GTK4** (via shared zero-dep C++20 `nve::PairClient`), in-app **Multi-Viewport Meta Workbench** (`Desktop 1280×800` + `Tablet 820×1180` + `Mobile 390×844` + 9-host status grid), and native `NVE_DRAW1` `SkiaCanvas` rendering across all 9 hosts.
  - *Remaining*: Mobile thin-client pairing UI for iOS/Android/Flutter/SwiftUI shells and Windows WinSock port of `nve::PairClient`.

- [ ] **Flagship Local-First Workspace & Messaging App**
  - Full Linear × Notion × Slack suite in one ~15 MB binary (`<50 ms` cold start, `<40 MB` RAM) with real native OS text, IME, accessibility, and `<Island>` embeds.
  - *Landed*: 100k-row Fenwick virtualized stream, Notion-class **Block-Doc Editor** (`ActiveView::Docs` with `Heading`/`Paragraph`/`Checklist`/`Code`/`Callout` + Markdown export), crash-safe local-first **`MmapSnapshot` + `.nvewal` WAL persistence** (`NWS1` v2 + FNV-1a64 checksum + torn-tail recovery + auto compaction), live Taffy/Yoga switcher, mobile portrait `< 620pt` collapse, `Cmd+K` palette, interactive `TextInput` composer & search, PNG `Image` avatars, `NVE_DRAW1` sparkline, `<Island>` sandbox, panic-isolated `nve_get_interface` plugin host, zero-JSON binary wire codecs (`NK1`/`NVP1`/`NWS1`/`NVE_MET1`), `285.8 KiB` gzipped WASM release build, `cargo nve bench` Peak RSS / cold-start profiling, and native accessibility/IME parity across all 9 hosts.
  - *Remaining*: Multi-device CRDT merge/conflict resolution across paired peers.
