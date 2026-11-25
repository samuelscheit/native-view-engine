# Backlog

Prioritized roadmap for `native-view-engine`, ordered from upcoming ergonomics & platform capabilities to the flagship showcase.

## P1 — Multi-Window, UI Component Kit & Native SDK (`expo-*` Equivalents)

- [ ] **Multi-Window Lifecycle Primitives (`crates/nve-abi`, `crates/nve-interop`, desktop/mobile/web hosts)**
  - Promote logical `SurfaceOrchestrator` surfaces into real multi-OS-window presentation (`PrimaryWindow`, `SecondaryWindow`, `Sheet`/`Popover`, `open_window`/`close_window`/`focus`/`set_title`) mapped 1:1 to `SurfaceId`s across `winit`/Skia, AppKit, Win32, Qt 6, GTK4, iPadOS scenes, and Web.

- [ ] **Tier-1 `nve-sdk` — Zero-Overhead Native Platform Capabilities (`NVE_IFACE_HOST_SERVICES` / binary C ABI)**
  - First-party, zero-JSON native modules avoiding JS-bridge overhead: Clipboard, File Picker & Standard Paths, Secure Store (Keychain / DPAPI / Secret Service / Android Keystore / Web Crypto), Deep Linking & URL Open, live System Theme & Locale (`NveChannelBus` wired end-to-end), Notifications, and Haptics.

- [ ] **`nve-ui` & `nve-router` — Reusable Accessible Component Kit & Type-Safe Router**
  - Accessible, themeable Dioxus component library (`Button`, `Input`, `Checkbox`, `Switch`, `Tabs`, `Dialog`, `Popover`, `DropdownMenu`, `Toast`, `Card`, `Badge`, `VirtualList`) with keyboard/ARIA parity plus a type-safe stack/tab/deep-link router (`nve-router`) integrated with the `navigation` channel.

## P1 — Architectural DRY, Single-Source ABI Lock & Zero-Copy Hot Paths (Wave 14)

- [ ] **Complete ABI Field-Offset Verification & Zero-Copy Engine/Codegen Hot Paths (`nve-abi`, `nve-engine`, `nve-codegen`)**
  - Add compile-time `size_of`/`align_of`/`offset_of!` assertions for every `#[repr(C)]` vtable and struct in `crates/nve-abi/src/{ffi.rs,host.rs}` cross-checked against `nve_abi.h` `offsetof(...)`.
  - Eliminate per-frame `.to_vec()` copies in `crates/nve-engine/src/loader.rs` (`render` and `drain_emitted_events`).
  - Remove `serde_json`/`JSONObject` from generated RN props/commands/events (`import_rn.rs`, `export_rn.rs`), replace `[String: Any]` / `buildMap<String, Any?>` in `export_swiftui.rs` / `export_compose.rs` with typed direct-FFI encoders, prune unused `export_*_json` code, and modularize >1,100-line codegen files (`core_abi.rs`, `parser.rs`, `emit_*.rs`).

- [ ] **Single-Source Cross-Language Wire Codecs & Remove Legacy Fallbacks (`nve-codegen`, `cxx`, `hosts/*`)**
  - Make `*_direct` entry points required and delete legacy `set_props` / ring `dispatch_events` fallback paths (`NveSurfaceController.swift`, `nve_rn_component.cpp`, `macos-shell/main.mm`, `windows-shell/main.cpp`).
  - Remove redundant `MethodChannel.invokeMethod` in Flutter `surface_widgets.dart` when direct FFI already ran, unify Kotlin/Java `NVE_MET1` readers (`NveSceneBuffer.kt`, `NveMainActivity.java`) and fix Kotlin per-update buffer copies, and generate shared `EventRing` / `NVE_DRAW1` codecs from specs.

- [ ] **Apple Shell DRY Consolidation & Oversized C++ Splits (`hosts/apple-common`, `cxx`, `hosts/android-shell`)**
  - Deduplicate `hosts/ios-shell/RCTNveComponentView.mm` & `hosts/macos-shell/RCTNveMacComponentView.mm` (plus shared `main.mm` CLI/pair plumbing) into `hosts/apple-common/`.
  - Split `mock_jsi_recorder.hpp` (1,250L), `nve_host_runtime.hpp` (1,170L), and `NveAndroidSurfaceJni.cpp` (1,229L) below 650 lines.

## P2 — Core Engine & Platform Completeness

- [x] **Direct3D 12 Skia GPU & Windows Native Host Compile/Runtime Verification (`crates/nve-backend-skia`, `hosts/windows-shell`)**
  - Verified live on Windows 11 Pro (Build 26200, MSVC `19.51`, `rustc 1.100.0`): Skia D3D12 (`--features skia-d3d`, 32/32 passed), full Rust workspace tests (257/257) + Clippy (`-D warnings`), `cxx/nve-fabric-bridge` (2/2, 0 `/W4` warnings), `hosts/windows-shell` Standalone & RNW 0.88 Fabric (`-DNVE_WITH_RNW_FABRIC=ON`, 3/3, 0 `/W4` warnings across all 63 Ninja targets, live Counter & Studio DLLs, WIC/Direct2D `Image` + `NVE_DRAW1` `SkiaCanvas` + modularized `NveWinD2D{Renderer,Media}.cpp`), and live `nve-skia-shell` Windows headless PNG render.

## Flagship / Vision

- [ ] **NVE Studio (Meta Workbench)**
  - Self-hosting multi-host workbench rendering AppKit, WinUI, Qt 6, GTK4, iOS/Android Fabric, Flutter Canvas, SwiftUI, Skia, and WASM surfaces side by side.
  - *Landed*: Bidirectional `NVT1`/`NVD1` binary delta transport (`InputEvents = 11`, `5.18 KB/s` @ 60fps under 15% loss + 20% reorder), `nve pair` / `nve dev --pair` daemon with live `SyncTelemetry` pushing + ephemeral port `NVE_PAIR_READY` readiness + live Chrome E2E (`[6/6]` Counter round-trip over binary WebSocket), zero-cdylib `--pair-connect` thin clients across **Skia Desktop**, **Web Browser (`?pair=ws://...`)**, **macOS AppKit**, **Qt 6 (Widgets + Quick)**, and **GTK4** (via shared zero-dep C++20 `nve::PairClient`), in-app **Multi-Viewport Meta Workbench** (`Desktop 1280×800` + `Tablet 820×1180` + `Mobile 390×844` + 9-host status grid), and native `NVE_DRAW1` `SkiaCanvas` rendering across all 9 hosts.
  - *Remaining*: Mobile thin-client pairing UI for Flutter/SwiftUI shells (iOS/Android/Windows WinSock2 `PairClient` landing in Wave 12B).

- [ ] **Flagship Local-First Workspace & Messaging App**
  - Full Linear × Notion × Slack suite in one ~15 MB binary (`<50 ms` cold start, `<40 MB` RAM) with real native OS text, IME, accessibility, and `<Island>` embeds.
  - *Landed*: 100k-row Fenwick virtualized stream, Notion-class **Block-Doc Editor** (`ActiveView::Docs` with `Heading`/`Paragraph`/`Checklist`/`Code`/`Callout` + Markdown export + Chrome E2E), crash-safe local-first **`MmapSnapshot` + `.nvewal` WAL persistence** (`NWS1` v2 + FNV-1a64 checksum + torn-tail recovery + auto compaction), **zero-dependency binary CRDT engine** (`crates/nve-interop/src/crdt/` `NVC1` + HLC/LWW/PnCounter/OrSet/RgaList + `StudioCrdtDoc` 5-peer partition-heal convergence), live Taffy/Yoga switcher, mobile portrait `< 620pt` collapse, `Cmd+K` palette, interactive `TextInput` composer & search, PNG `Image` avatars, `NVE_DRAW1` sparkline, `<Island>` sandbox, panic-isolated `nve_get_interface` plugin host, zero-JSON binary wire codecs (`NK1`/`NVP1`/`NWS1`/`NVE_MET1`/`NVC1`), `285.8 KiB` gzipped WASM release build, `cargo nve bench` Peak RSS / cold-start profiling, and native accessibility/IME parity across all 9 hosts.
  - *Remaining*: Live multi-peer CRDT transport wiring over paired `NVT1` sessions and `nve-ui`/`nve-router`/`nve-sdk` adoption.
