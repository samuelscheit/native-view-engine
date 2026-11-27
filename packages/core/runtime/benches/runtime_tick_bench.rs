//! Criterion micro-benchmarks for `SurfaceRuntime`: steady-state clean tick, dirty NVE2 frame
//! publish, and `scene_acquire`/`scene_release` + `hit_test`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nve_abi::EventMask;
use nve_components::tables::{Interaction, Layout, TextContent, Visual};
use nve_engine::Viewport;
use nve_runtime::RuntimeBuilder;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

fn build_tree_frame(count: usize) -> Vec<u8> {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        let container = tx.create();
        tx.edit::<Layout>(container)
            .width(Length::points(800.0))
            .height(Length::points(600.0));
        let mut kids = Vec::with_capacity(count);
        for i in 0..count {
            let item = tx.create();
            tx.edit::<Layout>(item)
                .width(Length::points(120.0))
                .height(Length::points(24.0));
            tx.edit::<Visual>(item).background(Rgba::rgb(30, 41, 59));
            if i % 4 == 0 {
                tx.edit::<Interaction>(item).event_mask(EventMask::PRESS as u32);
            }
            let lbl = tx.create();
            tx.edit::<TextContent>(lbl).text("Row Item");
            tx.set_children(item, &[lbl]);
            kids.push(item);
        }
        tx.set_children(container, &kids);
        tx.set_children(NodeId::ROOT, &[container]);
        tx.commit_into(&mut changes);
    }
    let mut enc = WireEncoder::default();
    let mut out = Vec::new();
    enc.encode(&store, &changes, &mut out);
    out
}

fn bench_runtime(c: &mut Criterion) {
    let frame = build_tree_frame(200);
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        })
        .build();
    rt.submit_nve2(&frame).expect("initial submit");

    let mut group = c.benchmark_group("nve_runtime");
    group.bench_function("clean_tick_and_scene_acquire_400_nodes", |b| {
        b.iter(|| {
            let changed = rt.tick(black_box(1.0 / 60.0)).expect("tick");
            let guard = rt.acquire_scene().expect("acquire");
            black_box((changed, guard.view().frame_id, guard.view().node_capacity));
        });
    });

    group.bench_function("hit_test_400_nodes", |b| {
        b.iter(|| {
            let hit = rt.hit_test(black_box(40.0), black_box(36.0)).expect("hit");
            black_box(hit);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_runtime);
criterion_main!(benches);
