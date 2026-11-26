# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md); full clean-slate capability coverage lives in [docs/ui-maturity-audit.md](./ui-maturity-audit.md).

---

## P1 — Direct Typed Render ABI, Interaction Primitives & Host Parity (from [quality-audit.md](./quality-audit.md))

- [ ] **Track 1B-1c — Mechanical Invariant Gates (`tests/arch_invariants.rs` + Workspace Lints), Pervasive Benchmarks & 4-Lens Dual Review**
  - Run fresh-eyes 4-lens dual review (`gemini-4-neon` + `anthropic/claude-opus-5-5`) over the landed Word-table `NodeStore` / `NVE2` / `nve-hash` / standalone modules work and fix all real findings.
  - Enforce strict `[workspace.lints.clippy]` across production code (`undocumented_unsafe_blocks = "deny"`, `todo = "deny"`, `unimplemented = "deny"`, `dbg_macro = "deny"`, and `unwrap_used` / `panic` denied on core/backend library targets).
  - Add a fast `syn` + `Cargo.toml` + filesystem `tests/arch_invariants.rs` suite wired into `cargo test --workspace` and `./scripts/check-all.sh` that mechanically fails CI on:
    - **Crate DAG & Layering**: `nve-abi`/`nve-hash`/`nve-store`/`nve-engine` depending on `dioxus*`; `packages/core/*` or `packages/backends/*` depending on `packages/modules/*`; `packages/core/*` depending on `web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`, or `serde_json`; `packages/modules/<cap>/` root spec crates depending on `nve-backend-*`/`web-sys`/`skia-safe`; library SDK code placed in `hosts/`; any workspace crate under `packages/` missing a non-empty `README.md`.
    - **Types, ABI & Legacy Regressions**: bare primitive domain fields (`*_id`, `*_key`, `*_slot`, `*_index`, `*_off`, `*_len`, `*_mask`, `*_flags`, `*_us`, `*_ms`, `parent`, `surface`, `node`, `rel`) in `packages/core/{abi,store,engine,tables}` structs; `#[repr(C)]` structs in `nve-abi` missing compile-time `size_of`/`align_of`/`offset_of!` asserts; regressions of `ComponentKind`, `NVE_HOST_CLASS_*`, `NativeEventKind::Custom`, `Opcode::SetRawProp`, `RawPropValue`, `RawStyleProps`, `NativeViewSpec`, `RenderSink`, or `BatchSink`; `serde_json` in core/backend/module runtime dependencies.
    - **Performance & Hot Paths**: `from_utf8` inside `StrArena` read methods or outside explicit ingress/wire decoders; `.to_ascii_lowercase()` / `format!(` allocations inside hot parsing/style/render functions; `HashMap`/`Box<` creeping onto per-node store structs (`NodeRow`/columns); core/backend crates lacking benchmarks.
    - **Hashing & Generated Files**: any FNV-1a symbol or offset/prime constant anywhere in the repo; hardcoded `*_SCHEMA_HASH = 0x...` hex literals in Rust; raw hex schema hashes in `docs/content/**/*.mdx`; `@generated` non-Java files lacking `.generated.` in their filename; tracked build/trace/screenshot/generated artifacts (`*.generated.*`, `*.trace.json`, `cxx/build*`, `hosts/*/build`).
    - **Platform Purity, Modularity & Anti-Slop**: Web CSS/HTML formatters (`write_css*`, `as_css`, `as_html*`) inside `packages/core/*`; non-test production Rust files exceeding `650` lines; banned cryptic 4-letter format acronyms (`NVH1`, `NVW1`, `NVT1`, `NVD1`, `NVC1`, `NVWS1`, `NVB1`, `NVNP`, `NVNC`, `NVERELOC`) or marketing buzzwords (`Zero-JSON`) outside historical `docs/CHANGELOG.md`; hardcoded personal endpoints/hosts.
  - Expand `CountingAlloc` zero-allocation steady-state hot-path tests and pervasive Criterion micro + macro benchmarks (reporting latency + allocation counts + sizes across small/realistic/stress inputs) plus release ASM checks on hot primitives.

- [ ] **Track 1B-1d — Uniform 64-Byte Typed `NveEvent`, Zero-Boilerplate Table/Codec Macros, Core/Motion Cleanup & Crate READMEs**
  - **Uniform 64-byte `NveEvent` (`#[repr(C, align(64))]`)**: Replace `NativeEventKind`, `NveEventPayloadTag`, heap-allocating `EventPayload`, global `EventMask`, and 96 B `NveDirectEvent` with one typed 1-cache-line `NveEvent` (`TimestampUs`, `TableKey`, `SurfaceId`, `NodeId`, `EventId`, `EventFlags`, `[Word; 4]` zero-alloc inline payload, `RawEventBytes`), moving event listener masks onto individual tables (`Interaction`, `Scroll`, `TextInput`, modules) so state, commands, and events are 100% uniform table operations with zero closed enums in `nve-abi`.
  - **Zero-boilerplate table, enum & wire macros (`nve-abi-macros` / `nve-store`)**:
    - Upgrade `#[derive(NveTable)]` to compute bit-packing offsets at compile time via `const` expressions over `<T as WordValue>::BITS` (deleting redundant `#[bits(N)]` on `bool`/`u8`/`u16`/enums everywhere).
    - Add auto-sized `#[derive(WordEnum)]` + first-class `Opt<T>` (`Opt<f32>`, `Opt<u16>`, `Opt<Rgba>`, `Opt<Enum>`) to delete manual wrapper structs (`OptAutofillHint`), magic sentinel comments (`u16::MAX`, `f32::NAN`), and split bool+value pairs (`tint_enabled`).
    - Add `word_flags!` for typed bitfields (`LinkRel`, `TextDecoration`) and `#[nve_table(inherited)]` to auto-generate `Typography`'s explicit-field mask (deleting manual `SET_*` constants).
    - Add one reusable `#[derive(NveCodec)]` for genuine Tier 2 binary codecs (`signal_codec.rs`, `crdt/hlc.rs`, `wire-core/frame.rs`), delete unnecessary byte codecs on Tier 1 in-memory structs (`KeyboardInsets::encode_bytes`/`decode_bytes`), and collapse duplicate internal NVE types (`PatchTextAlign` vs `TextAlign`, `schema_bridge.rs`).
  - **Core purity & standalone `packages/modules/motion/`**:
    - Move all Web CSS/HTML formatting & string parsing (`layout_ext.rs`, `motion.rs`, `paint.rs`, `transform.rs`, `media.rs`, `tables/*.rs` `write_css*`/`as_css`/`as_html*`/`parse`) out of `packages/core/*` strictly into `nve-backend-web` (zero-alloc `eq_ignore_ascii_case` / kebab macro where needed).
    - Extract `motion` out of the `components` crate into a uniform standalone capability module **`packages/modules/motion/`** (`nve-motion` with clean `src/{easing,spring,clock,interpolate}.rs` + thin leaf `web/` and `skia/` adapters, matching `svg`/`canvas`).
    - Clean up remaining `packages/core/components` into pure stateless core tables (`packages/core/tables` or trimmed `nve-components`).
  - **Lean `README.md` for every crate under `packages/`** (15–30 lines: role & layer, key public API + 5-line example, dependency invariants, test/bench commands).

- [ ] **Track 1B-1e — Codegen Modernization (Templates + Static Native Files)**
  - Delete dead legacy codegen targets and hand-duplicated schema definitions superseded by `#[derive(NveAbi)]` / `#[derive(NveTable)]` (`wire.rs` old `NVE_DRAW1`/old `EventRing`, `schema_bridge.rs`, duplicated schema specs).
  - Move static, unchanging foreign runtime helpers (`NVE_API` macros, XXH64 helpers, EventRing/Reloc helpers) out of escaped Rust `writeln!` string literals into real checked-in `.h`/`.hpp`/`.ts`/`.swift`/`.dart`/`.java` files (with native IDE highlighting, linting, and formatting).
  - Migrate remaining schema-derived multi-language emitters to **Askama** or **MiniJinja** templates (keeping type-name lowering as thin Rust IR helpers).

- [ ] **Track 1B-2 — Host-Owned `packages/core/runtime` (Model A + 5→2 Layout Unification, ADR-003/ADR-004)**
  - Build host-owned `packages/core/runtime` (`libnve_runtime`: direct `TaffyStoreLayout` default + opt-in `Yoga`, LRU-cached OS `TextMeasurer` C callback with batch mode, `InteractionTracker`, `MotionDriver`, finalized `NveSceneView`/`NveTableView`/`NveEvent`, Model A `measure_surface` with baselines/dry-run/invalidation callback) and bind all native hosts (`swiftui-kit`, `flutter-plugin`, `apple-common`, `android-shell`/`compose-kit`, `gtk-shell`, `qt-shell`, `windows-shell`) to it.
  - Delete ~8,000–11,500 LOC of duplicated C++/Dart layout & decoder code (`flexbox_solver.cpp`, `yoga_layout.cpp`, `shadow_tree.cpp`, `scene_publisher.cpp`, `fabric_c_api.cpp`, `command_{en,de}coder.hpp`, `nve_snapshot.hpp`, `flex_layout.dart`, `command_decoder.dart`), shrink C++ helpers into header-only `packages/core/host-cxx`, and extract optional Model A RN Fabric `ComponentView` adapters into `packages/modules/react-native-fabric`.

- [ ] **Track 2 — Real `nve-ui` Interaction Primitives, Floating Overlays & Behavioral E2E**
  - Wire real pointer events (`onpointerdown`/`onpointermove`/`onpointerup` + pointer capture), hover/enter/leave, real `position: fixed`/top-layer floating portals with live `getBoundingClientRect()` trigger anchoring, and live controlled scroll sync across `nve-dioxus`, `nve-backend-web`, and `nve-backend-skia`.
  - Delete duplicate `packages/framework/ui/src/style_buf/` builders and wire real `:hover`/`:active`/`:focus-visible`, `CursorKind`, pointer dragging (`SplitPane`, `BottomSheet`), hover/anchored floating overlays (`Tooltip`, `Popover`, `DropdownMenu`, `Dialog`), and real scroll sync (`VirtualList`) across `nve-ui` and `examples/kitchen-sink`, verified with live browser/window behavioral assertions.

- [ ] **Track 3 — Hardware Skia Performance & Broken Host Linkage Fixes**
  - Fix Skia full display-list rebuild on every commit & motion tick (retained per-node display-list / compositor-layer caching for 120 Hz opacity/transform updates) plus GPU Skia per-frame image/font churn (`gpu/skia_renderer.rs`: GPU texture cache, bold `Typeface` cache, `CosmicTextEngine` shaping) and add real Vulkan/D3D12 parity tests.
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
