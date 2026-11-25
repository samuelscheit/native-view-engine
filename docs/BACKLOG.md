# Backlog

Open roadmap for `native-view-engine`, ordered from active P0 foundations to P1 architecture & ergonomics and P2/flagship work. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md).

---

## P0 — `nve-docs` Site Assembly, CLI & GitHub Pages CI (In Flight)

- [ ] **Docs Site Assembly, `cargo nve docs` CLI, 5 Live Playgrounds, Native Docs App & GitHub Pages CI (Docs Phases 2B / 3B / 4)** *(in flight)*
  - Implement `cargo nve docs build|dev|check` in `crates/nve-cli/src/docs.rs` (MDRSX compile + per-route `WebBackend::render_ssr` static HTML + `search.nvs1` + `sitemap.xml` + `wasm-release` progressive hydrator + live dev watcher).
  - Build `examples/nve-docs-site` with all 5 live interactive playgrounds (`ui-playground`, `virtual-list-100k`, `router-sandbox`, `canvas-draw1`, `crdt-simulator`) running as both static+hydrated web site AND native desktop/mobile app (`export_nve_app!`).
  - Ship `.github/workflows/docs.yml` with link/schema checks, JS-off SSR + JS-on progressive hydration Chrome E2E, visual screenshot inspection, and GitHub Pages deployment.

---

## P1 — Host `<NativeView>` Registries & Host Glue

- [ ] **Post-Wave-13 Host Services & Multi-Window Integration Glue (`hosts/skia-shell`)** *(in flight)*
  - Wire `SERVICE_WINDOW = 0x0002` into `DesktopWindowHost` (`NveHostCallbacks::invoke` → `WindowManager` + `HostWakeEvent::WindowCommand` + `NVH1` lifecycle pushes) and bind `NativeHostServices` (`NveHostApi::bind_host` + live OS theme/locale pushes on `CHANNEL_ID_THEME`/`CHANNEL_ID_LOCALE`).

- [ ] **Host-Side `NativeViewRegistry` Mount Adapters (`hosts/*`, `crates/nve-backend-web`)**
  - Mount registered `NVNP`/`NVNC` `<NativeView>` specs (`BlurSurface`, `WebView`, `VideoPlayer`, `WebElement`) across Apple (`apple-common`: `NSVisualEffectView` / `UIBlurEffect` / `WKWebView`), Windows (`Mica` / `WebView2`), Android, Qt 6, GTK4, SwiftUI, Flutter, and Web (`"web.<tag>"` passthrough for `<video>`, `<audio>`, `<svg>`, `<iframe>`, `<canvas>`, `<input>`, Web Components inside Flex layout).

---

## P1 — Architectural DRY, Single-Source ABI Lock & Zero-Copy Hot Paths (Wave 14)

- [ ] **Zero-Copy Loader & Typed `nve-dioxus` Style Props (`nve-engine`, `nve-dioxus`, `nve-ui`)** *(in flight)*
  - Eliminate per-frame `.to_vec()` copies in `crates/nve-engine/src/loader.rs` (`render` and `drain_emitted_events`).
  - Add direct typed style props (`TypedStyle`) in `nve-dioxus` & `nve-ui` to eliminate the `StyleBuf` string-format → `apply_typed_attr_with_raw` string-reparse round-trip.

- [ ] **Single-Source Cross-Language Wire Codecs & Delete Legacy Fallbacks (`nve-codegen`, `cxx`, `hosts/*`)** *(in flight — Wave 14B)*
  - Make `*_direct` entry points required and delete legacy `set_props` / ring `dispatch_events` fallback paths (`NveSurfaceController.swift`, `nve_rn_component.cpp`, `macos-shell/main.mm`, `windows-shell/main.cpp`).
  - Remove redundant `MethodChannel.invokeMethod` in Flutter `surface_widgets.dart` when direct FFI already ran; unify Kotlin/Java `NVE_MET1` readers (`NveSceneBuffer.kt`, `NveMainActivity.java`) and fix Kotlin `NveBinValue.kt`/`NveSurfaceState.kt` per-update `ByteArrayOutputStream`/`copyOf` allocations; generate single-source `EventRing` / `NVE_DRAW1` codecs and deduplicate FNV-1a across languages.

- [ ] **Apple Shell DRY Consolidation & Oversized C++ Splits (`hosts/apple-common`, `cxx`, `hosts/android-shell`)** *(in flight — Wave 14B)*
  - Deduplicate `hosts/ios-shell/RCTNveComponentView.mm` & `hosts/macos-shell/RCTNveMacComponentView.mm` (plus shared `main.mm` CLI/pair/window plumbing) into `hosts/apple-common/`.
  - Split `mock_jsi_recorder.hpp` (1,250L), `nve_host_runtime.hpp` (1,170L), and `NveAndroidSurfaceJni.cpp` (1,229L) below 650 lines.

---

## Flagship / Vision (Remaining Work Only)

- [ ] **NVE Studio (Meta Workbench)**
  - Add thin-client pairing UI for Flutter and SwiftUI shells (Skia, Web, macOS, Windows WinSock2, iOS, Android, Qt 6, and GTK4 already landed).
- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Wire live multi-peer `NVC1` CRDT delta exchange over paired `NVT1` transport sessions and migrate inline Studio UI chrome onto `nve-ui`, `nve-router`, `nve-sdk`, and Environment Traits.
