//! Criterion benchmarks for `nve-ui`'s virtual list engine (`FenwickHeightTree`, `RowHeights`).
//!
//! Benchmarks:
//! - `virtual_list/slice_variable/{1000,10000,100000,1000000}`: full-range `xorshift64` random
//!   scroll jumps across 1k..1M variable rows (O(log N) 2-bit-lift scaling).
//! - `virtual_list/slice_variable_overscan64_100k`: `overscan = 64` `prefix_sum` fallback at 100k.
//! - `virtual_list/slice_uniform_100k`: O(1) uniform-height baseline at 100k rows.
//! - `virtual_list/set_row_height_100k`: uniquely owned O(log N) in-place mutation (`44 <-> 76`).
//! - `virtual_list/set_row_height_cow_shared_100k`: shared-`Arc` COW mutation cost at 100k rows.
//! - `virtual_list/promote_uniform_to_variable_100k`: one-time O(N) uniform→variable promotion.
//! - `virtual_list/partial_eq_revision_variable_100k`: O(1) `PartialEq` + `revision()` (variable).
//! - `virtual_list/partial_eq_revision_uniform_100k`: O(1) `PartialEq` + `revision()` (uniform).
//! - `virtual_list/render_sync_100k`: `Arc::clone` + `PartialEq` + `revision()` + drop at 100k.
//! - `virtual_list/build_from_fn/100000`: one-pass O(N) `FenwickHeightTree::from_fn` construction.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;
    use std::mem::size_of;

    use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
    use nve_ui::{FenwickHeightTree, RowHeights};

    struct XorShift64(u64);

    impl XorShift64 {
        fn new(seed: u64) -> Self {
            Self(seed.max(1))
        }

        #[inline]
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    fn build_variable(n: usize) -> RowHeights {
        RowHeights::variable_from_fn(n, |i| 28 + ((i.wrapping_mul(37) ^ (i >> 3)) % 53) as u32)
    }

    pub fn bench_virtual_list(c: &mut Criterion) {
        let sample_100k = build_variable(100_000);
        let tree_bytes = sample_100k.as_tree().map_or(0, FenwickHeightTree::memory_bytes);
        eprintln!(
            "[nve-ui virtual_list footprint] size_of::<RowHeights>()={} B | size_of::<FenwickHeightTree>()={} B | 100k tree={} B ({:.2} B/row)",
            size_of::<RowHeights>(),
            size_of::<FenwickHeightTree>(),
            tree_bytes,
            tree_bytes as f64 / 100_000.0
        );

        // 1. Variable-height slice() full-range random jumps across 1k..1M rows.
        let mut slice_group = c.benchmark_group("virtual_list/slice_variable");
        for &n in &[1_000usize, 10_000, 100_000, 1_000_000] {
            let heights = build_variable(n);
            let total = heights.total_height();
            let mut rng = XorShift64::new(0x9E37_79B9_7F4A_7C15 ^ (n as u64));
            slice_group.bench_with_input(BenchmarkId::from_parameter(n), &heights, |b, h| {
                b.iter(|| {
                    let offset = (rng.next_u64() % total) as f64;
                    black_box(h.slice(black_box(offset), black_box(600.0), black_box(3)))
                });
            });
        }
        slice_group.finish();

        // 2. Variable-height slice() with large overscan (64 > 32 triggers prefix_sum fallback).
        let total_100k = sample_100k.total_height();
        let mut ov_rng = XorShift64::new(0xA5A5_5A5A_1234_5678);
        c.bench_function("virtual_list/slice_variable_overscan64_100k", |b| {
            b.iter(|| {
                let offset = (ov_rng.next_u64() % total_100k) as f64;
                black_box(sample_100k.slice(black_box(offset), black_box(600.0), black_box(64)))
            });
        });

        // 3. Uniform-height slice() O(1) baseline at 100k rows.
        let uniform_100k = RowHeights::uniform(100_000, 44);
        let uniform_total = uniform_100k.total_height();
        let mut u_rng = XorShift64::new(0x1234_5678_9ABC_DEF0);
        c.bench_function("virtual_list/slice_uniform_100k", |b| {
            b.iter(|| {
                let offset = (u_rng.next_u64() % uniform_total) as f64;
                black_box(uniform_100k.slice(black_box(offset), black_box(600.0), black_box(3)))
            });
        });

        // 4. Uniquely owned O(log N) set_row_height() strictly mutating every iteration (0 allocs).
        let mut mutable_100k = build_variable(100_000);
        let mut upd_rng = XorShift64::new(0xDEAD_BEEF_CAFE_BABE);
        c.bench_function("virtual_list/set_row_height_100k", |b| {
            b.iter(|| {
                let idx = (upd_rng.next_u64() as usize) % 100_000;
                let cur = mutable_100k.row_height(idx);
                let next_h = if cur == 44 { 76 } else { 44 };
                black_box(mutable_100k.set_row_height(black_box(idx), black_box(next_h)))
            });
        });

        // 5. Shared-Arc COW set_row_height() at 100k rows (measures the 1.2 MB clone penalty).
        c.bench_function("virtual_list/set_row_height_cow_shared_100k", |b| {
            b.iter_batched(
                || sample_100k.clone(),
                |mut shared| {
                    let cur = shared.row_height(50_000);
                    let next_h = if cur == 44 { 76 } else { 44 };
                    black_box(shared.set_row_height(black_box(50_000), black_box(next_h)));
                    black_box(shared)
                },
                BatchSize::LargeInput,
            );
        });

        // 6. One-time O(N) uniform->variable promotion at 100k rows.
        c.bench_function("virtual_list/promote_uniform_to_variable_100k", |b| {
            b.iter_batched(
                || RowHeights::uniform(100_000, 44),
                |mut h| {
                    black_box(h.set_row_height(black_box(50_000), black_box(76)));
                    black_box(h)
                },
                BatchSize::LargeInput,
            );
        });

        // 7a. Pure O(1) PartialEq + revision() check (variable 100k).
        let cached_100k = sample_100k.clone();
        c.bench_function("virtual_list/partial_eq_revision_variable_100k", |b| {
            b.iter(|| {
                let eq = black_box(&cached_100k) == black_box(&sample_100k);
                let rev_eq =
                    black_box(&cached_100k).revision() == black_box(&sample_100k).revision();
                black_box((eq, rev_eq))
            });
        });

        // 7b. Pure O(1) PartialEq + revision() check (uniform 100k).
        let cached_uniform = RowHeights::uniform(100_000, 44);
        c.bench_function("virtual_list/partial_eq_revision_uniform_100k", |b| {
            b.iter(|| {
                let eq = black_box(&cached_uniform) == black_box(&uniform_100k);
                let rev_eq =
                    black_box(&cached_uniform).revision() == black_box(&uniform_100k).revision();
                black_box((eq, rev_eq))
            });
        });

        // 8. Full render-path O(1) Arc::clone + PartialEq + revision() + Arc::drop at 100k rows.
        c.bench_function("virtual_list/render_sync_100k", |b| {
            b.iter(|| {
                let incoming = black_box(sample_100k.clone());
                let eq = black_box(&cached_100k) == black_box(&incoming);
                let rev_eq = cached_100k.revision() == incoming.revision();
                black_box((eq, rev_eq))
            });
        });

        // 9. One-pass O(N) FenwickHeightTree::from_fn construction at 100k rows.
        let mut build_group = c.benchmark_group("virtual_list/build_from_fn");
        build_group.throughput(Throughput::Elements(100_000));
        build_group.bench_function("100000", |b| {
            b.iter(|| {
                black_box(FenwickHeightTree::from_fn(black_box(100_000), |i| {
                    28 + ((i.wrapping_mul(37) ^ (i >> 3)) % 53) as u32
                }))
            });
        });
        build_group.finish();
    }
}

#[cfg(not(target_arch = "wasm32"))]
criterion::criterion_group!(virtual_list_benches, native::bench_virtual_list);
#[cfg(not(target_arch = "wasm32"))]
criterion::criterion_main!(virtual_list_benches);
