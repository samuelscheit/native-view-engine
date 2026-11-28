# `nve-host-services`

Minimal binary `NVE_IFACE_HOST = 5` host-service transport, FFI client, binary codec, and composable handler registry (`packages/core/host-services`, **Core Runtime layer**) with zero concrete OS capability implementations.

## Key Public API

- `ServiceSlots`, `ServiceSlotsBuilder`, `ServiceSlot`, `ServiceSlotId`, `ServiceHandle` (lock-free dense O(1) typed slot table with zero double-`Arc` and zero atomic refcount increments on `get`)
- `HostServiceProvider`, `HostServiceHandler`, `HostServiceRegistry`, `MemoryHostServices`, `HostCallbackBridge`
- `FfiHostServices`, `RequestTable`, `TypedEventBus`, `TypedAsyncTable`
- `ByteReader`, `ByteWriter`, `codec::validate_url`, `error::ServiceError`

```rust
use nve_host_services::{ByteReader, ByteWriter};

let mut w = ByteWriter::new();
w.write_u32(42);
let mut r = ByteReader::new(w.as_slice());
assert_eq!(r.read_u32().unwrap(), 42);
r.finish().unwrap();
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-codec`, `nve-interop`.
- **Must NOT depend on**: `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, any concrete `packages/modules/*` capability crate, or `hosts/*`.

## Verification

```bash
cargo test -p nve-host-services
```
