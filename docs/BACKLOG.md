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

- [ ] **Collapse 4 Persistence Formats → 2 (`NVERELOC` + `MmapOffsetArena`), Native Instant-Boot & Hot-Path `FrameArena`**
  - `XXH64`, `FrameArena`, and crash-atomic double-buffered `MmapOffsetArena` are now production primitives in `nve-engine` (`023d77a`); remaining work is promoting `NVERELOC` into `nve-engine` + `cxx/nve-fabric-bridge`, deleting legacy `NVE_SNAP` & `NVE_WAL`, adding generic `InstantBootApp` + `nve prerender` (`frame0.nvereloc` cold boot + `MmapOffsetArena` per-user state restore), and adopting `FrameArena` on hot `nve-dioxus`/`nve-engine` per-frame allocation paths.

---

## P1 — UI & Engine Maturity (from [ui-maturity-audit.md](./ui-maturity-audit.md))

- [ ] **Layout & RTL Parity (`flex-basis`, CSS Grid, `position: sticky`, `aspect-ratio`, RTL Logical Props)**
  - Expose Taffy's CSS Grid (`display: grid`, `grid-template-columns`, `minmax`, spanning), `flex-basis`, `aspect-ratio`, `position: sticky`, and RTL logical layout mirroring (`padding-inline-start`, selective icon mirroring) cleanly alongside Wave C1's `nve-abi` schema update.

- [ ] **Rich Graphics (`transform`, Skia Gradients & Cached Skia SVG)**
  - Closed-form interruptible springs (`SpringSpec`), `TransitionSpec`, Web native CSS `transition:` + `prefers-reduced-motion`, and Skia `MotionDriver` are landed (`e6a44f1`); remaining work is typed 2D `transform` (`translate`/`scale`/`rotate`), linear/radial gradients, and cached vector SVG rendering in `SkiaBackend`.

- [ ] **Text Input, IME, BiDi & Mobile Host Autofill/Keyboard Wiring**
  - Subtree `ErrorBoundary`, `AutofillHint`/`InputMode` + Web autofill sync, and `KeyboardInsets` + Web `visualViewport` + `KeyboardAvoidingView` are landed (`38b351c`); remaining work is unified IME `CompositionStart/Update/End` + caret screen-rect candidate-popup feedback, BiDi run/caret mapping & grapheme-safe selection in Skia, and native Apple/Android host wiring for `AutofillHint`/`KeyboardInsets`.

- [ ] **Desktop OS Polish, `DatePicker`, Live Layout Inspector & Native AccessKit Parity**
  - OS drag-and-drop (internal + file drop), native menu bar / context menus / system tray, locale-aware `DatePicker`/`DateRangePicker` (built on `nve-i18n`'s `CalendarDate`/`first_day_of_week`), live visual layout/recomposition overlay inspector, and full native AccessKit/UIA/AT-SPI accessibility tree sync.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Core modules (`nve-ui`, `nve-router`, `nve-host-services` + `nve-clipboard` + `nve-system-env`, Environment `TraitKey`s, live `NVC1` CRDT peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) and live `/examples/studio/` GitHub Pages deploy are landed; next step is integrating opt-in `<NativeView<CameraPreview>>` / `BlurSurface` in Studio.
