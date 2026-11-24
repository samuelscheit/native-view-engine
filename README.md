# Native View Engine (`nve`)

A native-speed, multi-backend cross-platform UI engine with Rust HTML-like ergonomics, real platform-native views via React Native Fabric, GPU-rendered Skia targets for desktop & mobile, and Web DOM/WASM — architected around sub-second iteration time.

## Quick Start

```bash
# 1. Verify toolchain, mobile SDKs, RN Fabric checkout, and build caches
cargo nve doctor

# 2. Run the full multi-crate test suite (<300ms incremental loop)
cargo test --workspace

# 3. Inspect a Dioxus rsx! app's ShadowTree, Flexbox rects, and binary CommandBuffer
cargo nve inspect

# 4. Run frame-pipeline micro & 1,000-node benchmarks (with Chrome/Perfetto trace export)
cargo nve bench --trace bench.trace.json

# 5. Start the 2-tier hot-reload dev loop (RSX template patch + C-ABI dylib hot-swap)
cargo nve dev --example counter
```

## Architecture at a Glance

```text
Rust App (Dioxus rsx! + signals via nve-dioxus, or any custom frontend)
  │  Emits one batched binary CommandBuffer per frame (nve-abi)
  ▼
nve-engine (Surface, NodeId table, Stage Profiler, 2-Tier Hot Reload)
  │  RenderBackend trait / C ABI (include/nve_abi.h)
  ├──► cxx/nve-fabric-bridge ──► React Native Fabric C++ (iOS UIView / Android View / RN Ecosystem)
  ├──► nve-backend-skia      ──► Skia + Flexbox (macOS / Windows / Linux / iOS / Android)
  ├──► nve-backend-web       ──► WASM -> Browser DOM + CSS Flexbox
  └──► nve-backend-headless  ──► Pure-Rust Taffy layout + mutation recorder (<50ms tests)
```

## Documentation

- [Architecture & ADRs](docs/architecture.md)
- [Iteration Speed & Tooling Playbook](docs/iteration-speed.md)
