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

- [ ] **Adopt Measured Allocator & Persistence Architecture (`FrameArena` + `MmapOffsetArena` / Hardware-Hashed `NVERELOC`)**
  - Benchmark experiment (`bf6f52c`) proved `FrameArena` (`3.55×` at 50k nodes), pre-validated `NVERELOC` (`1.18×` faster than owned `Vec` in RAM), and `MmapOffsetArena` (`2.23 ns` / `39.6 µs` `msync` incremental edits, `41.8 µs` open) and identified serial FNV-1a as the full-verify bottleneck; promote `FrameArena` into `nve-engine`/`nve-dioxus`, replace serial blob FNV-1a with hardware CRC32C, and standardize local state persistence on `MmapOffsetArena` (or `NVERELOC` + WAL).

---

## P1 — UI & Engine Maturity (from [ui-maturity-audit.md](./ui-maturity-audit.md))

- [ ] **Layout & RTL Parity (`flex-basis`, CSS Grid, `position: sticky`, `aspect-ratio`, RTL Logical Props)**
  - Expose Taffy's CSS Grid (`display: grid`, `grid-template-columns`, `minmax`, spanning), `flex-basis`, `aspect-ratio`, `position: sticky`, and RTL logical layout mirroring (`padding-inline-start`, selective icon mirroring) cleanly alongside Wave C1's `nve-abi` schema update.

- [ ] **Animation & Rich Graphics (`transform`, Gradients, Interruptible Springs, Style Transitions, Skia SVG)**
  - Add typed `transform` (`translate`/`scale`/`rotate`), linear/radial gradients, interruptible velocity-preserving spring physics (`ReducedMotion`-aware), style transitions, and cached vector SVG rendering in `SkiaBackend`.

- [ ] **Text Input, IME, BiDi, Autofill/OTP, Soft Keyboard Insets & UI `ErrorBoundary`**
  - Cross-platform `autocomplete`/`textContentType` autofill/OTP hints, unified IME `CompositionStart/Update/End` + caret screen-rect candidate-popup feedback, BiDi run/caret mapping & grapheme-safe selection in Skia, cross-platform `KeyboardInsets` trait, and subtree UI `ErrorBoundary` with fallback + recovery.

- [ ] **Desktop OS Polish, `DatePicker`, Live Layout Inspector & Native AccessKit Parity**
  - OS drag-and-drop (internal + file drop), native menu bar / context menus / system tray, locale-aware `DatePicker`/`DateRangePicker` (built on `nve-i18n`'s `CalendarDate`/`first_day_of_week`), live visual layout/recomposition overlay inspector, and full native AccessKit/UIA/AT-SPI accessibility tree sync.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Core modules (`nve-ui`, `nve-router`, `nve-host-services` + `nve-clipboard` + `nve-system-env`, Environment `TraitKey`s, live `NVC1` CRDT peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) and live `/examples/studio/` GitHub Pages deploy are landed; next step is integrating opt-in `<NativeView<CameraPreview>>` / `BlurSurface` in Studio.
