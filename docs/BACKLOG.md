# Backlog

Open roadmap for `native-view-engine`, ordered from active P0 foundations to P1 architecture & ergonomics and P2/flagship work. Completed milestones live in [docs/CHANGELOG.md](./CHANGELOG.md).

---

## P0 — Web SSR/Progressive Hydration & `nve-docs` Framework (In Flight)

- [ ] **Reusable `crates/nve-docs` Framework (`crates/nve-docs`)** *(in flight — Docs Phase 2A)*
  - Fumadocs × Linear dark/light 3-column shell built 100% on `nve-ui` + `nve-router` + `nve-sdk` + `nve-mdx` (sidebar, sticky TOC, `⌘K` `NVS1` search dialog, syntax-highlighted code blocks with Clipboard copy, `<LiveDemo>` `PlaygroundRegistry`, responsive `< 960pt` mobile drawer).

- [ ] **Complete 6-Section NVE Documentation Content Suite (`docs/content/`)** *(in flight — Docs Phase 3A)*
  - Grounded `.mdx` + `_meta.toml` pages covering Getting Started, Core Architecture, Building Apps, all 10 Platform Hosts, Interop/Codegen/Pairing, and CLI/Benchmarks/Upstream Reference.

- [ ] **Docs Site Assembly, `cargo nve docs` CLI, 5 Live Playgrounds, Native Docs App & GitHub Pages CI (Docs Phases 2B / 3B / 4)**
  - Implement `cargo nve docs build|dev|check` in `crates/nve-cli/src/docs.rs` (MDRSX compile + per-route `WebBackend::render_ssr` static HTML + `search.nvs1` + `sitemap.xml` + `wasm-release` progressive hydrator + live dev watcher).
  - Build `examples/nve-docs-site` with all 5 live interactive playgrounds (`ui-playground`, `virtual-list-100k`, `router-sandbox`, `canvas-draw1`, `crdt-simulator`) running as both static+hydrated web site AND native desktop/mobile app (`export_nve_app!`).
  - Ship `.github/workflows/docs.yml` with link/schema checks, JS-off SSR + JS-on progressive hydration Chrome E2E, visual screenshot inspection, and GitHub Pages deployment.

---

## P1 — Unified Environment/Trait System, Native Primitive Escape Hatches & Host Glue

- [ ] **Post-Wave-13 Host Services & Multi-Window Integration Glue (`hosts/skia-shell`)** *(in flight)*
  - Wire `SERVICE_WINDOW = 0x0002` into `DesktopWindowHost` (`NveHostCallbacks::invoke` → `WindowManager` + `HostWakeEvent::WindowCommand` + `NVH1` lifecycle pushes) and bind `NativeHostServices` (`NveHostApi::bind_host` + live OS theme/locale pushes on `CHANNEL_ID_THEME`/`CHANNEL_ID_LOCALE`).

- [ ] **Unified Extensible Environment / Trait System (`crates/nve-interop`, `crates/nve-dioxus`, `crates/nve-ui`)**
  - Generalize `SurfaceEnvironment` into a typed, zero-string-lookup `TraitKey` / `Environment` system (SwiftUI `EnvironmentValues` × Compose `CompositionLocal` × CSS `@media`/`@container`):
    - Built-in traits: `PlatformOs`, `HostToolkit`, `SizeClass` (`Compact | Medium | Expanded`), scoped `ContainerSize` (`@container` queries on Skia/Fabric/Web), `Orientation`, `SafeAreaInsets`, `ScaleFactor`, `PointerKind` (`Fine | Coarse | None`), `CanHover`, `HasHardwareKeyboard`, `ColorScheme`, `ReducedMotion`, `HighContrast`, `TextScale`, `UiDensity`, `Locale`.
    - User-defined custom `TraitKey`s + `<TraitScope>` subtree overrides.
    - Breakpoint-only fine-grained subscriptions (`use_trait::<SizeClass>()` avoids 60fps re-renders on 1px window drags) + zero-alloc `env.resolve().when(...).default(...)` & `<PlatformOnly>` / `<TraitSwitch>`.

- [ ] **First-Class Native Platform Primitive Escape Hatches (`<NativeView>`, `NativeViewRegistry`)**
  - Typed `NativeView<P: NativeViewSpec>` + `NativeViewHandle` (imperative zero-JSON calls via `Command::DispatchCommand`) with graceful cross-platform fallback RSX slot when unregistered on a host.
  - Host-side `NativeViewRegistry` across Apple (`apple-common`), Windows, Android, Qt 6, GTK4, SwiftUI, Flutter, and Web (`"web.<tag>"` passthrough mounting real `<video>`, `<audio>`, `<svg>`, `<iframe>`, `<canvas>`, `<input type="date">`, Web Components directly inside Flex layout).
  - Starter native primitives: `BlurSurface` (`NSVisualEffectView` / `UIBlurEffect` / Win32 Mica / CSS `backdrop-filter`), `WebView` (`WKWebView` / `WebView2` / Android `WebView` / `<iframe>`), `VideoPlayer`, and `WebElement`.

---

## P1 — Architectural DRY, Single-Source ABI Lock & Zero-Copy Hot Paths (Wave 14)

- [ ] **Complete ABI Field-Offset Verification & Zero-Copy Engine/Dioxus/Codegen Hot Paths (`nve-abi`, `nve-engine`, `nve-dioxus`, `nve-codegen`)**
  - Add compile-time `size_of`/`align_of`/`offset_of!` assertions for every `#[repr(C)]` vtable and struct in `crates/nve-abi/src/{ffi.rs,host.rs,scene/types.rs}` cross-checked against `nve_abi.generated.h` / `nve_scene_abi.generated.h` `offsetof(...)`; add layout `static_assert`s for `NVE_SNAP` (`nve_snapshot.hpp`) and `nve_pair_client.hpp`; fix `TRANSPORT_MAGIC` (`"NT"` vs `"NVT1"`) doc mismatch in `sync_transport/protocol.rs`.
  - Eliminate per-frame `.to_vec()` copies in `crates/nve-engine/src/loader.rs` (`render` and `drain_emitted_events`).
  - Add typed style props directly in `nve-dioxus` to eliminate the `StyleBuf` string-format → `apply_typed_attr_with_raw` string-reparse round-trip (keeping `nve-abi`'s fixed POD structs untouched).
  - Remove `serde_json` / `JSONObject` from generated RN props/commands/events (`import_rn.rs`, `export_rn.rs`), replace `[String: Any]` / `buildMap<String, Any?>` in `export_swiftui.rs` & `export_compose.rs` with typed direct-FFI encoders, prune unused `export_*_json` dead entry points, and modularize >1,100-line codegen files (`core_abi.rs`, `parser.rs`, `emit_*.rs`).

- [ ] **Single-Source Cross-Language Wire Codecs & Delete Legacy Fallbacks (`nve-codegen`, `cxx`, `hosts/*`)**
  - Make `*_direct` entry points required and delete legacy `set_props` / ring `dispatch_events` fallback paths (`NveSurfaceController.swift`, `nve_rn_component.cpp`, `macos-shell/main.mm`, `windows-shell/main.cpp`).
  - Remove redundant `MethodChannel.invokeMethod` in Flutter `surface_widgets.dart` when direct FFI already ran; unify Kotlin/Java `NVE_MET1` readers (`NveSceneBuffer.kt`, `NveMainActivity.java`) and fix Kotlin `NveBinValue.kt`/`NveSurfaceState.kt` per-update `ByteArrayOutputStream`/`copyOf` allocations; generate single-source `EventRing` / `NVE_DRAW1` codecs and deduplicate FNV-1a across languages.

- [ ] **Apple Shell DRY Consolidation, Oversized C++ Splits & Build Hygiene (`hosts/apple-common`, `cxx`, `hosts/android-shell`)**
  - Deduplicate `hosts/ios-shell/RCTNveComponentView.mm` & `hosts/macos-shell/RCTNveMacComponentView.mm` (plus shared `main.mm` CLI/pair/window plumbing) into `hosts/apple-common/`.
  - Split `mock_jsi_recorder.hpp` (1,250L), `nve_host_runtime.hpp` (1,170L), and `NveAndroidSurfaceJni.cpp` (1,229L) below 650 lines.
  - Clear Cargo 1.100 manifest unused-dependency warnings (`hosts/web-shell/Cargo.toml` workspace dep ref for `nve-studio-workspace` + `crates/nve-bench/Cargo.toml` dev-deps) and add `.gitattributes` enforcing `eol=lf` for generated ABI artifacts.

---

## P2 — Tier-2/3 `nve-sdk` Hardware & Storage Capabilities

- [ ] **Expanded `nve-sdk` Native Modules (`crates/nve-sdk`)**
  - First-party zero-JSON capabilities via `NVE_IFACE_HOST = 5`: SQLite / Embedded KV Store, Custom Font Loading, Camera, Audio Playback/Recording, Geolocation, Device Sensors, Native Share Sheet, and In-App WebBrowser (`ASWebAuthenticationSession` / Custom Tabs).

---

## Flagship / Vision (Remaining Work Only)

- [ ] **NVE Studio (Meta Workbench)**
  - Add thin-client pairing UI for Flutter and SwiftUI shells (Skia, Web, macOS, Windows WinSock2, iOS, Android, Qt 6, and GTK4 already landed).
- [ ] **Flagship Local-First Workspace & Messaging App (`examples/studio-workspace`)**
  - Wire live multi-peer `NVC1` CRDT delta exchange over paired `NVT1` transport sessions and migrate inline Studio UI chrome onto `nve-ui`, `nve-router`, `nve-sdk`, and Environment Traits.
