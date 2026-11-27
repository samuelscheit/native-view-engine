//! Criterion micro-benchmarks for `nve-hash` (`xxh64`, `Xxh64Hasher`, `Key64`).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_hash::{xxh64, Key64, Xxh64Hasher};

    fn bench_xxh64(c: &mut Criterion) {
        let small = b"nve.table.layout";
        let medium = [0xA5u8; 256];
        let large = vec![0x5Au8; 4096];

        c.bench_function("xxh64_16b", |b| {
            b.iter(|| black_box(xxh64(black_box(small), 0)));
        });
        c.bench_function("xxh64_256b", |b| {
            b.iter(|| black_box(xxh64(black_box(&medium), 0)));
        });
        c.bench_function("xxh64_4kb", |b| {
            b.iter(|| black_box(xxh64(black_box(&large), 0)));
        });
        c.bench_function("xxh64_streaming_4kb", |b| {
            b.iter(|| {
                let mut h = Xxh64Hasher::new(0);
                for chunk in black_box(&large).chunks(64) {
                    h.update(chunk);
                }
                black_box(h.digest())
            });
        });
        c.bench_function("key64_of_ident", |b| {
            b.iter(|| black_box(Key64::of(black_box("nve.components.typography"))));
        });
    }

    criterion_group!(benches, bench_xxh64);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
