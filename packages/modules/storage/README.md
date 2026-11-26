# `nve-storage`

Standalone opt-in embedded crash-safe binary key-value / record store capability module (`packages/modules/storage`, **Capability Module layer**, `SERVICE_STORAGE = 0x0020`) with atomic WAL persistence and compaction.

## Key Public API

- `SERVICE_STORAGE`, `KvStore`, `KvStoreManager`, `StorageBackend`, `MemoryStorage`, `NativeStorage`
- `StorageWriteOp`, `StorageEntry`, `StorageOpenInfo`, `CompactStats`
- `METHOD_OPEN`, `METHOD_GET`, `METHOD_PUT`, `METHOD_DELETE`, `METHOD_SCAN_PREFIX`, `METHOD_COMMIT_BATCH`, `METHOD_COMPACT`, `dispatch_storage_wire`

```rust
use nve_storage::{MemoryStorage, StorageBackend};

let storage = MemoryStorage::new();
storage.open("default").unwrap();
storage.put("default", b"k", b"v").unwrap();
```

## Dependency Invariants

- **Depends on**: `nve-host-services`.
- **Must NOT depend on**: `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-storage
```
