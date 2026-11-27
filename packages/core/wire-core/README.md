# `nve-wire-core`

Sans-I/O binary wire framing (`NveWireFrame` v2), Bearer-token handshake, compact `NveNodePatchDelta` codec, URI parser, and client session state machine (`packages/core/wire-core`, **Core Wire layer**).

## Key Public API

- `frame::{NveWireFrame, NveWireFrameRef, NveWireHeader, NveWireChannel, NveWireKind, NveWireError}`, `handshake`, `session`
- `patch::{NveNodePatchDeltaBuilder, NveNodePatchDeltaView, NVE_NODE_PATCH_DELTA_HEADER_SIZE}`
- `uri`, `NveCommandBatch`

```rust
use nve_abi::NodeId;
use nve_wire_core::NveNodePatchDeltaBuilder;

let mut builder = NveNodePatchDeltaBuilder::new();
builder.set_text(NodeId::ROOT, "hello");
let bytes = builder.encode().unwrap();
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-engine` (zero async runtime, TLS, or socket crates — `wasm32`-ready).
- **Must NOT depend on**: `tokio`, `quinn`, `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-wire-core
```
