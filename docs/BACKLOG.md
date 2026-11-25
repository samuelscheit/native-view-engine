# Backlog

Open roadmap for `native-view-engine`, ordered from active P0/P1 work to Flagship/Vision items. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md).

---

## P0 — Public Landing Page, Live Examples Showcase & GitHub Pages Deploy

- [ ] **Landing Page + Docs + Interactive Example Apps on GitHub Pages**
  - Build a dedicated NVE landing page at `/` (with `/docs/...` for the documentation suite and `/examples/{studio,counter,...}` hosting live interactive WASM apps like **Studio Workspace** and **Counter** directly in the browser) deployed automatically via `.github/workflows/docs.yml`.

---

## P1 — Host `<NativeView>` Registries & Single-Source Wire Codecs

- [ ] **Host-Side `NativeViewRegistry` Mount Adapters (`hosts/*`, `crates/nve-backend-web`)**
  - Mount registered `NVNP`/`NVNC` `<NativeView>` specs (`BlurSurface`, `WebView`, `VideoPlayer`, `WebElement`) across Apple (`apple-common`: `NSVisualEffectView` / `UIBlurEffect` / `WKWebView`), Windows (`Mica` / `WebView2`), Android, Qt 6, GTK4, SwiftUI, Flutter, and Web (`"web.<tag>"` passthrough for `<video>`, `<audio>`, `<svg>`, `<iframe>`, `<canvas>`, `<input>`, Web Components inside Flex layout).

- [ ] **Single-Source Cross-Language Wire Codecs (`nve-codegen`, `cxx`, `hosts/*`)**
  - Generate single-source `EventRing` / `NVE_DRAW1` codecs from `nve-codegen` and deduplicate FNV-1a implementations across Rust, C++, Swift, Dart, Kotlin/Java, and TypeScript.

- [ ] **Windows 11 (`samuel-pc`) Re-Verification of Wave 14A/14B/14C & Skia Glue**
  - Re-verify `NVE_STATIC_ASSERT` offset checks, split C++ headers (`mock_jsi_recorder.hpp`, `nve_host_runtime.hpp`), direct-only `hosts/windows-shell/main.cpp`, zero-copy `HotAppInstance`, and Tier-2/3 `nve-sdk` natively on MSVC 19.51 / RNW 0.88.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Adopt `nve-router`, `nve-sdk`, and Environment `TraitKey`s across Studio (live multi-peer `NVC1` CRDT delta transport + `nve-ui` chrome migration already landed).
