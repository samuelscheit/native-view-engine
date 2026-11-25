# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md).

---

## P1 — Multi-Device Backends, Camera Polish & Host Parity

- [ ] **Pluggable Camera Package — Thin SwiftUI/Flutter Wrappers & Live Emulator Camera Verification**
  - All primary camera adapters are landed (`CameraPreview` spec + `NVCF`, Android VisionCamera v5 + real Jetpack `CameraX` `PreviewView`, Apple `AVFoundation` + VisionCamera v5 iOS bridge, Web `getUserMedia` `<video>`, C++20 Windows `MediaCapture` + Linux PipeWire/V4L2); remaining follow-ups are thin SwiftUI & Flutter camera wrappers and live Android Emulator virtual-scene verification once Android SDK cmdline-tools are installed.

- [ ] **Unified `packages/` Monorepo Layout & `nve-abi` Option 1A Zero-Copy Raw Memory Refactor (Wave B)** *(Wave B1 selective feature flags in flight)*
  - Move `crates/*` into unified `packages/{core,backends,framework,tools}/` once Wave A3 & B1 merge (zero `crates/` vs `packages/` split).
  - Slim `nve-abi` to codegen-driven pure `#[repr(C)]` zero-copy raw-memory / `mmap` slabs (`NveSceneView`, `NveEventBatch`, `NvePropSlice`, `NveCommandSlice`), deleting duplicate `value/bin_*`, unused `ffi_views`, and local in-process codecs; isolate transport/disk/WASM serialization in optional `packages/core/wire/`.

- [ ] **TV, Watch & Spatial Platform Backends — Phase 3 (`cargo nve build` Tier-3 Target Recipes & Web TV)**
  - Phase 1 (`DeviceFormFactor` traits, `SpatialFocusEngine`, Android TV D-Pad) and Phase 2 (standalone `packages/modules/tv` + real `tvOS`, `watchOS`, `visionOS`, and `iOS` SDK builds + static `NveInterfaceResolver` in `hosts/swiftui-kit`) are landed; remaining work is adding `aarch64-apple-{tvos,visionos,watchos}` staticlib recipes in `nve-cli` and Web TV D-Pad key handling in `nve-backend-web`.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - All core modules (`nve-ui`, `nve-router`, `nve-sdk`, Environment `TraitKey`s, live `NVC1` CRDT peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) are landed; next step is deploying Studio live at `/examples/studio/` on GitHub Pages and integrating `<NativeView<CameraPreview>>` / `BlurSurface` once host adapters land.
