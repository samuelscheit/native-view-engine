# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md).

---

## P1 — Multi-Device Backends, Camera Polish & Host Parity

- [ ] **Pluggable Camera Package — Thin SwiftUI/Flutter Wrappers & Live Emulator Camera Verification**
  - All primary camera adapters are landed (`CameraPreview` spec + `NVCF`, Android VisionCamera v5 + real Jetpack `CameraX` `PreviewView`, Apple `AVFoundation` + VisionCamera v5 iOS bridge, Web `getUserMedia` `<video>`, C++20 Windows `MediaCapture` + Linux PipeWire/V4L2); remaining follow-ups are thin SwiftUI & Flutter camera wrappers and live Android Emulator virtual-scene verification once Android SDK cmdline-tools are installed.

- [ ] **Finish Zero-Copy `packages/core/abi` & Extract `packages/core/wire` (Wave C1 — Option A1)**
  - Move all byte-stream codecs (`NVE1` `CommandBuffer`, `NVEV` `EventRing`, `BinValueView`, `NVH1`, `NK1`) out of `packages/core/abi` into `packages/core/wire` (network/disk transport only) and migrate in-process render (`nve-dioxus` / `nve-engine` / backends / hosts) to pure zero-copy `#[repr(C)]` `NveCommandSlice`/`NvePropSlice`/`NveSceneView` slabs.

- [ ] **Eliminate Remaining In-Process Codecs & Add Codegen Zero-Copy `RelocSlice` Wire Views (Wave C3)**
  - Replace `NVE_DRAW1` with a codegen-locked `#[repr(C)]` `NveDrawOp` slab (`&[NveDrawOp]`), delete the 3× duplicated `NVNP`/`NVNC` codecs in favor of direct `NvePropSlice`/`NveValue`, unify `NVCF` to one `#[repr(C)]` struct, and replace `NVT2`/`NVL1`/`NVW1`/`NVN1`/`NVDS`/`NVE_MET1` in-process codecs with direct typed / `#[repr(C)]` structs.
  - Extend `packages/core/codegen` to generate zero-copy aligned `#[repr(C)]` + `RelocSlice` views (`view_from_aligned_bytes` / `as_bytes`) for legitimate disk/network formats in `packages/core/wire` instead of hand-written Rust encode/decode code.

- [ ] **TV, Watch & Spatial Platform Backends — Phase 3 (`cargo nve build` Tier-3 Target Recipes & Web TV)**
  - Phase 1 (`DeviceFormFactor` traits, `SpatialFocusEngine`, Android TV D-Pad) and Phase 2 (standalone `packages/modules/tv` + real `tvOS`, `watchOS`, `visionOS`, and `iOS` SDK builds + static `NveInterfaceResolver` in `hosts/swiftui-kit`) are landed; remaining work is adding `aarch64-apple-{tvos,visionos,watchos}` staticlib recipes in `nve-cli` and Web TV D-Pad key handling in `nve-backend-web`.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Core modules (`nve-ui`, `nve-router`, `nve-host-services` + `nve-clipboard` + `nve-system-env`, Environment `TraitKey`s, live `NVC1` CRDT peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) and live `/examples/studio/` GitHub Pages deploy are landed; next step is integrating opt-in `<NativeView<CameraPreview>>` / `BlurSurface` in Studio.
