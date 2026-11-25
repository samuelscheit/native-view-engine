# Backlog

Prioritized roadmap for `native-view-engine`, ordered from active foundational work to platform polish and flagship apps.

---

## P0 — Web SSR/Progressive Hydration, `nve-docs` Framework & Generated-File Hygiene (In Flight)

- [ ] **Web Engine CSS Optimization, Keyed Diffing, Semantic HTML5 SSR & Progressive Hydration (`crates/nve-backend-web`, `hosts/web-shell`)** *(in flight — Docs Phase 1A)*
  - **Option 5 Hybrid CSS Architecture** (adversarially verified: `-86.3%` CSS bytes on Studio Frame 0, `723 -> 269` flush crossings, `~95%+` Blink MPC / Stylo sharing): deferred end-of-frame `dirty_styles` flush, static `.nve` base reset (`font-family:inherit` replacing `font:inherit`), bounded content-addressed `.s<base36(fnv1a64)>` structural class cache via batched `CSSStyleSheet::replaceSync`/`insertRule`, volatile/continuous inline escape hatch (`> 2048px` `VirtualList` spacers, fractional px, animated opacity, `MAX_DYNAMIC_RULES = 4096`), `.nve-tl` text-leaf inheritance, and fix bogus HTML attributes for `font-weight`/`text-align`/`box-shadow`/`overflow`.
  - **Keyed minimal-diff `SetChildren`**: replace `set_text_content(None)` child wipe with O(N) head/tail-trimmed DOM reconciliation preserving focus, IME, and scroll offsets.
  - **Semantic HTML5 SSR (`render_ssr`)**: map roles/levels/`href` to `<h1>..<h6>`, `<nav>`, `<main>`, `<article>`, `<a href>`, `<pre><code>`, plus deterministic `<style data-nve-base>` + `<style data-nve-ssr data-nve-hashes="...">`, Open Graph `<meta>`, and `sitemap.xml`.
  - **True zero-flicker progressive/island DOM hydration (`hydrate_into`)**: adopt existing `[data-nve-id]` DOM nodes without wiping `innerHTML`, root event delegation replacing per-node `Closure` fan-out, mismatch diagnostics + subtree recovery, and island triggers (`load` / `visible` via `IntersectionObserver` / `interaction` with event capture & replay / `idle`).

- [x] **`nve-router::web` Browser History API Adapter (`crates/nve-router/src/web.rs`)** *(landed — `0827eab`)*
  - Cross-platform `HistoryBackend` (`MemoryHistoryBackend` + `wasm32` `BrowserHistoryBackend`), `WebHistoryAdapter` with GitHub Pages `base_path` normalization, `pushState`/`replaceState`/`popstate` loop-free sync, browser-accurate link click interception, and `#anchor` scrolling.

- [ ] **Zero-Dep Pure-Rust `.md`/`.mdx` Compiler & Binary `NVS1` Search Index (`crates/nve-mdx`)** *(in flight — Docs Phase 1C)*
  - Standard `.md`/`.mdx` authoring: CommonMark/GFM + frontmatter + Nextra-style `_meta.toml` + fenced directives (`:::callout`, `:::tabs`, `:::steps`, `:::cards`, `:::demo`) + declarative `<Component prop="val" num={42} />` tags + ` ```rsx live ` compiled playground blocks, compile-time schema/link/anchor validator, pure-Rust multi-language syntax highlighter, and binary `NVS1` full-text search index (`< 1 ms` `⌘K` search).

- [ ] **Reusable `crates/nve-docs` Framework, Full 6-Section Docs Suite, Native Docs App & GitHub Pages CI (Docs Phases 2–4)**
  - Build Fumadocs × Linear dark/light docs framework 100% on `nve-ui` + `nve-router` + `nve-sdk` (`cargo nve docs build|dev|check`).
  - Author all 6 documentation sections (`docs/content/`) + 5 live interactive playgrounds (`ui-playground`, `virtual-list-100k`, `router-sandbox`, `canvas-draw1`, `crdt-simulator`) + native desktop/mobile `examples/nve-docs-site` binary.
  - Ship `.github/workflows/docs.yml` with link/schema checks, JS-off SSR + JS-on progressive hydration Chrome E2E, visual screenshot verification, and GitHub Pages deployment.

- [ ] **Enforce `.generated.<ext>` Naming (and Java `generated/` Package) Across All Codegen Targets** *(in flight)*
  - Rename all 8 non-Java generated ABI files to `*.generated.{h,dart,ts}`, move `NveCoreAbi.java` & `NveSceneBuffer.java` into package `dev.nve.android.generated`, update `nve-codegen` + `cargo nve codegen abi --check` + `AGENTS.md`, and keep zero legacy shim headers.

---

## P1 — Unified Environment/Trait System, Native Primitive Escape Hatches & Host Glue

- [ ] **Post-Wave-13 Host Services & Multi-Window Integration Glue (`hosts/skia-shell`, `hosts/windows-shell`)**
  - Wire `SERVICE_WINDOW = 0x0002` into `DesktopWindowHost` (`NveHostCallbacks::invoke` → `WindowManager` + `HostWakeEvent::WindowCommand` + `NVH1` lifecycle pushes) and bind `NativeHostServices` (`NveHostApi::bind_host` + live OS theme/locale pushes on `CHANNEL_ID_THEME`/`CHANNEL_ID_LOCALE`).
  - Live-verify `NveWinWindowManager` + `crates/nve-sdk/src/native/windows.rs` (Win32 Clipboard, `CredReadW`/`CredWriteW`, `ShellExecuteW`, Registry theme, `GetUserDefaultLocaleName`) natively on Windows 11 (`samuel-pc`).

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

## Completed Milestones (Waves 12–13)

- [x] **Multi-Window Lifecycle Primitives (`21d86ac`)**: `SurfacePresentation`, `WindowManager` (modal blocking, cascading sheet/popover close, cancelable `CloseRequested` veto), binary `NVW1` codec, and multi-window host managers across Skia `winit`, macOS AppKit, iOS/iPadOS `UIWindowScene`, Windows Win32, GTK4, Qt 6, and Web WASM (`ACTIVE_SHELLS` + `BroadcastChannel`).
- [x] **`NVE_IFACE_HOST = 5` & Tier-1 `nve-sdk` (`0aeee16`)**: Single zero-JSON binary host-capability C ABI (`NVH1`, `NVT2`, `NVL1`) across all 5 generated targets + zero-dep Clipboard, Filesystem/File Picker, Secure Store, Linking, live Theme/Locale `NveChannelBus` bridge, Notifications, and Haptics.
- [x] **`nve-ui` & Extended ARIA/`tabIndex` (`0995977`)**: 15 accessible, themeable Dioxus components (`Stack`, `Row`, `Col`, `Separator`, `Card`, `Button`, `IconButton`, `Badge`, `Input`, `Checkbox`, `Switch`, `Tabs`, `Dialog`/`Modal`, `Popover`/`Tooltip`, `DropdownMenu`, `Toast`, 100k-row Fenwick `VirtualList`) with `UiTheme`, `StyleBuf`, `FocusTrap`/`RovingFocus`, and 12 typed ARIA/`tabIndex` attributes in `nve-dioxus`.
- [x] **`nve-router` (`2c63272` + `0827eab`)**: Zero-proc-macro `define_routes!`, stack/tab/modal/guard/deep-link state machines, binary `NVN1` `navigation` channel bridge, Dioxus `RouterProvider`/`Outlet`/`Link`, and `nve-router::web` History API adapter.
- [x] **Swift/Dart Direct FFI Codegen DRY, `NveDirectEncodable` & `offset_of!` ABI Checks (`66832d2`)**: Split `NveDirectFfi.swift` (exemption removed), deleted generated Swift `pinnedBuffers` and duplicate Dart `NveBinWriter` in favor of shared zero-alloc arenas/codecs, added `NveDirectEncodable` with `StaticString` keys, and cross-checked Rust `offset_of!` against C11 `_Static_assert(offsetof(...))` in `nve_abi.h`.
- [x] **Direct3D 12 Skia GPU, RNW 0.88 & Live Windows 11 WinSock2 Verification (`58106d1`, `020f6d0`)**: Verified live on Windows 11 Pro (MSVC 19.51, Cargo 1.100): Skia D3D12 (32/32), full Rust workspace (257/257, 0 Clippy warnings), `cxx/nve-fabric-bridge` + `ws2_32`, `hosts/windows-shell` Standalone & RNW 0.88 Fabric (0 `/W4` warnings), and live cross-process `nve.exe pair` → `nve_windows_shell.exe --pair-connect` byte-for-byte identical across WebSocket, TCP, and UDP.

---

## Flagship / Vision

- [ ] **NVE Studio (Meta Workbench)**
  - Self-hosting multi-host workbench rendering AppKit, WinUI, Qt 6, GTK4, iOS/Android Fabric, Flutter Canvas, SwiftUI, Skia, and WASM surfaces side by side.
  - *Landed*: Bidirectional `NVT1`/`NVD1` binary delta transport (`InputEvents = 11`, `5.18 KB/s` @ 60fps under 15% loss + 20% reorder), `nve pair` / `nve dev --pair` daemon with live `SyncTelemetry` pushing + `NVE_PAIR_READY` + live Chrome E2E, zero-cdylib `--pair-connect` thin clients across **Skia Desktop**, **Web Browser (`?pair=ws://...`)**, **macOS AppKit**, **Windows Direct2D/RNW (WinSock2)**, **iOS**, **Android**, **Qt 6 (Widgets + Quick)**, and **GTK4** (via modular zero-dep C++20 `nve::PairClient`), in-app **Multi-Viewport Meta Workbench**, and native `NVE_DRAW1` `SkiaCanvas` rendering across all 9 hosts.
  - *Remaining*: Thin-client pairing UI for Flutter/SwiftUI shells.

- [ ] **Flagship Local-First Workspace & Messaging App**
  - Full Linear × Notion × Slack suite in one ~15 MB binary (`<50 ms` cold start, `<40 MB` RAM) with real native OS text, IME, accessibility, and `<Island>` embeds.
  - *Landed*: 100k-row Fenwick virtualized stream, Notion-class **Block-Doc Editor** (`ActiveView::Docs` with Markdown export + Chrome E2E), crash-safe local-first **`MmapSnapshot` + `.nvewal` WAL persistence** (`NWS1` v2 + FNV-1a64), **zero-dependency binary CRDT engine** (`crates/nve-interop/src/crdt/` `NVC1` + `StudioCrdtDoc` 5-peer partition-heal convergence), live Taffy/Yoga switcher, mobile portrait collapse, `Cmd+K` palette, interactive `TextInput` with drag selection + zero-dep native clipboard, PNG avatars, `NVE_DRAW1` sparkline, `<Island>` sandbox, panic-isolated `nve_get_interface` plugin host, `285.8 KiB` gzipped WASM release build, and native accessibility/IME parity across all 9 hosts.
  - *Remaining*: Live multi-peer CRDT transport wiring over paired `NVT1` sessions and adopting `nve-ui`/`nve-router`/`nve-sdk`/Environment Traits.
