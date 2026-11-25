# Changelog

Complete reverse-chronological log of shipped milestones in `native-view-engine`, from bootstrap (`63b381e`) to current `main`. Open roadmap items live in [docs/BACKLOG.md](./BACKLOG.md).

---

## Docs, Web Foundations & Wave 14 (Zero-Copy, DRY & Production Docs)

- **Pluggable Camera Core (`CameraPreview` `<NativeView>` + `NVCF`) & Android VisionCamera v5 / CameraX Adapters** (`694d222`)
  - Implemented typed zero-JSON `CameraPreview` (`"nve.camera.preview"`, `NVNP`/`NVNC`) and 40-byte `#[repr(C)]` `CameraFrameDescriptor` (`NVCF`, compile-time `size_of`/`offset_of!` locked) in `crates/nve-interop/src/native_view/camera.rs`.
  - Added pure-Kotlin zero-JSON `NveCameraCodec.kt`, pluggable `NveCameraRegistry` (`NveCameraBackend.kt`), zero-reflection `NveVisionCameraAdapter.kt` (`react-native-vision-camera` v5 Nitro bridge with zero RN compile dependency in standalone builds), and standalone `NveCameraXAdapter.kt` in `hosts/compose-kit` (`59/59` `nve-interop` tests including 2,000-case fuzzing, `0` Clippy warnings, `wasm32` clean, `68/68` Debug+Release Gradle tests).
- **Host-Side `NativeViewRegistry` Mount Adapters (Web, C++20 Fabric Bridge & Apple AppKit/UIKit)** (`37eae45`)
  - Implemented zero-JSON `NVNP`/`NVNC` binary decoders and mount adapters in `crates/nve-backend-web/src/native_view.rs` (`"web.<tag>"` safe HTML5/custom-element passthrough, `BlurSurface` `backdrop-filter`, sandboxed `WebView` `<iframe>`, and `VideoPlayer` `<video>` with `Play`/`Pause`/`SeekMs`/`SetVolume` commands across live DOM + SSR + zero-flicker hydration).
  - Added header-only zero-alloc `cxx/nve-fabric-bridge/include/nve_native_view.hpp` (`NvnpDecoder`, `NvncDecoder`, wildcard `NativeViewRegistry`, `bind_to_fabric_host`) and `hosts/apple-common/NveAppleNativeViews.{h,mm}` mounting real `NSVisualEffectView`/`UIVisualEffectView` + `WKWebView` (`22/22` Web tests, `0` Clippy warnings, `wasm32` clean, `3/3` C++ bridge CTest, `5/5` macOS CTest, iOS Simulator ObjC++ `-Werror` clean).
- **Flagship Studio Adoption of `nve-router`, `nve-sdk` & Unified Environment `TraitKey`s** (`9762be7`)
  - Added typed `StudioRoute` (`define_routes!` for `/board`, `/task/:id`, `/channels`, `/docs`, `/doc/:id`, `/workbench`, `/stream`, `/islands`, `/metrics`) with Back/Forward stack sync and `aether://studio/...` deep links (`routes.rs`, `state/navigation.rs`).
  - Wired root `TraitProvider` + `ReactiveTraitStore` with custom `StudioAccentTrait`/`StudioCompactModeTrait` and breakpoint-only `use_trait::<SizeClass>()` + `env.resolve()` (eliminating 1px desktop drag churn), plus dual-mode `StudioHostServices` (`FfiHostServices` over `NVE_IFACE_HOST = 5` with `MemoryHostServices` fallback) for `ClipboardClient` Markdown/URL copy and `SystemEnvClient` theme/locale sync (`34/34` tests, `0` Clippy warnings, `wasm32` clean).
- **Wave 14D — Single-Source Generated `EventRing`, `NVE_DRAW1` & FNV-1a Wire Codecs** (`f970d29`)
  - Added single-source declarative wire codec spec (`crates/nve-codegen/src/wire_codecs/`, schema hash `0x95F1C5176E2F6C99`) emitting compiler-verified `.generated.` wire codecs (`nve_wire_codecs.generated.{hpp,dart,ts}`, `NveWireCodecs.generated.swift`, `dev.nve.android.generated.NveWireCodecs`) wired into `cargo nve codegen abi [--check]` (`15/15` artifacts) and `cargo nve codegen wire [--check]`.
  - Cross-checked every opcode/constant against `nve-abi`/`nve-components` and verified real cross-language execution (`clang++`, `swiftc`, `dart`, `javac`/`java`, `node`) decoding each host's emitted `EventRing` + `NK1` wire bytes through Rust (`62/62` Rust tests, `0` Clippy warnings, `50/50` Swift, `61/61` Flutter).
- **Windows 11 Live Re-Verification of Wave 14A/B/C, Docs Finale, Tier-2/3 SDK & RNW 0.88 (`samuel-pc`)** (`2c2efd0`)
  - Fixed `NVE_WITH_RN_JSI=1` protected `facebook::jsi::Runtime::PointerValue` visibility in split `mock_jsi_arena.hpp`/`mock_jsi_runtime.hpp` and restored `aria-label="Command Palette"` / `role="searchbox"` on `nve_ui::Dialog` & `nve_ui::Input` in Studio (`ui_palette.rs`).
  - Verified natively on Windows 11 (MSVC 19.51, Ninja Release): `411/411` full Rust workspace tests, `0` Cargo/rustc/Clippy warnings, `10/10` `codegen abi --check`, `2/2` `cxx/nve-fabric-bridge`, `3/3` Standalone + `3/3` RNW 0.88 Fabric (`67/67` Ninja targets, `0` `/W4` warnings), and `40/40` Skia D3D12 tests.
- **Skia Desktop Host Services (`NVE_IFACE_HOST = 5`), Multi-Window Glue & Zero-Copy `HotAppInstance`** (`c2f83c0`)
  - Refactored `HotAppInstance::render`, `drain_emitted_events`, and `snapshot_state` to borrow `&[u8]` directly from `NveBufferSlice::as_slice()` (eliminating per-frame `.to_vec()` allocations across `nve-engine`, `nve-cli/pair`, and `hosts/skia-shell`).
  - Bound `Arc<SkiaHostServices>` (`nve-sdk` Tier-1 + `SERVICE_WINDOW = 0x0002` `NVW1`) into `DesktopWindowHost`, wired `WindowEvent::ThemeChanged` / `ScaleFactorChanged` live `NVT2` & `NVL1` pushes, modal parent close-blocking, cascading sheet/popover cleanup, `CloseRequested` veto/allow, and safe Tier-2 `HotAppSlot::reload` callback re-binding (`82/82` tests, `0` Clippy warnings).
- **Weighted Text Measurement & `flex-wrap: wrap` Pipeline Fix (`nve-engine`, `nve-backend-headless`, `nve-backend-skia`, `nve-backend-web`, `nve-docs`)**
  - Added `TextMeasurer::measure_with_weight` + `LayoutEngine::set_raw_layout_prop`, wired `font-weight` and `flex-wrap` into `TaffyLayoutEngine` and Web `RawStyleProps`, and added weight-aware `CosmicTextEngine::measure_text_with_weight` with a `+1px` subpixel overhang guard—fixing bold/semibold clipping and inline prose wrapping on native Skia.
- **Flagship Studio Live Multi-Peer `NVC1` CRDT Transport & `nve-ui` Migration** (`075141d`)
  - Added zero-JSON binary `CrdtPeerTransport` (`InMemoryMeshTransport` with fault injection + `LoopbackTcpCrdtTransport` over `NVT1` OS loopback TCP with automatic `NVCD` anti-entropy catch-up), wired live CRDT sync metrics and partition/heal controls into Studio, and migrated Studio chrome across 9 view modules onto shared `crates/nve-ui` components (`29/29` tests, `0` Clippy warnings, `wasm32` clean).
- **Wave 14C — Direct Zero-String-Parse `TypedStyle` Props (`nve-dioxus`, `nve-ui`) & `writer.rs` Split** (`192b726`)
  - Added `nve_style: TypedStyle` via `AttributeValue::Any` with deterministic inline `style` override merging, cached pre-parsed `UiPaletteRgba` tokens in `ThemeProvider`, migrated all 15 `nve-ui` components off `StyleBuf` string formatting/reparsing, and split `writer.rs` (912L) into 5 `< 415`-line submodules (`45/45` tests, `0` Clippy warnings, `wasm32` clean).
- **Wave 14B — Delete Legacy Fallbacks, Apple DRY Consolidation, Flutter/Kotlin Hot-Path Fixes & C++/JNI Splits** (`1d566bc`)
  - Removed legacy non-direct `set_props` / ring `dispatch_events` fallbacks across SwiftUI, `cxx/nve-fabric-bridge`, macOS, and Windows; eliminated redundant Flutter `MethodChannel` calls; added zero-alloc reusable `NveBinWriter` and unified `NVE_MET1` decoding across Kotlin/Java; extracted `NveAppleComponentViewBase` & `NveAppleValueBridge` into `hosts/apple-common/`; and split `mock_jsi_recorder.hpp` (1,250L), `nve_host_runtime.hpp` (1,170L), and `NveAndroidSurfaceJni.cpp` (1,229L) below 615 lines. Verified across all 8 host CTest/Swift/Flutter/Gradle/`javac` suites.
- **Docs Finale (Phases 2B/3B/4) — `cargo nve docs` CLI, `examples/nve-docs-site` (5 Live Playgrounds + Native App) & GitHub Pages CI** (`1ee43c2`)
  - Shipped `cargo nve docs check|build|dev`, embedded zero-FS `examples/nve-docs-site` with all 5 real interactive playgrounds (`ui-playground`, `virtual-list-100k`, `router-sandbox`, `canvas-draw1`, `crdt-simulator`), native `export_nve_app!`, `wasm32` progressive hydration (`adopted=894, mismatches=0, domIdentityPreserved=true`), zero-npm headless Chrome E2E (`run_docs_e2e.mjs`), and `.github/workflows/docs.yml` (`43/43` tests, `0` Clippy warnings).
- **SwiftUI Kit & Flutter Plugin Thin-Client `NVT1`/`NVD1` Pairing (10/10 Host Parity)** (`ffe1d5f`)
  - Implemented pure-Swift (`URLSessionWebSocketTask`, `NvePairProtocol.swift`, `NvePairClient.swift`, `NvePairedSurfaceView.swift`) and pure-Dart (`dart:io` `WebSocket`, `nve_pair_protocol.dart`, `nve_pair_client.dart`, `nve_paired_surface.dart`) zero-JSON binary `NVT1` + `NVD1` (`Keyframe`/`XorRleDelta`/`CommandOpDelta`) + `InputEvents = 11` thin-client pairing clients and interactive UI views, completing live pairing parity across all 10 NVE platform hosts. Verified: 50/50 `swift test`, 60/60 `flutter test`, 0 `flutter analyze` issues, all source files `< 550` lines.
- **Tier-2/3 `crates/nve-sdk` Native Storage & Hardware Modules** (`a5ae15c`)
  - Implemented 8 zero-JSON binary `NVH1` services (`0x0020..=0x0027`, pairwise-distinct from Tier-1 `0x0010..=0x0016`) over `NVE_IFACE_HOST = 5` across 15 modular files (`< 600` lines each, zero new external deps): crash-safe `NVK1` binary KV store (`StorageClient` with FNV-1a64 checksums, atomic batches, torn-tail recovery, atomic compaction), custom font loader (`FontClient` validating TTF/OTF/WOFF/WOFF2/TTC), `AudioClient`, `CameraClient`, `LocationClient`, `SensorClient` (zero-copy 20-byte `#[repr(C)]` `SensorSample`), `ShareClient`, and `WebAuthClient` (`ASWebAuthenticationSession`/Custom Tabs/OAuth). Wired into `NativeHostServices` and `MemoryHostServices`. Verified: 14/14 tests, 0 native & `wasm32` Clippy warnings.
- **Wave 14A — Complete ABI Field-Offset Locks, Zero-JSON Codegen, Modular Splits & `.gitattributes`** (`99c1d99`)
  - Locked every `#[repr(C)]` vtable, struct, union, and enum across `nve-abi` (`layout_asserts.rs`), generated C11/C++20 `NVE_STATIC_ASSERT(offsetof(...))` in `nve_abi.generated.h` / `nve_scene_abi.generated.h`, and `static_assert`s on `NVE_SNAP` (`nve_snapshot.hpp`) and `PairClient` (`nve_pair_client.hpp`).
  - Removed all JSON/`serde_json`/`JSONObject`/`[String: Any]`/`buildMap<String, Any?>` and dead `export_*_json` wrappers from `crates/nve-codegen` (migrated RN, SwiftUI, Compose, Flutter, and Web emitters to zero-JSON `NveBinValue`/`NveDirectEncodable`/direct FFI), split all 13 oversized `nve-abi`/`nve-codegen` files (`<= 586` lines max), fixed `nve-bench` Cargo 1.100 dev-deps, and added root `.gitattributes` (`eol=lf`). Verified: 77/77 Rust tests, 0 Clippy warnings, 10/10 `codegen abi --check`, 2/2 CTest, 42/42 Swift tests.
- **Unified Extensible Environment / `TraitKey` System & First-Class `<NativeView>` Core** (`ab624d7`)
  - Added typed `TraitKey`, `Copy` 17-trait `BuiltinTraits` fast path (`PlatformOs`, `HostToolkit`, `SizeClass`, `VerticalSizeClass`, `ContainerSize`, `Orientation`, `SafeAreaInsets`, `ScaleFactor`, `PointerKind`, `CanHover`, `HasHardwareKeyboard`, `ColorScheme`, `ReducedMotion`, `HighContrast`, `TextScale`, `UiDensity`, zero-alloc `Locale` with `is_rtl()`), persistent CoW custom `TraitKey` overlay, zero-alloc `env.resolve()`, and breakpoint-only `TraitStore` revision tracking in `crates/nve-interop`.
  - Added zero-JSON `NVNP`/`NVNC` binary `<NativeView<P: NativeViewSpec>>`, imperative `NativeViewHandle<P>` (`Command::DispatchCommand`), wildcard-aware `NativeViewRegistry` (`"web.*"`), starter specs (`BlurSurface`, `WebView`, `VideoPlayer`, `WebElement`), and Dioxus `TraitProvider`/`TraitScope`/`use_trait`/`PlatformOnly`/`TraitSwitch`/`NativeView` (with graceful unregistered `fallback` slot) in `crates/nve-dioxus`, plus fixed `TRANSPORT_MAGIC` docs. 70/70 tests, 0 Clippy warnings, `wasm32` clean.
- **Reusable `crates/nve-docs` Fumadocs × Linear Docs Framework** (`8d58496`)
  - Built 21 modular files (`< 455` lines each, zero new external deps) exclusively on `nve-ui`, `nve-router`, `nve-sdk`, and `nve-mdx`: dark-first/light `DocsTheme` + exhaustive `SyntaxPalette`, `DocsSiteConfig`, `DocsRoute`, `PlaygroundRegistry` + strict 13-component registry, responsive 3-column shell (sticky navbar, collapsible `260px` sidebar / `< 960pt` mobile drawer, `220px` sticky TOC, `⌘K` `NVS1` search modal with `FocusTrap`, breadcrumbs, GitHub edit link, `PrevNext` cards), and complete `nve-mdx` AST renderer with copy-to-clipboard code blocks and `<LiveDemo>` islands. 9/9 tests, 0 Clippy warnings, `wasm32` clean.
- **Complete 6-Section NVE Documentation Content Suite (`docs/content/`)** (`2f11098`)
  - Authored 30 `.mdx` pages + 7 `_meta.toml` navigation manifests (16,317 words across Getting Started, Architecture, Guides, all 10 Platform Hosts, Interop/Codegen/Pairing, and Reference) plus all 5 `<LiveDemo>` hooks, validated with 0 diagnostics and 177 `NVS1` search sections via `nve-mdx`.
- **Windows 11 Live Verification of Wave 13 SDK, `NveWinWindowManager`, RNW 0.88 & `.generated.h`** (`4e046ca`)
  - Verified natively on Windows 11 (`samuel-pc`, MSVC 19.51): `codegen abi --check` (10/10 files, 0 CRLF/LF drift), `nve-sdk` (`windows.rs`: Win32 Clipboard, CredMan, `ShellExecuteW`, Registry theme, `GetUserDefaultLocaleName`) + `nve-router` + `nve-ui` + `nve-mdx` + `nve-interop` (87/87 tests, 0 Clippy warnings), `cxx/nve-fabric-bridge` (2/2 CTest, 0 `/W4` warnings), `hosts/windows-shell` Standalone & RNW 0.88 Fabric (`NveWinWindowManager`, 3/3 CTest each, 0 `/W4` warnings across 67 Ninja targets), and Skia D3D12 (40/40 tests).
- **Option 5 Hybrid Web CSS, Keyed `SetChildren`, Semantic HTML5 SSR & Zero-Flicker Progressive Hydration** (`36a6e5a`)
  - Replaced inline `cssText` thrashing with deferred end-of-frame style flushing, static `NVE_BASE_RESET_CSS`, deterministic FNV-1a64 `.s<base36>` structural stylesheet cache (`MAX_DYNAMIC_RULES = 4096`, batched `adoptedStyleSheets`), volatile/continuous inline escape hatch, `.nve-tl` leaf-text inheritance, and structured `RawStyleProps` (`font-weight`, `text-align`, `box-shadow`, `overflow`, `object-fit`). Cut Studio Frame 0 HTML+CSS payload by **-77.2%** (`112,281 B -> 25,637 B`).
  - Added O(N) head/tail + LIS keyed `SetChildren` DOM reconciliation preserving `<input>` focus/IME and scroll state, Semantic HTML5 `render_ssr`/`render_ssr_document`, single-root `RootEventDelegator`, in-place `hydrate_into` with mismatch recovery, and `ProgressiveIslandHost` (`Load` / `Visible` / `Interaction` with replay / `Idle`).
  - Verified: 37/37 unit tests, 0 native/`wasm32` Clippy warnings, 7/7 live headless Chrome E2E suites (`adopted=270, mismatches=0, elBefore === elAfter`), and 18/18 `nve-studio-workspace` tests.
- **Enforce `.generated.<ext>` & Java `generated/` Package Convention** (`b305607`)
  - Renamed all 8 non-Java checked-in ABI artifacts to `*.generated.{h,dart,ts}`, moved `NveCoreAbi.java` and `NveSceneBuffer.java` into `package dev.nve.android.generated;`, updated all `nve-codegen` emitters, `cargo nve codegen abi --check` (10/10 artifacts), host builds, and docs with zero legacy shims.
- **`crates/nve-mdx` Pure-Rust `.md`/`.mdx` Compiler & `NVS1` Search Index** (`47e793b`)
  - Zero-dependency CommonMark/GFM parser, YAML/TOML frontmatter, Nextra-style `_meta.toml` navigation + `PrevNext`, fenced directives (`:::callout`, `:::tabs`, `:::steps`, `:::cards`, `:::demo`), declarative `<Component prop="val" num={42} />` tags + ` ```rsx live ` blocks, compile-time schema/link/anchor validator, 13-language pure-Rust syntax highlighter, deterministic Rust/Dioxus codegen, and binary `NVS1` full-text search index (`< 1 ms` queries, FNV-1a64 checksum). 12/12 tests, 0 Clippy warnings, `wasm32` clean.
- **`nve-router::web` Browser History API Adapter** (`0827eab`, `e691874`)
  - Cross-platform `HistoryBackend` (`MemoryHistoryBackend` + `wasm32` `BrowserHistoryBackend`), `WebHistoryAdapter` with GitHub Pages `base_path` normalization, deduplicated `pushState`/`replaceState`, loop-free `popstate`, browser-accurate link interception, and `#anchor` scrolling. 17/17 tests, 0 native/`wasm32` Clippy warnings.

---

## Wave 13 — Multi-Window, Native SDK, UI Component Kit, Router & Direct FFI DRY

- **Swift/Dart Direct FFI Codegen DRY, `NveDirectEncodable` & C11/Rust `offset_of!` ABI Checks** (`66832d2`)
  - Split `NveDirectFfi.swift` (551L) into `NveDirectArena.swift` (188L), `NveDirectValueEncoding.swift` (268L), and `NveDirectWriters.swift` (246L) and removed its line-count test exemption.
  - Added zero-boxing `NveDirectEncodable` + `StaticString` keys (`NveDirectPropsWriter.withEncodedProps`) across SwiftUI and generated Swift (deleting `pinnedBuffers`), unified Dart codegen onto shared `nve_bin_codec.dart` + `nve_hybrid_runtime.dart` + `direct_ffi.dart`, and added compile-time `offset_of!` assertions on `ffi_types.rs` cross-checked against C11 `_Static_assert(offsetof(...))`.
  - Verified: 52/52 Rust, 42/42 Swift, 52/52 Flutter (`0` analyze issues).
- **Multi-Window Lifecycle Primitives Across 7 Hosts** (`21d86ac`, `b1a7922`)
  - Added `SurfacePresentation`, `WindowManager` (modal parent focus blocking, cascading child sheet/popover close, cancelable `CloseRequested` veto), and zero-JSON binary `NVW1` command/event codec in `crates/nve-interop`.
  - Implemented multi-window managers across Skia `winit`, macOS AppKit (`NveMacWindowManager`), iOS/iPadOS `UIWindowScene` allocator, Windows Win32 (`NveWinWindowManager`), GTK4 (`NveGtkWindowManager`), Qt 6 (`NveQtWindowManager`), and Web WASM (`ACTIVE_SHELLS` + `BroadcastChannel`).
- **`NVE_IFACE_HOST = 5` Binary Capability ABI & Tier-1 `crates/nve-sdk`** (`0aeee16`, `9174788`, `36efdd3`)
  - Added single zero-JSON binary host-capability interface (`NVE_IFACE_HOST = 5`, `NVH1`, `NVT2`, `NVL1`) regenerated across all 5 ABI targets (`0x54A4A32B6F258107`).
  - Shipped zero-dependency Tier-1 native capabilities: Clipboard, Filesystem/File Picker, Secure Store (macOS Keychain / Win32 CredMan / Unix fallback), Linking, live System Theme & Locale (`NveChannelBus` retained binary replay), Notifications (with unbundled macOS `bundleIdentifier` guard), and Haptics. Fixed Retina 2x SwiftUI `NSHostingView` bitmap backing test (`9174788`).
- **`crates/nve-ui` Accessible Component Kit & Extended ARIA/`tabIndex`** (`0995977`)
  - Added 12 typed ARIA + `tabIndex` attributes to `crates/nve-dioxus` with zero ABI changes.
  - Built 15 accessible, themeable components (`Stack`, `Row`, `Col`, `Separator`, `Card`, `Button`, `IconButton`, `Badge`, `Input`, `Checkbox`, `Switch`, `Tabs`, `Dialog`/`Modal`, `Popover`/`Tooltip`, `DropdownMenu`, `Toast`, 100k-row Fenwick `VirtualList`) with `UiTheme`, `StyleBuf`, `FocusTrap`, and `RovingFocus`.
- **`crates/nve-router` Type-Safe Stack, Tab, Modal & Deep-Link Router** (`2c63272`)
  - Zero-proc-macro `define_routes!`, cursor-based stack + independent per-tab stacks + isolated modal overlays, synchronous redirect/cycle-safe guards, deep-link URI matching, binary `NVN1` `CHANNEL_ID_NAVIGATION` bridge, and Dioxus `RouterProvider`/`TabRouterProvider`/`Outlet`/`Link`.

---

## Wave 12 — Binary CRDT, WinSock2/Mobile Thin Clients, Live Pair Chrome E2E, Skia Polish & Windows 11 Live Verification

- **Live Windows 11 (`samuel-pc`) Verification — D3D12, RNW 0.88, Full Rust Workspace & WinSock2 `PairClient`** (`58106d1`, `7509985`, `020f6d0`)
  - Verified live on Windows 11 Pro (Build 26200, MSVC 19.51, Cargo 1.100): Skia D3D12 (`--features skia-d3d`, 32/32), full Rust workspace (257/257 + 0 Clippy warnings), `cxx/nve-fabric-bridge` + `ws2_32` (2/2, 0 `/W4` warnings), `hosts/windows-shell` Standalone & RNW 0.88 Fabric (3/3, 0 `/W4` warnings across 63 Ninja targets), and live cross-process `nve.exe pair` → `nve_windows_shell.exe --pair-connect` byte-for-byte identical across WebSocket, TCP, and UDP.
  - Fixed D3D12 `submit_frame` double borrow, MSVC `LNK2005` `mod cdylib_export`, Windows `afd.sys` non-blocking UDP + `WSAECONNRESET` in both Rust and C++, Cargo 1.100 per-unit `build-dir` + `@response_file`, CRLF ABI drift checks, and documented 5 upstream quirks in [docs/UPSTREAM.md](./UPSTREAM.md).
- **Skia Rendering Polish, Drag Range Selection, Native Clipboard & Modular Perf Refactor** (`bc92d40`, `8d96bb0`)
  - Forwarded `font-weight`, `text-align`, `box-shadow`, `overflow` via `raw_props` (zero ABI churn); added weighted `cosmic-text`, aligned text, `ScrollView` overlay scrollbars, analytical `erfc` `box-shadow`, mouse-drag & double-click range selection, and zero-dep native OS clipboard.
  - Split `raster.rs`, `image_cache.rs`, and `gpu/mod.rs` into `< 635`-line modules; added rounded-clip interior-span fast path, `O(log n)` LRU, per-filter PNG row unfiltering, and DRY `gpu/select.rs`.
- **Modular `PairClient`, WinSock2 Runtime & Windows/Android/iOS Thin-Client Pairing** (`8ea05dc`)
  - Split `pair_client.cpp` (2,977L) into 5 `< 700`-line units, eliminated 100 Hz idle polling via deadline-driven `poll`/`WSAPoll` + wake fd, added WinSock2 runtime + `ws2_32`, and wired zero-cdylib `--pair-connect` across Windows, Android (JNI + `NvePairController` + `INTERNET`), and iOS (`nve://pair?...` + `NSLocalNetworkUsageDescription`).
- **Zero-Dep Binary CRDT Engine (`NVC1`) & Studio Multi-Peer Convergence** (`b3ecf8f`, `b1585e1`)
  - Implemented HLC, `LwwRegister`, `GCounter`/`PnCounter`, `OrSet`, `RgaList`, and `NVC1` binary delta wire format in `crates/nve-interop/src/crdt/`, plus modular `StudioCrdtDoc` (5-peer partition-heal convergence + Criterion benchmarks).
- **Live Browser E2E for Docs, Workbench & Real `nve pair` Binary WebSocket** (`eb5a7ac`)
  - Added ephemeral `--ws-port 0` + `NVE_PAIR_READY` readiness line in `nve pair` and expanded `hosts/web-shell/e2e/run_e2e.mjs` to 6/6 live headless Chrome suites (including live `?pair=ws://...` Counter streaming & `InputEvents=11` round-trips).

---

## Wave 11 — Workspace Clippy Gate, Security Hardening, C++20 `PairClient`, Studio Docs/Workbench & Perf Breakthroughs

- **Strict Full-Workspace & `wasm32` Clippy `-D warnings` CI Gate** (`36c52c3`, `04c7f91`, `88c74d9`)
  - Eliminated every Clippy warning across all workspace crates and `wasm32-unknown-unknown` targets and enforced `-D warnings` in CI.
- **Codegen Security Hardening, PNG/Vulkan Safety & Tighter CI Gates** (`6a2731b`, `a205e34`)
  - Neutralized string literal and comment injection across codegen emitters, hardened PNG/Vulkan decoding paths, and tightened CI verification gates.
- **Zero-Dep C++20 `nve::PairClient` & Desktop Thin-Client Pairing** (`67fe847`)
  - Implemented zero-dependency C++20 `nve::PairClient` (`NVT1`/`NVD1` over UDP/TCP/WS), wired paired thin-client mode into macOS AppKit, Qt 6, and GTK4 shells, and hardened `EventRingWriter`.
- **Studio Crash-Safe Persistence (`NWS1` v2 + `.nvewal`), Block-Doc Editor & Meta Workbench** (`77e8ba5`)
  - Added local-first `MmapSnapshot` + append-only `.nvewal` WAL persistence (`NWS1` v2 + FNV-1a64 checksums), Notion-class `ActiveView::Docs` block editor with Markdown export, and `ActiveView::Workbench` multi-viewport meta workbench.
- **Codec, Gesture, WASM Size & Cold-Start/RSS Perf Wave** (`2d1ceae`, `9406d1e`, `9545a54`, `cb25fe1`)
  - Stack-packed `CommandBufferBuilder` (**44.14 µs → 21.17 µs**, -51.9%), zero-alloc inline `GestureArbitrator` (**246.96 ns → 64.51 ns**, -73.8%), added `[profile.wasm-release]` (**1,500 KB → 786 KB raw / 285.8 KiB gzipped**), added `cargo nve bench` Cold-Start & Peak RSS profiling, validated `text_slab` `NVE_DRAW1` in Android JNI, cleared Flutter analyzer diagnostics, and refreshed quiet-machine benchmark baselines.

---

## Wave 10 — Live `NVT1`/`NVD1` Pairing, Universal `NVE_DRAW1` Canvas & Zero-JSON Wire Codecs

- **Live `NVT1`/`NVD1` Thin-Client Pairing & `SyncTelemetry` Push** (`f6b35bd`, `1fc01bb`)
  - Added bidirectional `InputEvents = 11` backchannel, `nve pair` / `nve dev --pair` daemon, and live thin-client pairing across `hosts/skia-shell` and `hosts/web-shell` with real-time `SyncTelemetry` pushing.
- **Universal Native `NVE_DRAW1` Vector Canvas, IME & Accessibility Across All 9 Hosts** (`de61d5d`, `93ea769`, `2aa1431`, `1232b8c`, `590a6e2`, `6491531`, `79ca053`, `0987c7c`, `b67ecf8`)
  - Implemented zero-alloc `NVE_DRAW1` vector canvas rendering across Skia Desktop, Web HTML5 Canvas, Apple CoreGraphics (`NveAppleCanvasView`), Qt 6, GTK4, Windows Direct2D, Android/Compose (`NveCanvasView`), Flutter Impeller, and SwiftUI.
  - Added interactive `TextInput` focus/IME/caret/placeholder rendering, zero-JSON `NK1` binary keydown codecs, Web ARIA/IME guards, batched Android TalkBack metadata, and `Image` caches across all hosts.
- **Zero-JSON Binary Wire Codecs & Workspace Modularization** (`f17586d`, `c354281`, `4b5f835`, `7c77d54`)
  - Eliminated remaining JSON on internal wire codecs (`crates/nve-abi`, `nve-engine`, `nve-dioxus`, `studio-workspace`, `nve-backend-web`) and split oversized Rust/C++ host, CLI, and component modules into focused submodules.

---

## Wave 9 — `O(changed)` `NveSceneView` Migration, Tier-1 RSX Hot-Patch & Zero-Copy Rust/C++ Pipeline

- **Direct `NveSceneView` Host Migration Across All Platforms** (`cf1f3a7`, `bbb4687`, `1469a31`, `a3b6017`, `eec1993`, `fa47479`, `f8d24fd`)
  - Added direct `NveSceneView` publisher, scratch arena, and slot table in `cxx/nve-fabric-bridge`, and migrated Apple, Android, Qt 6 (retained `QSGNode` tree), GTK4, Windows, Flutter, and Web shells to `O(changed)` `NveSceneView` reconciliation with `Image` rendering and `ScrollView` events.
- **Tier-1 `< 10 ms` RSX Hot-Patcher & Full Event Dispatch** (`350258b`, `77b8e0e`)
  - Implemented targeted `NVP1` binary RSX template hot-patching across `nve-cli`, `nve-dioxus`, and `nve-backend-web` with instant live style/text patching without recompiling.
- **Zero-Copy Streaming ABI, FFI Panic Boundaries & Transactional Reloads** (`d3b43d4`, `27776f4`, `73cbdda`, `5cd04b8`)
  - Built zero-copy streaming `CommandBuffer` decoder, slot-indexed headless/Taffy layout engine, zero-clone Skia display list, strict FFI `catch_unwind` panic boundaries, guarded raw slices, and transactional `HotAppInstance::reload(&mut self)`.
- **Native OS Accessibility & IME Across Apple, Qt 6 & GTK4 + Full CI Parity** (`db0b9c8`, `dcbef0e`, `f254529`)
  - Wired native VoiceOver/AT-SPI/Qt accessibility and `TextInput`/IME across Apple, Qt 6, and GTK4; expanded `studio-workspace` to exercise every NVE primitive; brought `scripts/check-all.sh` and GitHub Actions CI to 100% host & suite parity.

---

## Waves 6–8 — Unified Nitro-Style Hybrid Codegen, Live `NVD1` Transport, Skia GPU Swapchains & Snapshot WAL

- **Nitro-Inspired 6-Target Hybrid FFI Codegen & Unified Core + Scene ABI** (`d131b04`, `2e0fe6a`, `8213168`)
  - Created Rust-first 6-target Hybrid FFI codegen (`crates/nve-codegen`) and unified both Core ABI and zero-decode 64-byte `NveRenderNode` Scene ABI onto the single-source schema pipeline (`C`, `Rust`, `Swift`, `Dart`, `Kotlin`/`Java`, `TypeScript`).
- **Live UDP, TCP & RFC 6455 WebSocket `NVD1` Delta Transport** (`3078642`)
  - Implemented frame delta diffing/patching (`NVD1`) and live UDP (fragmented + RTO resync), TCP, and zero-dep RFC 6455 WebSocket transports in `crates/nve-interop`.
- **Skia Hardware GPU Swapchains (Metal, Vulkan, D3D12) & Standalone `hosts/skia-shell`** (`23f690a`, `b3d2c06`, `937d137`)
  - Added `cosmic-text` native font shaping/rasterization, wired Metal, Vulkan, and Direct3D 12 hardware GPU swapchains in `crates/nve-backend-skia`, and extracted the runnable `winit` desktop host into `hosts/skia-shell` (`nve-skia-shell`).
- **Flagship Studio Depth & Standalone Flexbox Parity** (`360f735`, `07aba59`)
  - Added 100k Fenwick-tree virtualized stream, live Taffy/Yoga layout switcher, binary sync delta telemetry, panic-isolated plugin host, and mobile portrait layout in `examples/studio-workspace`.
  - Achieved full standalone C++20 Flexbox parity for post-flex text wrapping and `Baseline` alignment in `cxx/nve-fabric-bridge`.
- **Instant-Boot `MmapSnapshot` Phase 2, WAL, `Wizer` Pre-Init & `nve dev` DX** (`22d6eaf`, `bb5a054`, `1d8dcd1`, `32f24ed`, `bcb71e7`)
  - Wired zero-copy Frame-0 `MmapSnapshot` across C++20 runtime and desktop shells, added append-only `.nvewal` WAL + Dioxus signal archive + GTK4/iOS replay + WASM `Wizer` Frame-0 pre-init, and upgraded `nve dev` with LAN `--host 0.0.0.0` binding, default `studio-workspace` launch, and `--web` live auto-rebuild.

---

## Waves 3–5 — Multi-Host Expansion (Qt 6, GTK4, Windows, iOS, Android, Flutter, Web) & Zero-Copy Direct FFI

- **Qt 6 Widgets + GPU Qt Quick, GTK4, Windows Direct2D/RNW & Android Compose Kit** (`7037a97`, `d5702c5`, `d6ac926`, `0be9bbf`, `22f4ddd`, `098f95f`, `51c5ae4`)
  - Built native hosts on top of the shared C++20 `nve_host_runtime`: Qt 6 `QWidget` (`hosts/qt-shell`) + GPU-composited `NveQtQuickItem`, GTK4 (`hosts/gtk-shell`), Windows Win32/Direct2D + `react-native-windows` Fabric (`hosts/windows-shell`), and Android Library Gradle module (`hosts/compose-kit`).
  - Added continuous Rust Criterion & C++20 benchmark regression tracking in CI (`51c5ae4`).
- **Zero-Xcode iOS, Zero-Gradle Android, Flutter `RenderBox` & Headless Chrome Web E2E** (`f89e4ae`, `ac341ab`, `fe11f2d`, `bc37d92`, `2c84c74`, `9b2dcae`, `c48f7a6`, `1870fd6`, `714e84e`, `e111231`)
  - Added standalone zero-Xcode iOS Simulator launcher (with iOS 27 `UIScene` lifecycle and Xcode 27 `DeviceHub.app`), standalone zero-Gradle Android APK builder/emulator launcher, Flutter Path 1 direct `RenderBox` + `Canvas` host (`NveCanvasView`), default Studio Web WASM build, headless Chrome E2E test harness, initial live Windows 11 MSVC 19.51 verification, and `docs/UPSTREAM.md`.
- **Zero-Copy Direct FFI (`NveBinValue`), Instant-Boot Frame-0 Snapshot & Flagship Studio** (`7430122`, `cc7727e`, `a4a51ae`, `a94dd2c`, `b7dc482`)
  - Replaced JSON event/props crossings with zero-copy direct FFI and binary `NveBinValue` (`7430122`), introduced Instant-Boot Frame-0 `CommandBuffer` + `mmap` state snapshots (`cc7727e`), created the flagship `Aether Workspace + NVE Studio` example (`a4a51ae`), and added unified `scripts/check-all.sh` + multi-OS GitHub Actions CI (`a94dd2c`).

---

## Waves 1–2 — Core Binary Engine, Modular C ABI, Pluggable Layout & Testing Foundation

- **Composable Layout Engines, Shared C++20 Runtime & Apple DRY Extraction** (`e38f333`, `264e359`, `0b916c3`, `533fd24`, `8d3afd9`)
  - Introduced composable `LayoutEngine`/`TextMeasurer` traits, generic layout backends, and `nve-layout-yoga` (`e38f333`); built the shared C++20 native host runtime with Windows/MSVC portability (`264e359`); implemented intrinsic auto-sizing, `justify-content`, and `align-items` in standalone Flexbox (`0b916c3`); extracted shared iOS/macOS host logic into `hosts/apple-common/` (`533fd24`); fixed text style inheritance and native text measurement (`8d3afd9`).
- **Deep Benchmarks, Property/Fuzz Tests & Host Kit Suites** (`c95d108`, `eda7340`, `135c8ae`, `23676ab`, `d6cf3c2`)
  - Added C++20 microbenchmarks, fuzz tests, and decoder hardening (`c95d108`); Rust Criterion benchmarks and deep unit/property tests (`eda7340`); unit & integration suites for SwiftUI, Flutter, and Android JNI host kits (`135c8ae`); and initial `AGENTS.md`, README, and roadmap docs.
- **Workspace Bootstrap & Modular `nve_get_interface` C ABI** (`63b381e`, `a90e7ec`, `5bae7f6`)
  - Bootstrapped the multi-backend `native-view-engine` Cargo workspace (`nve-abi`, `nve-engine`, `nve-dioxus`, `nve-components`, `nve-interop`, `nve-codegen`, `nve-cli`, headless/Skia/Web backends, C++20 Fabric/Yoga/JSI bridge, Apple/Android/Flutter/SwiftUI host shells) and refactored the C ABI to the modular `nve_get_interface(1..=4)` architecture backed by the safe Rust `NveApp` trait and `export_nve_app!` macro.
