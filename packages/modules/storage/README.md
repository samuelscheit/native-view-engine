# `nve-storage`

Standalone opt-in pure-Rust crash-safe `NVK2` binary key-value / record store library (`packages/modules/storage`, **Capability Module layer**) with atomic WAL persistence, torn-tail recovery, and compaction.

## Key Public API

- `KvStore` (`KvStore::open(path)`, `KvStore::in_memory()`), `KvStoreManager`
- `StorageBackend`, `NativeStorage`, `MemoryStorage`
- `StorageWriteOp`, `StorageEntry`, `StorageOpenInfo`, `CompactStats`, `StorageError`

```rust
use nve_storage::{KvStore, MemoryStorage, StorageBackend};

let mut kv = KvStore::in_memory();
kv.put(b"greeting", b"hello").unwrap();
assert_eq!(kv.get(b"greeting").unwrap().as_deref(), Some(b"hello".as_slice()));

let storage = MemoryStorage::new();
storage.open("default").unwrap();
storage.put("default", b"k", b"v").unwrap();
```

## Dependency Invariants

- **Depends ONLY on**: `nve-codec`, `nve-hash`.
- **Must NOT depend on**: `nve-engine`, `nve-store`, `nve-host-services`, `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-storage
```
