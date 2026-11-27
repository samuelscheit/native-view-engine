# `nve-abi`

Zero-copy `#[repr(C)]` C ABI and binary wire definitions (`packages/core/abi`, **Core Foundation layer**) shared across Rust engines, codegen targets, and every native/web host.

## Key Public API

- `ffi::{NveAppHandle, NveComponentApi, NveDevApi, NveBufferSlice}`, `host::{NveHostApi, NveHostCallbacks, NveHostStatus}`
- `scene::{NveRenderNode, NveSceneView, NveSceneTable, NveNodeMeta, NVE_PRIMARY_SLOT_NONE}`
- `table::{NveTableView, TableDesc, FieldDesc, FieldKind, compute_table_schema_hash}`
- `value::{NveValue, NveValueRef, NveDirectEvent, BinValueView}`, `events`, `schema`, `NveAbi`

```rust
use nve_abi::{FrameId, NodeId, NveSceneTable, PrimarySlot, SceneNodeInput};

let mut table = NveSceneTable::new();
table.begin_frame(FrameId(1));
table
    .upsert_node(SceneNodeInput {
        node_id: NodeId::ROOT,
        primary_slot: PrimarySlot::NONE,
        ..SceneNodeInput::default()
    })
    .expect("valid scene node");
let view = table.as_view();
assert_eq!(view.node_count, 1);
```

## Dependency Invariants

- **Depends on**: `nve-abi-macros`, `nve-hash`, `serde`, `rustc-hash`.
- **Must NOT depend on**: `dioxus*`, `serde_json`, `web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-abi
```
