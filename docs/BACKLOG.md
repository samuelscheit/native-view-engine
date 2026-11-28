# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md); full clean-slate capability coverage lives in [docs/ui-maturity-audit.md](./ui-maturity-audit.md).

---

## P1 — Direct Typed Render ABI, Interaction Primitives & Host Parity (from [quality-audit.md](./quality-audit.md))

- [ ] **Track 1B-2 — Host-Owned `packages/core/runtime` Follow-Ups (Slices 3–4; Slices 1 & 2 — ABI v6 / `RuntimeHostRouter` / `measure_surface` / Frame-0 / `DioxusRuntimeApp` / `skia-shell` cleanup / Android & Compose ABI v6 + `NveRuntime.cmake` NDK derivation + `export_rn`/`export_compose` rebind — landed)**
  - **Apple hosts follow-ups (Slice 3)**: wire `apple-common` / `macos-shell` / `ios-shell` / `swiftui-kit` onto ABI v6 `save_frame0`/`boot_frame0`, `measure_surface`, `NVE_IFACE_HOST = 5`, canvas `draw_list` rendering, CoreText typography parity, `swiftui-kit` editable `TextInput`/IME, `NVE_ENABLE_NATIVE_VIEWS` / `NVE_ENABLE_CAMERA` adapters, and `--pair-transport` selection.
  - **Android emulator / Compose instrumentation CI**: add real emulator / Compose instrumentation runs in CI (JNI/Robolectric/Compose JVM suites already run in `check-all.sh`).
  - **Desktop native hosts (Slice 4)**: `windows-shell` keyboard/IME/UIA (`packages/modules/accessibility/windows`), `gtk-shell` keyboard/IME/`GtkAccessible` relations, and desktop parity across `windows-shell` / `gtk-shell` / `qt-shell` (leaving host-service FFI binding for Direct-First Phase 4).
  - Dart / TS wasm32 `SceneReader`s read the row-index layout in tests only: wire them to a wasm32 `libnve_runtime` publisher once one exists (the browser shell lays out in the DOM and publishes no scene).

- [ ] **Direct-First Hybrid FFI, Generic `ByteTransport`, Lock-Free `ServiceSlots` & Host-Service Decoupling (Phases 2–5; Phase 1 `8e0e535d` landed)**
  - **Phase 2 — Direct-FFI codegen & runtime plumbing**: extend `.nve.rs` `#[nve_host_module]` codegen (`nve-codegen`) and host runtimes to emit typed direct-FFI C/Swift/Kotlin/C++/WASM host stubs over lock-free `ServiceSlots` and `AsyncBytesDoneGuard` with compile-time layout locks.
  - **Phase 3 — Migrate 6 OS capability modules**: move `clipboard`, `haptics`, `linking`, `secure-store`, `filesystem`, and `system-env` onto `.nve.rs` specs and delete hand-written `ByteWriter`/`ByteReader` codecs and `dispatch_*_wire`.
  - **Phase 4 — Wire all hosts & delete legacy 4 KiB retry plumbing**: wire generated direct-FFI stubs into `ios-shell`, `android-shell`, `macos-shell`, `windows-shell`, `gtk-shell`, `qt-shell`, `skia-shell`, `web-shell`, and `nve pair` (`ByteTransport`); delete `SERVICE_CLIPBOARD..SERVICE_HAPTICS`, `OversizedRetryCache`, and legacy 4 KiB retry plumbing; lock with `nve-arch-invariants`.
  - **Phase 5 — `#[batchable]` command-buffer codegen**: emit flat `u32`/byte command-buffer encoders/decoders directly from `.nve.rs` traits.

- [ ] **Option A — Multi-Run Inline Attributed `text {}` (converged hybrid design)**
  - **Authoring & store**: `text { "Hello " b { "bold" } a { href, "link" } }` keeps reactive inline children as ordinary `NodeStore` nodes marked `NVE_NODE_ROW_FLAG_INLINE` (no layout box, no host view) so signals, events, and hot patches address each run by `NodeId`.
  - **Single producer, compiled runs**: the owning paragraph compiles its inline subtree into one contiguous slab of 48-byte POD `NveTextRun` records (`text_off`, `text_len`, `node`, resolved typography words, decoration / link flags) — one producer, zero per-run heap, republished only when a run is dirty.
  - **Unboxed inline nodes + 1-hop dirty lift**: inline nodes get no rect and no `dirty_indices` entry of their own; a content change on an inline node lifts exactly one hop to its paragraph, so hosts restyle one paragraph instead of walking runs.
  - **Unified measurer**: replace the single-style `NveRuntimeMeasureTextFn` call with one `NveTextParagraphView` (text slab span + run slab + max width) callback so CoreText / `StaticLayout` / Pango / `QTextLayout` / DirectWrite measure and paint the same attributed paragraph, with per-run hit-testing for links and selection.
  - Invariants: zero per-run allocation after warm-up, `NveTextRun` locked with `offset_of!` / `static_assert`, byte-identical runs on every host.

- [ ] **Track 2 Slice 2C — Real `nve-ui` Controls, Kitchen Sink De-Faking & Behavioral E2E (Slices 2A & 2B landed)**
  - Wire real `:hover`/`:active`/`:focus-visible`, `CursorKind`, pointer dragging (`SplitPane`, `BottomSheet`), hover/anchored floating overlays (`Tooltip`, `Popover`, `DropdownMenu`, `Dialog`), and live controlled scroll sync (`VirtualList`) across `nve-ui` and `examples/kitchen-sink`, verified with live Skia `DesktopSession` & Chrome CDP behavioral assertions and desktop/mobile screenshot critique, plus Slice 2A verification polish.

- [ ] **Track 3B — Native Host Linkage, Hardware Vulkan/D3D12 Verification & OS Parity (Post-Track 1B-2)**
  - Finish `swiftui-kit` editable `TextInput`/IME & `NveSurfaceRepresentable`, `windows-shell` keyboard/IME/UIA, bind direct-FFI host services (Direct-First Phase 4) across native hosts directly against `libnve_runtime` (zero throwaway C++/JNI/Swift rewrites), and run real hardware Vulkan/D3D12 Skia parity tests on Linux/Windows GPUs (`samuel-pc`).

- [ ] **TV, Watch & Spatial Platform Backends — Web TV D-Pad Dogfood & Live Verification**
  - Engine (`nve-tv`), backend key interceptor, and `nve-tv-web` controller are landed and live-probed in Chrome with trusted keys (E2E suite 5: shelves, `data-nve-next-*`, ARIA / `<select>` arrow ownership, `<dialog>` trap, Select activation, re-entrancy); remaining: install `WebTvController` in an example app, a live gamepad E2E (CDP has no gamepad input: inject `navigator.getGamepads`), on-device Tizen/webOS key-code validation, and Android TV host forwarding of Back (consumed by the Activity).

- [ ] **Shared hit walk & CSS stacking order**: (a) port the headless hit semantics to Skia (or move one hit walk into a shared crate): handler-less boxes occlude siblings painted below them, a covered hit bubbles to ancestors only, and box-less `display: contents` nodes never stack (their `z_index` ignored in paint and hit order); (b) CSS 2.1 Appendix E painting order in every backend: positioned boxes (step 8) above in-flow siblings without `z_index`, stacking contexts (opacity / transform / `z_index`), and `display: contents` children ordered with their box parent's stacking context. Today paint and hit order are DOM order + a per-sibling-list `z_index` sort.

---

## P1 — UI & OS Capability Maturity (from [ui-maturity-audit.md](./ui-maturity-audit.md))

- [ ] **Mobile Host Autofill/Keyboard Wiring & Mobile Selection Handles**
  - Subtree `ErrorBoundary`, `AutofillHint`/`InputMode` + Web autofill sync, `KeyboardInsets` + Web `visualViewport` + `KeyboardAvoidingView` (`38b351c`), UAX #29 grapheme-safe editing/selection, unified `CompositionStart/Update/End` + live caret screen-rect IME feedback, and Skia BiDi run/caret/selection mapping (`be5e394`) are landed; remaining work is native Apple/Android host prop wiring for `AutofillHint`/`KeyboardInsets` and mobile touch selection handles/loupes.

- [ ] **Desktop OS Polish, Live Layout Inspector & Remaining Native Accessibility Hosts**
  - Locale-aware `DatePicker`/`DateRangePicker` (`c69cdd0`), typed gradients/cached SVG (`7cfa635`), standalone `packages/modules/accessibility` (`nve-accessibility`), and `hosts/skia-shell` `accesskit_winit` wiring are landed; remaining work is OS drag-and-drop (internal + file drop), native menu bar / context menus / system tray, live visual layout/recomposition overlay inspector, and reusing `nve-accessibility` inside `windows-shell` (`accesskit_windows`/UIA) and `gtk-shell`.

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
