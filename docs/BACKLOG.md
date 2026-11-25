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

- [ ] **Single-Source Cross-Language Wire Codecs (`nve-codegen`, `cxx`, `hosts/*`)** *(in flight — Wave 14D)*
  - Generate single-source `EventRing` / `NVE_DRAW1` codecs from `nve-codegen` and deduplicate FNV-1a implementations across Rust, C++, Swift, Dart, Kotlin/Java, and TypeScript.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Adopt `nve-router`, `nve-sdk`, and Environment `TraitKey`s across Studio (live multi-peer `NVC1` CRDT delta transport + `nve-ui` chrome migration already landed).
