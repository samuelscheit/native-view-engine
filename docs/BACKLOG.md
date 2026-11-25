# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md); full clean-slate capability coverage lives in [docs/ui-maturity-audit.md](./ui-maturity-audit.md).

---

## P1 — Zero-Serialization ABI, Multi-Device Backends & Host Parity

- [ ] **Pluggable Camera Package — Thin SwiftUI/Flutter Wrappers & Live Emulator Camera Verification**
  - All primary camera adapters are landed (`CameraPreview` spec + `NVCF`, Android VisionCamera v5 + real Jetpack `CameraX` `PreviewView`, Apple `AVFoundation` + VisionCamera v5 iOS bridge, Web `getUserMedia` `<video>`, C++20 Windows `MediaCapture` + Linux PipeWire/V4L2); remaining follow-ups are thin SwiftUI & Flutter camera wrappers and live Android Emulator virtual-scene verification once Android SDK cmdline-tools are installed.

- [ ] **Finish Zero-Copy `packages/core/abi` & Extract `packages/core/wire` (Wave C1 — Option A1)**
  - Move all byte-stream codecs (`NVE1` `CommandBuffer`, `NVEV` `EventRing`, `BinValueView`, `NVH1`, `NK1`) out of `packages/core/abi` into `packages/core/wire` (network/disk transport only) and migrate in-process render (`nve-dioxus` / `nve-engine` / backends / hosts) to pure zero-copy `#[repr(C)]` `NveCommandSlice`/`NvePropSlice`/`NveSceneView` slabs.

- [ ] **Eliminate Remaining In-Process Codecs & Add Codegen Zero-Copy `RelocSlice` Wire Views (Wave C3)**
  - Replace `NVE_DRAW1` with a codegen-locked `#[repr(C)]` `NveDrawOp` slab (`&[NveDrawOp]`), delete the 3× duplicated `NVNP`/`NVNC` codecs in favor of direct `NvePropSlice`/`NveValue`, unify `NVCF` to one `#[repr(C)]` struct, and replace `NVT2`/`NVL1`/`NVW1`/`NVN1`/`NVDS`/`NVE_MET1` in-process codecs with direct typed / `#[repr(C)]` structs.
  - Extend `packages/core/codegen` to generate zero-copy aligned `#[repr(C)]` + `RelocSlice` views (`view_from_aligned_bytes` / `as_bytes`) for legitimate disk/network formats in `packages/core/wire` instead of hand-written Rust encode/decode code.

- [ ] **TV, Watch & Spatial Platform Backends — Phase 3 (Web TV D-Pad Remaining)**
  - Phase 1 (`DeviceFormFactor` traits, `SpatialFocusEngine`, Android TV D-Pad), Phase 2 (standalone `packages/modules/tv` + real `tvOS`/`watchOS`/`visionOS`/`iOS` SDK builds + static `NveInterfaceResolver`), and Phase 3 CLI (`nve build --target tvos/visionos/watchos` + `.xcframework` recipes) are landed; remaining work is Web TV D-Pad key handling in `nve-backend-web`.

- [ ] **Adopt `FrameArena` on Hot `nve-dioxus` / `nve-engine` Per-Frame Allocation Paths**
  - `XXH64`, `FrameArena`, crash-atomic `MmapOffsetArena` (`023d77a`), 4→2 persistence collapse (`NVERELOC` + `MmapOffsetArena`, deleting `NVE_SNAP`/`NVE_WAL`), and reusable Native Instant-Boot (`nve prerender` + `NativeInstantBootStore`, `2ef5a78`) are landed; remaining work is wiring `FrameArena` bump-reset allocation into hot `nve-dioxus`/`nve-engine` per-frame paths.

---

## P1 — UI & Engine Maturity (from [ui-maturity-audit.md](./ui-maturity-audit.md))

- [ ] **Rich Graphics (Skia Gradients & Cached Skia SVG)**
  - Closed-form interruptible springs (`SpringSpec`), `TransitionSpec`, Web native CSS `transition:` + `prefers-reduced-motion`, Skia `MotionDriver` (`e6a44f1`), and typed 2D `Transform2D` (`c09dd54`) are landed; remaining work is linear/radial gradients and cached vector SVG rendering in `SkiaBackend`.

- [ ] **Text Input, IME, BiDi & Mobile Host Autofill/Keyboard Wiring**
  - Subtree `ErrorBoundary`, `AutofillHint`/`InputMode` + Web autofill sync, and `KeyboardInsets` + Web `visualViewport` + `KeyboardAvoidingView` are landed (`38b351c`); remaining work is unified IME `CompositionStart/Update/End` + caret screen-rect candidate-popup feedback, BiDi run/caret mapping & grapheme-safe selection in Skia, and native Apple/Android host wiring for `AutofillHint`/`KeyboardInsets`.

- [ ] **Desktop OS Polish, Live Layout Inspector & Native AccessKit Parity**
  - Locale-aware `DatePicker`/`DateRangePicker` (`c69cdd0`) is landed; remaining work is OS drag-and-drop (internal + file drop), native menu bar / context menus / system tray, live visual layout/recomposition overlay inspector, and full native AccessKit/UIA/AT-SPI accessibility tree sync.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Unified Multiplayer Collaboration (`nve-sync`: `Room` + `Presence` + `use_synced_state`)**
  - Ergonomic `use_synced_state` / `use_synced_list` / `use_presence` hooks bridging server-authoritative thin-client rooms (0-JS Web & Native SDUI) and offline-first local CRDT peers (`packages/core/interop/src/crdt`) in the same live collaborative room, with ephemeral 20–30Hz cursors/selections/online presence.

- [ ] **Self-Documenting Protocol & Format Naming Cleanup**
  - Replace cryptic 4-letter format acronyms (`NVT1`, `NVD1`, `NVC1`, `NVERELOC`, `NVWS1`, `NVB1`, `NWS1`, `NVE_MET1`, etc.) across code, wire headers, and docs with clear human-readable names (`NveTransportPacket`, `NveFrameDelta`, `NveCrdtDelta`, `NveRelocContainer`, `NveWebSnapshot`, `NveWebState`, etc.) as part of the transport/SDUI upgrade wave.

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Core modules (`nve-ui`, `nve-router`, `nve-host-services` + `nve-clipboard` + `nve-system-env`, Environment `TraitKey`s, live `NVC1` CRDT peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) and live `/examples/studio/` GitHub Pages deploy are landed; next step is integrating opt-in `<NativeView<CameraPreview>>` / `BlurSurface` in Studio.
