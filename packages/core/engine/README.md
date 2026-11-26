# `nve-engine`

Framework-agnostic core runtime (`packages/core/engine`, **Core Runtime layer**) defining `RenderBackend`, `SurfaceRunner`, `NveApp`, `FrameArena`, `xxh64`, crash-atomic `MmapOffsetArena`, 6-stage `FrameProfiler`, and hot-reload primitives.

## Key Public API

- `RenderBackend`, `SurfaceRunner`, `Viewport`, `LayoutRect`, `BackendApplyStats`, `BackendError`
- `app::NveApp`, `frame_arena::FrameArena`, `hash::{xxh64, Xxh64Hasher}`, `mmap_arena`, `instant_boot`
- `layout::{LayoutEngine, TextMeasurer, HeuristicTextMeasurer}`, `profiler`, `hotreload::HotPatchMsg`

```rust
use nve_engine::hash::xxh64;

let id: u64 = xxh64(b"nve.surface.main");
assert_ne!(id, 0);
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-components`, `serde`, and non-`wasm32` `libloading`.
- **Must NOT depend on**: `dioxus*`, `web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`, runtime `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-engine
```
