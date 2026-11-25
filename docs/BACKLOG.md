# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md).

---

## P0 — Public Landing Page, Live Examples Showcase & GitHub Pages Deploy

- [ ] **Landing Page + Docs + Interactive Example Apps on GitHub Pages**
  - Build a dedicated NVE landing page at `/` (with `/docs/...` for the documentation suite and `/examples/{studio,counter,...}` hosting live interactive WASM apps like **Studio Workspace** and **Counter** directly in the browser) deployed automatically via `.github/workflows/docs.yml`.

---

## P1 — Host `<NativeView>` Registries & Single-Source Wire Codecs

- [ ] **Host-Side `NativeViewRegistry` Mount Adapters (`hosts/*`, `crates/nve-backend-web`)** *(in flight — Web + Apple + C++ core)*
  - Mount registered `NVNP`/`NVNC` `<NativeView>` specs (`BlurSurface`, `WebView`, `VideoPlayer`, `WebElement`) across Apple (`apple-common`: `NSVisualEffectView` / `UIBlurEffect` / `WKWebView`), Windows (`Mica` / `WebView2`), Android, Qt 6, GTK4, SwiftUI, Flutter, and Web (`"web.<tag>"` passthrough for `<video>`, `<audio>`, `<svg>`, `<iframe>`, `<canvas>`, `<input>`, Web Components inside Flex layout).

- [ ] **Pluggable Camera Package (`CameraPreview` `<NativeView>` + VisionCamera v5 & Native Adapters)**
  - Add typed `CameraPreview` `NativeViewSpec` (`"nve.camera.preview"`) paired with `SERVICE_CAMERA = 0x0023` (`crates/nve-sdk/src/camera.rs`) and zero-copy native frame-buffer hooks.
  - Ship an optional **VisionCamera v5** adapter (`react-native-vision-camera` Nitro backend) for RN Fabric / Expo iOS + Android hosts, plus zero-RN native adapters for standalone hosts: shared `AVFoundation` (`hosts/apple-common` for macOS + standalone iOS), Android `CameraX` (`hosts/compose-kit`), Windows `MediaCapture` / Media Foundation, Linux PipeWire / V4L2, and Web `getUserMedia` → `<video>`.

- [ ] **Single-Source Cross-Language Wire Codecs (`nve-codegen`, `cxx`, `hosts/*`)** *(in flight — Wave 14D)*
  - Generate single-source `EventRing` / `NVE_DRAW1` codecs from `nve-codegen` and deduplicate FNV-1a implementations across Rust, C++, Swift, Dart, Kotlin/Java, and TypeScript.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Adopt `nve-router`, `nve-sdk`, and Environment `TraitKey`s across Studio (live multi-peer `NVC1` CRDT delta transport + `nve-ui` chrome migration already landed).
