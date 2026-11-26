//! Criterion benchmarks for `nve-ui`'s virtual list engine (`FenwickHeightTree`, `RowHeights`,
//! `VirtualListState`).
//!
//! Measures:
//! - `virtual_list/slice_variable`: `RowHeights::slice()` across `N = 1k, 10k, 100k, 1M` rows
//!   (verifying O(log N) 2-bit-lift scaling).
//! - `virtual_list/slice_uniform`: `RowHeights::slice()` O(1) baseline at `N = 100k`.
//! - `virtual_list/set_row_height_100k`: in-place O(log N) update on uniquely owned 100k tree.
//! - `virtual_list/render_sync_100k`: render-path `RowHeights::clone()` + `PartialEq` +
//!   `revision()` check at 100k rows (verifying strictly O(1) ~ns cost).
//! - `virtual_list/build_from_fn_100k`: one-pass O(N) `FenwickHeightTree::from_fn` construction.

use std::hint::black_box;
use std::mem::size_of;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nve_ui::{FenwickHeightTree, RowHeights};

fn build_variable(n: usize) -> RowHeights {
    RowHeights::variable_from_fn(n, |i| 28 + ((i.wrapping_mul(37) ^ (i >> 3)) % 53) as u32)
}

fn bench_virtual_list(c: &mut Criterion) {
    // Emit exact memory footprint telemetry once when benchmark binary runs.
    let sample_100k = build_variable(100_000);
    let tree_bytes = sample_100k.as_tree().map_or(0, FenwickHeightTree::memory_bytes);
    eprintln!(
        "[nve-ui virtual_list footprint] size_of::<RowHeights>()={} B | size_of::<FenwickHeightTree>()={} B | 100k tree={} B ({:.2} B/row)",
        size_of::<RowHeights>(),
        size_of::<FenwickHeightTree>(),
        tree_bytes,
        tree_bytes as f64 / 100_000.0
    );

    // 1. Variable-height slice() scaling across 1k .. 1M rows (O(log N), 2 bit-lifts).
    let mut slice_group = c.benchmark_group("virtual_list/slice_variable");
    for &n in &[1_000usize, 10_000, 100_000, 1_000_000] {
        let heights = build_variable(n);
        let total = heights.total_height();
        let mut cursor = 0usize;
        slice_group.bench_with_input(BenchmarkId::from_parameter(n), &heights, |b, h| {
            b.iter(|| {
                cursor = cursor.wrapping_add(1);
                let frac = ((cursor & 1023) as f32) * (1.0 / 1024.0);
                let offset = frac * total;
                black_box(h.slice(black_box(offset), black_box(600.0), black_box(3)))
            });
        });
    }
    slice_group.finish();

    // 2. Uniform-height slice() O(1) baseline at 100k rows.
    let uniform_100k = RowHeights::uniform(100_000, 44.0);
    let uniform_total = uniform_100k.total_height();
    let mut u_cursor = 0usize;
    c.bench_function("virtual_list/slice_uniform_100k", |b| {
        b.iter(|| {
            u_cursor = u_cursor.wrapping_add(1);
            let frac = ((u_cursor & 1023) as f32) * (1.0 / 1024.0);
            black_box(uniform_100k.slice(
                black_box(frac * uniform_total),
                black_box(600.0),
                black_box(3),
            ))
        });
    });

    // 3. In-place O(log N) set_row_height() at 100k rows (uniquely owned, 0 allocs).
    //    `epoch` increments on wraparound (997 wraps per 100k steps, coprime to 64) so every
    //    single iteration mutates the Fenwick tree instead of hitting the unchanged-height no-op.
    let mut mutable_100k = build_variable(100_000);
    let mut upd_idx = 0usize;
    let mut epoch = 0u32;
    c.bench_function("virtual_list/set_row_height_100k", |b| {
        b.iter(|| {
            let next = upd_idx + 997;
            if next >= 100_000 {
                upd_idx = next - 100_000;
                epoch = epoch.wrapping_add(1);
            } else {
                upd_idx = next;
            }
            let next_h = 28 + ((epoch.wrapping_add(upd_idx as u32)) & 63);
            black_box(mutable_100k.set_row_height(black_box(upd_idx), black_box(next_h)))
        });
    });

    // 4a. Pure O(1) PartialEq + revision() check at 100k rows (Dioxus props diff / sync check).
    let cached_100k = sample_100k.clone();
    c.bench_function("virtual_list/partial_eq_revision_100k", |b| {
        b.iter(|| {
            let eq = black_box(&cached_100k) == black_box(&sample_100k);
            let rev_eq = black_box(&cached_100k).revision() == black_box(&sample_100k).revision();
            black_box((eq, rev_eq))
        });
    });

    // 4b. Full render-path O(1) Arc::clone + PartialEq + revision() check + Arc::drop at 100k rows.
    c.bench_function("virtual_list/render_sync_100k", |b| {
        b.iter(|| {
            let incoming = black_box(sample_100k.clone());
            let eq = black_box(&cached_100k) == black_box(&incoming);
            let rev_eq = cached_100k.revision() == incoming.revision();
            black_box((eq, rev_eq))
        });
    });

    // 5. One-pass O(N) FenwickHeightTree::from_fn construction at 100k rows.
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

criterion_group!(virtual_list_benches, bench_virtual_list);
criterion_main!(virtual_list_benches);
