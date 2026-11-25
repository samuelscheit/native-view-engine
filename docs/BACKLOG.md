# Backlog

Prioritized roadmap for `native-view-engine`, ordered from upcoming ergonomics & platform capabilities to the flagship showcase.

## P1 — Multi-Window, UI Component Kit & Native SDK (`expo-*` Equivalents)

- [ ] **Multi-Window Lifecycle Primitives (`crates/nve-abi`, `crates/nve-interop`, desktop/mobile/web hosts)**
  - Promote logical `SurfaceOrchestrator` surfaces into real multi-OS-window presentation (`PrimaryWindow`, `SecondaryWindow`, `Sheet`/`Popover`, `open_window`/`close_window`/`focus`/`set_title`) mapped 1:1 to `SurfaceId`s across `winit`/Skia, AppKit, Win32, Qt 6, GTK4, iPadOS scenes, and Web.

- [ ] **Tier-1 `nve-sdk` — Zero-Overhead Native Platform Capabilities (`NVE_IFACE_HOST_SERVICES` / binary C ABI)**
  - First-party, zero-JSON native modules avoiding JS-bridge overhead: Clipboard, File Picker & Standard Paths, Secure Store (Keychain / DPAPI / Secret Service / Android Keystore / Web Crypto), Deep Linking & URL Open, live System Theme & Locale (`NveChannelBus` wired end-to-end), Notifications, and Haptics.

- [ ] **`nve-ui` & `nve-router` — Reusable Accessible Component Kit & Type-Safe Router**
  - Accessible, themeable Dioxus component library (`Button`, `Input`, `Checkbox`, `Switch`, `Tabs`, `Dialog`, `Popover`, `DropdownMenu`, `Toast`, `Card`, `Badge`, `VirtualList`) with keyboard/ARIA parity plus a type-safe stack/tab/deep-link router (`nve-router`) integrated with the `navigation` channel.

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
