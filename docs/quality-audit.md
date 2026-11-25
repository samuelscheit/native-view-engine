# Code-Truth Quality & Maturity Audit (`native-view-engine`)

> **Date:** 2026-11-25 (audited against `main` at `3c7e749`–`7cfa635`; `t3/wave-s2-wire-quic-naming-timeline` noted where it deletes legacy `pair` / `NVT1` / `NVD1`).
> **Method:** Direct read-only source inspection across all **53 targets** (9 core crates, 3 backends, 5 framework crates, 19 modules, 2 tool crates, `cxx/nve-fabric-bridge`, 12 hosts, 3 examples). Zero self-grading from READMEs or mock-only unit tests.

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
| `packages/core/*` | 9 | 0 | 3 (`host-services`, `interop`, `layout-yoga`) | 6 (`abi`, `abi-macros`, `codegen`, `components`, `dioxus`, `engine`) | 0 | 0 |
| `packages/backends/*` | 3 | 0 | 1 (`skia`) | 2 (`headless`, `web`) | 0 | 0 |
| `packages/framework/*` | 5 | 0 | 3 (`docs`, `i18n`, `ui`) | 2 (`mdx`, `router`) | 0 | 0 |
| `packages/modules/*` | 19 | **7** (`audio`, `fonts`, `location`, `notifications`, `sensors`, `share`, `web-auth`) | **12** (incl. `camera`, `native-views`, `pair`, etc.) | 0 | 0 | 0 |
| `packages/tools/*` | 2 | 0 | 1 (`cli`) | 1 (`bench`) | 0 | 0 |
| `cxx/nve-fabric-bridge` | 1 | 0 | 1 | 0 | 0 | 0 |
| `hosts/*` | 12 | 0 | 6 (`compose-kit`, `flutter-plugin`, `gtk-shell`, `ios-shell`, `swiftui-kit`, `windows-shell`) | 6 (`android-shell`, `apple-common`, `macos-shell`, `qt-shell`, `skia-shell`, `web-shell`) | 0 | 0 |
| `examples/*` | 3 | 0 | 1 (`kitchen-sink`) | 2 (`nve-docs-site`, `studio-workspace`) | 0 | 0 |
| **Total** | **54** | **7** | **28** | **19** | **0** | **0** |

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
5. **Marketing Overclaims vs. Actual Code (`"Zero-JSON"` in 134 Files While `serde_json` Ships in Core & Studio)**:
   - `"zero-json"` appears in **134 files** outside `CHANGELOG.md`, yet `serde_json` is still compiled unconditionally into `packages/core/engine` (`profiler.rs:112`), `packages/core/interop` (`channel.rs:88,312`, `handoff.rs:210`), and `examples/studio-workspace` (`ui_islands.rs:360`, `lib.rs:468`), plus dead deps in `hosts/skia-shell`.
   - Real RN Fabric ShadowTree mounting (`cxx/nve-fabric-bridge/src/fabric_c_api.cpp:143-159`) is literally `old_root.clone({})` (a no-op stub), and `cxx/build-fabric` compiles with `NVE_WITH_RN_FABRIC=OFF`.

> **Workspace Hygiene Note:** An untracked **7.6 GB** OpenJ9 JVM dump (`core.20261125.032910.85615.0001.dmp` + `javacore.*.txt` + `Snap.*.trc`) is sitting in `hosts/android-shell/` and should be deleted.

---

## 1. `packages/core/*`

| Crate | LOC | Level | What Is Real | Key Flaws / Stubs / API & Perf Debt | Disposition |
| :--- | ---: | :---: | :--- | :--- | :--- |
| `packages/core/abi` | 8,438 | **L2** | `nve_get_interface(1..=5)`, `#[repr(C)]` `NveValue`/`NveDirectEvent`, `NveSceneTable`/`NveSceneView`, `offset_of!` layout asserts, schema hash cross-check, property tests. | `NveSurfaceApi::render` still serializes `NVE1` byte streams in-process; duplicate scene structs (`64B NveRenderNode` vs `128B RelocSceneNode`); legacy `NK1`/`BinValueView`/`set_props(&[u8])`; 1,075-LOC test file; cryptic magics (`NVE1`, `NVP1`, `NK1`). | **Keep & Harden API** (Wave C1 direct `NveFrameCommandBatch` + `NveDirectEvent`) |
| `packages/core/abi-macros` | 496 | **L2** | Zero-dep `#[derive(NveAbi)]` emitting `NveAbiDescriptor` + size/offset asserts. | Rejects generics (`parse.rs:39`); ignores `#[cfg(...)]` on fields (silent schema mismatch risk); no `trybuild` compile-fail tests. | **Keep & Harden API** |
| `packages/core/codegen` | 32,379 | **L2** | Single-source ABI/Scene/Wire/Reloc/Hybrid emitter generating `15/15` bindings, executing real `clang`/`clang++`/`rustc`/`swiftc`/`javac`/`dart`/`node` tests. | Silent test skips when tools absent; `kotlinc`/`tsc` never executed; heavy emitter duplication across 6 backends; `export_rn_cpp.rs:172-200` builds `std::unordered_map` on every `update_props`; 4 test files > 650 LOC (`hybrid_codegen_tests.rs` 1,999 LOC); `README.md` claims "Zero-JSON" while using `serde_json` for export specs; cryptic magics (`NVH1`, `NVT2`, `NVL1`, `NVCF`, `NVEV`). | **Refactor / Simplify** |
| `packages/core/components` | 10,860 | **L2** | `DrawList` IR, `LayoutExt` (Grid/Sticky/RTL), `Spring`/`TransitionSpec`, `Transform2D`, `InteractionTracker`, `BackgroundFill`, `SvgPicture`. | `NativePropDecoder::decode` (`native_view/codec.rs:314-347`) claims "zero-copy" but heap-allocates `Vec<(u16, NativeValueRef)>` per decode; several native-view specs (`WebElement`, `VideoPlayer`, `BlurSurface`) lack verified native host implementations; cryptic magics (`NVNP`, `NVNC`, `NVE_DRAW1`). | **Refactor / Simplify** |
| `packages/core/dioxus` | 10,059 | **L2** | `WriteMutations` (`NveMutationWriter`), `DioxusSurface`, `TypedStyle`, `ErrorBoundary`, `SignalRegistry`. | `NodeRecord` (`writer/mod.rs:32-65`) stores 30+ cold fields inline (~600B/node instead of boxing cold extras); `in_process_mode` defaults `false` and still encodes `NVE1` bytes; `render_frame` `.to_vec()` copies frame bytes; `DioxusSurface::new` only takes bare `fn() -> Element`, forcing `nve-docs` thread-local bootstrap hacks; 781-LOC test file. | **Refactor / Simplify** |
| `packages/core/engine` | 10,510 | **L2** | `NveApp` + `export_nve_app!` with `catch_unwind`, `HotAppInstance` dylib hot-swap, Unix/Win32 `MmapOffsetArena`, `RelocSceneRoot`, `FrameArena`, `xxh64`. | Unconditional `serde_json` dep (`Cargo.toml:11`) for 1 line in `profiler.rs:112`; `dispatch_events_direct` default bridges back through `EventRing` bytes; `try_dispatch_bin_props_direct` allocates per-call boxed slices; `$TMPDIR/nve-hotswap/` leaks staged dylibs on crash; 1,361-LOC test file. | **Refactor / Simplify** |
| `packages/core/host-services` | 2,991 | **L1** | `NVE_IFACE_HOST = 5` client, binary codec, `HostServiceRegistry`, `apple_objc.rs`. | **Real concurrency bug** (`ffi_client.rs:172-190`): invokes FFI before registering pending callback, so fast cross-thread completion can be overwritten and hang forever. False zero-alloc fast path (`ffi_client.rs:294` does `.to_vec()`). Unwind-unsafe `objc_autoreleasePoolPop` (`apple_objc.rs:24-29`). Dead 104-LOC `NativeHostServices` forwarding wrapper (`registry.rs:245-348`). Mock-only tests. | **Refactor / Simplify** (fix race bug immediately) |
| `packages/core/interop` | 19,263 | **L1** | Real L2 `crdt` (HLC/LWW/OR-Set/RGA), `traits`, `gesture` arbitrator. *(Legacy `sync_transport`/`sync_delta` deleted in Wave S2.)* | **L0 spec-only stubs**: `handoff.rs` (foreign `<Island>` + GPU texture specs with `serde_json::from_str` at `:210`, zero runtime mounting) and `window/manager.rs` (497-LOC pure `BTreeMap`, zero OS windows). Duplicate dylib loader (`plugin_host/sandbox.rs:111-195`). `NveChannelBus` uses `serde_json` (`channel.rs:88,312`) and clones topic/subscribers on every publish. 7 files > 650 LOC. | **Refactor / Simplify** (delete L0 `handoff.rs`, `window/manager.rs`, duplicate loader, and `serde_json`) |
| `packages/core/layout-yoga` | 1,886 | **L1** | Compiles real RN C++20 `yogacore` + `YGNodeSetMeasureFunc` + Taffy parity tests when `RN_REACT_COMMON_DIR` is set. | Silently compiles to uninhabited stub when env var unset (`lib.rs:450-529`). Drops `set_raw_layout_prop` (`flex-wrap`, `font-weight`, `align-self`), CSS Grid, gaps, borders; hardcodes `YGDirectionLTR` (`yoga_shim.cpp:457`); ignores `font_weight` in text measurement; clones `Vec<NodeId>` per node on every layout pass (`lib.rs:267`). | **Keep & Harden API** |

---

## 2. `packages/backends/*`

| Crate | LOC | Level | What Is Real | Key Flaws / Stubs / API & Perf Debt | Disposition |
| :--- | ---: | :---: | :--- | :--- | :--- |
| `packages/backends/headless` | 4,387 | **L2** | Fast Taffy Flexbox+Grid layout, `NodeSlotMap`, hit-testing (scroll/sticky/`Transform2D`), `InteractionTracker`, `prerender_native_frame0`. | Uses SipHash `HashMap` for `raw_props` and `scroll_offsets` (`lib.rs:62,102`); duplicates shadow-tree state already in `NveMutationWriter` when used in-process. | **Keep & Harden API** |
| `packages/backends/skia` | 17,691 | **L1** | CPU software rasterizer + `cosmic-text` BiDi/grapheme/IME text + PNG LRU cache + gradients/SVG (`7cfa635`) + `objc2-metal`/`ash`/`d3d12` swapchain wrappers + 2 Metal offscreen parity tests. | **Severe GPU perf bugs**: `gpu/skia_renderer.rs:308-318` copies full RGBA pixels & recreates `skia_safe::Image` on **every frame** (zero GPU texture cache); `gpu/skia_renderer.rs:77-95` scans OS `FontMgr` on **every bold `DrawText` op**; `gpu/skia_renderer.rs:251-290` uses raw `draw_str` with **no word wrap, no BiDi, no emoji/CJK fallback**; `gpu/vulkan.rs` is **1,221 LOC with 0 Vulkan tests**; `gpu/d3d12.rs` has **0 unit tests**; `DisplayList` clones `String` per text/image op every frame. | **Keep & Harden API** |
| `packages/backends/web` | 15,820 | **L2** | Real `web-sys` DOM backend, Option 5 Hybrid CSS, LIS keyed child diffing, semantic HTML5 SSR/SSG + DSD streaming + hydration replay, gradients + inline `<svg>`. | **WASM↔DOM readbacks on hot paths**: `flush_dirty_styles` (`dom.rs:244-253`) reads `class_name()` / `style().css_text()` across JS bridge; `reconcile_dom_children` (`dom.rs:278-300`) reads `get_attribute("data-nve-id")` per child instead of diffing Rust state. 2 src files > 650 LOC (`snapshot.rs` 690, `events.rs` 665). Zero in-crate `wasm_bindgen_test`s. Cryptic magics (`NVWS1`, `NVB1`). | **Keep & Harden API** |

---

## 3. `packages/framework/*`

| Crate | LOC | Level | What Is Real | Key Flaws / Stubs / API & Perf Debt | Disposition |
| :--- | ---: | :---: | :--- | :--- | :--- |
| `packages/framework/docs` | 5,653 | **L1** | Working Fumadocs-inspired shell dogfooded by `nve-docs-site`. | Thread-local FIFO bootstrap hack (`lib.rs:49,149`) with silent `fallback_empty_state()` because `DioxusSurface::new` cannot inject initial props/context. Mirror-only tests. | **Refactor / Simplify** (after fixing `DioxusSurface` root props) |
| `packages/framework/i18n` | 6,057 | **L1** | Pure-Rust BCP-47, ICU MessageFormat, CLDR plurals/numbers/dates/relative-time, pseudolocalization, `wasm32` `Intl.*` inline-JS bridge. | **0% native OS integration**: macOS/iOS (`Foundation`), Android (`android.icu`), and Windows (`icu.dll`) are completely unimplemented — all silently fall back to hardcoded 18–41 locale tables. `wasm_intl.rs:118-128` allocates `new Intl.Locale` per call. Zero `wasm32` tests. | **Keep & Harden API** |
| `packages/framework/mdx` | 6,976 | **L2** | Pure-Rust Markdown/MDX parser, YAML frontmatter, `_meta.toml` nav, Rust static codegen, search index with edge-case tests. | Docs-subset Markdown/MDX (not full CommonMark/GFM); keyword-level syntax highlighter; cryptic `NVS1` format name (`search_codec.rs:7`). | **Keep & Harden API** |
| `packages/framework/router` | 5,795 | **L2** | Typed `Routable`, stack/tab navigation, guards, query params, deep-link matching, `wasm32` `web_sys::History` + `popstate`. | Zero `wasm32` browser tests (`web_history_tests.rs` only tests `MemoryHistoryBackend`); no native OS universal-link host bridge; cryptic `src/nvn1.rs` naming. | **Keep & Harden API** |
| `packages/framework/ui` | 17,825 | **L1** | Solid headless math/algorithms (`FenwickHeightTree`, `positioning.rs`, `FormState`, `FocusTrap`, `DatePicker` calendar math). | **Fake-compliant UI wiring**: `0` hover/active/cursor/pointer/drag handlers anywhere in `src/`; `SplitPane` & `BottomSheet` cannot be dragged; `Tooltip` doesn't open on hover; `Tooltip`/`Popover`/`DropdownMenu` render inline with inert `data-x`/`data-y` attributes instead of floating; `Dialog` backdrop lacks `position: fixed; inset: 0`; 772 LOC of dead duplicate style builders (`TypedStyleBuilder` 471 + legacy `StyleBuf` 301). | **Refactor / Simplify** (rebuild component interaction & positioning primitives properly, delete duplicate `style_buf`) |

---

## 4. `packages/modules/*` (All 19 Crates)

### Special Deep Dive: `packages/modules/camera` (11,971 LOC) — **L1 (Mixed L0/L1)**

Compared honestly against **`react-native-vision-camera` (L4 Gold Standard)**, `packages/modules/camera` is nowhere near a production camera library:

- **Useful Adapter Surface (Keep & Harden)**:
  - Typed `CameraPreview` `NativeViewSpec` + 40B `#[repr(C)]` `CameraFrameWireLayout` (`src/spec.rs`, `src/frame.rs`).
  - Pluggable **VisionCamera hybrid adapter contracts** (`@protocol NveVisionCameraAppleBridge`, `interface VisionCameraSessionBridge` / `NveVisionCameraAdapter.kt`, `HybridCameraSession` in `nve_vision_camera_hybrid.hpp`) — though note these have never yet been compiled/tested against real Margelo VisionCamera v5 headers.
- **Half-Baked Standalone Platform Implementations (Delete & Postpone Full L4 Camera to `docs/BACKLOG.md`)**:
  - **Rust `NativeCamera`** (`src/host_service_handler.rs:237-263`): **L0** — returns `Vec::new()` and `Err(Unsupported)` for every action.
  - **C++ Linux (`nve_camera_linux.hpp`)**: **L0** — zero PipeWire, zero V4L2 capture/DMABUF; registers `InMemoryLinuxCameraBridge` returning fake `"PipeWire Integrated Webcam"` and fake `/tmp/nve_linux_capture_1.jpg`.
  - **C++ Windows (`nve_camera_windows.hpp`)**: **L0** — zero Media Foundation streaming/DXGI; registers `InMemoryWindowsMediaCaptureBridge` returning fake `"Surface Studio Camera Front"`.
  - **Apple standalone (`NveAppleCamera.mm`)**: **L1 with hardware bugs** — raw `CVPixelBuffer` pointer use-after-free (`:452`, missing `CVPixelBufferRetain`), hardcoded `rotationDeg:0u` (`:477`), main-thread vs `_sessionQueue` config race (`:356-384`), zero `AVCaptureDeviceFormat` negotiation (no 4K/HDR/high-FPS/ultra-wide/telephoto/multi-cam), ignores photo `quality`, uses `AVCaptureMovieFileOutput` (no `AVAssetWriter`/pause/resume), discards errors, zero interruption observers.
  - **Android standalone (`NveAndroidCameraXBinding.kt`)**: **L1 with hardware bugs** — silently falls back to `InMemoryCameraXHostBinding.DEFAULT_DEVICES` (`:92-94`), passes `System.identityHashCode(hb)` after closing `HardwareBuffer` (`:364-375`), returns `NveCapturedPhoto` before file write completes (`:150-181`), unconditionally binds 4 simultaneous CameraX use cases (crashes LEGACY/LIMITED devices, `:287-296`), never run on real Android hardware.
  - **Web standalone (`web/src/nve_web_camera.ts`)**: **L1** — calls real `getUserMedia`/`MediaRecorder` but only tested against Node mocks (never real browser E2E, no `ImageCapture`/`requestVideoFrameCallback`).
  - **Manifest Overclaim**: `react-native/package.json:4` claims `"Zero-JSON cross-platform camera module for React Native, React Native Windows (0.88), React Native macOS, React Native Web, and NVE"` while shipping `InMemoryWindowsMediaCaptureBridge` on Windows.

### Remaining 18 Module Crates

| Crate | LOC | Level | Reality Summary | Disposition |
| :--- | ---: | :---: | :--- | :--- |
| `packages/modules/audio` | 1,042 | **L0** | `NativeAudio` returns `Unsupported` on all 10 methods. Zero OS audio code, zero consumers. | **Delete Slop → Backlog** |
| `packages/modules/fonts` | 893 | **L0** | Only puts magic-checked metadata in an in-memory map. Never registers with CoreText/DirectWrite/Skia/Web. Zero consumers. | **Delete Slop → Backlog** |
| `packages/modules/location` | 677 | **L0** | `NativeLocation` returns `Unsupported` on all 4 methods; `MemoryLocation` returns hardcoded SF coords. Zero consumers. | **Delete Slop → Backlog** |
| `packages/modules/notifications` | 511 | **L0** | `HostServiceHandler::handle` hardcoded `Unsupported`; `show()` always `Unsupported` even on macOS. Yet registered in `skia-shell`! | **Delete Slop → Backlog** |
| `packages/modules/sensors` | 609 | **L0** | `NativeSensors` returns `false` / `Unsupported` for everything. Zero OS sensor APIs, zero consumers. | **Delete Slop → Backlog** |
| `packages/modules/share` | 452 | **L0** | `NativeShare` returns `Unsupported` on all 3 methods. Zero OS share APIs, zero consumers. | **Delete Slop → Backlog** |
| `packages/modules/web-auth` | 566 | **L0** | `NativeWebAuth` returns `Unsupported` on both methods. Zero `ASWebAuthenticationSession`/CustomTabs/OAuth, zero consumers. | **Delete Slop → Backlog** |
| `packages/modules/pair` | 2,122 | **L1** | Unlinked pairing state machine with zero sockets and zero callers *(already deleted in Wave S2 `t3/wave-s2-wire-quic-naming-timeline`)*. | **Deleted in Wave S2** |
| `packages/modules/clipboard` | 531 | **L1** | Real macOS `NSPasteboard` + Win32 `CF_UNICODETEXT`; Linux/iOS/Android/Web `Unsupported`; plain text only; duplicated in `skia-shell/src/window/clipboard.rs`. | **Keep & Harden** |
| `packages/modules/filesystem` | 754 | **L1** | Real Win32 `SHGetKnownFolderPath` + `$HOME`/`XDG_*` paths; **`pick_files` and `save_file` return `Unsupported` on every OS**. | **Keep & Harden** (trim picker claims or build real dialogs) |
| `packages/modules/haptics` | 406 | **L1** | macOS Force Touch only; **iOS, Android, Windows, Linux, Web all `Unsupported`**. | **Delete / Defer → Backlog** |
| `packages/modules/linking` | 552 | **L1** | Outbound `NSWorkspace.openURL` / `ShellExecuteW` / `xdg-open` only; `initial_url()` hardcoded `Ok(None)` everywhere; zero inbound deep links. | **Keep & Harden** |
| `packages/modules/secure-store` | 755 | **L1** | Real macOS Keychain + Win32 CredMan (no 2560B check, mock-only tests); Linux/iOS/Android/Web `Unsupported`. | **Keep & Harden** |
| `packages/modules/system-env` | 1,079 | **L1** | Real macOS appearance/locale + Win32 theme/locale, but hardcodes accent/scale/24h/contrast fields; **WASM returns `Default::default()`** (no `matchMedia`/`Intl`). | **Keep & Harden** |
| `packages/modules/storage` | 1,670 | **L1** | Crash-safe `NVK1` WAL disk KV engine is solid L2 code, but **zero hosts register it**, zero examples use it, WASM is volatile in-memory, uses cryptic `NVK1` name. | **Keep & Harden** |
| `packages/modules/native-views` | 5,161 | **L1** | Apple `NveAppleNativeViews.mm` has real `NSVisualEffectView`/`WKWebView`/`AVPlayer`; Win32 has `DwmSetWindowAttribute` blur; Flutter has `BackdropFilter`. **Windows/GTK/Qt/Flutter WebView & VideoPlayer are counter-only `nullptr` stubs**; GTK/Qt blur blurs self, not backdrop. | **Keep Apple/Adapter, Delete Stubs** |
| `packages/modules/nitro-bridge` | 2,350 | **L1** | Clean standalone C++20 `HybridObject`/JNI layer, but has **zero dependency on Margelo `react-native-nitro-modules`**; only linked by `camera`. | **Defer / Fold Into RN Adapter** |
| `packages/modules/tv` | 1,780 | **L1** | Real pure-Rust 2D spatial focus & overscan math + Kotlin key bridge, but **zero workspace consumers** and zero Android TV / Apple TV tests (`swiftui-kit` duplicates overscan). | **Defer → Backlog** |

---

## 5. `packages/tools/*` & `cxx/nve-fabric-bridge`

| Target | LOC | Level | Reality Summary | Disposition |
| :--- | ---: | :---: | :--- | :--- |
| `packages/tools/bench` | 4,341 | **L2** | Real Criterion suites (`abi`, `dioxus`, `backends`, `interop`, `arena_and_mmap`) + WASM/Brotli budget checker. Redundant hand-rolled JSON codec in `baseline_codec.rs`; zero GPU Skia or real browser DOM benchmarks. | **Keep & Harden API** |
| `packages/tools/cli` | 7,526 | **L1** | Working `codegen`, `dev`, `pair`, `docs`, `prerender`, `build`, `bench`, `doctor`. Hardcodes 5 internal example/shell crates in `Cargo.toml` (cannot prerender external apps); `main.rs:172` docs mismatch `prerender.rs`; `dev.rs` hardcodes `target/debug` and `./hosts/web-shell/build.sh` (broken on Windows). | **Refactor / Simplify** |
| `cxx/nve-fabric-bridge` | 22,416 | **L1** | Real C++20 RN Yoga (`NVE_WITH_RN_YOGA=1`) + RN JSI (`NVE_WITH_RN_JSI=1`) + standalone scene publisher + `NVERELOC` mmap + CTest/fuzz. **CRITICAL**: `commit_rn_shadow_tree` (`src/fabric_c_api.cpp:143-159`) is a **no-op stub** (`old_root.clone({})`), `cxx/build-fabric` compiles with `NVE_WITH_RN_FABRIC=OFF`, `update_props_json` (`nve_rn_component.cpp:206`) is broken, **11 files exceed 650 LOC** (`nve_snapshot.hpp` 1,358 LOC hand-duplicates `nve-codegen`). | **Refactor / Simplify** |

---

## 6. `hosts/*` (All 12) & `examples/*` (All 3)

| Target | LOC | Level | What Is Real | Key Broken / Missing / Stubbed Items | Disposition |
| :--- | ---: | :---: | :--- | :--- | :--- |
| `hosts/skia-shell` | 9,337 | **L2** | `winit` multi-window, GPU Skia (opt-in) + `softbuffer` CPU, full IME/text editing, hot reload, Frame-0 instant boot, **only** host binding `NVE_IFACE_HOST = 5`. | **Zero accessibility** (no AccessKit); duplicates `nve-clipboard` (`window/clipboard.rs`); registers L0 `NativeNotifications` & stub pickers; dead `serde_json` dep; side-channel styles (`:hover`, transitions, gradients, SVG) don't cross dylib FFI. | **Keep & Harden** |
| `hosts/web-shell` | 5,835 | **L2** | `wasm32` `talc`, real browser DOM/Canvas2D/ARIA, SSR hydration + replay, `localStorage` snapshot boot, real headless Chrome CDP E2E. | **Zero `NVE_IFACE_HOST = 5`** (no `navigator.clipboard`/`matchMedia`/`Intl`); hardcodes demo apps; `wasm_api.rs` 736 LOC (> 650); doesn't yet sync newly added gradient/SVG side-channels. | **Keep & Harden** |
| `hosts/apple-common` | 3,778 | **L2** | Shared AppKit/UIKit reconciler, CoreGraphics `NVE_DRAW1`, LRU image cache, accessibility + IME guards, Frame-0 replay. | **Zero `NVE_IFACE_HOST = 5`**; single-line `NSTextField`/`UITextField` only; no hover/transition/gradient/SVG across FFI. | **Keep & Harden** |
| `hosts/macos-shell` | 4,145 | **L2** | CMake `-Werror` `NSWindow`/`NSView` host, multi-window, Frame-0 instant boot + state restore, real dylib CTest. | **Zero `NVE_IFACE_HOST = 5`**; RN Fabric `RCTViewComponentView` path untested in `check-all.sh`; no global keyboard shortcuts outside text fields. | **Keep & Harden** |
| `hosts/qt-shell` | 9,905 | **L2** | Real Qt 6 Widgets & Qt Quick `QSGNode`, IME preedit/commit, `QAccessible`, multi-window, Frame-0 boot, real dylib CTest. | **Zero `NVE_IFACE_HOST = 5`**; single-line `QLineEdit` only; linked `nve_qt_native_views` has `nullptr` WebView/Video stubs. | **Keep & Harden** |
| `hosts/android-shell` | 6,587 | **L2** | Real C++20 JNI bridge + Android Java Activity (`View`/`EditText`/`ScrollView`/`Canvas`, IME guard, TalkBack, `DirectByteBuffer`). | **Zero `NVE_IFACE_HOST = 5`**; zero Frame-0 instant boot; `NveMainActivity.java` is **1,378 LOC**; fails to export 15/21 Compose JNI symbols; desktop tests use fake `jni_test_recorder`; **7.6 GB untracked JVM core dump** in dir. | **Keep & Harden** |
| `hosts/ios-shell` | 801 | **L1** | Real `UIWindowScene`/`UIView`/`UITextField` code + Frame-0 replay + ad-hoc `run-simulator.sh`. | **No CMake/Xcode/SwiftPM/podspec, zero automated iOS tests** (`check-all.sh` only runs `-fsyntax-only`), zero `NVE_IFACE_HOST`, no keyboard/safe-area wiring. | **Keep & Harden** |
| `hosts/gtk-shell` | 5,319 | **L1** | Real GTK4 widgets + Cairo `NVE_DRAW1` + `gtk_accessible` + multi-window + Frame-0 boot. | **Zero IME preedit guard**, **zero keyboard event controller**, no multiline `GtkTextView`, **zero `NVE_IFACE_HOST`**, fake self-blur + `nullptr` WebView/Video. | **Keep & Harden** |
| `hosts/windows-shell` | 5,835 | **L1** | Real Win32 `HWND` + Direct2D/DirectWrite/WIC + click/wheel + DPI + Frame-0 + RNW `ComponentView` (937 LOC). | **Zero keyboard input** (`WM_KEYDOWN`/`WM_CHAR`), **zero IME**, **zero interactive `TextInput`**, **zero hover/drag/pointer-down**, **zero UIA accessibility**, **zero `NVE_IFACE_HOST`**. | **Keep & Harden** |
| `hosts/compose-kit` | 6,878 | **L1** | Clean Jetpack Compose `Canvas`/`BasicTextField`/`Modifier.semantics` + `DirectByteBuffer` reader. | **Crashes on real Android (`UnsatisfiedLinkError`)**: declares 21 `external fun`s in `NveJniBridge`, but `android-shell` only exports 6! 100% tested against `FakeNveBridgeBackend`. | **Keep & Harden** |
| `hosts/swiftui-kit` | 10,562 | **L1** | Direct Swift C-ABI FFI + `NveSceneCanvasView` SwiftUI `Canvas` + VoiceOver + `WKWebView` + `swift test`. | **`NveSurfaceRepresentable` renders an empty `UIView`/`NSView` (`NveHostContainerView`)**; **zero interactive `TextInput`/keyboard/IME**; `NveSwiftUIVideoPlayerView` is `Color.black`; `NveHostApi` resolved but never bound. | **Keep & Harden** |
| `hosts/flutter-plugin` | 14,833 | **L1** | Direct `dart:ffi` + `NveCanvasHost` custom `RenderBox` + Flutter `SemanticsNode` + `flutter test`. | **Phantom `NveFlutterPlugin`**: `pubspec.yaml` declares native plugins for 5 OSes, **none of those directories exist**; `NveTextureHostWidget` & `NvePlatformViewWidget` throw `MissingPluginException`; **zero interactive `TextInput`/keyboard/IME**; 3 files > 650 LOC. | **Keep & Harden** |
| `examples/kitchen-sink` | 3,400 | **L1** | Real multi-section Dioxus app replacing `examples/counter`, 0 `serde_json`, SSG + WASM hydration passing. | Exposes the `nve-ui` defects above: ~80% of showcased controls lack hover/cursor/drag/floating positioning/real scroll sync; lists L0 `nve-audio` in a combobox option. | **Keep & Harden** (turns into real interaction testbed) |
| `examples/nve-docs-site` | 5,648 | **L2** | Full dogfooded docs site + landing page, SSG + WASM + native `cdylib`, 0 `serde_json`, live Chrome CDP E2E. | Hardcodes `MemoryClipboard` + `MemorySystemEnv` (`src/lib.rs:80-83`) — "Copy" buttons don't copy to OS/browser clipboard. | **Keep & Harden** |
| `examples/studio-workspace` | 17,604 | **L2** | Flagship multi-view app, binary `NWS1` state + native disk persistence, real TCP CRDT tests, dual-mode `NVE_IFACE_HOST`. | **Compiles `serde_json` into production WASM & `cdylib`** (`ui_islands.rs:360`, `lib.rs:468`); live UI CRDT button uses in-memory mesh; WASM persistence is no-op. | **Keep & Harden** |

---

## 7. Prioritized Reset & Single-Track Roadmap

Stop fanning out 3–4 parallel feature waves. Execute **one vertical slice at a time at full depth**, proving every change with real interaction tests (real mouse hover computed styles, real pointer drags, real screen coordinates, real native/browser execution):

1. **Cleanup & Truthfulness Pass (Immediate)**
   - Delete the **7 pure-L0 module crates** (`audio`, `fonts`, `location`, `notifications`, `sensors`, `share`, `web-auth`), delete standalone camera stubs (`InMemoryLinuxCameraBridge`, `InMemoryWindowsMediaCaptureBridge`, `NativeCamera`, broken standalone Apple/Android capture — keeping the **VisionCamera adapter** + `CameraPreview` spec), delete fake counter-only WebView/Video stubs in `native-views`, delete L0 `handoff.rs` / `window/manager.rs` in `interop`, delete phantom Flutter plugin declarations, and remove the 7.6 GB JVM dump in `hosts/android-shell/`.
   - Log **L4 Gold-Standard Camera (VisionCamera-grade)** and deferred L0 modules honestly in `docs/BACKLOG.md`.
   - Strip `"Zero-JSON"` marketing copy across the repo and actually remove `serde_json` from `nve-engine` default features, `nve-interop`, `studio-workspace`, and `skia-shell`.
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
