# Backlog

Prioritized roadmap for `native-view-engine`, ordered from remaining platform verification to the flagship showcase.

## P2 — Core Engine & Platform Completeness

- [ ] **Direct3D 12 Skia GPU & Windows Native Host Compile/Runtime Verification (`crates/nve-backend-skia`, `hosts/windows-shell`)**
  - Verify `cargo check -p nve-backend-skia --features skia-d3d` and the Windows `#ifdef _WIN32` WIC/Direct2D `Image` + `NVE_DRAW1` `SkiaCanvas` + modularized `NveWinD2D{Renderer,Media}.cpp` paths on `samuel-pc` once online (Metal GPU, Vulkan compile, and cross-platform Windows visual-tree/bridge CTests are verified on macOS).

## Flagship / Vision

- [ ] **NVE Studio (Meta Workbench)**
  - Self-hosting multi-host workbench rendering AppKit, WinUI, Qt 6, GTK4, iOS/Android Fabric, Flutter Canvas, SwiftUI, Skia, and WASM surfaces side by side.
  - *Landed*: Bidirectional `NVT1`/`NVD1` binary delta transport (`InputEvents = 11`, `5.18 KB/s` @ 60fps under 15% loss + 20% reorder), `nve pair` / `nve dev --pair` daemon with live `SyncTelemetry` pushing, zero-cdylib thin-client pairing in `nve-skia-shell --pair-connect` (UDP/TCP/WS) and `hosts/web-shell` (`?pair=ws://...`), and native `NVE_DRAW1` `SkiaCanvas` rendering across all 9 hosts.
  - *Remaining*: Native thin-client `--pair-connect` adapters in Apple/Qt/GTK/Android shells and side-by-side multi-viewport split view inside Studio.

- [ ] **Flagship Local-First Workspace & Messaging App**
  - Full Linear × Notion × Slack suite in one ~15 MB binary (`<50 ms` cold start, `<40 MB` RAM) with real native OS text, IME, accessibility, and `<Island>` embeds.
  - *Landed*: 100k-row Fenwick virtualized stream, live Taffy/Yoga switcher, mobile portrait `< 620pt` collapse, `Cmd+K` palette, interactive `TextInput` composer & search, PNG `Image` avatars, `NVE_DRAW1` sparkline, `<Island>` sandbox, panic-isolated `nve_get_interface` plugin host, zero-JSON binary wire codecs (`NK1`/`NVP1`/`NWS1`), and native accessibility/IME parity across Apple, Qt 6, GTK4, Web ARIA, Android TalkBack, Compose Semantics, Flutter Semantics, SwiftUI VoiceOver, and Skia Desktop.
  - *Remaining*: Local-first CRDT/SQLite persistence layer, real-time multi-device sync UI, and cold-start / memory budget profiling on release binaries.
