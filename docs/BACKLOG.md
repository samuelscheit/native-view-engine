# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md); full clean-slate capability coverage lives in [docs/ui-maturity-audit.md](./ui-maturity-audit.md).

---

## P1 — Direct Typed Render ABI, Interaction Primitives & Host Parity (from [quality-audit.md](./quality-audit.md))

- [ ] **Track 1B-1c — Mechanical Invariant Gates (`arch_invariants` + Workspace Lints), Pervasive Benchmarks & 4-Lens Dual Review**
  - Run fresh-eyes 4-lens dual review (`gemini-4-neon` + `anthropic/claude-opus-5-5`) over the landed Word-table `NodeStore` / `NVE2` / `nve-hash` / standalone modules work, fix all real findings, and flip `ENFORCE_POST_NODESTORE = true` in `arch_invariants` (activating `ENFORCE_POST_1B1D` / `ENFORCE_POST_1B1E` in the exact commits that complete each track).
  - Enforce strict `[workspace.lints.clippy]` across production code (`undocumented_unsafe_blocks = "deny"`, `todo = "deny"`, `unimplemented = "deny"`, `dbg_macro = "deny"`, and `unwrap_used` / `panic` denied on core/backend library targets).
  - Extract `arch_invariants` out of `packages/tools/cli` into its own zero-dependency crate `packages/tools/arch-invariants` (`publish = false`, zero `[dependencies]`) so Suite 0 compiles in <1s without building `nve-cli`/`skia-safe`.
  - Add `#[cfg(doctest)] #[doc = include_str!("../README.md")] pub struct ReadmeDoctests;` to every workspace crate's `lib.rs` (plus `README.md`s for new `hash`/`store`/`codec`/`modules/*` crates) and enforce the doctest hook mechanically in `arch_invariants` so crate README snippets always compile under `cargo test --doc`.
  - Extract `nve-skia-shell` library code out of `hosts/skia-shell` (currently depended on by `packages/tools/cli`) into `packages/backends/skia` or `packages/tools`, removing the `PENDING_CLEANUP_HOST_DEPS` allowlist entry so `packages/**` has zero dependencies on `hosts/**`.
  - Keep `arch_invariants` enforcing all 6 invariant categories as staged gates flip on (crate DAG/packaging via `cargo metadata`, typed domain fields & sealed arenas & `#[repr(C)]`/cache-line layout locks & zero legacy regressions, zero-boilerplate macros/converters, hot-path zero-alloc & zero-UTF-8-revalidation & benchmark coverage, zero FNV-1a / zero hardcoded schema hashes / zero raw-string codegen / 53-bit JS/Dart safety / generated-file hygiene, platform purity & `< 650`-line file cap & banned acronyms/buzzwords & open-only backlog & zero personal endpoints).
  - Expand `CountingAlloc` zero-allocation steady-state hot-path tests and pervasive Criterion micro + macro benchmarks (reporting latency + allocation counts + sizes across small/realistic/stress inputs) plus release ASM checks on hot primitives.

- [ ] **Track 1B-1d — Uniform Pointer-Free 64-Byte `NveEvent`, Zero-Boilerplate Table/Codec Macros, Core Purity & Standalone Motion**
  - **Uniform pointer-free 64-byte `NveEvent` (`#[repr(C, align(64))]`)** in `nve-abi::event` (identical offsets on 64-bit & 32-bit `wasm32`/`armv7`/`i686`): `timestamp: TimestampUs` (`0`), `table: TableKey` (`8`, `lo@8`/`hi@12` for JS/Dart), `surface: SurfaceId` (`16`), `node: NodeId` (`20`, unified newtype in `nve-abi` re-exported by `nve-store`), `event: EventId` (`24`, `< 32` listener bit), `flags: EventFlags` (`28`), `payload: [Word; 6]` (`32`, supporting wide 2-word `f64`/`u64`, `PhysicalKey` + 16 B inline UTF-8 `key`), and `bytes: RawEventBytes { offset: ArenaOffset, len: ByteLen }` (`56`, indexing into Rust-owned `NveEventBatch` byte arena). Validate once at ingress into `ValidatedEvents<'a>` (zero-alloc `ev.text()` via documented `from_utf8_unchecked`); delete `NativeEventKind`, `NveEventPayloadTag`, `EventPayload`, global `EventMask`, old EventRing, 96 B `NveDirectEvent`, and `TypedTableEvent`.
  - **Per-table `EventSet<E>` listener masks & zero-alloc host/Dioxus routing**: `#[nve_table(events = ...)]` auto-adds `listeners: EventSet<E>` (`EVENTS` effect); hosts track per-surface `[u32; 32]` refcounts per table (attach DOM/OS listener on 0→1, detach on 1→0) and bubble via parent chain (zero `HashMap`); `nve-dioxus` uses compile-time `rsx_events!` + reused `Rc::get_mut` slot (`NveEventData { raw: NveEvent, text: String }`) + typed `ComponentView<T: HasEvents>`.
  - **Zero-boilerplate table, enum, flags, inheritance & codec macros (`nve-abi-macros` / `nve-store` / `nve-codec`)**:
    - Upgrade `#[derive(NveTable)]` to `const fn` first-fit bin-packing per effect class from `<T as WordValue>::BITS` (plus wide 2-word fields for `f64`/`u64` and `schema_hash` covering both layout and defaults), deleting redundant `#[bits(N)]` on `bool`/`u8`/`u16`/enums/flags (keep `#[bits(N)]` only for integer narrowing).
    - Implement `impl<T: OptWord> WordValue for std::option::Option<T>` directly with all-zero = `None`: presence bit for `u8`/`u16`/`i16`/`bool`/flags, `discriminant + 1` for `WordEnum`, NaN XOR niche for `f32`, tag niche for `Length`, `NULL = 0` for `NodeId`, and **wide 1-bit presence flag + 32-bit value word** for `Option<Rgba>`/`Option<u32>`/`Option<i32>` (100% lossless, zero magic XOR math in C++/Swift/Kotlin/Dart). Delete `OptAutofillHint`, magic sentinel comments (`f32::NAN`, `u16::MAX`), and `tint_enabled`.
    - Add `#[derive(WordEnum)]` (total `from_word`, `OptWord`, `const fn to_word`, zero CSS strings), `word_flags!` (wrapping `define_bitflags!` for `LinkRel`, `TextDecoration`), and `#[nve_table(inherited)]` (hidden `__explicit` mask in word 0 with `word_effects[0] = 0`, `put_with_effects`, and branchless `resolve_inherited`, deleting all 14 `Typography::SET_*` constants and 3 per-backend inheritance loops). Move `GridLine` next to `Length` in `nve-store::value`.
    - Add zero-dependency **`packages/core/codec` (`nve-codec`)** + `#[derive(NveCodec)]` (`ByteReader`/`ByteWriter`/`CodecError`, little-endian, strict bool/enum/magic/version/reserved/len validation, zero `unsafe`) replacing all 7 private byte readers and hand-written Tier 2 codecs (`hlc.rs`, `frame.rs` header, `handshake.rs`, `patch.rs`, `signal_archive/codec.rs`, `crdt/wire.rs`, `host-services/codec.rs`, `hotreload.rs`, plus `canvas`/`svg`/`camera` codecs); delete unused `KeyboardInsets::{encode,decode}_bytes`.
    - Replace `StylePatch` with table-generic **`RowPatch { table, mask, words }`** overlays, deleting `style_patch.rs` (`524` L), `style_patch_codec.rs` (`443` L), duplicate `style_patch::TextAlign` (fixing its discriminant mismatch with `visual::TextAlign`), and Skia's `decode_text_align`/`map_text_align`.
  - **Core purity, standalone `packages/modules/motion/` & `packages/core/tables` rename**:
    - Move all Web CSS/HTML formatting & string parsing (`layout_ext.rs`, `motion.rs`, `paint.rs`, `transform.rs`, `media.rs`, `tables/*.rs`, `box_shadow.rs`, `interaction.rs`) out of `packages/core/*` strictly into `nve-backend-web/src/css/` (`css_keywords!` macro with `eq_ignore_ascii_case`, unified transform writer, reused buffer `write!` for media queries; delete dead `client_hints.rs`, `input_hints.rs` parsers, and `parse_box_shadow`).
    - Extract `motion` into a uniform standalone capability module **`packages/modules/motion/`**: `nve-motion` (`src/{easing,spring,clock,interpolate,table}.rs` with dense slab keyed by `(NodeId, TransitionProperty)` and `Key64`, zero `Cow`/`HashMap`), thin leaf `nve-motion-web` (CSS `transition` + springs via CSS `linear()`), and `nve-motion-skia` (receives `MotionDriver` from `skia/src/motion.rs` via a generic core frame-tick hook).
    - Move `tracker.rs` → `packages/core/engine` and `reconcile.rs` → `packages/core/interop`, then **rename `packages/core/components` → `packages/core/tables` (`nve-tables`)** holding strictly stateless core tables, pure value types, and event payloads.

- [ ] **Track 1B-1e — Codegen Modernization (Askama 0.16 Templates + Static Native Packages)**
  - **Delete dead legacy codegen & duplicate parsers (~6,776 LOC removed)**: delete `schema/wire.rs` + 5 `*/wire.rs` emitters (`2,919` L: FNV, old EventRing, hand-written `NVE_DRAW1` constants — generate `DrawList` decoders from `nve-codec` descriptors via one generic codec template per language), delete `reloc` codegen (`schema/reloc` + `rust` + `c_cpp`, `2,280` L; only used by a bench and tests), and delete `frontend/hybrid_rust` (`1,577` L duplicate hand-written lexer), keeping `schema_bridge.rs` as the single `NveAbi` Rust frontend.
  - **Extract ~5,000 LOC of raw `r#"` runtime strings into checked-in static native packages**: move unchanging runtime preludes (`NVE_API` macros, XXH64 implementations, thin `NveEventRing` readers) out of escaped Rust `writeln!` blocks into checked-in `packages/core/host-{cxx,swift,java,dart,ts}/` packages tested by their own native toolchains (only XXH64 test vectors stay generated).
  - **Migrate schema-derived multi-language emitters to Askama 0.16 (`escape = "none"`)**: configure `escape = "none"` in `packages/core/codegen/askama.toml` and on every `#[template]`; borrow `&'a AbiFileDescriptor` and `&'a [&'static TableDesc]` directly for compile-time template verification across `core_abi` + `scene_abi` (merged into one ABI template per language, dropping hand-written `scene_abi_spec`), `hybrid`, `codec`, and framework wrappers (`export_rn_*` moving into `packages/modules/react-native-fabric` in Track 1B-2).

- [ ] **Track 1B-2 — Host-Owned `packages/core/runtime` (Model A + 5→2 Layout Unification, ADR-003/ADR-004)**
  - Build host-owned `packages/core/runtime` (`libnve_runtime`: direct `TaffyStoreLayout` default + opt-in `Yoga`, LRU-cached OS `TextMeasurer` C callback with batch mode, `InteractionTracker`, generic frame-tick hook for `nve-motion-skia`, finalized `NveSceneView`/`NveTableView`/`NveEvent`, Model A `measure_surface` with baselines/dry-run/invalidation callback) and bind all native hosts (`swiftui-kit`, `flutter-plugin`, `apple-common`, `android-shell`/`compose-kit`, `gtk-shell`, `qt-shell`, `windows-shell`) to it.
  - Delete ~8,000–11,500 LOC of duplicated C++/Dart layout & decoder code (`flexbox_solver.cpp`, `yoga_layout.cpp`, `shadow_tree.cpp`, `scene_publisher.cpp`, `fabric_c_api.cpp`, `command_{en,de}coder.hpp`, `nve_snapshot.hpp`, `flex_layout.dart`, `command_decoder.dart`), consolidate C++ helpers in header-only `packages/core/host-cxx`, and extract optional Model A RN Fabric `ComponentView` adapters into `packages/modules/react-native-fabric`.

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
