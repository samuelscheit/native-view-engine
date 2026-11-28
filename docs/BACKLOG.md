# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md); full clean-slate capability coverage lives in [docs/ui-maturity-audit.md](./ui-maturity-audit.md).

---

## P1 — Direct Typed Render ABI, Interaction Primitives & Host Parity (from [quality-audit.md](./quality-audit.md))

- [ ] **`DrawList` codegen follow-up**: generate `DrawList` decoders from `nve-codec` descriptors via one generic codec template per language (Askama 0.16, next to the static `packages/core/host-*` runtimes), replacing the deleted hand-written wire emitters.

- [ ] **Track 1B-2 — Host-Owned `packages/core/runtime` Follow-Ups (ADR-003/ADR-004/ADR-008)**
  - Expose `DioxusDesktopApp` as a shared library crate (move it out of the binary-only `hosts/skia-shell`), plus the generic frame-tick hook for `nve-motion-skia` and Model A `measure_surface` (baselines / dry-run / invalidation callback) on `NveRuntimeApi`.
  - **Bundle `nve-wire` behind default-on `feature = "wire"` in `libnve_runtime` (A5)**: back `pair_connect` / `pair_disconnect` with Rust `nve-wire` (QUIC + WebTransport + WSS + reconnect writing `NVE2` straight into `NodeStore`; release embeds pass `--no-default-features` to strip Tokio/Quinn/Rustls; `web-shell` keeps `nve-wire-core` + browser WS/WT) and delete `hosts/skia-shell/src/window/pair_client.rs`.
  - **Apple hosts follow-ups**: runtime-owned persisted instant boot (`save_frame0` / `boot_frame0` slots over Rust `Frame0File` + `StateFile`, so `macos-shell` / `ios-shell` / `swiftui-kit` need no host-side snapshot code), canvas draw-list rendering, `NVE_ENABLE_NATIVE_VIEWS` / `NVE_ENABLE_CAMERA` adapters over `packages/modules/native-views` + `packages/modules/camera`, and `--pair-transport` selection.
  - **Runtime ABI follow-ups (Android/Compose)**: (a) bind `set_props` / `drain_emitted_events` / `snapshot_state` / `apply_hot_patch` through `NativeRuntime` so `compose-kit` regains component props and `onEvent`; (c) Android emulator / Compose instrumentation runs in CI (today: desktop-JVM JNI suites, Robolectric, NDK build); (d) pack `NveSceneView::scroll_offsets` into the Java frame so native `ScrollView` / Compose scroll states follow app-driven (programmatic) scrolls; (e) `cxx/cmake/NveRuntime.cmake` should derive `CC_<triple>` / `AR_<triple>` / the Cargo linker from the CMake toolchain when `NVE_RUNTIME_CARGO_TARGET` is set (today `run-emulator.sh` exports them; a bare NDK CMake build fails in `ring`).
  - **RN export**: `export_rn`'s Android `ViewManager.kt` still targets a nonexistent `dev.nve.host.NveNativeBridge` / `dev.nve.compose.NveBinValue`; rebind it onto `compose-kit`'s `NveSurfaceController` (`libnve_runtime`) like the iOS `.mm` (`RCTNveComponentView`) and Windows `.hpp` (`NveWinHost`), with a compile test.
  - Dart / TS wasm32 `SceneReader`s read the row-index layout in tests only: wire them to a wasm32 `libnve_runtime` publisher once one exists (the browser shell lays out in the DOM and publishes no scene).

- [ ] **Option A — Multi-Run Inline Attributed `text {}` (converged hybrid design)**
  - **Authoring & store**: `text { "Hello " b { "bold" } a { href, "link" } }` keeps reactive inline children as ordinary `NodeStore` nodes marked `NVE_NODE_ROW_FLAG_INLINE` (no layout box, no host view) so signals, events, and hot patches address each run by `NodeId`.
  - **Single producer, compiled runs**: the owning paragraph compiles its inline subtree into one contiguous slab of 48-byte POD `NveTextRun` records (`text_off`, `text_len`, `node`, resolved typography words, decoration / link flags) — one producer, zero per-run heap, republished only when a run is dirty.
  - **Unboxed inline nodes + 1-hop dirty lift**: inline nodes get no rect and no `dirty_indices` entry of their own; a content change on an inline node lifts exactly one hop to its paragraph, so hosts restyle one paragraph instead of walking runs.
  - **Unified measurer**: replace the single-style `NveRuntimeMeasureTextFn` call with one `NveTextParagraphView` (text slab span + run slab + max width) callback so CoreText / `StaticLayout` / Pango / `QTextLayout` / DirectWrite measure and paint the same attributed paragraph, with per-run hit-testing for links and selection.
  - Invariants: zero per-run allocation after warm-up, `NveTextRun` locked with `offset_of!` / `static_assert`, byte-identical runs on every host.

- [ ] **Track 2 — Real `nve-ui` Interaction Primitives, Floating Overlays, Typed Tier-1 Hot Reload & Behavioral E2E**
  - Fix live `requested_scroll_x` lock-in bug (`skia/src/scroll.rs:115`) and wire full `Pointer*` events (`onpointerdown`/`onpointermove`/`onpointerup` + declarative synchronous `Interaction::capture_mode = OnPrimaryDown`), ancestor hover chain, `CursorKind`, CSS Top Layer (`showModal()`/`popover=auto`) + CSS Anchor Positioning with `getBoundingClientRect()` & shared `compute_overlay_position` fallback, and live controlled scroll sync across `nve-dioxus`, `nve-backend-web`, and `nve-backend-skia`.
  - **Delete `style_buf/` + all 62 `nve-ui` `style:` props, ban author-facing `style: "..."` strings completely (zero runtime CSS parser), AND upgrade Tier-1 hot reload (A6=6a)**: extend `packages/tools/cli/src/rsx_diff` + `HotPatchMsg::TemplatePatch` to diff typed RSX literal/enum/color attributes into typed **`RowPatch` word updates** applied straight to `NodeStore` (< 10 ms zero-recompile style hot reload on physical iOS/Web/Desktop with zero CSS parser).
  - Wire real `:hover`/`:active`/`:focus-visible`, pointer dragging (`SplitPane`, `BottomSheet`), hover/anchored floating overlays (`Tooltip`, `Popover`, `DropdownMenu`, `Dialog`), and real scroll sync (`VirtualList`) across `nve-ui` and `examples/kitchen-sink`, verified with live browser/window behavioral assertions & desktop/mobile screenshot critique.

- [ ] **Track 3A — Hardware Skia Retained Compositor/Caches, `skia-shell` AccessKit & `web-shell` `NVE_IFACE_HOST`**
  - **Skia inset `box-shadow`**: `PaintOp::DrawShadow` only rasterizes outer shadows, so `BoxShadow::inset` is skipped on Skia (web renders it natively); add an inset shadow op (clip to the rounded rect, invert the blurred mask) on CPU and GPU paths with pixel tests against the web output.
  - Fix Skia full display-list rebuild on every commit & motion tick (retained per-node display-list / compositor-layer tree for 120 Hz opacity/transform updates) plus GPU Skia per-frame image/font churn (`gpu/skia_renderer.rs`: GPU texture cache, bold `Typeface` cache, `CosmicTextEngine` glyph-run shaping) and add real Vulkan/D3D12 parity tests.
  - Wire `skia-shell` AccessKit accessibility tree sync and bind `NVE_IFACE_HOST = 5` in `hosts/web-shell`.

- [ ] **Track 3B — Native Host Linkage & OS Parity (Post-Track 1B-2)**
  - Fix `swiftui-kit` empty `NveSurfaceRepresentable` + `TextInput`/IME, `windows-shell` keyboard/IME/UIA, and bind `NVE_IFACE_HOST = 5` across native hosts (Apple, Android, Qt, GTK, Windows) directly against `libnve_runtime` (zero throwaway C++/JNI/Swift rewrites).

- [ ] **TV, Watch & Spatial Platform Backends — Web TV D-Pad Dogfood & Live Verification**
  - Engine (`nve-tv`), backend key interceptor, and `nve-tv-web` controller are landed and live-probed in Chrome with trusted keys (E2E suite 5: shelves, `data-nve-next-*`, ARIA / `<select>` arrow ownership, `<dialog>` trap, Select activation, re-entrancy); remaining: install `WebTvController` in an example app, a live gamepad E2E (CDP has no gamepad input: inject `navigator.getGamepads`), on-device Tizen/webOS key-code validation, and Android TV host forwarding of Back (consumed by the Activity).

- [ ] **Shared hit walk & CSS stacking order**: (a) port the headless hit semantics to Skia (or move one hit walk into a shared crate): handler-less boxes occlude siblings painted below them, a covered hit bubbles to ancestors only, and box-less `display: contents` nodes never stack (their `z_index` ignored in paint and hit order); (b) CSS 2.1 Appendix E painting order in every backend: positioned boxes (step 8) above in-flow siblings without `z_index`, stacking contexts (opacity / transform / `z_index`), and `display: contents` children ordered with their box parent's stacking context. Today paint and hit order are DOM order + a per-sibling-list `z_index` sort.
- [ ] **Dense writer side tables**: `NveMutationWriter`'s four sparse per-node `HashMap`s (`attached_tables`, `typed_styles`, `inline_styles`, `standalone_style_attrs`) become dense generation-checked per-slot tables like `writer/element_map.rs` `ElementMap` (one indexed load per attribute write); keep `tests/writer_zero_alloc.rs` at 0 allocations.

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

- [ ] **Fullstack Server↔Client DX — Slices 2 & 3 (Edge Adapters & `nve-wire` Multiplexed RPC)**
  - **Slice 2 — Edge/Serverless Adapters & `nve-wire` QUIC/WebTransport Multiplexing**: Add thin `nve-http-wasip2` (`wasi:http/incoming-handler` for Cloudflare/Fastly/Spin) adapter over sans-I/O `nve-server`, plus persistent QUIC/WebTransport RPC transport riding `nve-wire` alongside live `NVE2` frames and server-push tag invalidation streams.
  - **Slice 3 — Multi-Language Client Codegen & Ergonomic Macros (Demand-Driven)**: Add optional `#[nve::service]` / `#[nve::query]` sugar only where real app usage proves clear ergonomic wins over `procedures!`, and emit typed Swift/Kotlin/Dart/TypeScript CBOR RPC clients from shared procedure descriptors if polyglot hosts need direct server calls.

- [ ] **Unified Server-Driven UI & Multiplayer Collaboration (`nve-sync`: `Room` + `Presence` + `use_synced_state`)**
  - Unify `ZeroJsLiveSession` (HTML Declarative Shadow DOM `<slot>` streaming in `packages/backends/web/src/streaming_live.rs`) and binary wire SDUI (`NveWireFrame` v2 / `NVE2`) as **two encoders over one shared server-driven UI session model** (eliminating parallel remote-placement stacks).
  - Ergonomic `use_synced_state` / `use_synced_list` / `use_presence` hooks bridging server-authoritative thin-client rooms (0-JS Web & Native SDUI) and offline-first local CRDT peers (`packages/core/interop/src/crdt`) in the same live collaborative room, with ephemeral 20–30Hz cursors/selections/online presence.

- [ ] **Self-Documenting Protocol & Format Naming Cleanup**
  - Wave S2 replaced `NVT1`/`NVD1`/`NVC1` with `NveWireFrame`/`NveCrdtDelta`/`NveTimelineArchive`; finish removing stale `NVT1`/`NVD1` comments/log strings (e.g., `hosts/web-shell/src/{wasm_pair,lib}.rs`, `packages/tools/cli/src/{dev,main}.rs`) and replacing remaining cryptic 4-letter format acronyms (`NVWS1`, `NVB1`, `NWS1`, `NVE_MET1`, `NVE1`, `NVP1`, `NK1`, `NVNP`/`NVNC`) across code, wire headers, and docs with clear self-documenting names.

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Core modules (`nve-ui`, `nve-router`, `nve-host-services` + `nve-clipboard` + `nve-system-env`, Environment `TraitKey`s, live `NveCrdtDelta` peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v3 + `StateFile`) and live `/examples/studio/` GitHub Pages deploy are landed; next step is wiring live OS host services + opt-in `<NativeView<CameraPreview>>` / `BlurSurface` in Studio.
