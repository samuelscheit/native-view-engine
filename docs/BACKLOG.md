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

- [ ] **Pluggable Camera Package — Remaining Host Adapters (Apple `AVFoundation` + VisionCamera v5 iOS, Web `getUserMedia`, Windows `MediaCapture`, Linux PipeWire)** *(in flight — Apple + Web next)*
  - Wire `"nve.camera.preview"` (`NVNP`/`NVNC`/`NVCF`) into `hosts/apple-common` (shared `AVFoundation` for macOS + standalone iOS + pluggable VisionCamera v5 iOS bridge), `crates/nve-backend-web` (`getUserMedia` → `<video>`), Windows `MediaCapture` / Media Foundation, and Linux PipeWire / V4L2 (`CameraPreview` spec + `NVCF` + Android VisionCamera v5 / CameraX already landed).

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - All core modules (`nve-ui`, `nve-router`, `nve-sdk`, Environment `TraitKey`s, live `NVC1` CRDT peer transport, 100k Fenwick stream, Docs editor, Meta Workbench, `NWS1` v2 + `.nvewal`) are landed; next step is deploying Studio live at `/examples/studio/` on GitHub Pages and integrating `<NativeView<CameraPreview>>` / `BlurSurface` once host adapters land.
