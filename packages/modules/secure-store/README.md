# `nve-secure-store`

Standalone opt-in secure credential store capability module (`packages/modules/secure-store`, **Capability Module layer**, `SERVICE_SECURE_STORE = 0x0012`) with direct in-process `SecureStoreBackend` dispatch and OS keychain adapters.

## Key Public API

- `SERVICE_SECURE_STORE`, `SecureStoreBackend`, `SecureStoreClient`, `MemorySecureStore`, `NativeSecureStore`
- `METHOD_GET`, `METHOD_SET`, `METHOD_DELETE`
- `dispatch_secure_store_wire`, `encode_key_request`, `encode_set_request`, `decode_get_response`

```rust
use nve_secure_store::{MemorySecureStore, SecureStoreBackend};

let store = MemorySecureStore::new();
store.set("app", "token", b"secret").unwrap();
assert_eq!(store.get("app", "token").unwrap().as_deref(), Some(&b"secret"[..]));
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-host-services`.
- **Must NOT depend on**: `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-secure-store
```
