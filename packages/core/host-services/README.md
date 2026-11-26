# `nve-host-services`

Minimal binary `NVE_IFACE_HOST = 5` host-service transport, FFI client, binary codec, and composable handler registry (`packages/core/host-services`, **Core Runtime layer**) with zero concrete OS capability implementations.

## Key Public API

- `HostServiceProvider`, `HostServiceHandler`, `ServiceRegistry`, `HostCallbackBridge`
- `FfiHostServices`, `RequestTable`, `TypedEventBus`, `TypedAsyncTable`
- `codec::{BinReader, BinWriter, validate_url}`, `error::ServiceError`

```rust
use nve_host_services::{BinReader, BinWriter};

let mut w = BinWriter::new();
w.write_u32(42);
let mut r = BinReader::new(w.as_slice());
assert_eq!(r.read_u32().unwrap(), 42);
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-interop`.
- **Must NOT depend on**: `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, any concrete `packages/modules/*` capability crate, or `hosts/*`.

## Verification

```bash
cargo test -p nve-host-services
```
