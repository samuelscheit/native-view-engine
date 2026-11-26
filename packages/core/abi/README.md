# `nve-abi`

Zero-copy `#[repr(C)]` C ABI and binary wire definitions (`packages/core/abi`, **Core Foundation layer**) shared across Rust engines, codegen targets, and every native/web host.

## Key Public API

- `ffi::{NveAppHandle, NveComponentApi, NveDevApi, NveBufferSlice}`, `host::{NveHostApi, NveHostCallbacks, NveHostStatus}`
- `scene::{NveRenderNode, NveSceneView, NveSceneTable, NveNodeMeta}`
- `codec::{CommandBufferBuilder, CommandDecoder, ColorRgba, Dimension, EdgeInsets}`, `events`, `schema`, `NveAbi`

```rust
use nve_abi::{CommandBufferBuilder, ComponentKind};

let mut builder = CommandBufferBuilder::new(1, 1);
builder.create_node(1, ComponentKind::View, None);
let bytes = builder.finish();
```

## Dependency Invariants

- **Depends on**: `nve-abi-macros`, `serde`, `rustc-hash`.
- **Must NOT depend on**: `dioxus*`, `serde_json`, `web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-abi
```
