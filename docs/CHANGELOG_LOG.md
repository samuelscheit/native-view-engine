# Engineering Completion Log (`docs/CHANGELOG_LOG.md`)

Chronological log of completed, verified milestones in `native-view-engine`.

---

## Docs & Web Foundations (Phase 1B–1C)

- **`crates/nve-mdx` Pure-Rust `.md`/`.mdx` Compiler & `NVS1` Search Index** (`47e793b`)
  - Zero-dependency CommonMark/GFM parser, YAML/TOML frontmatter, Nextra-style `_meta.toml` navigation + `PrevNext`, fenced directives (`:::callout`, `:::tabs`, `:::steps`, `:::cards`, `:::demo`), declarative `<Component prop="val" num={42} />` tags + ` ```rsx live ` blocks, compile-time schema/link/anchor validator, 13-language pure-Rust syntax highlighter, deterministic Rust/Dioxus codegen, and binary `NVS1` full-text search index (`< 1 ms` queries, FNV-1a64 checksum). 12/12 tests, 0 Clippy warnings, `wasm32` clean.
- **`nve-router::web` Browser History API Adapter** (`0827eab`)
  - Cross-platform `HistoryBackend` (`MemoryHistoryBackend` + `wasm32` `BrowserHistoryBackend`), `WebHistoryAdapter` with GitHub Pages `base_path` normalization, deduplicated `pushState`/`replaceState`, loop-free `popstate`, browser-accurate link interception, and `#anchor` scrolling. 17/17 tests, 0 native/`wasm32` Clippy warnings.

---

## Wave 13 — Multi-Window, Native SDK, UI Component Kit, Router & Direct FFI DRY

- **Swift/Dart Direct FFI Codegen DRY, `NveDirectEncodable` & C11/Rust `offset_of!` ABI Checks** (`66832d2`)
  - Split `NveDirectFfi.swift` (551L) into `NveDirectArena.swift` (188L), `NveDirectValueEncoding.swift` (268L), and `NveDirectWriters.swift` (246L) and removed its line-count test exemption.
  - Added zero-boxing `NveDirectEncodable` + `StaticString` keys (`NveDirectPropsWriter.withEncodedProps`) across SwiftUI and generated Swift (deleting `pinnedBuffers`), unified Dart codegen onto shared `nve_bin_codec.dart` + `nve_hybrid_runtime.dart` + `direct_ffi.dart`, and added compile-time `offset_of!` assertions on `ffi_types.rs` cross-checked against C11 `_Static_assert(offsetof(...))` in `nve_abi.h`.
  - Verified: 52/52 Rust, 42/42 Swift, 52/52 Flutter (`0` analyze issues).
- **Multi-Window Lifecycle Primitives Across 7 Hosts** (`21d86ac`, `b1a7922`)
  - `SurfacePresentation`, `WindowManager` (modal parent focus blocking, cascading child sheet/popover close, cancelable `CloseRequested` veto), and zero-JSON binary `NVW1` command/event codec in `crates/nve-interop`.
  - Multi-window managers across Skia `winit`, macOS AppKit (`NveMacWindowManager`), iOS/iPadOS `UIWindowScene` allocator, Windows Win32 (`NveWinWindowManager`), GTK4 (`NveGtkWindowManager`), Qt 6 (`NveQtWindowManager`), and Web WASM (`ACTIVE_SHELLS` + `BroadcastChannel`).
- **`NVE_IFACE_HOST = 5` Binary Capability ABI & Tier-1 `crates/nve-sdk`** (`0aeee16`, `9174788`)
  - Single zero-JSON binary host-capability interface (`NVE_IFACE_HOST = 5`, `NVH1`, `NVT2`, `NVL1`) regenerated across all 5 ABI targets (`0x54A4A32B6F258107`).
  - Zero-dependency Tier-1 native capabilities: Clipboard, Filesystem/File Picker, Secure Store (macOS Keychain / Win32 CredMan / Unix fallback), Linking, live System Theme & Locale (`NveChannelBus` retained binary replay), Notifications (with unbundled macOS `bundleIdentifier` guard), and Haptics.
- **`crates/nve-ui` Accessible Component Kit & Extended ARIA/`tabIndex`** (`0995977`)
  - Added 12 typed ARIA + `tabIndex` attributes to `crates/nve-dioxus` with zero ABI changes.
  - Built 15 accessible, themeable components (`Stack`, `Row`, `Col`, `Separator`, `Card`, `Button`, `IconButton`, `Badge`, `Input`, `Checkbox`, `Switch`, `Tabs`, `Dialog`/`Modal`, `Popover`/`Tooltip`, `DropdownMenu`, `Toast`, 100k-row Fenwick `VirtualList`) with `UiTheme`, atomic `StyleBuf`, `FocusTrap`, and `RovingFocus`.
- **`crates/nve-router` Type-Safe Stack, Tab, Modal & Deep-Link Router** (`2c63272`)
  - Zero-proc-macro `define_routes!`, cursor-based stack + independent per-tab stacks + isolated modal overlays, synchronous redirect/cycle-safe guards, deep-link URI matching, binary `NVN1` `CHANNEL_ID_NAVIGATION` bridge, and Dioxus `RouterProvider`/`TabRouterProvider`/`Outlet`/`Link`.

---

## Wave 12 — Binary CRDT, WinSock2/Mobile Thin Clients, Live Pair Chrome E2E, Skia Polish & Windows 11 Live Verification

- **Live Windows 11 (`samuel-pc`) Verification — D3D12, RNW 0.88, Full Rust Workspace & WinSock2 `PairClient`** (`58106d1`, `7509985`, `020f6d0`)
  - Verified live on Windows 11 Pro (Build 26200, MSVC 19.51, Cargo 1.100): Skia D3D12 (`--features skia-d3d`, 32/32), full Rust workspace (257/257 + 0 Clippy warnings), `cxx/nve-fabric-bridge` + `ws2_32` (2/2, 0 `/W4` warnings), `hosts/windows-shell` Standalone & RNW 0.88 Fabric (3/3, 0 `/W4` warnings across 63 Ninja targets), and live cross-process `nve.exe pair` → `nve_windows_shell.exe --pair-connect` byte-for-byte identical across WebSocket, TCP, and UDP.
  - Fixed D3D12 `submit_frame` double borrow, MSVC `LNK2005` `mod cdylib_export`, Windows `afd.sys` non-blocking UDP + `WSAECONNRESET` in both Rust and C++, Cargo 1.100 per-unit `build-dir` + `@response_file`, CRLF ABI drift checks, and documented 5 upstream quirks in `docs/UPSTREAM.md`.
- **Skia Rendering Polish, Drag Range Selection, Native Clipboard & Modular Perf Refactor** (`bc92d40`, `8d96bb0`)
  - Forwarded `font-weight`, `text-align`, `box-shadow`, `overflow` via `raw_props` (zero ABI churn); added weighted `cosmic-text`, aligned text, `ScrollView` overlay scrollbars, analytical `erfc` `box-shadow`, mouse-drag & double-click range selection, and zero-dep native OS clipboard.
  - Split `raster.rs`, `image_cache.rs`, and `gpu/mod.rs` into `< 635`-line modules; added rounded-clip interior-span fast path, `O(log n)` LRU, per-filter PNG row unfiltering, and DRY `gpu/select.rs`.
- **Modular `PairClient`, WinSock2 Runtime & Windows/Android/iOS Thin-Client Pairing** (`8ea05dc`)
  - Split `pair_client.cpp` (2,977L) into 5 `< 700`-line units, eliminated 100 Hz idle polling via deadline-driven `poll`/`WSAPoll` + wake fd, added WinSock2 runtime + `ws2_32`, and wired zero-cdylib `--pair-connect` across Windows, Android (JNI + `NvePairController` + `INTERNET`), and iOS (`nve://pair?...` + `NSLocalNetworkUsageDescription`).
- **Zero-Dep Binary CRDT Engine (`NVC1`) & Studio Multi-Peer Convergence** (`b3ecf8f`, `b1585e1`)
  - HLC, `LwwRegister`, `GCounter`/`PnCounter`, `OrSet`, `RgaList`, and `NVC1` binary delta wire format in `crates/nve-interop/src/crdt/`, plus modular `StudioCrdtDoc` (5-peer partition-heal convergence + Criterion benchmarks).
- **Live Browser E2E for Docs, Workbench & Real `nve pair` Binary WebSocket** (`eb5a7ac`)
  - Added ephemeral `--ws-port 0` + `NVE_PAIR_READY` readiness line in `nve pair` and expanded `hosts/web-shell/e2e/run_e2e.mjs` to 6/6 live headless Chrome suites (including live `?pair=ws://...` Counter streaming & `InputEvents=11` round-trips).

---

## Waves 1–11 (Summary of Core Engine & Multi-Host Foundation)

- **Core Binary Engine & Modular C ABI**: Flat binary `CommandBuffer`/`EventRing`, 64-byte `NveRenderNode` Scene ABI, modular `nve_get_interface(1..=4)`, `MmapSnapshot` + `.nvewal` WAL persistence (`NWS1` v2), pluggable Taffy/Yoga layout engines, Tier-1 `< 50 ms` `NVP1` RSX diff patching, Tier-2 `cdylib` hot-swap, and strict full-workspace + `wasm32` Clippy `-D warnings` CI gates (`a205e34`..`88c74d9`).
- **Performance & Size Wins**: Stack-packed `CommandBufferBuilder` (`44.14 µs → 21.17 µs`, -51.9%), inline zero-alloc `GestureArbitrator` (`246.96 ns → 64.51 ns`, -73.8%), `[profile.wasm-release]` (`1,500 KB → 786 KB raw / 285.8 KiB gzipped`), and `cargo nve bench` Cold-Start & Peak RSS profiling (`2d1ceae`).
- **9 Native + Web Hosts & 5-Target Codegen**: Skia Desktop, Web WASM (`Wizer` Frame-0), macOS AppKit, iOS UIKit/Fabric, Windows Direct2D/RNW, Android JNI/Jetpack Compose, Qt 6 Widgets+Quick, GTK4, SwiftUI Kit, Flutter Impeller — all supporting native `NVE_DRAW1` vector canvas, accessibility/IME, and bi-directional 5-target `nve-codegen`.
- **Flagship Studio Workspace**: 100k-row Fenwick virtualized stream, Notion-class Block-Doc Editor (`ActiveView::Docs`), Multi-Viewport Meta Workbench (`ActiveView::Workbench`), `Cmd+K` palette, PNG avatars, `<Island>` sandbox, and live sync telemetry.
