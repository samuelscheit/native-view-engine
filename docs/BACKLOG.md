# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md); full clean-slate capability coverage lives in [docs/ui-maturity-audit.md](./ui-maturity-audit.md).

---

## P1 — Direct Typed Render ABI, Interaction Primitives & Host Parity (from [quality-audit.md](./quality-audit.md))

- [ ] **Track 1 — Wave C1: Direct `#[repr(C)]` Frame ABI & Zero Out-of-Band Side-Channels**
  - Wave S2 (`packages/core/wire` + `packages/core/wire-core` QUIC/WebTransport, `NveTimelineArchive`, deleting `packages/modules/pair` & `NVT1`/`NVD1`, `752ea2e`) is landed.
  - Replace in-process `NVE1`/`NVEV`/`NK1`/`NVE_MET1` byte streams with unified `#[repr(C)]` `NveFrameCommandBatch`, `NveDirectEvent`, `NveEventBatch`, `NvePropsRef`, and `NveNodeMeta`, and fold interactive pseudo-states, `CursorKind`, `TransitionSpec`, `BackgroundFill`, `SvgPicture`, and `DrawList` into the single canonical render ABI (deleting every `sync_*_from` side-channel).
  - Fix `DioxusSurface` root-props injection (removing `nve-docs` thread-local bootstrap), slim `NodeRecord` via boxed cold extras, fix the `FfiHostServices` async callback registration race + ObjC RAII pool, and eliminate `WebBackend` WASM→DOM readbacks.

- [ ] **Track 2 — Real `nve-ui` Interaction Primitives, Floating Overlays & Behavioral E2E**
  - Wire real pointer events (`onpointerdown`/`onpointermove`/`onpointerup` + pointer capture), hover/enter/leave, real `position: fixed`/top-layer floating portals with live `getBoundingClientRect()` trigger anchoring, and live controlled scroll sync across `nve-dioxus`, `nve-backend-web`, and `nve-backend-skia`.
  - Delete duplicate `packages/framework/ui/src/style_buf/` builders and wire real `:hover`/`:active`/`:focus-visible`, `CursorKind`, pointer dragging (`SplitPane`, `BottomSheet`), hover/anchored floating overlays (`Tooltip`, `Popover`, `DropdownMenu`, `Dialog`), and real scroll sync (`VirtualList`) across `nve-ui` and `examples/kitchen-sink`, verified with live browser/window behavioral assertions.

- [ ] **Track 3 — Hardware Skia Performance & Broken Host Linkage Fixes**
  - Fix GPU Skia per-frame image/font churn (`gpu/skia_renderer.rs`: GPU texture cache, bold `Typeface` cache, `CosmicTextEngine` shaping) and add real Vulkan/D3D12 parity tests.
  - Fix `compose-kit` 15 missing JNI exports in `hosts/android-shell`, `swiftui-kit` empty `NveSurfaceRepresentable` + `TextInput`, `windows-shell` keyboard/IME/UIA, `skia-shell` AccessKit, and bind `NVE_IFACE_HOST = 5` across real hosts (`web-shell`, Apple, Android, Qt, GTK, Windows).

- [ ] **Adopt `FrameArena` on Hot `nve-dioxus` / `nve-engine` Per-Frame Allocation Paths**
  - `XXH64`, `FrameArena`, crash-atomic `MmapOffsetArena` (`023d77a`), 4→2 persistence collapse (`2ef5a78`), and default-on Native Instant-Boot (`b5dabc3`) are landed; remaining work is wiring `FrameArena` bump-reset allocation into hot `nve-dioxus`/`nve-engine` per-frame paths.

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

- [ ] **Unified Multiplayer Collaboration (`nve-sync`: `Room` + `Presence` + `use_synced_state`)**
  - Ergonomic `use_synced_state` / `use_synced_list` / `use_presence` hooks bridging server-authoritative thin-client rooms (0-JS Web & Native SDUI) and offline-first local CRDT peers (`packages/core/interop/src/crdt`) in the same live collaborative room, with ephemeral 20–30Hz cursors/selections/online presence.

- [ ] **Self-Documenting Protocol & Format Naming Cleanup**
  - Wave S2 replaced `NVT1`/`NVD1`/`NVC1` with `NveWireFrame`/`NveCrdtDelta`/`NveSceneArchive`/`NveTimelineArchive`; finish replacing remaining cryptic 4-letter format acronyms (`NVERELOC`, `NVWS1`, `NVB1`, `NWS1`, `NVE_MET1`, `NVE1`, `NVP1`, `NK1`, `NVNP`/`NVNC`) across code, wire headers, and docs with clear self-documenting names.

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Core modules (`nve-ui`, `nve-router`, `nve-host-services` + `nve-clipboard` + `nve-system-env`, Environment `TraitKey`s, live `NveCrdtDelta` peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) and live `/examples/studio/` GitHub Pages deploy are landed; next step is wiring live OS host services + opt-in `<NativeView<CameraPreview>>` / `BlurSurface` in Studio.
