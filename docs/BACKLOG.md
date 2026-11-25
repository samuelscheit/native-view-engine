# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md).

---

## P0 — Public Landing Page, Live Examples Showcase & GitHub Pages Deploy

- [ ] **Landing Page + Docs + Interactive Example Apps on GitHub Pages**
  - Build a dedicated NVE landing page at `/` (with `/docs/...` for the documentation suite and `/examples/{studio,counter,...}` hosting live interactive WASM apps like **Studio Workspace** and **Counter** directly in the browser) deployed automatically via `.github/workflows/docs.yml`.

---

## P1 — Host `<NativeView>` Registries & Single-Source Wire Codecs

- [ ] **Remaining Host `NativeViewRegistry` Mount Adapters (`hosts/windows-shell`, `hosts/qt-shell`, `hosts/gtk-shell`, `hosts/swiftui-kit`, `hosts/flutter-plugin`, `hosts/compose-kit`)**
  - Extend `nve_native_view.hpp` (`NVNP`/`NVNC`) adapters to Windows (`Mica` / `WebView2`), Android/Compose, Qt 6, GTK4, SwiftUI, and Flutter (Web `"web.<tag>"` + `BlurSurface`/`WebView`/`VideoPlayer`, C++20 `nve_native_view.hpp`, and Apple `NSVisualEffectView`/`UIVisualEffectView`/`WKWebView` already landed).

- [ ] **Pluggable Camera Package (`CameraPreview` `<NativeView>` + VisionCamera v5 & Native Adapters)** *(in flight — Core + Android VisionCamera/CameraX)*
  - Add typed `CameraPreview` `NativeViewSpec` (`"nve.camera.preview"`) paired with `SERVICE_CAMERA = 0x0023` (`crates/nve-sdk/src/camera.rs`) and zero-copy native frame-buffer hooks.
  - Ship an optional **VisionCamera v5** adapter (`react-native-vision-camera` Nitro backend) for RN Fabric / Expo iOS + Android hosts, plus zero-RN native adapters for standalone hosts: shared `AVFoundation` (`hosts/apple-common` for macOS + standalone iOS), Android `CameraX` (`hosts/compose-kit`), Windows `MediaCapture` / Media Foundation, Linux PipeWire / V4L2, and Web `getUserMedia` → `<video>`.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - All core modules (`nve-ui`, `nve-router`, `nve-sdk`, Environment `TraitKey`s, live `NVC1` CRDT peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) are landed; next step is deploying Studio live at `/examples/studio/` on GitHub Pages and integrating `<NativeView<CameraPreview>>` / `BlurSurface` once host adapters land.
