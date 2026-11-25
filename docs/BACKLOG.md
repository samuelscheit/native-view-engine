# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md).

---

## P1 — Multi-Device Backends, Camera Polish & Host Parity

- [ ] **Pluggable Camera Package — Thin SwiftUI/Flutter Wrappers & Live Emulator Camera Verification**
  - All primary camera adapters are landed (`CameraPreview` spec + `NVCF`, Android VisionCamera v5 + real Jetpack `CameraX` `PreviewView`, Apple `AVFoundation` + VisionCamera v5 iOS bridge, Web `getUserMedia` `<video>`, C++20 Windows `MediaCapture` + Linux PipeWire/V4L2); remaining follow-ups are thin SwiftUI & Flutter camera wrappers and live Android Emulator virtual-scene verification once Android SDK cmdline-tools are installed.

- [ ] **TV, Watch & Spatial Platform Backends (`tvOS`, `watchOS`, `visionOS`, `Android TV` / `Fire TV`)** *(Phase 1 landed; Phase 2 next)*
  - **Phase 2 (Multi-Apple `hosts/swiftui-kit` — `tvOS`, `watchOS`, `visionOS`)**:
    - Extend `hosts/swiftui-kit/Package.swift` with `.tvOS(.v16)`, `.watchOS(.v9)`, `.visionOS(.v1)`.
    - Add `#if os(tvOS)` (`.focusable()` + `onMoveCommand` + Siri Remote), `#if os(watchOS)` (pure-SwiftUI view path without UIKit/AppKit + `.digitalCrownRotation`), and `#if os(visionOS)` (`.hoverEffect()` + `.glassBackgroundEffect()` + ornaments), guard `WKWebView` with `#if !os(tvOS) && !os(watchOS)`, and add static-linked `NveInterfaceResolver` for App Store `tvOS`/`watchOS`/`visionOS` targets.
  - **Phase 3 (`cargo nve build` Tier-3 Target Recipes & Web TV / Tizen / webOS)**:
    - Add `aarch64-apple-tvos`, `aarch64-apple-visionos`, and `aarch64-apple-watchos` staticlib build recipes in `crates/nve-cli`, plus Web TV D-Pad key handling in `crates/nve-backend-web`.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - All core modules (`nve-ui`, `nve-router`, `nve-sdk`, Environment `TraitKey`s, live `NVC1` CRDT peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) are landed; next step is deploying Studio live at `/examples/studio/` on GitHub Pages and integrating `<NativeView<CameraPreview>>` / `BlurSurface` once host adapters land.
