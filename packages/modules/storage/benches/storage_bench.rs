//! Criterion micro-benchmarks for `nve-storage` (`NVK2` WAL put, batch, zero-copy `get_ref`,
//! prefix scan, full `XXH64`+`NKCM` WAL replay, and compaction).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, BatchSize, Criterion};
    use nve_storage::{KvStore, StorageWriteOp};

    fn seeded_store(count: u32) -> KvStore {
        let mut store = KvStore::in_memory();
        for i in 0..count {
            let group = if i < 32 { "hot" } else { "cold" };
            let key = format!("{group}:item:{i:04}");
            let value = format!("value-payload-{i:04}-0123456789abcdef");
            store.put(key.as_bytes(), value.as_bytes()).expect("seed put");
        }
        store
    }

    fn bench_storage_hot_paths(c: &mut Criterion) {
        let store_1k = seeded_store(1000);
        let lookup_key = b"hot:item:0017";
        let prefix = b"hot:item:";

        c.bench_function("storage_get_ref_1k_keys", |b| {
            b.iter(|| {
                let val = store_1k
                    .get_ref(black_box(lookup_key))
                    .expect("valid get_ref")
                    .expect("existing key");
                black_box(val.len())
            });
        });

        c.bench_function("storage_scan_prefix_32_of_1k", |b| {
            b.iter(|| {
                let hits = store_1k.scan_prefix(black_box(prefix), 64);
                black_box(hits.len())
            });
        });

        let wal_512 = seeded_store(512)
            .memory_log_bytes()
            .expect("memory WAL")
            .to_vec();

        c.bench_function("storage_replay_512_wal_frames", |b| {
            b.iter_batched(
                || wal_512.clone(),
                |buf| {
                    let reopened = KvStore::open_from_memory_log(black_box(buf))
                        .expect("replay 512 frames");
                    black_box(reopened.open_info().key_count)
                },
                BatchSize::SmallInput,
            );
        });

        let batch_ops: Vec<StorageWriteOp> = (0..32u32)
            .map(|i| StorageWriteOp::Put {
                key: format!("batch:k:{i:02}").into_bytes(),
                value: vec![0xCDu8; 64],
            })
            .collect();

        c.bench_function("storage_commit_batch_32_ops", |b| {
            b.iter_batched(
                KvStore::in_memory,
                |mut store| {
                    store
                        .commit_batch(black_box(&batch_ops))
                        .expect("commit batch");
                    black_box(store.log_bytes())
                },
                BatchSize::SmallInput,
            );
        });

        c.bench_function("storage_compact_overwritten_log", |b| {
            b.iter_batched(
                || {
                    let mut store = KvStore::in_memory();
                    for round in 0..8u32 {
                        for k in 0..64u32 {
                            let key = format!("k:{k:03}");
                            let val = format!("round-{round}-payload-{k:03}");
                            store.put(key.as_bytes(), val.as_bytes()).expect("put");
                        }
                    }
                    store
                },
                |mut store| {
                    let stats = store.compact().expect("compact");
                    black_box(stats.bytes_after)
                },
                BatchSize::SmallInput,
            );
        });
    }

    criterion_group!(benches, bench_storage_hot_paths);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
