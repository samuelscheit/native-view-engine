# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md).

---

## P1 — Multi-Device Backends, Camera Polish & Host Parity

- [ ] **Pluggable Camera Package — Thin SwiftUI/Flutter Wrappers & Live Emulator Camera Verification**
  - All primary camera adapters are landed (`CameraPreview` spec + `NVCF`, Android VisionCamera v5 + real Jetpack `CameraX` `PreviewView`, Apple `AVFoundation` + VisionCamera v5 iOS bridge, Web `getUserMedia` `<video>`, C++20 Windows `MediaCapture` + Linux PipeWire/V4L2); remaining follow-ups are thin SwiftUI & Flutter camera wrappers and live Android Emulator virtual-scene verification once Android SDK cmdline-tools are installed.

- [ ] **Finish Zero-Copy `packages/core/abi` & Extract `packages/core/wire` (Option A)**
  - Move all byte-stream codecs (`CommandBufferBuilder`, `CommandDecoder`, `encode_events`/`decode_events`, `BinValueView`, `NVH1`) out of `packages/core/abi` into `packages/core/wire` (network/disk transport only) and migrate in-process render (`nve-dioxus` / `nve-engine` / backends / hosts) to pure zero-copy `#[repr(C)]` `NveCommandSlice`/`NveSceneView` slabs.

- [ ] **Unify `packages/framework/sdk` into Standalone `packages/modules/<name>/` (Option A)** *(in flight)*
  - Delete monolithic `packages/framework/sdk/`, extract tiny `packages/core/host-services/`, move every capability into uniform `packages/modules/<name>/`, and merge duplicate `camera` into `packages/modules/camera/`.

- [ ] **SSR Responsive `@media` Compilation, Reusable 3-Way Reconciler & Pre-Hydration Event Replay** *(Phase 1 in flight)*
  - Zero-serialization `MediaQuery`/`ResponsiveStylePatch` (`#[repr(C)]`) + generic 3-way `StateReconciler` + Web `@media` SSR emission + ~800 B pre-hydration event capture/replay (`<a href>` native before hydration, SPA after) + zero-CLS mobile `DocsShell`.

- [ ] **TV, Watch & Spatial Platform Backends — Phase 3 (`cargo nve build` Tier-3 Target Recipes & Web TV)**
  - Phase 1 (`DeviceFormFactor` traits, `SpatialFocusEngine`, Android TV D-Pad) and Phase 2 (standalone `packages/modules/tv` + real `tvOS`, `watchOS`, `visionOS`, and `iOS` SDK builds + static `NveInterfaceResolver` in `hosts/swiftui-kit`) are landed; remaining work is adding `aarch64-apple-{tvos,visionos,watchos}` staticlib recipes in `nve-cli` and Web TV D-Pad key handling in `nve-backend-web`.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Core modules (`nve-ui`, `nve-router`, `nve-sdk`, Environment `TraitKey`s, live `NVC1` CRDT peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) and live `/examples/studio/` GitHub Pages deploy are landed; next step is integrating opt-in `<NativeView<CameraPreview>>` / `BlurSurface` in Studio.
