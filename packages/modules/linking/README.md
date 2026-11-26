# `nve-linking`

Standalone opt-in deep linking and external URL opening capability module (`packages/modules/linking`, **Capability Module layer**, `SERVICE_LINKING = 0x0013`) with direct in-process `LinkingBackend` dispatch and URL validation.

## Key Public API

- `SERVICE_LINKING`, `LinkingBackend`, `LinkingClient`, `MemoryLinking`, `NativeLinking`, `LinkingEvent`
- `METHOD_OPEN_URL`, `METHOD_CAN_OPEN_URL`, `METHOD_INITIAL_URL`, `EVENT_URL_RECEIVED`
- `validate_url`, `dispatch_linking_wire`, `encode_url_request`, `decode_url_request`

```rust
use nve_linking::{LinkingBackend, MemoryLinking};

let linking = MemoryLinking::new();
assert!(linking.can_open_url("https://example.com").unwrap());
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-host-services`.
- **Must NOT depend on**: `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-linking
```
