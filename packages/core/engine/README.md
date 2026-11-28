# `nve-engine`

Framework-agnostic core runtime (`packages/core/engine`, **Core Runtime layer**) defining `StoreBackend`, `SurfaceRunner`, `NveApp`, crash-atomic `Frame0File` + `StateFile` persistence, 6-stage `FrameProfiler`, and hot-reload primitives.

## Key Public API

- `StoreBackend`, `StoreWireReplica`, `Viewport`, `LayoutRect`, `BackendApplyStats`, `BackendError`
- `app::NveApp`, `persistence::{Frame0File, StateFile, write_file_atomic}`
- `layout::{StoreLayoutEngine, TextMeasurer, HeuristicTextMeasurer}`, `profiler`, `hotreload::HotPatchMsg`

```rust
use nve_engine::{LayoutRect, Viewport};

let rect = LayoutRect { x: 0.0, y: 0.0, width: 120.0, height: 48.0 };
let vp = Viewport::default();
assert_eq!(rect.width, 120.0);
assert!(vp.scale_factor > 0.0);
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-tables`, `serde`, and non-`wasm32` `libloading`.
- **Must NOT depend on**: `dioxus*`, `web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`, runtime `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-engine
```
