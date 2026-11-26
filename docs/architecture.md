# Architecture & Design Decisions

## 1. Core Thesis

`native-view-engine` separates **UI authoring**, **mutation transport**, and **platform rendering** along one narrow, zero-overhead boundary: a versioned, flat binary `CommandBuffer` and `EventRing` (`nve-abi` / `include/nve_abi.generated.h`).

This eliminates React Native's JS reconciler, JSI per-prop crossings, `RawProps` string parsing, and `folly::dynamic` serialization while preserving **real platform-native views** (iOS `UIView`, Android `View`) and **native RN Fabric ecosystem compatibility** — and lets the exact same binary IR drive **Skia** (macOS, Windows, Linux, iOS, Android), **Web DOM/WASM**, and **Headless** test hosts.

## 2. Layer Stack

| Layer | Crate / Dir | Responsibility |
| :--- | :--- | :--- |
| **Universal IR & C ABI** | `packages/core/abi`, `cxx/nve-fabric-bridge/include/nve_abi.generated.h` | Flat binary `CommandBuffer` encoder/decoder, `EventRing`, and modular `nve_get_interface(uint32_t)` C ABI (`NveSurfaceApi`, `NveLayoutApi`, `NveComponentApi`, `NveDevApi`). |
| **Typed Primitives** | `packages/core/components` | Cross-platform primitives (`View`, `Text`, `ScrollView`, `Image`, `TextInput`, `Pressable`, `SkiaCanvas`, `Custom`), Yoga/Taffy-aligned `LayoutStyle`, and `ViewStyle`. |
| **Core Engine** | `packages/core/engine` | Safe `NveApp` trait + `export_nve_app!` macro, `RenderBackend` trait, `SurfaceRunner`, 6-stage `FrameProfiler` + Perfetto trace export, 2-tier hot-reload client & modular `cdylib` loader. |
| **Interop Orchestrator** | `packages/core/interop` | Multi-framework `SurfaceOrchestrator`, intrinsic `LayoutNegotiator`, nested-scroll `GestureArbitrator`, `NveChannelBus`, CRDT (`NveCrdtDelta`), traits, and window state. |
| **Host Services Core** | `packages/core/host-services` | Minimal capability dispatch foundation (`HostServiceProvider`, `HostServiceHandler`, typed in-process `HostServiceRegistry`, `FfiHostServices` over `NVE_IFACE_HOST = 5`, `HostCallbackBridge`, `MemoryHostServices`). |
| **Capability Modules** | `packages/modules/*` | Standalone opt-in capability crates (`clipboard`, `filesystem`, `secure-store`, `linking`, `system-env`, `haptics`, `storage`, `tv`, `native-views`, `nitro-bridge`, plus VisionCamera adapter in `camera`). |
| **Network Wire & Transport** | `packages/core/wire-core`, `packages/core/wire` | Sans-I/O `NveWireFrame` v2 (`XXH64`, `NveNodePatch`, `wasm32`-clean) and native QUIC / HTTP/3 WebTransport + WSS Happy-Eyeballs transport with 13-day ECDSA cert pinning and C ABI. |
| **Bi-Directional Codegen** | `packages/core/codegen` | Single-source ABI/Scene/Wire codegen (`15/15` Core + Scene + Wire artifacts), `import-rn`, and multi-target export (RN `ComponentView`, Flutter `dart:ffi`, SwiftUI, Jetpack Compose, Web). |
| **Frontend Adapter** | `packages/core/dioxus` | `dioxus-core` `WriteMutations` adapter emitting batched frames, typed direct event dispatch (`NveEventBatch`) back into `VirtualDom`, root-props/context injection, and Tier-1 RSX template hot-reload hook. |
| **Native Host Runtime (Transitioning)** | `cxx/nve-fabric-bridge` → `packages/core/runtime` + `packages/core/host-cxx` + `packages/modules/react-native-fabric` | Currently hosts C++20 Yoga/fallback layout + `NveSceneView` publishing + dylib hot-swap for C++ hosts; being replaced in Track 1B by host-owned Rust `packages/core/runtime` (Taffy default, opt-in Yoga) exposing extended `NveSceneView` directly across C ABI, shrinking C++ to a thin header-only helper + optional RN Fabric `ComponentView` adapter. |
| **Skia Backend** | `packages/backends/skia` | Cross-platform GPU renderer (macOS, Windows, Linux, iOS, Android) consuming the same `CommandBuffer`, computing Flexbox layout, producing a retained display list, and painting via Skia (`skia-safe` feature). |
| **Web Backend** | `packages/backends/web` | `wasm32` DOM backend applying the same `CommandBuffer` directly to browser DOM nodes + CSS Flexbox. |
| **Headless Backend** | `packages/backends/headless` | Pure-Rust `taffy` Flexbox layout + `MutationLog` + ASCII/JSON tree inspector for <50ms tests, complemented by headless C++ Fabric tests for bit-exact Yoga/Differentiator parity. |
| **Mobile Shells** | `hosts/ios-shell`, `hosts/android-shell` | Prebuilt native Fabric host apps that load/hot-swap compiled Rust app modules (`app_ui.dylib` / `libapp_ui.so`) without Xcode/Gradle rebuilds. |
| **Dev Tooling** | `packages/tools/cli` | `nve doctor`, `nve dev`, `nve inspect`, `nve bench`, and future `nve codegen` / `nve prebuild-fabric`. |

## 3. Architectural Decision Records (ADRs)

### ADR-001: Single Batched Binary `CommandBuffer` Over Per-Call FFI
- **Decision:** Every frame serializes all mutations into one contiguous byte buffer (`u32` header + TLV opcodes) handed to the backend in a single call (`submit_frame`).
- **Why:** Eliminates chatty FFI/JNI crossings, removes `folly::dynamic` and string-key prop lookup, makes every backend (Fabric C++, Skia, Web WASM, Headless) implement one identical decoder, and enables deterministic replay/time-travel debugging.

### ADR-002: Framework-Agnostic Engine With Dioxus First
- **Decision:** `nve-engine` has zero dependency on `dioxus-core`. `nve-dioxus` plugs into `nve-engine` by translating `WriteMutations` into `CommandBuffer` ops.
- **Why:** We get production-grade `rsx!`, signals, and template hot reload on day 1, while keeping the core compile graph tiny (<300ms) and leaving the door open for a custom zero-VDOM reactive frontend or non-Rust frontends later.

### ADR-003: Model A (Sized Surface Island) Across RN Fabric, SwiftUI, Flutter & Compose
- **Decision:** Retire deep `facebook::react::ShadowNode` tree injection (`commit_rn_shadow_tree` no-op stub) and standardize every host framework embedding (RN Fabric `RCTViewComponentView` / Android `ViewGroup` / RNW Composition `ComponentView`, SwiftUI, Flutter, Jetpack Compose) on **Model A (Sized Surface Island)**:
  - Outer layout coordination uses `NVE_IFACE_LAYOUT = 2` (`measure_surface(constraints) -> NveIntrinsicSize` with `first_baseline`/`last_baseline`, `DRY_RUN`/`IDEAL` flags, and `on_intrinsic_size_invalidated` host callback) so NVE participates cleanly in the host's native layout tree and 120Hz VSync cycle.
  - Inner layout & commits are owned by NVE (`NveSceneView`), with foreign host child views mounted inside `<NativeView>` slot holes using back-canvas → host-child (native clip + gesture arbitration via `hit_test`) → front-overlay-canvas layering.
- **Why:** Eliminates fragile coupling to private RN C++ internals (Folly/glog/boost/per-minor `ShadowNode` ABI drift and RNW C++/WinRT divergence), preserves full NVE feature parity (CSS Grid, `sticky`, `:hover`/transitions, gradients, SVG, BiDi text), and shares one architecture identically across RN, SwiftUI, Flutter, Compose, and standalone native shells.

### ADR-004: Unified 2-Engine Layout via Host-Owned `packages/core/runtime`
- **Decision:** Collapse NVE's 5 layout engines down to **2**:
  1. **Browser CSS** on Web (`packages/backends/web` consumes pre-layout `NveFrameCommandBatch` directly in-process; `taffy` and `nve-runtime` are never linked on WASM).
  2. **Host-owned Rust `packages/core/runtime` (`libnve_runtime`)** using `Taffy` by default (with optional `Yoga` via `LayoutEngine` trait for RN Yoga parity) and an LRU-cached OS `TextMeasurer` C callback (`CoreText`, batched Android `StaticLayout`, Pango, `QFontMetrics`, `DirectWrite`), publishing extended `NveSceneView` directly across C ABI to all 10 native hosts.
- **Why:** Keeping `Taffy` in the host runtime (and statically LTO-linking `app_rlib + nve-runtime` into one binary in release `nve build`) keeps Tier-2 hot-swappable app `cdylib`s thin and fast, preserves text-measurement caches and scroll/hover state across `dlopen` swaps, supports zero-dylib paired thin clients (`nve-wire`) and Frame-0 instant boot, deletes ~8,000–11,500 LOC of duplicated C++/Dart layout & decoder code (`flexbox_solver.cpp`, `yoga_layout.cpp`, `shadow_tree.cpp`, `flex_layout.dart`, `command_decoder.dart`), and brings CSS Grid, RTL, `sticky`, `:hover`/transitions, gradients, and SVG uniformly to every native host.
