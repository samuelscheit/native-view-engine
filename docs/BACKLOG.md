# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md); full clean-slate capability coverage lives in [docs/ui-maturity-audit.md](./ui-maturity-audit.md).

---

## P1 — Direct Typed Render ABI, Interaction Primitives & Host Parity (from [quality-audit.md](./quality-audit.md))

- [ ] **Track 1B-1e — Codegen Modernization (Askama 0.16 Templates + Static Native Packages)**
  - **Delete dead legacy codegen & duplicate parsers (~6,776 LOC removed)**: delete `schema/wire.rs` + 5 `*/wire.rs` emitters (`2,919` L: FNV, old EventRing, hand-written `NVE_DRAW1` constants — generate `DrawList` decoders from `nve-codec` descriptors via one generic codec template per language), delete `reloc` codegen (`schema/reloc` + `rust` + `c_cpp`, `2,280` L; only used by a bench and tests), and delete `frontend/hybrid_rust` (`1,577` L duplicate hand-written lexer), keeping `schema_bridge.rs` as the single `NveAbi` Rust frontend.
  - **Extract ~5,000 LOC of raw `r#"` runtime strings into checked-in static native packages**: move unchanging runtime preludes (`NVE_API` macros, XXH64 implementations, thin `NveEventRing` readers) out of escaped Rust `writeln!` blocks into checked-in `packages/core/host-{cxx,swift,java,dart,ts}/` packages tested by their own native toolchains (only XXH64 test vectors stay generated).
  - **Migrate schema-derived multi-language emitters to Askama 0.16 (`escape = "none"`)**: configure `escape = "none"` in `packages/core/codegen/askama.toml` and on every `#[template]`; borrow `&'a AbiFileDescriptor` and `&'a [&'static TableDesc]` directly for compile-time template verification across `core_abi` + `scene_abi` (merged into one ABI template per language, dropping hand-written `scene_abi_spec`), `hybrid`, `codec`, and framework wrappers (`export_rn_*` moving into `packages/modules/react-native-fabric` in Track 1B-2).

- [ ] **Track 1B-2 — Host-Owned `packages/core/runtime` (Model A + 5→2 Layout Unification, ADR-003/ADR-004)**
  - Keep `nve-engine` lean/wasm32 app-side SDK; build host-owned **`packages/core/runtime` (`libnve_runtime`)** moving host-side `TaffyStoreLayout` (default), `loader.rs`, `hotreload.rs`, `interaction.rs`/`hit_test.rs`/`scroll.rs`/`focus.rs`, generic frame-tick hook for `nve-motion-skia`, LRU-cached OS `TextMeasurer` C callback with batch mode, Model A `measure_surface` (baselines/dry-run/invalidation callback), zero-copy `ScenePublisher` (pointing `NveSceneView::{text_slab,children_slab}` straight at `NodeStore`'s slabs, `[u32; 4]` `TableSet` for 32-bit C ABI, per-surface lock + `scene_acquire`/`scene_release` for RN Fabric & Qt Quick multi-threading, deleting fat 64 B `NveRenderNode` duplication), and exposing `DioxusDesktopApp` as a shared library crate (moving it out of `#[cfg(test)]`-only binary `hosts/skia-shell`).
  - **Move `packages/core/layout-yoga` → optional `packages/modules/layout-yoga` (A3)** (zero C++ in `packages/core/*`, keeps opt-in RN parity).
  - **Delete `RelocSceneNode` + `mmap_arena` + `nve_snapshot.hpp` (−7.8k L, A4=4a)**: boot Frame 0 from an `NVE2` keyframe + Taffy layout (< 2 ms 1k-node boot gate, zero viewport/DPI/theme staleness, 100% table parity) and persist app state (`studio-workspace`) via plain atomic-rename `nve-codec` file.
  - **Bundle `nve-wire` behind default-on `feature = "wire"` in `libnve_runtime` (A5)**: expose `nve_surface_pair_{connect,disconnect,status}` backed by Rust `nve-wire` (QUIC + WebTransport + WSS + reconnect writing `NVE2` straight into `NodeStore`; release embeds pass `--no-default-features` to strip Tokio/Quinn/Rustls; `web-shell` keeps `nve-wire-core` + browser WS/WT) and **delete ~2,000 LOC of duplicated native pair clients** (`pair_client.{cpp,hpp}`, `pair_wire_v2.cpp`, `NveAndroidPairJni.cpp`, `NvePairClient.swift`, `nve_pair_client.dart`).
  - Bind all native hosts (`swiftui-kit`, `flutter-plugin`, `apple-common`, `android-shell`/`compose-kit`, `gtk-shell`, `qt-shell`, `windows-shell`) to `libnve_runtime`, delete ~12k–24k LOC of duplicated C++/Dart layout, snapshot, pair-client, and decoder code (`flexbox_solver.cpp`, `yoga_layout.cpp`, `shadow_tree.cpp`, `scene_publisher.cpp`, `fabric_c_api.cpp`, `command_{en,de}coder.hpp`, `nve_snapshot.hpp`, `flex_layout.dart`, `command_decoder.dart`), consolidate C++ helpers in header-only `packages/core/host-cxx`, and extract optional Model A RN Fabric `ComponentView` adapters into `packages/modules/react-native-fabric`.

- [ ] **Track 2 — Real `nve-ui` Interaction Primitives, Floating Overlays, Typed Tier-1 Hot Reload & Behavioral E2E**
  - Fix live `requested_scroll_x` lock-in bug (`skia/src/scroll.rs:115`) and wire full `Pointer*` events (`onpointerdown`/`onpointermove`/`onpointerup` + declarative synchronous `Interaction::capture_mode = OnPrimaryDown`), ancestor hover chain, `CursorKind`, CSS Top Layer (`showModal()`/`popover=auto`) + CSS Anchor Positioning with `getBoundingClientRect()` & shared `compute_overlay_position` fallback, and live controlled scroll sync across `nve-dioxus`, `nve-backend-web`, and `nve-backend-skia`.
  - **Delete `style_buf/` + all 62 `nve-ui` `style:` props, ban author-facing `style: "..."` strings completely (zero runtime CSS parser), AND upgrade Tier-1 hot reload (A6=6a)**: extend `packages/tools/cli/src/rsx_diff` + `HotPatchMsg::TemplatePatch` to diff typed RSX literal/enum/color attributes into typed **`RowPatch` word updates** applied straight to `NodeStore` (< 10 ms zero-recompile style hot reload on physical iOS/Web/Desktop with zero CSS parser).
  - Wire real `:hover`/`:active`/`:focus-visible`, pointer dragging (`SplitPane`, `BottomSheet`), hover/anchored floating overlays (`Tooltip`, `Popover`, `DropdownMenu`, `Dialog`), and real scroll sync (`VirtualList`) across `nve-ui` and `examples/kitchen-sink`, verified with live browser/window behavioral assertions & desktop/mobile screenshot critique.

- [ ] **Track 3A — Hardware Skia Retained Compositor/Caches, `skia-shell` AccessKit & `web-shell` `NVE_IFACE_HOST`**
  - **Skia inset `box-shadow`**: `PaintOp::DrawShadow` only rasterizes outer shadows, so `BoxShadow::inset` is skipped on Skia (web renders it natively); add an inset shadow op (clip to the rounded rect, invert the blurred mask) on CPU and GPU paths with pixel tests against the web output.
  - Fix Skia full display-list rebuild on every commit & motion tick (retained per-node display-list / compositor-layer tree for 120 Hz opacity/transform updates) plus GPU Skia per-frame image/font churn (`gpu/skia_renderer.rs`: GPU texture cache, bold `Typeface` cache, `CosmicTextEngine` glyph-run shaping) and add real Vulkan/D3D12 parity tests.
  - Wire `skia-shell` AccessKit accessibility tree sync and bind `NVE_IFACE_HOST = 5` in `hosts/web-shell`.

- [ ] **Track 3B — Native Host Linkage & OS Parity (Post-Track 1B-1e / 1B-2)**
  - Fix `compose-kit` 15 missing JNI exports in `hosts/android-shell`, `swiftui-kit` empty `NveSurfaceRepresentable` + `TextInput`/IME, `windows-shell` keyboard/IME/UIA, and bind `NVE_IFACE_HOST = 5` across native hosts (Apple, Android, Qt, GTK, Windows) directly against `libnve_runtime` (zero throwaway C++/JNI/Swift rewrites).

- [ ] **Adopt `FrameArena` on Hot `nve-dioxus` / `nve-engine` Per-Frame Allocation Paths**
  - Wire `FrameArena` bump-reset allocation into hot `nve-dioxus`/`nve-engine` per-frame paths (`XXH64` and `FrameArena` landed).

- [ ] **TV, Watch & Spatial Platform Backends — Phase 3 (Web TV D-Pad Remaining)**
  - Phase 1 (`DeviceFormFactor` traits, `SpatialFocusEngine`, Android TV D-Pad), Phase 2 (standalone `packages/modules/tv` + real `tvOS`/`watchOS`/`visionOS`/`iOS` SDK builds + static `NveInterfaceResolver`), and Phase 3 CLI (`nve build --target tvos/visionos/watchos` + `.xcframework` recipes) are landed; remaining work is Web TV D-Pad key handling in `nve-backend-web`.

---

## P1 — UI & OS Capability Maturity (from [ui-maturity-audit.md](./ui-maturity-audit.md))

- [ ] **Mobile Host Autofill/Keyboard Wiring & Mobile Selection Handles**
  - Subtree `ErrorBoundary`, `AutofillHint`/`InputMode` + Web autofill sync, `KeyboardInsets` + Web `visualViewport` + `KeyboardAvoidingView` (`38b351c`), UAX #29 grapheme-safe editing/selection, unified `CompositionStart/Update/End` + live caret screen-rect IME feedback, and Skia BiDi run/caret/selection mapping (`be5e394`) are landed; remaining work is native Apple/Android host prop wiring for `AutofillHint`/`KeyboardInsets` and mobile touch selection handles/loupes.

- [ ] **Desktop OS Polish, Live Layout Inspector & Native AccessKit Parity**
  - Locale-aware `DatePicker`/`DateRangePicker` (`c69cdd0`) and typed gradients/cached SVG (`7cfa635`) are landed; remaining work is OS drag-and-drop (internal + file drop), native menu bar / context menus / system tray, live visual layout/recomposition overlay inspector, and full native AccessKit/UIA/AT-SPI accessibility tree sync.

- [ ] **Deferred OS Capability Modules (Deleted Pure-L0 Stubs)**
  - The 7 pure-L0 stub crates (`audio`, `fonts`, `location`, `notifications`, `sensors`, `share`, `web-auth`) were deleted in Step 1 Part B because they had zero real OS implementations. Re-introduce each module only when backed by real OS APIs (CoreAudio/WASAPI/AAudio, CoreText/DirectWrite/Fontconfig, CoreLocation/FusedLocation, UNUserNotificationCenter/WinRT Toast/NotificationManagerCompat, CoreMotion/SensorManager, UIActivityViewController/Android ShareSheet, ASWebAuthenticationSession/CustomTabs) and verified on real targets.

- [ ] **Native `WebView` & `VideoPlayer` Adapters for Windows / GTK / Qt / Flutter**
  - Apple `packages/modules/native-views/apple` ships real `NSVisualEffectView`/`WKWebView`/`AVPlayer`, Windows ships real DWM blur, and Flutter ships real `BackdropFilter`. Counter-only `nullptr` WebView/VideoPlayer stubs and self-blur hacks were removed from Windows/GTK/Qt/Flutter; implement real WebView2/MediaFoundation, WebKitGTK/GStreamer, QtWebEngine/QtMultimedia, and platform-view adapters when needed.

- [ ] **Foreign Framework `<Island>` Compositor Runtime**
  - Spec-only `packages/core/interop/src/handoff.rs` was deleted in Step 1 Part B because zero runtime mounting existed. Design and build real native view-hierarchy and GPU texture handoff (`IOSurface`/`AHardwareBuffer`/`DXGI`/`DMA-BUF`) only when an end-to-end host compositor requires it.

---

## Postponed / Long-Horizon

- [ ] **L4 Gold-Standard Standalone Cross-Platform Camera (`react-native-vision-camera` Bar)**
  - `packages/modules/camera` intentionally retains only the typed `CameraPreview` `NativeViewSpec`, 40B `#[repr(C)]` `CameraFrameWireLayout`, and pluggable VisionCamera v5 hybrid adapter contracts (`@protocol NveVisionCameraAppleBridge`, `NveVisionCameraAdapter.kt`, `HybridCameraSession`). All incomplete standalone platform stubs (`NativeCamera`, `InMemoryLinuxCameraBridge`, `InMemoryWindowsMediaCaptureBridge`, standalone `AVCaptureSession`/`CameraX`/`getUserMedia` wrappers) were removed.
  - Shipping a true L4 standalone cross-platform camera requires a dedicated multi-milestone effort covering:
    - Real hardware device/lens discovery (wide, ultra-wide, telephoto, logical multi-cam, continuity/external cameras) and explicit format negotiation (4K, HDR10/Dolby Vision, 60/120/240fps, aspect ratios, stabilization modes).
    - Lifetime-safe zero-copy GPU frame processors (`CVPixelBufferRetain`/`Release`, `AHardwareBuffer_acquire`/`release`, `ID3D11Texture2D`, PipeWire `DMA-BUF`, Web `requestVideoFrameCallback`) with synchronous worklet/callback backpressure.
    - Production photo capture (RAW/HEIF/JPEG, flash, HDR, quality/speed prioritization, EXIF/orientation) and hardware video+audio muxing (`AVAssetWriter`, `MediaCodec`/`MediaMuxer`, Media Foundation Sink Writer, GStreamer/PipeWire) with pause/resume and disk-full recovery.
    - Full camera controls (tap-to-focus/exposure metering, ISO/shutter/EV, optical/digital zoom ramps, torch, OIS/EIS, sensor/device rotation handling) and OS lifecycle hardening (backgrounding, phone call / audio-session interruptions, thermal throttling, permissions).
    - Real physical device QA matrix across iOS, Android (LEGACY/LIMITED/FULL/LEVEL_3), macOS, Windows, Linux, and Web browsers.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Fullstack Server↔Client DX — Slices 1c, 2 & 3 (Streaming SSR, Edge Adapters & `nve-wire` Multiplexed RPC)**
  - **Slice 1c — Streaming SSR Hydration & Out-of-Order `<Suspense>`**: Bridge `DirectTransport` SSR loader execution into initial HTML + compact binary hydration payloads (`<script type="application/octet-stream">` / base64 CBOR) and stream deferred boundaries via `StreamingSsrSession` (`packages/backends/web/src/streaming_live.rs`) with `mismatches=0` hydration parity.
  - **Slice 2 — Edge/Serverless Adapters & `nve-wire` QUIC/WebTransport Multiplexing**: Add thin `nve-http-wasip2` (`wasi:http/incoming-handler` for Cloudflare/Fastly/Spin) adapter over sans-I/O `nve-server`, plus persistent QUIC/WebTransport RPC transport riding `nve-wire` alongside live `NVE2` frames and server-push tag invalidation streams.
  - **Slice 3 — Multi-Language Client Codegen & Ergonomic Macros (Demand-Driven)**: Add optional `#[nve::service]` / `#[nve::query]` sugar only where real app usage proves clear ergonomic wins over `procedures!`, and emit typed Swift/Kotlin/Dart/TypeScript CBOR RPC clients from shared procedure descriptors if polyglot hosts need direct server calls.

- [ ] **Unified Server-Driven UI & Multiplayer Collaboration (`nve-sync`: `Room` + `Presence` + `use_synced_state`)**
  - Unify `ZeroJsLiveSession` (HTML Declarative Shadow DOM `<slot>` streaming in `packages/backends/web/src/streaming_live.rs`) and binary wire SDUI (`NveWireFrame` v2 / `NVE2`) as **two encoders over one shared server-driven UI session model** (eliminating parallel remote-placement stacks).
  - Ergonomic `use_synced_state` / `use_synced_list` / `use_presence` hooks bridging server-authoritative thin-client rooms (0-JS Web & Native SDUI) and offline-first local CRDT peers (`packages/core/interop/src/crdt`) in the same live collaborative room, with ephemeral 20–30Hz cursors/selections/online presence.

- [ ] **Self-Documenting Protocol & Format Naming Cleanup**
  - Wave S2 replaced `NVT1`/`NVD1`/`NVC1` with `NveWireFrame`/`NveCrdtDelta`/`NveSceneArchive`/`NveTimelineArchive`; finish removing stale `NVT1`/`NVD1` comments/log strings (e.g., `hosts/web-shell/src/{wasm_pair,lib}.rs`, `packages/tools/cli/src/{dev,main}.rs`) and replacing remaining cryptic 4-letter format acronyms (`NVERELOC`, `NVWS1`, `NVB1`, `NWS1`, `NVE_MET1`, `NVE1`, `NVP1`, `NK1`, `NVNP`/`NVNC`) across code, wire headers, and docs with clear self-documenting names.

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Core modules (`nve-ui`, `nve-router`, `nve-host-services` + `nve-clipboard` + `nve-system-env`, Environment `TraitKey`s, live `NveCrdtDelta` peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) and live `/examples/studio/` GitHub Pages deploy are landed; next step is wiring live OS host services + opt-in `<NativeView<CameraPreview>>` / `BlurSurface` in Studio.
