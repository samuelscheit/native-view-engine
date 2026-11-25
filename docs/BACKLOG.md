# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md).

---

## P1 — Multi-Device Backends, Camera Polish & Host Parity

- [ ] **Pluggable Camera Package — Thin SwiftUI/Flutter Wrappers & Live Emulator Camera Verification**
  - All primary camera adapters are landed (`CameraPreview` spec + `NVCF`, Android VisionCamera v5 + real Jetpack `CameraX` `PreviewView`, Apple `AVFoundation` + VisionCamera v5 iOS bridge, Web `getUserMedia` `<video>`, C++20 Windows `MediaCapture` + Linux PipeWire/V4L2); remaining follow-ups are thin SwiftUI & Flutter camera wrappers and live Android Emulator virtual-scene verification once Android SDK cmdline-tools are installed.

- [ ] **Unified `packages/` Monorepo Layout — Apple Native Views Follow-up**
  - Wave B1 (selective `default-features = false` feature flags), Wave B2 (`nve-abi` Option 1A codegen-locked zero-copy `#[repr(C)]` `NvePropEntry`/`NvePropSlice`/`NveCommandSlice` + `feature = "wire"` isolation), and Wave B3 (unified `packages/{core,backends,framework,tools,modules}/` monorepo layout with `packages/modules/{camera,tv,pair}` folded into root `Cargo.toml`) are landed.
  - Remaining: move `hosts/apple-common/NveAppleNativeViews.{h,mm}` into `packages/modules/native-views/apple/`.

- [ ] **SSR Responsive `@media` Compilation & Pre-Hydration Event Capture/Replay**
  - Compile declarative responsive layout/style overrides and `show_on: SizeClassMask` into CSS `@media` rules so mobile SSR pages render with `0.000` CLS and zero hydration flicker, paired with a tiny inline pre-hydration event capture/replay shim (pending design sign-off).

- [ ] **TV, Watch & Spatial Platform Backends — Phase 3 (`cargo nve build` Tier-3 Target Recipes & Web TV)**
  - Phase 1 (`DeviceFormFactor` traits, `SpatialFocusEngine`, Android TV D-Pad) and Phase 2 (standalone `packages/modules/tv` + real `tvOS`, `watchOS`, `visionOS`, and `iOS` SDK builds + static `NveInterfaceResolver` in `hosts/swiftui-kit`) are landed; remaining work is adding `aarch64-apple-{tvos,visionos,watchos}` staticlib recipes in `nve-cli` and Web TV D-Pad key handling in `nve-backend-web`.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Core modules (`nve-ui`, `nve-router`, `nve-sdk`, Environment `TraitKey`s, live `NVC1` CRDT peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) and live `/examples/studio/` GitHub Pages deploy are landed; next step is integrating opt-in `<NativeView<CameraPreview>>` / `BlurSurface` in Studio.
