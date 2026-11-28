//! Criterion `writer_steady_state`: `NveMutationWriter` mutation throughput on a 32-item keyed
//! template list (same-value and changed attributes, keyed reorder, subtree remove +
//! re-instantiate), reporting heap allocations per frame via a thread-local counting allocator.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
#[path = "../tests/support/counting_alloc.rs"]
mod counting_alloc;
#[cfg(not(target_arch = "wasm32"))]
#[path = "../tests/support/writer_list.rs"]
mod writer_list;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;
    use std::time::{Duration, Instant};

    use criterion::{Criterion, criterion_group};
    use nve_abi::SurfaceId;
    use nve_dioxus::NveMutationWriter;

    use crate::counting_alloc::count_allocs;
    use crate::writer_list::{
        ITEMS, Values, all_side_tables_attributes, changed_attributes, keyed_reorder, mount_list,
        remove_and_reinstantiate, same_value_attributes, store_changed, values,
    };

    type Lane = fn(&mut NveMutationWriter, &Values, usize);

    /// `(name, lane, whether every frame changes the store)`.
    const LANES: [(&str, Lane, bool); 5] = [
        ("same_value_attributes_32", same_value_attributes, false),
        ("changed_attributes_32", changed_attributes, true),
        ("all_side_tables_32", all_side_tables_attributes, true),
        ("keyed_reorder", |w, _, i| keyed_reorder(w, i), true),
        ("remove_and_reinstantiate", remove_and_reinstantiate, true),
    ];

    /// Frames between remounts. `NodeStore` retires a slot after 255 generations, so millions of
    /// remove + re-instantiate iterations would otherwise drift into fresh-slot allocation and
    /// table growth; every lane remounts a warmed writer (untimed) this often.
    const REMOUNT_EVERY: u64 = ITEMS as u64 * 128;

    fn mounted(lane: Lane, v: &Values) -> NveMutationWriter {
        let mut writer = NveMutationWriter::new(SurfaceId::ROOT);
        mount_list(&mut writer, v);
        for i in 0..ITEMS * 4 {
            lane(&mut writer, v, i);
        }
        writer
    }

    fn bench_writer_steady_state(c: &mut Criterion) {
        let v = values();
        let mut group = c.benchmark_group("writer_steady_state");
        for (name, lane, changes) in LANES {
            let mut writer = mounted(lane, &v);
            const FRAMES: usize = 256;
            let (_, allocs) = count_allocs(|| {
                for i in 0..FRAMES {
                    lane(&mut writer, &v, ITEMS * 4 + i);
                    assert_eq!(store_changed(&writer), changes, "{name}: frame {i}");
                }
            });
            eprintln!(
                "writer_steady_state/{name}: {:.2} heap allocs/frame",
                allocs as f64 / FRAMES as f64
            );
            let mut frame = ITEMS * 4 + FRAMES;
            let mut since_mount = FRAMES as u64;
            group.bench_function(name, |b| {
                b.iter_custom(|iters| {
                    let mut timed = Duration::ZERO;
                    let mut done = 0u64;
                    while done < iters {
                        if since_mount >= REMOUNT_EVERY {
                            writer = mounted(lane, &v);
                            frame = ITEMS * 4;
                            since_mount = 0;
                        }
                        let n = (iters - done).min(REMOUNT_EVERY - since_mount);
                        let start = Instant::now();
                        for _ in 0..n {
                            lane(black_box(&mut writer), &v, frame);
                            frame += 1;
                        }
                        timed += start.elapsed();
                        done += n;
                        since_mount += n;
                    }
                    timed
                })
            });
        }
        group.finish();
    }

    criterion_group!(benches, bench_writer_steady_state);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // The per-frame allocation counts below are only meaningful if the counter is thread-scoped.
    counting_alloc::assert_counts_only_this_thread();
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
