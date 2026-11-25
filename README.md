# Native View Engine (`nve`)

**Native View Engine (`nve`)** is a native-speed, multi-backend cross-platform UI engine written in Rust and C++20. Write declarative UI once with Rust HTML-like ergonomics (Dioxus 0.7 `rsx!` + fine-grained signals, or any custom frontend) and drive **real platform-native views**, **GPU/Skia canvases**, and **Web WASM DOM** from a single zero-copy binary ABI — with sub-second iteration and zero overhead.

## Big Goals

1. **Zero-Overhead Binary Architecture**: Cross the Rust ↔ host boundary via a single modular C ABI (`nve_get_interface(uint32_t)`), one batched binary `CommandBuffer` per frame, and a zero-copy `#[repr(C)]` `NveValue` bump-arena FFI sink — zero Hermes/JS VM, zero JSON bridge, zero per-prop FFI chatter.
2. **Real Native Views Everywhere, From One Core**: Ship first-class native hosts on **macOS (AppKit)**, **iOS (UIKit / RN Fabric)**, **Windows (Win32 / Direct2D / DirectWrite + `react-native-windows` Fabric)**, **Linux (GTK4 / Pango)**, **Cross-Platform Qt 6**, **Android (JNI / Jetpack Compose)**, **SwiftUI**, **Flutter (`dart:ffi`)**, **Desktop/Mobile Skia**, and **Web (WASM DOM)** — all sharing one DRY C++20 host runtime (`nve_host_runtime.hpp`) and one Apple host core (`hosts/apple-common/`).
3. **Composable, Switchable Layout Engines**: Zero-cost generic `LayoutEngine` + `TextMeasurer` traits so apps can choose pure-Rust **Taffy**, real **React Native Yoga** (`nve-layout-yoga`), or custom layout engines at compile time (zero vtable overhead) or runtime.
4. **Instant Cold Start (`< 5 ms`) & Universal Native State Save/Restore**: Freeze-dry the last rendered binary `CommandBuffer` alongside a pointer-free `mmap` relocatable state arena (`MmapSnapshot` + `RelocatableStateArena`) so apps paint Frame 0 directly from virtual memory before logic even mounts — surviving crashes, reboots, ASLR, and app upgrades across every OS and WASM.
5. **Sub-Second 2-Tier Hot Reload & `NVE Studio`**: Live-patch styles/templates in `< 5 ms` (Tier-1) and hot-swap compiled native `.dylib`/`.dll`/`.so` logic in `< 300 ms` (Tier-2) without losing state, powering **NVE Studio** — a self-hosting live workbench with X-Ray layout/FFI inspection, side-by-side multi-host preview, deterministic `CommandBuffer` time-travel scrubbing, and ~10 KB/s binary delta live pairing.
6. **Universal Framework Interop & 5-Target Bi-Directional Codegen**: Embed NVE inside existing React Native, Flutter, SwiftUI, Jetpack Compose, or Web apps — and embed native framework islands inside NVE — with shared gesture arbitration, intrinsic layout negotiation, and zero-copy channel pub/sub (`nve-interop`, `nve-codegen`).
7. **Flagship Real-World Proof**: Build a **local-first workspace & messaging suite** (Linear × Notion × Slack class in ~15 MB / `< 40 MB` RAM, `< 50 ms` cold start, 120fps 100k-item views, native OS feel on every device, instant Web WASM share links, and native-speed `nve_get_interface` plugins) using NVE Studio.

## Measured Baselines

Real numbers from `cargo bench -p nve-bench`, `nve_fabric_bridge_bench` (Release, RN Yoga + JSI), and `crates/nve-engine` snapshot tests:

| Pipeline Stage | Measured Latency | Throughput / Notes |
| :--- | ---: | :--- |
| `nve_get_interface` FFI lookup / render slice | **1.44 – 1.54 ns** | Zero-cost C ABI dispatch |
| Direct Bump-Arena FFI JSI event dispatch | **137.5 ns / event** | **7.27 M/s** (~13x faster than JSON, 0 alloc) |
| Rust `NveValueRef` zero-copy nested field read | **21.9 ns** | Borrowed straight from C++ arena |
| Binary `CommandBuffer` decode (1,000 nodes) | **117 µs** | ~800 MiB/s |
| Dioxus incremental state diff | **1.10 µs** | Tier-1 hot-patch: **2.22 µs** |
| 1,000-row Flexbox layout (`Yoga` / `Taffy`) | **1.86 – 2.51 ms** / **2.14 – 3.05 ms** | Switchable via `LayoutEngine` trait |
| Skia software rasterize (card / 1,000 nodes) | **158 µs** / **1.50 ms** | `winit` + `softbuffer` live desktop window |
| Cold `mmap` Frame-0 + state restore (100 rows) | **82 – 136 µs** | `29 µs` raw `mmap` open, 0 copies |

## Architecture at a Glance

```text
Rust App (Dioxus 0.7 rsx! via nve-dioxus, or any frontend implementing NveApp)
  │  Exposes nve_get_interface(uint32_t): Surface | Layout | Component | Dev
  │  Batched binary CommandBuffer + Zero-Copy Direct FFI NveValue Arena / EventRing
  ▼
nve-engine (NveApp, Generic LayoutEngine/TextMeasurer, MmapSnapshot, Profiler, 2-Tier Hot Reload)
  ├──► cxx/nve-fabric-bridge (C++20 Yoga/JSI/Fabric + nve_host_runtime.hpp)
  │     ├──► hosts/apple-common + hosts/{macos-shell,ios-shell}  (AppKit & UIKit / RN Fabric)
  │     ├──► hosts/windows-shell                                 (Win32/Direct2D/DirectWrite & RNW Fabric)
  │     ├──► hosts/qt-shell                                      (Qt 6 QWidget + QFontMetricsF)
  │     ├──► hosts/gtk-shell                                     (GTK4 + Pango)
  │     └──► hosts/android-shell                                 (JNI + NDK)
  ├──► hosts/{swiftui-kit,flutter-plugin,compose-kit}            (SwiftUI, Flutter dart:ffi, Jetpack Compose)
  ├──► nve-backend-skia                                          (Desktop winit/softbuffer + optional skia-safe)
  ├──► nve-backend-web + hosts/web-shell                         (WASM -> Browser DOM + CSS Flexbox)
  ├──► nve-backend-headless + nve-layout-yoga                    (Generic Taffy / real RN Yoga + sub-ms tests)
  └──► nve-codegen + nve-interop                                 (5-target codegen + multi-framework islands)
```

## Quick Start

```bash
# 1. Run the unified verification runner across all locally installed toolchains
./scripts/check-all.sh --quick

# 2. Run Rust tests & Criterion benchmarks
cargo test --workspace
cargo bench -p nve-bench

# 3. Build example cdylibs (counter + flagship Aether Workspace / NVE Studio POC)
cargo build -p nve-example-counter -p nve-studio-workspace

# 4. Launch in the native macOS AppKit host (or Qt 6 / Skia Desktop / Web WASM)
cmake -S hosts/macos-shell -B hosts/macos-shell/build -G Ninja && cmake --build hosts/macos-shell/build
./hosts/macos-shell/build/nve_macos_shell --dylib target/debug/libnve_example_counter.dylib

# 5. Launch in the cross-platform Skia desktop preview window
cargo run -p nve-backend-skia --features desktop --example preview -- \
  target/debug/libnve_example_counter.dylib --no-ws

# 6. Start the 2-tier hot-reload dev daemon
cargo nve dev --example counter
```

## Documentation & Roadmap

- [Architecture & ADRs](docs/architecture.md)
- [Iteration Speed & Tooling Playbook](docs/iteration-speed.md)
- [Examples & Flagship Vision](examples/README.md)
- [Prioritized Roadmap & Backlog](docs/BACKLOG.md)
- [Agent & Engineering Invariants](AGENTS.md)
