# Architecture & Design Decisions

## 1. Core Thesis

`native-view-engine` separates **UI authoring**, **mutation transport**, and **platform rendering** along one narrow, zero-overhead boundary: a versioned, flat binary `CommandBuffer` and `EventRing` (`nve-abi` / `include/nve_abi.h`).

This eliminates React Native's JS reconciler, JSI per-prop crossings, `RawProps` string parsing, and `folly::dynamic` serialization while preserving **real platform-native views** (iOS `UIView`, Android `View`) and **native RN Fabric ecosystem compatibility** — and lets the exact same binary IR drive **Skia** (macOS, Windows, Linux, iOS, Android), **Web DOM/WASM**, and **Headless** test hosts.

## 2. Layer Stack

| Layer | Crate / Dir | Responsibility |
| :--- | :--- | :--- |
| **Universal IR & C ABI** | `crates/nve-abi`, `cxx/nve-fabric-bridge/include/nve_abi.h` | Flat binary `CommandBuffer` encoder/decoder, `EventRing`, `#[repr(C)]` FFI structs, and `NveAppVTable` for Tier-2 hot-swappable app modules. |
| **Typed Primitives** | `crates/nve-components` | Cross-platform primitives (`View`, `Text`, `ScrollView`, `Image`, `TextInput`, `Pressable`, `SkiaCanvas`, `Custom`), Yoga/Taffy-aligned `LayoutStyle`, and `ViewStyle`. |
| **Core Engine** | `crates/nve-engine` | `RenderBackend` trait, `Surface` & `NodeId` lifecycle, `FrameTiming` 6-stage profiler + Perfetto trace export, 2-tier hot-reload client & `cdylib` loader. |
| **Frontend Adapter** | `crates/nve-dioxus` | `dioxus-core` `WriteMutations` adapter emitting batched `CommandBuffer`s, event dispatch back into `VirtualDom`, and Tier-1 RSX template hot-reload hook. |
| **Fabric C++ Bridge** | `cxx/nve-fabric-bridge` | Thin C++20 adapter insulating Rust from RN `FrameworksGuard.h` / `UmbrellaGuard.h`. Maps `NodeId -> ShadowNode`, writes typed props, commits `ShadowTree`, intercepts `RawEvent` via `EventListener` + mock `jsi::Runtime` recorder (zero Hermes). |
| **Skia Backend** | `crates/nve-backend-skia` | Cross-platform GPU renderer (macOS, Windows, Linux, iOS, Android) consuming the same `CommandBuffer`, computing Flexbox layout, producing a retained display list, and painting via Skia (`skia-safe` feature). |
| **Web Backend** | `crates/nve-backend-web` | `wasm32` DOM backend applying the same `CommandBuffer` directly to browser DOM nodes + CSS Flexbox. |
| **Headless Backend** | `crates/nve-backend-headless` | Pure-Rust `taffy` Flexbox layout + `MutationLog` + ASCII/JSON tree inspector for <50ms tests, complemented by headless C++ Fabric tests for bit-exact Yoga/Differentiator parity. |
| **Mobile Shells** | `hosts/ios-shell`, `hosts/android-shell` | Prebuilt native Fabric host apps that load/hot-swap compiled Rust app modules (`app_ui.dylib` / `libapp_ui.so`) without Xcode/Gradle rebuilds. |
| **Dev Tooling** | `crates/nve-cli` | `nve doctor`, `nve dev`, `nve inspect`, `nve bench`, and future `nve codegen` / `nve prebuild-fabric`. |

## 3. Architectural Decision Records (ADRs)

### ADR-001: Single Batched Binary `CommandBuffer` Over Per-Call FFI
- **Decision:** Every frame serializes all mutations into one contiguous byte buffer (`u32` header + TLV opcodes) handed to the backend in a single call (`submit_frame`).
- **Why:** Eliminates chatty FFI/JNI crossings, removes `folly::dynamic` and string-key prop lookup, makes every backend (Fabric C++, Skia, Web WASM, Headless) implement one identical decoder, and enables deterministic replay/time-travel debugging.

### ADR-002: Framework-Agnostic Engine With Dioxus First
- **Decision:** `nve-engine` has zero dependency on `dioxus-core`. `nve-dioxus` plugs into `nve-engine` by translating `WriteMutations` into `CommandBuffer` ops.
- **Why:** We get production-grade `rsx!`, signals, and template hot reload on day 1, while keeping the core compile graph tiny (<300ms) and leaving the door open for a custom zero-VDOM reactive frontend or non-Rust frontends later.

### ADR-003: Zero-Hermes Fabric + Mock `jsi::Runtime` Event Recorder
- **Decision:** Boot RN `UIManager` with a no-op `RuntimeExecutor` and `startEmptySurface`. Capture native events via `EventDispatcher::addListener` (`EventListener` returning `true` to suppress JS dispatch). Downcast core `EventPayload` types directly; for arbitrary 3rd-party RN components whose `ValueFactoryEventPayload` only implements `asJSIValue(jsi::Runtime&)`, pass a lightweight recording `jsi::Runtime` subclass that writes fields straight into `EventRing`.
- **Why:** 0ms Hermes startup, 0MB JS heap, and 100% event compatibility with third-party native RN Fabric components. JS-only RN libraries can still live in explicit opt-in `<RnIsland>` surfaces.

### ADR-004: Hybrid Native + Skia Multi-Platform Strategy
- **Decision:** Use RN Fabric for real native platform views on iOS/Android, and Skia for macOS/Windows/Linux desktop *and* optional mobile Skia surfaces (`SkiaCanvas` or full-app Skia backend).
- **Why:** Gives macOS/Windows/Linux instant native desktop preview windows with sub-second iteration while preserving native `UIView`/Android `View` fidelity and RN ecosystem access on mobile.
