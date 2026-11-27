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
- Exports the single versioned C ABI entry point `nve_runtime_get_interface(uint32_t abi_version)`
  returning `const NveRuntimeApi*`. Build shared or static host libraries on demand with:
  `cargo rustc -p nve-runtime --crate-type staticlib` or `--crate-type cdylib`.

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
