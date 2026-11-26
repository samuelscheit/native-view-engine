# `nve-wire-core`

Sans-I/O binary wire framing (`NveWireFrame` v2), Bearer-token handshake, compact `NveNodePatchDelta` codec, URI parser, and client session state machine (`packages/core/wire-core`, **Core Wire layer**).

## Key Public API

- `frame::{encode_frame, decode_frame, WireFrameHeader}`, `handshake`, `session`
- `patch::{NveNodePatchDeltaBuilder, NveNodePatchDeltaView, NVE_NODE_PATCH_DELTA_HEADER_LEN}`
- `uri`, `NveCommandBatch`

```rust
use nve_wire_core::NveNodePatchDeltaBuilder;

let builder = NveNodePatchDeltaBuilder::new(1);
let bytes = builder.finish();
assert!(!bytes.is_empty());
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-engine` (zero async runtime, TLS, or socket crates — `wasm32`-ready).
- **Must NOT depend on**: `tokio`, `quinn`, `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-wire-core
```
