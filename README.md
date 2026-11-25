# Native View Engine (`nve`)

**Native View Engine (`nve`)** is a native-speed, multi-backend cross-platform UI engine written in Rust and C++20. Write declarative UI once with Rust HTML-like ergonomics (Dioxus 0.7 `rsx!` + fine-grained signals, or any custom frontend) and drive **real platform-native views**, **GPU/Skia canvases**, and **Web WASM DOM** from a single zero-copy binary ABI — with sub-second iteration and zero overhead.

## Big Goals

1. **Zero-Overhead Binary Architecture**: Cross the Rust ↔ host boundary via a single modular C ABI (`nve_get_interface(uint32_t)`), one batched binary `CommandBuffer` per frame, and a zero-copy `#[repr(C)]` `NveValue` bump-arena FFI sink — zero Hermes/JS VM, direct binary bridge, zero per-prop FFI chatter.
2. **Real Native Views Everywhere, From One Core**: Ship first-class native hosts on **macOS (AppKit)**, **iOS (UIKit / RN Fabric)**, **Windows (Win32 / Direct2D / DirectWrite + `react-native-windows` Fabric)**, **Linux (GTK4 / Pango)**, **Cross-Platform Qt 6**, **Android (JNI / Jetpack Compose)**, **SwiftUI**, **Flutter (`dart:ffi`)**, **Desktop/Mobile Skia**, and **Web (WASM DOM)** — all sharing one DRY C++20 host runtime (`nve_host_runtime.hpp`) and one Apple host core (`hosts/apple-common/`).
3. **Composable, Switchable Layout Engines**: Zero-cost generic `LayoutEngine` + `TextMeasurer` traits so apps can choose pure-Rust **Taffy**, real **React Native Yoga** (`nve-layout-yoga`), or custom layout engines at compile time (zero vtable overhead) or runtime.
4. **Instant Cold Start (`< 5 ms`) & Universal Native State Save/Restore**: Freeze-dry the last rendered binary `CommandBuffer` alongside a pointer-free `mmap` relocatable state arena (`MmapSnapshot` + `RelocatableStateArena`) so apps paint Frame 0 directly from virtual memory before logic even mounts — surviving crashes, reboots, ASLR, and app upgrades across every OS and WASM.
5. **Sub-Second 2-Tier Hot Reload & `NVE Studio`**: Live-patch styles/templates in `< 5 ms` (Tier-1) and hot-swap compiled native `.dylib`/`.dll`/`.so` logic in `< 300 ms` (Tier-2) without losing state, powering **NVE Studio** — a self-hosting live workbench with X-Ray layout/FFI inspection, side-by-side multi-host preview, deterministic `CommandBuffer` time-travel scrubbing, and ~10 KB/s binary delta live pairing.
6. **Universal Framework Interop & 5-Target Bi-Directional Codegen**: Embed NVE inside existing React Native, Flutter, SwiftUI, Jetpack Compose, or Web apps — and embed native framework islands inside NVE — with shared gesture arbitration, intrinsic layout negotiation, and zero-copy channel pub/sub (`nve-interop`, `nve-codegen`).
7. **Flagship Real-World Proof**: Build a **local-first workspace & messaging suite** (Linear × Notion × Slack class in ~15 MB / `< 40 MB` RAM, `< 50 ms` cold start, 120fps 100k-item views, native OS feel on every device, instant Web WASM share links, and native-speed `nve_get_interface` plugins) using NVE Studio.

## Measured Baselines

Real numbers from `benchmarks/baseline.json` (`cargo bench -p nve-bench` + Release `nve_fabric_bridge_bench` on Apple Silicon):

| Pipeline Stage | Measured Latency | Throughput / Notes |
| :--- | ---: | :--- |
| `nve_get_interface` FFI lookup / render slice | **0.98 – 1.33 ns** | Zero-cost C ABI dispatch (`catch_unwind` panic-safe) |
| Direct `NveSceneView` dirty read (3 of 1,000 nodes) | **24.82 ns** | 64-byte cache-line `NveRenderNode`, zero decode |
| Rust `NveValueRef` zero-copy nested field read | **17.09 ns** | Borrowed straight from C++ bump arena |
| `GestureArbitrator` nested scroll arbitration | **64.51 ns** | Zero-alloc inline `SurfaceChain` + `FastU32Map` |
| `CommandBuffer` encode / streaming decode (1,000 nodes) | **21.17 µs** / **18.05 µs** | Stack-packed encode & zero-alloc `CommandRef` (~4.7 GiB/s) |
| C++20 Fabric incremental patch (10 / 1,000 nodes) | **7.12 µs** | Direct `NveSceneView` publish: **7.54 µs** |
| Dioxus incremental state diff / Tier-1 hot-patch | **931 ns** / **1.33 µs** | Real RSX AST hot-patch diff `< 2.5 ms` |
| 1,000-row Flexbox layout (`Taffy` / C++ standalone) | **1.77 ms** / **451 µs** | 1,000-row hit-test: **3.62 µs** |
| Web DOM HTML SSR (100 / 500 rows) | **12.45 µs** / **65.05 µs** | Studio WASM bundle: **285.8 KiB gzipped** (`wasm-release`) |
| Skia display list (1,000 nodes) / card rasterize | **962 µs** / **136.6 µs** | Software + Apple Metal GPU swapchains |
| Cold `mmap` Frame-0 + state restore (100 rows) | **82 – 136 µs** | `29 µs` raw `mmap` open + append-only `.nvewal` |

## Architecture at a Glance

```text
Rust App (Dioxus 0.7 rsx! via nve-dioxus, or any frontend implementing NveApp)
  │  Exposes nve_get_interface(uint32_t): Surface | Layout | Component | Dev
  │  Binary CommandBuffer, 64B NveSceneView, NVE_DRAW1 vectors, NK1/NVP1/NVT1 wire codecs
  ▼
nve-engine (NveApp, Generic LayoutEngine/TextMeasurer, MmapSnapshot + WAL, 2-Tier Hot Reload)
  ├──► cxx/nve-fabric-bridge (C++20 Yoga/JSI/Fabric, nve_host_runtime.hpp, nve_draw_list.hpp)
  │     ├──► hosts/apple-common + hosts/{macos-shell,ios-shell}  (AppKit & UIKit / CoreGraphics / VoiceOver)
  │     ├──► hosts/windows-shell                                 (Win32/Direct2D/WIC/DirectWrite & RNW Fabric)
  │     ├──► hosts/qt-shell                                      (Qt 6 QWidget + Retained Qt Quick QSG)
  │     ├──► hosts/gtk-shell                                     (GTK4 + Pango + Cairo + AT-SPI)
  │     └──► hosts/android-shell                                 (JNI DirectByteBuffer Slabs + TalkBack)
  ├──► hosts/{swiftui-kit,flutter-plugin,compose-kit}            (SwiftUI, Flutter dart:ffi, Jetpack Compose)
  ├──► nve-backend-skia + hosts/skia-shell                       (Skia CPU/Metal/Vulkan + winit + IME + --pair-connect)
  ├──► nve-backend-web + hosts/web-shell                         (WASM -> DOM + ARIA/IME + <canvas> + ?pair=ws://)
  ├──► nve-backend-headless + nve-layout-yoga                    (Generic Taffy / real RN Yoga + sub-ms tests)
  └──► nve-codegen + nve-interop + nve-cli                       (6-target Nitro-style hybrid FFI + NVD1 live pairing)
```

## Quick Start

```bash
# 1. Run the unified verification runner across all locally installed toolchains
./scripts/check-all.sh --quick

# 2. Run Rust tests & Criterion benchmarks
cargo test --workspace
cargo bench -p nve-bench

# 3. Build example cdylibs (Kitchen Sink gallery + flagship Aether Workspace / NVE Studio)
cargo build -p nve-example-kitchen-sink -p nve-studio-workspace

# 4. Launch in the cross-platform Skia desktop shell (or capture a PNG screenshot)
cargo run -p nve-skia-shell -- \
  target/debug/libnve_studio_workspace.dylib --no-ws --width 1280 --height 800

# 5. Start the 2-tier hot-reload dev daemon (with optional live NVD1 pairing)
cargo nve dev --package nve-studio-workspace --pair

# 6. Or stream any app over bidirectional NVT1/NVD1 UDP + WebSocket pairing to zero-cdylib thin clients
#    (works simultaneously across Skia Desktop, macOS AppKit, Qt 6, GTK4, and Web Browser!)
cargo nve pair --package nve-studio-workspace --udp-port 8091 --ws-port 8092 --token dev-secret
cargo run -p nve-skia-shell -- --pair-connect 127.0.0.1:8091 --pair-transport udp --pair-token dev-secret
./hosts/macos-shell/build/nve_macos_shell --pair-connect 127.0.0.1:8092 --pair-transport ws --pair-token dev-secret
```

## Documentation & Roadmap

- [Architecture & ADRs](docs/architecture.md)
- [Iteration Speed & Tooling Playbook](docs/iteration-speed.md)
- [Examples & Flagship Vision](examples/README.md)
- [Prioritized Roadmap & Backlog](docs/BACKLOG.md)
- [Agent & Engineering Invariants](AGENTS.md)
