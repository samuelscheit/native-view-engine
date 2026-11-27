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

    use criterion::{Criterion, criterion_group};
    use nve_dioxus::NveMutationWriter;

    use crate::counting_alloc::count_allocs;
    use crate::writer_list::{
        ITEMS, Values, changed_attributes, keyed_reorder, mount_list, remove_and_reinstantiate,
        same_value_attributes, values,
    };

    type Lane = fn(&mut NveMutationWriter, &Values, usize);

    const LANES: [(&str, Lane); 4] = [
        ("same_value_attributes_32", same_value_attributes),
        ("changed_attributes_32", changed_attributes),
        ("keyed_reorder", |w, _, i| keyed_reorder(w, i)),
        ("remove_and_reinstantiate", |w, _, i| {
            remove_and_reinstantiate(w, i)
        }),
    ];

    fn bench_writer_steady_state(c: &mut Criterion) {
        let v = values();
        let mut group = c.benchmark_group("writer_steady_state");
        for (name, lane) in LANES {
            let mut writer = NveMutationWriter::new(1);
            mount_list(&mut writer);
            for i in 0..ITEMS * 4 {
                lane(&mut writer, &v, i);
            }
            const FRAMES: usize = 256;
            let (_, allocs) = count_allocs(|| {
                for i in 0..FRAMES {
                    lane(&mut writer, &v, i);
                }
            });
            eprintln!(
                "writer_steady_state/{name}: {:.2} heap allocs/frame",
                allocs as f64 / FRAMES as f64
            );
            let mut frame = 0usize;
            group.bench_function(name, |b| {
                b.iter(|| {
                    frame += 1;
                    lane(black_box(&mut writer), &v, frame);
                })
            });
        }
        group.finish();
    }

    criterion_group!(benches, bench_writer_steady_state);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
