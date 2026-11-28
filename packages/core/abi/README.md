# `nve-abi`

Zero-copy `#[repr(C)]` C ABI and binary wire definitions (`packages/core/abi`, **Core Foundation layer**) shared across Rust engines, codegen targets, and every native/web host.

## Key Public API

- `ffi::{NveAppHandle, NveComponentApi, NveDevApi, NveBufferSlice}`, `host::{NveHostApi, NveHostCallbacks, NveHostStatus}`
- `runtime::{NveRuntimeApi, NveSceneView, NveNodeRow, NveLayoutRect, NveScrollOffset, NVE_RUNTIME_SCHEMA_HASH}`: the one scene ABI every host reads (`scene_acquire` / `scene_release`)
- `table::{NveTableView, NVE_PRIMARY_SLOT_NONE, TableDesc, FieldDesc, FieldKind, compute_table_schema_hash}`
- `value::{NveValue, NveValueRef, NveDirectEvent, BinValueView}`, `events`, `schema`, `NveAbi`

```rust
use core::mem::{offset_of, size_of};
use nve_abi::{NveNodeRow, NveSceneView, NVE_NODE_ROW_SIZE, NVE_RUNTIME_SCHEMA_HASH};

// `libnve_runtime` publishes `NodeStore`'s 32-byte rows straight into the scene view.
assert_eq!(size_of::<NveNodeRow>(), NVE_NODE_ROW_SIZE);
assert_eq!(offset_of!(NveSceneView, rows), 56);
let empty = NveSceneView { schema_hash: NVE_RUNTIME_SCHEMA_HASH, ..NveSceneView::default() };
assert_eq!(empty.node_capacity, 0);
```

## Dependency Invariants

- **Depends on**: `nve-abi-macros`, `nve-hash`, `serde`, `rustc-hash`.
- **Must NOT depend on**: `dioxus*`, `serde_json`, `web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-abi
```
