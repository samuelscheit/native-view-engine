# `nve-runtime`

Framework-agnostic host runtime for `native-view-engine`.

## Responsibilities

- Owns the per-surface lifecycle (`SurfaceRuntime` / `RuntimeBuilder`) over `NodeStore` and
  `nve_layout::LaidOutScene<DynLayoutEngine>`, with zero dependency on `dioxus-core` or any
  optional capability module.
- Publishes zero-copy `NveRuntimeSceneView` snapshots (`rows: *const NveNodeRow`,
  `rects: *const NveLayoutRect`, `text_slab`, `children_slab`, `dirty_indices`, `removed_ids`,
  and `NveTableView` descriptors) guarded by a per-surface reader/writer lock
  (`scene_acquire` / `scene_release`).
- Exposes generic `WakeBridge`, `RuntimeApp`, and `FrameTickHook` extension points so hosts can
  attach in-process Rust apps (such as `nve_dioxus::DioxusRuntimeApp`), cross-boundary `NveApp` /
  `nve_get_interface` modules, custom layout engines (`TaffyStoreLayout`, `YogaLayoutEngine`),
  and frame-tick hooks without coupling core to any UI framework or animation engine.
- Embeds `nve-wire` (`features = ["client"]`) behind default `feature = "wire"` (opt-outable via
  `--no-default-features`) to drive live `pair_connect`/`pair_disconnect`/`pair_is_connected`
  sessions on a single `current_thread` Tokio worker thread with exponential-backoff reconnect,
  automatic `KeyframeRequest` recovery, and `WakeBridge` integration.
- Exports the single versioned C ABI entry point `nve_runtime_get_interface(uint32_t abi_version)`
  returning `const NveRuntimeApi*`. Link from native C/C++ hosts via `cxx/cmake/NveRuntime.cmake`
  (`nve::runtime`) + `packages/core/host-cxx/include/nve/host/runtime_session.hpp`, or build on
  demand with `cargo rustc -p nve-runtime --lib --crate-type staticlib` (or `cdylib`).

```rust
use nve_engine::Viewport;
use nve_runtime::{RuntimeBuilder, SurfaceRuntime};

let mut rt: SurfaceRuntime = RuntimeBuilder::new()
    .surface_id(1)
    .viewport(Viewport { width: 800.0, height: 600.0, scale_factor: 1.0 })
    .build();
let _ = rt.tick(1.0 / 60.0);
let guard = rt.acquire_scene().expect("scene available");
assert_eq!(guard.view().schema_hash, nve_abi::NVE_RUNTIME_SCHEMA_HASH);
```
