# Code-Truth Quality & Maturity Audit (`native-view-engine`)

> **Date:** 2026-11-25 (audited against `main` at `3c7e749`–`752ea2e`, updated after **Step 1 Part B: Cleanup & Truthfulness Pass** on `t3/cleanup-truthfulness-pass`).
> **Method:** Direct read-only source inspection across all workspace targets (12 core crates incl. Wave S2 `wire`/`wire-core` + `hash`, 3 backends, 12 framework crates incl. Slice 1b/1c `query`/`http-web`/`query-dioxus`/`ssr`, 11 remaining modules after deleting 7 pure-L0 crates and Wave S2 `pair`, 2 tool crates, `cxx/nve-fabric-bridge`, 12 hosts, 4 examples). Zero self-grading from READMEs or mock-only unit tests.

---

## Grading Rubric (L0–L4)

Default posture is skepticism: nothing is graded above L1 without verified real OS/GPU/browser execution and non-mock tests.

| Level | Name | Definition |
| :---: | :--- | :--- |
| **L0** | **Stub / Mock / Spec-Only** | Types, wire codecs, contracts, or in-memory fakes only. Zero real OS, GPU, or browser execution. |
| **L1** | **Prototype** | Calls a real OS/browser/renderer API on happy paths, or ships pure state machines without real UI/host wiring. Missing critical edge cases, real interaction wiring, lifecycle/error handling, or real-target verification. |
| **L2** | **Working Core** | Real OS/browser/renderer implementation verified end-to-end on real targets (not only mocks), with explicit known gaps in API design, performance, or cross-platform coverage. |
| **L3** | **Production-Grade** | Complete core feature surface, hardened edge cases, real-target E2E & benchmarks, clean API design, zero stubs or silent fallbacks. |
| **L4** | **Gold Standard** | Category-leading reference quality (the bar set by `react-native-vision-camera`): full hardware/platform capabilities, maximum achievable performance, exhaustive real-device/OS edge-case coverage, best-in-class API ergonomics. |

---

## Executive Summary & Distribution

**Zero crates or hosts currently reach L3 or L4.**

| Area | Total | L0 (Stub) | L1 (Prototype) | L2 (Working Core) | L3 (Prod) | L4 (Gold) |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| `packages/core/*` | 12 | 0 | 3 (`host-services`, `interop`, `layout-yoga`) | 9 (`abi`, `abi-macros`, `codegen`, `components`, `dioxus`, `engine`, `hash`, `wire`, `wire-core`) | 0 | 0 |
| `packages/backends/*` | 3 | 0 | 1 (`skia`) | 2 (`headless`, `web`) | 0 | 0 |
| `packages/framework/*` | 12 | 0 | 3 (`docs`, `i18n`, `ui`) | 9 (`http-hyper`, `http-web`, `mdx`, `query`, `query-dioxus`, `router`, `rpc`, `server`, `ssr`) | 0 | 0 |
| `packages/modules/*` | 11 | **0** *(7 L0 crates deleted)* | **11** *(trimmed `camera` & `native-views`; `pair` deleted in Wave S2)* | 0 | 0 | 0 |
| `packages/tools/*` | 3 | 0 | 1 (`cli`) | 2 (`arch-invariants`, `bench`) | 0 | 0 |
| `cxx/nve-fabric-bridge` | 1 | 0 | 1 | 0 | 0 | 0 |
| `hosts/*` | 12 | 0 | 6 (`compose-kit`, `flutter-plugin`, `gtk-shell`, `ios-shell`, `swiftui-kit`, `windows-shell`) | 6 (`android-shell`, `apple-common`, `macos-shell`, `qt-shell`, `skia-shell`, `web-shell`) | 0 | 0 |
| `examples/*` | 4 | 0 | 1 (`kitchen-sink`) | 3 (`fullstack-issues`, `nve-docs-site`, `studio-workspace`) | 0 | 0 |
| **Total** | **57** | **0** | **27** | **30** | **0** | **0** |

### Five Systemic Root Causes Across the Repo

1. **Out-of-Band Side-Channels Instead of One Canonical Render ABI**:
   - Interactive pseudo-states (`:hover`/`:active`/`:focus-visible`), `CursorKind`, `TransitionSpec`, `@media` rules, `DrawList`, `NativeViewProps`, `BackgroundFill` gradients, and `SvgPicture` were all bolted onto `DioxusSurface` as separate Rust iterators (`iter_*` → `sync_*_from`) instead of living in the single frame ABI.
   - Consequence: **none of them cross the C ABI boundary** to `macos-shell`, `ios-shell`, `android-shell`, `qt-shell`, `gtk-shell`, `windows-shell`, `swiftui-kit`, `flutter-plugin`, or `skia-shell --dylib`. Even `web-shell` forgets to sync newly added side-channels (`background_fill`, `svg_picture`).
2. **Headless State-Machine Theater in `nve-ui` (Why ~80% of Kitchen Sink Controls Are Fake-Compliant)**:
   - Across the entire 17,800-LOC `packages/framework/ui/src/` crate, `rg "on_hover|onhover|onpointer|ondrag|onmouse|onwheel|cursor\("` returns **0 matches**.
   - **Zero `nve-ui` components** (`Button`, `IconButton`, `Checkbox`, `Switch`, `Input`, `TabTrigger`, `MenuItem`, `Combobox`, `DatePicker`) define hover, active, or cursor styles.
   - `SplitPane` (`split_pane.rs:508-536`) and `BottomSheet` (`bottom_sheet.rs:389-593`) implement 350+ lines of unit-tested drag physics in pure structs, but attach **zero pointer/drag handlers** to the rendered DOM/native nodes (`SplitPane` divider click only collapses; dragging does nothing).
   - `Tooltip` (`tooltip.rs:107-160`) never opens on mouse hover (only `onfocus`) and renders as an **inline flex child** with inert `data-x`/`data-y` attributes rather than floating at screen coordinates.
   - `Popover` / `DropdownMenu` (`overlays.rs`) dump inert `data-x`/`data-y` attributes on inline flex nodes; Kitchen Sink feeds `Popover` **hardcoded fake coordinates** (`anchor = Rect { x: 180, y: 120 }`, `vp = Viewport { width: 640, height: 360 }`) instead of measuring the real trigger element.
   - `Dialog` / `Modal` (`overlays.rs:145-160`) renders `backdrop_style` with `width: 100%; height: 100%; z-index: 1000` without `position: fixed; inset: 0`, so controlled dialogs participate in document flow instead of overlaying the viewport.
   - `VirtualList` in Kitchen Sink (`layout.rs:223-225`) remounts the entire list (`key: "vl-{epoch}"`) to simulate jump-to-row rather than binding live scroll synchronization.
3. **Mock-Only & Screenshot-Only Verification Trap**:
   - Component tests call `state.handle_drag_delta(40.0)` directly or assert `data-x="180"` in `MirrorElement` strings, never asserting real `getComputedStyle`, `getBoundingClientRect`, mouse hover transitions, pointer drags, or scroll wheel updates in a real browser/window.
   - Host tests frequently test mocks (`FakeNveBridgeBackend`, `jni_test_recorder`, `InMemory*CameraBridge`, `MemoryClipboard`) while real native linkage is broken (`compose-kit` missing 15 JNI symbols, `flutter-plugin` missing all native `NveFlutterPlugin` directories, `swiftui-kit` `NveSurfaceRepresentable` empty view).
4. **Unwired Host Services (`11 of 12` Hosts Never Bind `NVE_IFACE_HOST = 5`)**:
   - Only `hosts/skia-shell/src/host_services.rs` binds `NVE_IFACE_HOST = 5` (registering 7 of 15 services, 3 of which return `Unsupported` for primary operations). Every other host has zero runtime host-service dispatch, causing `nve-docs-site` and `studio-workspace` to silently fall back to `MemoryClipboard`/`MemorySystemEnv`.
5. **Marketing Overclaims vs. Actual Code *(Partially Resolved in Step 1 Part B)***:
   - **Resolved**: `"Zero-JSON"` marketing copy was stripped repo-wide (0 hits outside `CHANGELOG.md`/`quality-audit.md`/`AGENTS.md`), and production `serde_json` was removed from `packages/core/engine` (moved to `[dev-dependencies]` only), `packages/core/interop`, `examples/studio-workspace`, and `hosts/skia-shell` (leaving `serde_json` only in build/dev tools `nve-codegen`, `nve-cli`, `nve-bench`, and `nve-engine` dev-deps).
   - **Still open**: Real RN Fabric ShadowTree mounting (`cxx/nve-fabric-bridge/src/fabric_c_api.cpp:143-159`) is still `old_root.clone({})` (a no-op stub), and `cxx/build-fabric` compiles with `NVE_WITH_RN_FABRIC=OFF`.

> **Workspace Hygiene Note:** An untracked **7.6 GB** OpenJ9 JVM dump (`core.20261125.032910.85615.0001.dmp` + `javacore.*.txt` + `Snap.*.trc`) is sitting in `hosts/android-shell/` and should be deleted.

---

## 1. `packages/core/*`

| Crate | LOC | Level | What Is Real | Key Flaws / Stubs / API & Perf Debt | Disposition |
| :--- | ---: | :---: | :--- | :--- | :--- |
| `packages/core/abi` | 8,438 | **L2** | `nve_get_interface(1..=5)`, `#[repr(C)]` `NveValue`/`NveDirectEvent` (96B with typed key/pointer/wheel fields), 64B `NveNodeMeta`, `NveSceneTable`/`NveSceneView`, `offset_of!` layout asserts, schema hash cross-check, property tests. | *(Track 1A deleted `NK1`, `set_props(&[u8])`, `dispatch_events(&[u8])`, `NVE_MET1`, and `BinValueArena`; unified on direct `NveEventBatch`/`NveMapEntry` FFI.)* `NveSurfaceApi::render` still serializes `NVE1` byte streams in-process; duplicate scene structs (`64B NveRenderNode` vs `128B RelocSceneNode`); 1,075-LOC test file. | **Keep & Harden API** (Wave C1b direct `NveFrameCommandBatch`) |
| `packages/core/abi-macros` | 496 | **L2** | Zero-dep `#[derive(NveAbi)]` emitting `NveAbiDescriptor` + size/offset asserts. | Rejects generics (`parse.rs:39`); ignores `#[cfg(...)]` on fields (silent schema mismatch risk); no `trybuild` compile-fail tests. | **Keep & Harden API** |
| `packages/core/codegen` | 32,379 | **L2** | Single-source ABI/Scene/Wire/Reloc/Hybrid emitter generating `15/15` bindings, executing real `clang`/`clang++`/`rustc`/`swiftc`/`javac`/`dart`/`node` tests. | Silent test skips when tools absent; `kotlinc`/`tsc` never executed; heavy emitter duplication across 6 backends; `export_rn_cpp.rs:172-200` builds `std::unordered_map` on every `update_props`; 4 test files > 650 LOC (`hybrid_codegen_tests.rs` 1,999 LOC); `README.md` claims "Zero-JSON" while using `serde_json` for export specs; cryptic magics (`NVH1`, `NVT2`, `NVL1`, `NVCF`, `NVEV`). | **Refactor / Simplify** |
| `packages/core/components` | 10,860 | **L2** | `DrawList` IR, `LayoutExt` (Grid/Sticky/RTL), `Spring`/`TransitionSpec`, `Transform2D`, `InteractionTracker`, `BackgroundFill`, `SvgPicture`. | `NativePropDecoder::decode` (`native_view/codec.rs:314-347`) claims "zero-copy" but heap-allocates `Vec<(u16, NativeValueRef)>` per decode; several native-view specs (`WebElement`, `VideoPlayer`, `BlurSurface`) lack verified native host implementations; cryptic magics (`NVNP`, `NVNC`, `NVE_DRAW1`). | **Refactor / Simplify** |
| `packages/core/dioxus` | 10,059 | **L2** | `WriteMutations` (`NveMutationWriter`), `DioxusSurface` (with `new_with_props`/`new_with_context`), `TypedStyle`, `ErrorBoundary`, `SignalRegistry`. | *(Track 1A slimmed `NodeRecord` from 1,112B → 192B via boxed `NodeColdExtras` and added `DioxusSurface::new_with_props`/`new_with_context`.)* `in_process_mode` defaults `false` and still encodes `NVE1` bytes; `render_frame` `.to_vec()` copies frame bytes; 781-LOC test file. | **Refactor / Simplify** |
| `packages/core/engine` | 10,510 | **L2** | `NveApp` + `export_nve_app!` with `catch_unwind`, `HotAppInstance` + shared `StagedDylib` loader, Unix/Win32 `MmapOffsetArena`, `RelocSceneRoot`, `FrameArena`, `xxh64`. | *(Step 1 Part B removed production `serde_json`; Track 1A unified `NveApp` on direct `dispatch_events(&[NativeEvent])` / `set_props(&[PropEntry])` / `drain_emitted_events() -> Vec<NativeEvent>` with zero `EventRing` round-trip.)* `$TMPDIR/nve-hotswap/` leaks staged dylibs on crash; 1,361-LOC test file. | **Refactor / Simplify** |
| `packages/core/host-services` | 2,991 | **L1** | `NVE_IFACE_HOST = 5` client, binary codec, `HostServiceRegistry`, `apple_objc.rs`. | *(Track 1A fixed the `FfiHostServices` async callback registration race with concurrent stress tests, added stack `ResponseBuf` `<= 4096B` inline buffer, made `objc_autoreleasePoolPop` unwind-safe via RAII guard, and deleted `NativeHostServices`.)* Still needs real multi-host `NVE_IFACE_HOST = 5` bindings beyond `skia-shell`. | **Keep & Harden** |
| `packages/core/interop` | ~17,500 | **L1** | Real L2 `crdt` (HLC/LWW/OR-Set/RGA), `traits`, `gesture` arbitrator, `WindowManager` (used by `skia-shell` & `web-shell` multi-window hosts). | *(Step 1 Part B deleted `handoff.rs`, `serde_json`/`PayloadEncoding::Json`, and deduplicated `plugin_host/sandbox.rs` onto `nve_engine::loader::StagedDylib`.)* `NveChannelBus` still clones topic/subscribers on every publish; oversized files remain. | **Refactor / Simplify** |
| `packages/core/layout-yoga` | 1,886 | **L1** | Compiles real RN C++20 `yogacore` + `YGNodeSetMeasureFunc` + Taffy parity tests when `RN_REACT_COMMON_DIR` is set. | Silently compiles to uninhabited stub when env var unset (`lib.rs:450-529`). Drops `set_raw_layout_prop` (`flex-wrap`, `font-weight`, `align-self`), CSS Grid, gaps, borders; hardcodes `YGDirectionLTR` (`yoga_shim.cpp:457`); ignores `font_weight` in text measurement; clones `Vec<NodeId>` per node on every layout pass (`lib.rs:267`). | **Keep & Harden API** |
| `packages/core/wire-core` | ~2,600 | **L2** | Sans-I/O `NveWireFrame` v2 framing, `NveTimelineArchive`, `NveSceneArchive` keyframes, `XXH64` integrity (`752ea2e`). | Landed in Wave S2; replaces hand-rolled `NVT1`/`NVD1`. | **Keep & Harden API** |
| `packages/core/wire` | ~2,100 | **L2** | Unified `quinn` QUIC + `web-transport-quinn` + WSS Happy-Eyeballs transport (`752ea2e`). | Landed in Wave S2; needs broader real-network soak tests. | **Keep & Harden API** |
| `packages/core/hash` | 445 | **L2** | `#![no_std]` zero-dep compile-time + streaming `XXH64` (`xxh64`, `const_id`, `Key64`, `Xxh64Hasher`) locked against official reference vectors. | Coexists with legacy `packages/core/engine/src/hash.rs` until `t3/word-table-nodestore-nve2` merges. | **Keep** |

---

## 2. `packages/backends/*`

| Crate | LOC | Level | What Is Real | Key Flaws / Stubs / API & Perf Debt | Disposition |
| :--- | ---: | :---: | :--- | :--- | :--- |
| `packages/backends/headless` | 4,387 | **L2** | Fast Taffy Flexbox+Grid layout, `NodeSlotMap`, hit-testing (scroll/sticky/`Transform2D`), `InteractionTracker`, `prerender_native_frame0`. | Uses SipHash `HashMap` for `raw_props` and `scroll_offsets` (`lib.rs:62,102`); duplicates shadow-tree state already in `NveMutationWriter` when used in-process. | **Keep & Harden API** |
| `packages/backends/skia` | 17,691 | **L1** | CPU software rasterizer + `cosmic-text` BiDi/grapheme/IME text + PNG LRU cache + gradients/SVG (`7cfa635`) + `objc2-metal`/`ash`/`d3d12` swapchain wrappers + 2 Metal offscreen parity tests. | **Severe GPU perf bugs**: `gpu/skia_renderer.rs:308-318` copies full RGBA pixels & recreates `skia_safe::Image` on **every frame** (zero GPU texture cache); `gpu/skia_renderer.rs:77-95` scans OS `FontMgr` on **every bold `DrawText` op**; `gpu/skia_renderer.rs:251-290` uses raw `draw_str` with **no word wrap, no BiDi, no emoji/CJK fallback**; `gpu/vulkan.rs` is **1,221 LOC with 0 Vulkan tests**; `gpu/d3d12.rs` has **0 unit tests**; `DisplayList` clones `String` per text/image op every frame. | **Keep & Harden API** |
| `packages/backends/web` | 18,797 | **L2** | Real `web-sys` DOM backend, Option 5 Hybrid CSS (`SsrStyleTracker` streaming delta dedup + CSP `style_nonce`), LIS keyed child diffing, top-down structural hydration + atomic DSD promotion, semantic HTML5 SSR/SSG + DSD streaming + Shadow-aware `composedPath` replay, gradients + inline `<svg>`, `xxh64` SSR hashing. | 1 allowlisted src file > 650 LOC (`snapshot.rs` 690). Zero in-crate `wasm_bindgen_test`s. Cryptic magics (`NVWS1`, `NVB1`). | **Keep & Harden API** |

---

## 3. `packages/framework/*`

| Crate | LOC | Level | What Is Real | Key Flaws / Stubs / API & Perf Debt | Disposition |
| :--- | ---: | :---: | :--- | :--- | :--- |
| `packages/framework/docs` | 5,653 | **L1** | Working Fumadocs-inspired shell dogfooded by `nve-docs-site`. | *(Track 1A deleted `PENDING_BOOTSTRAPS` thread-local FIFO hack and `fallback_empty_state()` via `DioxusSurface::new_with_context`.)* Mirror-only tests. | **Keep & Harden** |
| `packages/framework/i18n` | 6,057 | **L1** | Pure-Rust BCP-47, ICU MessageFormat, CLDR plurals/numbers/dates/relative-time, pseudolocalization, `wasm32` `Intl.*` inline-JS bridge. | **0% native OS integration**: macOS/iOS (`Foundation`), Android (`android.icu`), and Windows (`icu.dll`) are completely unimplemented — all silently fall back to hardcoded 18–41 locale tables. `wasm_intl.rs:118-128` allocates `new Intl.Locale` per call. Zero `wasm32` tests. | **Keep & Harden API** |
| `packages/framework/mdx` | 6,976 | **L2** | Pure-Rust Markdown/MDX parser, YAML frontmatter, `_meta.toml` nav, Rust static codegen, search index with edge-case tests. | Docs-subset Markdown/MDX (not full CommonMark/GFM); keyword-level syntax highlighter; cryptic `NVS2` format name (`search_codec.rs:7`). | **Keep & Harden API** |
| `packages/framework/router` | 5,795 | **L2** | Typed `Routable`, stack/tab navigation, guards, query params, deep-link matching, `wasm32` `web_sys::History` + `popstate`. | Zero `wasm32` browser tests (`web_history_tests.rs` only tests `MemoryHistoryBackend`); no native OS universal-link host bridge; cryptic `src/nvn1.rs` naming. | **Keep & Harden API** |
| `packages/framework/ui` | 17,825 | **L1** | Solid headless math/algorithms (`FenwickHeightTree`, `positioning.rs`, `FormState`, `FocusTrap`, `DatePicker` calendar math). | **Fake-compliant UI wiring**: `0` hover/active/cursor/pointer/drag handlers anywhere in `src/`; `SplitPane` & `BottomSheet` cannot be dragged; `Tooltip` doesn't open on hover; `Tooltip`/`Popover`/`DropdownMenu` render inline with inert `data-x`/`data-y` attributes instead of floating; `Dialog` backdrop lacks `position: fixed; inset: 0`; 772 LOC of dead duplicate style builders (`TypedStyleBuilder` 471 + legacy `StyleBuf` 301). | **Refactor / Simplify** (rebuild component interaction & positioning primitives properly, delete duplicate `style_buf`) |
| `packages/framework/rpc` | 2,789 | **L2** | Transport-agnostic `Procedure`/`procedures!`, direct RFC 8949 byte-slice `validate_cbor` + `CborCodec`, domain-separated `Tag64` + O(1) `*` overflow sink/header, Connect-compatible JSON error codec (surrogate pairs, zero `serde_json`), bounded `FormDecode`, `wasm32` & `wasm32-wasip2` clean, `CountingAlloc` + Criterion benchmarks. | Optional Protobuf/polyglot client codegen deferred to Slice 3 if demanded by apps. | **Keep** |
| `packages/framework/server` | 3,342 | **L2** | Sans-I/O `Server` with preflight-before-body-read, fail-closed `CsrfPolicy` + opt-in `CorsPolicy` (`OPTIONS` preflight, `Access-Control-Expose-Headers`, `Cookie` stripping when credentials disallowed), `AuthResolver` (`try_resolve_sync` 0-alloc fast path), sorted-slice `RpcRegistry`, built-in `FormRoute` + `FormOutcome`, trusted SSR vs strict HTTP `DirectTransport`, `wasm32-wasip2` clean. | Edge `wasi:http` adapter arrives in Slice 2 (`nve-http-wasip2`). | **Keep & Build Slice 2** |
| `packages/framework/http-hyper` | 1,852 | **L2** | Hyper HTTP/1.1 `NveHyperService` (`hyper::Service` + `tower_service::Service`, live Axum `nest_service` & `route_service` verified), streaming `ChannelBody` + `HttpBody::{Full, Stream}` with loopback disconnect cancellation, zero-alloc single-frame `collect_body_limited` + `408` body timeout, `ServeConfig`, and client-only `HyperTransport` (`with_background_runtime()`). | Plain HTTP/1.1 only (`h2` avoided due to `fnv`); TLS/H2 terminated by proxy until Slice 2 `nve-wire` QUIC/H3 transport. | **Keep & Build Slice 2** |
| `packages/framework/query` | 6,423 | **L2** | Sans-I/O flat-SoA `QueryClient` (64B `QueryMeta` + `FLAG_STREAM_PENDING`, flat `TagEdge` O(k) tag index, O(1) wildcard epoch), deduplicated cancellation-safe fetches, outside-lock rebaseable optimistic layers, retry backoff, bounded GC, `NVEQ` v2 streaming/error dehydration (`DehydrateCursor`, `dehydrate_initial_stream_chunk`, `dehydrate_handles_delta`, fill-only hydration, `finish_ssr_stream`, `wait_for_settled`), `wasm32` & `wasm32-wasip2` clean, 0-alloc hot paths, 2,000-step property test. | Multi-tab BroadcastChannel sync / offline persistence deferred beyond Slice 1. | **Keep** |
| `packages/framework/http-web` | 721 | **L2** | Browser `wasm32` `FetchTransport` (`wasm-bindgen(inline_js)` + `js-sys` browser `fetch` with zero `web-sys` dependency, `AbortController` timeout/drop abort, `ReadableStream` streaming byte cap, `redirect: "manual"` 302 rejection, `Nve-Invalidate` / `Nve-Error-Kind` parsing), `BrowserClock`, cancellation-safe `browser_sleep_ms`, and `attach_browser_lifecycle` (`focus`/`online`), verified live in Headless Chrome CDP E2E (incl. cross-origin CORS). | Native non-`wasm32` targets return clean `TransportErrorKind::Config` (use `nve-http-hyper` natively). | **Keep** |
| `packages/framework/query-dioxus` | 5,589 | **L2** | Transport-erased `ErasedQueryClient`, `QueryClientProvider` (`SleepHandle` + single-thread native `Condvar` timer wheel), dense recycled slot `Signal<u32>`s, provider-scoped `use_query`/`use_query_with` (`suspend`/`suspend_result`), `use_action` (`run_optimistic`), portable `<QuerySuspense>` (`SsrBoundaryRegistry`, `BoundaryQuerySet`, `ErrorBoundary` retry), progressive `<ActionForm<P>>`, verified headless + live Chrome CDP E2E. | None blocking Slice 1; polyglot client sugar deferred to Slice 3 if needed. | **Keep** |
| `packages/framework/ssr` | 2,854 | **L2** | Dedicated `!Send` `LocalSet` worker pool (`SsrLocalPool`), adaptive Light-DOM `HttpBody::Full` fast path (`~64.5 µs`) + out-of-order DSD `HttpBody::Stream` path (`~98.3 µs` 2-slot), atomic `NVEQ` v2 + `<div slot>` deltas, immediate TCP disconnect cancellation (`tx.closed()`), concurrent multi-user isolation, `generate_csp_nonce()`, strict zero-inline-JS CSP defaults, verified with real loopback TCP & 7-suite live Chrome CDP E2E. | Edge `wasm32-wasip2` streaming SSR runtime adapter deferred to Slice 2. | **Keep** |

---

## 4. `packages/modules/*` (11 Remaining Crates; 7 Pure-L0 Crates + `pair` Deleted)

### Special Deep Dive: `packages/modules/camera` — **L1 (Trimmed to VisionCamera Adapter + `CameraPreview` Spec)**

In **Step 1 Part B**, all half-baked standalone platform stubs (`src/host_service*.rs`, `nve_camera_linux.hpp`, `nve_camera_windows.hpp`, standalone `AVCaptureSession` capture in `NveAppleCamera.mm`, `NveAndroidCameraXBinding.kt`/`NveCameraXAdapter.kt`/`NveCameraPreviewComposable.kt`, `camera/web/`, and `react-native/windows/`) were **deleted**, and `react-native/package.json` was corrected to state honestly that `@nve/camera` is an optional VisionCamera adapter / `CameraPreview` binding.

- **Retained Adapter Surface (Keep & Harden)**:
  - Typed `CameraPreview` `NativeViewSpec` + 40B `#[repr(C)]` `CameraFrameWireLayout` (`src/spec.rs`, `src/frame.rs`).
  - Pluggable **VisionCamera hybrid adapter contracts** (`@protocol NveVisionCameraAppleBridge`, `interface VisionCameraSessionBridge` / `NveVisionCameraAdapter.kt`, `HybridCameraSession` in `nve_vision_camera_hybrid.hpp`) — note these still need compilation/verification against real Margelo VisionCamera v5 headers.
- **Postponed**: Full standalone L4 Gold-Standard cross-platform camera (the `react-native-vision-camera` bar) is tracked honestly in [docs/BACKLOG.md](./BACKLOG.md#postponed--long-horizon).

### Remaining & Deleted Module Crates

| Crate | LOC | Level | Reality Summary | Disposition |
| :--- | ---: | :---: | :--- | :--- |
| `packages/modules/audio` | 0 *(was 1,042)* | **Deleted** | Pure L0 stub (`Unsupported` on all 10 methods). | **Deleted in Step 1 Part B → Backlog** |
| `packages/modules/fonts` | 0 *(was 893)* | **Deleted** | Pure L0 in-memory metadata map with zero OS font registration. | **Deleted in Step 1 Part B → Backlog** |
| `packages/modules/location` | 0 *(was 677)* | **Deleted** | Pure L0 stub (`Unsupported` / hardcoded SF coords). | **Deleted in Step 1 Part B → Backlog** |
| `packages/modules/notifications` | 0 *(was 511)* | **Deleted** | Pure L0 stub (`Unsupported` everywhere); unregistered from `skia-shell`. | **Deleted in Step 1 Part B → Backlog** |
| `packages/modules/sensors` | 0 *(was 609)* | **Deleted** | Pure L0 stub (`false` / `Unsupported`). | **Deleted in Step 1 Part B → Backlog** |
| `packages/modules/share` | 0 *(was 452)* | **Deleted** | Pure L0 stub (`Unsupported` on all 3 methods). | **Deleted in Step 1 Part B → Backlog** |
| `packages/modules/web-auth` | 0 *(was 566)* | **Deleted** | Pure L0 stub (`Unsupported` on both methods). | **Deleted in Step 1 Part B → Backlog** |
| `packages/modules/pair` | 0 *(was 2,122)* | **Deleted** | Unlinked pairing state machine with zero sockets (`752ea2e`). | **Deleted in Wave S2** |
| `packages/modules/clipboard` | 531 | **L1** | Real macOS `NSPasteboard` + Win32 `CF_UNICODETEXT`; Linux/iOS/Android/Web `Unsupported`; plain text only; duplicated in `skia-shell/src/window/clipboard.rs`. | **Keep & Harden** |
| `packages/modules/filesystem` | 754 | **L1** | Real Win32 `SHGetKnownFolderPath` + `$HOME`/`XDG_*` paths; **`pick_files` and `save_file` return `Unsupported` on every OS**. | **Keep & Harden** (trim picker claims or build real dialogs) |
| `packages/modules/haptics` | 406 | **L1** | macOS Force Touch only; **iOS, Android, Windows, Linux, Web all `Unsupported`**. | **Delete / Defer → Backlog** |
| `packages/modules/linking` | 552 | **L1** | Outbound `NSWorkspace.openURL` / `ShellExecuteW` / `xdg-open` only; `initial_url()` hardcoded `Ok(None)` everywhere; zero inbound deep links. | **Keep & Harden** |
| `packages/modules/secure-store` | 755 | **L1** | Real macOS Keychain + Win32 CredMan (no 2560B check, mock-only tests); Linux/iOS/Android/Web `Unsupported`. | **Keep & Harden** |
| `packages/modules/system-env` | 1,079 | **L1** | Real macOS appearance/locale + Win32 theme/locale, but hardcodes accent/scale/24h/contrast fields; **WASM returns `Default::default()`** (no `matchMedia`/`Intl`). | **Keep & Harden** |
| `packages/modules/storage` | 1,670 | **L1** | Crash-safe `NVK2` WAL disk KV engine is solid L2 code, but **zero hosts register it**, zero examples use it, WASM is volatile in-memory, uses cryptic `NVK2` name. | **Keep & Harden** |
| `packages/modules/native-views` | ~4,100 | **L1** | Apple `NveAppleNativeViews.mm` has real `NSVisualEffectView`/`WKWebView`/`AVPlayer`; Win32 has `DwmSetWindowAttribute` blur; Flutter has `BackdropFilter`. *(Step 1 Part B deleted fake counter-only `nullptr` WebView/VideoPlayer stubs and GTK/Qt self-blur hacks; adapter hooks now return `nullptr` unless a real backend is injected.)* | **Keep Apple + Clean Adapters** |
| `packages/modules/nitro-bridge` | 2,350 | **L1** | Clean standalone C++20 `HybridObject`/JNI layer, but has **zero dependency on Margelo `react-native-nitro-modules`**; only linked by `camera`. | **Defer / Fold Into RN Adapter** |
| `packages/modules/tv` | 1,780 | **L1** | Real pure-Rust 2D spatial focus & overscan math + Kotlin key bridge, but **zero workspace consumers** and zero Android TV / Apple TV tests (`swiftui-kit` duplicates overscan). | **Defer → Backlog** |

---

## 5. `packages/tools/*` & `cxx/nve-fabric-bridge`

| Target | LOC | Level | Reality Summary | Disposition |
| :--- | ---: | :---: | :--- | :--- |
| `packages/tools/arch-invariants` | ~3,500 | **L2** | Standalone `syn` + `guppy` + `target-spec` L2 architectural invariant suite (workspace DAG, RPC/server purity, `[INV:<id>]` spec registry, `#[repr(C)]` locks, 650-line ratchet, README doctest & benchmark guards) with synthetic positive/negative unit tests. | **Keep** |
| `packages/tools/bench` | 4,341 | **L2** | Real Criterion suites (`abi`, `dioxus`, `backends`, `interop`, `arena_and_mmap`) + WASM/Brotli budget checker. Redundant hand-rolled JSON codec in `baseline_codec.rs`; zero GPU Skia or real browser DOM benchmarks. | **Keep & Harden API** |
| `packages/tools/cli` | 7,526 | **L1** | Working `codegen`, `dev` (spawning binary-only `nve-skia-shell` subprocess), `pair`, `docs`, `prerender`, `build`, `bench`, `doctor`. Hardcodes internal example crates in `Cargo.toml` (cannot prerender external apps); `main.rs:172` docs mismatch `prerender.rs`; `dev.rs` hardcodes `target/debug` and `./hosts/web-shell/build.sh` (broken on Windows). | **Refactor / Simplify** |
| `cxx/nve-fabric-bridge` | 22,416 | **L1** | Real C++20 RN Yoga (`NVE_WITH_RN_YOGA=1`) + RN JSI (`NVE_WITH_RN_JSI=1`) + standalone scene publisher + `NVERELOC` mmap + CTest/fuzz. **CRITICAL**: `commit_rn_shadow_tree` (`src/fabric_c_api.cpp:143-159`) is a **no-op stub** (`old_root.clone({})`), `cxx/build-fabric` compiles with `NVE_WITH_RN_FABRIC=OFF`, `update_props_json` (`nve_rn_component.cpp:206`) is broken, **11 files exceed 650 LOC** (`nve_snapshot.hpp` 1,358 LOC hand-duplicates `nve-codegen`). | **Refactor / Simplify** |

---

## 6. `hosts/*` (All 12) & `examples/*` (All 4)

| Target | LOC | Level | What Is Real | Key Broken / Missing / Stubbed Items | Disposition |
| :--- | ---: | :---: | :--- | :--- | :--- |
| `hosts/skia-shell` | 9,500 | **L2** | Binary-only `winit` multi-window host (zero `[lib]`), GPU Skia (opt-in, generic `GaneshPresenter<S>`) + `softbuffer` CPU, full IME/text editing, hot reload, Frame-0 instant boot, `#[cfg(test)]`-only `DioxusDesktopApp` + coalesced `METHOD_WINDOW_REQUEST_FRAME` async wakeup (moving to shared `packages/core/runtime` library in Track 1B-2), **only** host binding `NVE_IFACE_HOST = 5`. | **Zero accessibility** (no AccessKit); duplicates `nve-clipboard` (`window/clipboard.rs`); *(L0 `NativeNotifications` & dead `serde_json` removed in Step 1 Part B)*; side-channel styles (`:hover`, transitions, gradients, SVG) don't cross dylib FFI. | **Keep & Harden** |
| `hosts/web-shell` | 7,985 | **L2** | `wasm32` `talc`, real browser DOM/Canvas2D/ARIA, coalesced `WebAsyncWakeGuard`/`WakeScheduler` microtask + `setTimeout(0)` macrotask paint-yield async wakeup, Option B mid-stream/post-stream `NVEQ` v2 `MutationObserver` hydration (`wasm_query_stream.rs`, SSR auto-detect), `localStorage` snapshot boot, feature-gated `app-issues` fullstack bundle, real headless Chrome CDP E2E. | **Zero `NVE_IFACE_HOST = 5`** (no `navigator.clipboard`/`matchMedia`/`Intl`); hardcodes demo apps; *(split `wasm_api.rs` below 650 LOC into `wasm_hotreload.rs`, `wasm_wake.rs`, and `wake_sched.rs` in Slice 1b)*; doesn't yet sync newly added gradient/SVG side-channels. | **Keep & Harden** |
| `hosts/apple-common` | 3,778 | **L2** | Shared AppKit/UIKit reconciler, CoreGraphics `NVE_DRAW1`, LRU image cache, accessibility + IME guards, Frame-0 replay. | **Zero `NVE_IFACE_HOST = 5`**; single-line `NSTextField`/`UITextField` only; no hover/transition/gradient/SVG across FFI. | **Keep & Harden** |
| `hosts/macos-shell` | 4,145 | **L2** | CMake `-Werror` `NSWindow`/`NSView` host, multi-window, Frame-0 instant boot + state restore, real dylib CTest. | **Zero `NVE_IFACE_HOST = 5`**; RN Fabric `RCTViewComponentView` path untested in `check-all.sh`; no global keyboard shortcuts outside text fields. | **Keep & Harden** |
| `hosts/qt-shell` | 9,905 | **L2** | Real Qt 6 Widgets & Qt Quick `QSGNode`, IME preedit/commit, `QAccessible`, multi-window, Frame-0 boot, real dylib CTest. | **Zero `NVE_IFACE_HOST = 5`**; single-line `QLineEdit` only; linked `nve_qt_native_views` has `nullptr` WebView/Video stubs. | **Keep & Harden** |
| `hosts/android-shell` | 6,587 | **L2** | Real C++20 JNI bridge + Android Java Activity (`View`/`EditText`/`ScrollView`/`Canvas`, IME guard, TalkBack, `DirectByteBuffer`). | **Zero `NVE_IFACE_HOST = 5`**; zero Frame-0 instant boot; `NveMainActivity.java` is **1,378 LOC**; fails to export 15/21 Compose JNI symbols; desktop tests use fake `jni_test_recorder`; **7.6 GB untracked JVM core dump** in dir. | **Keep & Harden** |
| `hosts/ios-shell` | 801 | **L1** | Real `UIWindowScene`/`UIView`/`UITextField` code + Frame-0 replay + ad-hoc `run-simulator.sh`. | **No CMake/Xcode/SwiftPM/podspec, zero automated iOS tests** (`check-all.sh` only runs `-fsyntax-only`), zero `NVE_IFACE_HOST`, no keyboard/safe-area wiring. | **Keep & Harden** |
| `hosts/gtk-shell` | 5,319 | **L1** | Real GTK4 widgets + Cairo `NVE_DRAW1` + `gtk_accessible` + multi-window + Frame-0 boot. | **Zero IME preedit guard**, **zero keyboard event controller**, no multiline `GtkTextView`, **zero `NVE_IFACE_HOST`**, fake self-blur + `nullptr` WebView/Video. | **Keep & Harden** |
| `hosts/windows-shell` | 5,835 | **L1** | Real Win32 `HWND` + Direct2D/DirectWrite/WIC + click/wheel + DPI + Frame-0 + RNW `ComponentView` (937 LOC). | **Zero keyboard input** (`WM_KEYDOWN`/`WM_CHAR`), **zero IME**, **zero interactive `TextInput`**, **zero hover/drag/pointer-down**, **zero UIA accessibility**, **zero `NVE_IFACE_HOST`**. | **Keep & Harden** |
| `hosts/compose-kit` | 6,878 | **L1** | Clean Jetpack Compose `Canvas`/`BasicTextField`/`Modifier.semantics` + `DirectByteBuffer` reader. | **Crashes on real Android (`UnsatisfiedLinkError`)**: declares 21 `external fun`s in `NveJniBridge`, but `android-shell` only exports 6! 100% tested against `FakeNveBridgeBackend`. | **Keep & Harden** |
| `hosts/swiftui-kit` | 10,562 | **L1** | Direct Swift C-ABI FFI + `NveSceneCanvasView` SwiftUI `Canvas` + VoiceOver + `WKWebView` + `swift test`. | **`NveSurfaceRepresentable` renders an empty `UIView`/`NSView` (`NveHostContainerView`)**; **zero interactive `TextInput`/keyboard/IME**; `NveSwiftUIVideoPlayerView` is `Color.black`; `NveHostApi` resolved but never bound. | **Keep & Harden** |
| `hosts/flutter-plugin` | ~14,400 | **L1** | Direct `dart:ffi` + `NveCanvasHost` custom `RenderBox` + Flutter `SemanticsNode` + `flutter test`. | *(Step 1 Part B deleted phantom `flutter.plugin.platforms` declarations and dead `NveTextureHostWidget`/`NvePlatformViewWidget`.)* **Zero interactive `TextInput`/keyboard/IME**; 3 files > 650 LOC. | **Keep & Harden** |
| `examples/kitchen-sink` | 3,400 | **L1** | Real multi-section Dioxus app replacing `examples/counter`, 0 `serde_json`, SSG + WASM hydration passing. | Exposes the `nve-ui` defects above: ~80% of showcased controls lack hover/cursor/drag/floating positioning/real scroll sync. *(Removed deleted `nve-audio` combobox option in Step 1 Part B.)* | **Keep & Harden** (turns into real interaction testbed) |
| `examples/nve-docs-site` | 5,648 | **L2** | Full dogfooded docs site + landing page, SSG + WASM + native `cdylib`, 0 `serde_json`, live Chrome CDP E2E. | Hardcodes `MemoryClipboard` + `MemorySystemEnv` (`src/lib.rs:80-83`) — "Copy" buttons don't copy to OS/browser clipboard. | **Keep & Harden** |
| `examples/studio-workspace` | 17,604 | **L2** | Flagship multi-view app, binary `NWS1` state + native disk persistence, real TCP CRDT tests, dual-mode `NVE_IFACE_HOST`, **0 `serde_json`** *(removed in Step 1 Part B)*. | Live UI CRDT button uses in-memory mesh; WASM persistence is no-op. | **Keep & Harden** |
| `examples/fullstack-issues` | 3,417 | **L2** | End-to-end fullstack issue tracker: `api` (`wasm32`/`wasip2`-clean open enums + `procedures!` tags/invalidations), `server` (Hyper/Axum `Response<HttpBody>`, Cookie+Bearer, opt-in CORS, 0-JS HTML `<form>` `303 -> GET /issues` + `422`, streaming `nve-ssr` `GET /issues` with pre-headers `404`/critical data, out-of-order `stats`/`comments` DSD slots, strict zero-inline-JS per-request nonce CSP + external `/pkg/issues/boot.js`, opt-in `/_test/*` stream barrier), portable zero-platform `client` (`IssuesApp` with `use_query_with`, `use_action::run_optimistic`, `<ActionForm>`, `<QuerySuspense>` + `ErrorBoundary`), and live 7-suite Headless Chrome CDP E2E (post-/mid-stream hydration with 0 RPCs, 0 CSP violations, true 0-JS DSD slots). | In-memory store only. | **Keep & Expand** |

---

## 7. Prioritized Reset & Single-Track Roadmap

Stop fanning out 3–4 parallel feature waves. Execute **one vertical slice at a time at full depth**, proving every change with real interaction tests (real mouse hover computed styles, real pointer drags, real screen coordinates, real native/browser execution):

1. **Cleanup & Truthfulness Pass — Done (`t3/cleanup-truthfulness-pass`)**
   - Deleted the **7 pure-L0 module crates** (`audio`, `fonts`, `location`, `notifications`, `sensors`, `share`, `web-auth`), trimmed `packages/modules/camera` to the VisionCamera v5 adapter + `CameraPreview` spec, deleted fake counter-only WebView/VideoPlayer stubs & self-blur hacks in `native-views`, deleted L0 `handoff.rs` + duplicate dylib loader + `serde_json` in `interop` (retaining `window/manager.rs` because `skia-shell` and `web-shell` actively call it), and deleted phantom Flutter plugin declarations + dead `NveTextureHostWidget`/`NvePlatformViewWidget`.
   - Logged **L4 Gold-Standard Camera (VisionCamera-grade)** and deferred L0 modules honestly in `docs/BACKLOG.md`.
   - Stripped `"Zero-JSON"` marketing copy repo-wide and eliminated production `serde_json` from `nve-engine`, `nve-interop`, `studio-workspace`, and `skia-shell`.
2. **Track 1 — Core API Design & Performance (Zero-Serialization Direct FFI, Zero Side-Channels)**
   - Land Wave S2 (`nve-wire-core` + `nve-wire` QUIC/WebTransport, deleting `packages/modules/pair` & `NVT1`/`NVD1`).
   - Execute **Wave C1**: replace `NVE1` `CommandBuffer` / `NVEV` `EventRing` / `NK1` / `set_props(&[u8])` / `BinValueView` / `NVE_MET1` with unified `#[repr(C)]` `NveFrameCommandBatch`, `NveDirectEvent`, `NveEventBatch`, `NvePropsRef`, `NveNodeMeta` — AND fold interaction state rules (`:hover`/`:active`/`:focus-visible`), `CursorKind`, `TransitionSpec`, `BackgroundFill`, `SvgPicture`, and `DrawList` into the **single canonical render ABI** (deleting every out-of-band `sync_*_from` side-channel!).
   - Fix `DioxusSurface` root-props injection (deleting `nve-docs` thread-local bootstrap), slim `NodeRecord` (~600B → ~128B via boxed cold extras), and fix the `FfiHostServices` async race + ObjC RAII pool + `WebBackend` WASM→DOM readbacks.
3. **Track 2 — Make `nve-ui` & Kitchen Sink Actually Real (Real Interaction Primitives + Real Behavioral E2E)**
   - Add real **pointer events (`onpointerdown`/`onpointermove`/`onpointerup` + pointer capture)**, **hover/enter/leave**, **real `position: fixed` / top-layer floating portal + live trigger `getBoundingClientRect()` anchoring**, and **real controlled scroll sync** to `nve-dioxus` + `nve-backend-web` + `nve-backend-skia`.
   - Delete `packages/framework/ui/src/style_buf/` (772 LOC duplicate builders) and wire real `:hover`/`:active`/`:focus-visible` + `CursorKind` + pointer dragging + floating overlays across every `nve-ui` control (`Button`, `SplitPane`, `BottomSheet`, `Tooltip`, `Popover`, `DropdownMenu`, `Dialog`, `Combobox`, `DatePicker`, `VirtualList`).
   - Upgrade browser & Skia E2E tests to **behavioral assertions**: move the real mouse cursor and assert `getComputedStyle().backgroundColor` changes on `:hover`, drag the `SplitPane` divider and assert pane pixel widths change, hover `Tooltip` and assert floating `getBoundingClientRect()`, open `Dialog` and assert viewport scrim overlay without shifting page layout, and wheel-scroll `VirtualList`!
4. **Track 3 — Hardware Skia & Broken Host Linkage Fixes**
   - Fix GPU Skia texture caching, bold `Typeface` caching, and `CosmicTextEngine` shaping; add real Vulkan/D3D12 parity tests.
   - Fix `compose-kit` 15 missing JNI exports, `swiftui-kit` empty `NveSurfaceRepresentable` + `TextField`, `windows-shell` keyboard/IME/UIA, `skia-shell` AccessKit, and `NVE_IFACE_HOST = 5` across real hosts.
