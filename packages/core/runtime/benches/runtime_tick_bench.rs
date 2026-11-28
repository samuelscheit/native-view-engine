//! Criterion micro-benchmarks for `SurfaceRuntime`: steady-state clean tick, dirty NVE2 frame
//! publish, and `scene_acquire`/`scene_release` + `hit_test`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nve_abi::InteractionEvent;
use nve_tables::tables::{Interaction, Layout, TextContent, Visual};
use nve_engine::Viewport;
use nve_runtime::RuntimeBuilder;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

fn build_tree_and_dirty_deltas(count: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut items = Vec::with_capacity(count);
    {
        let mut tx = store.tx();
        let container = tx.create();
        tx.edit::<Layout>(container)
            .width(Length::points(800.0))
            .height(Length::points(600.0));
        for i in 0..count {
            let item = tx.create();
            tx.edit::<Layout>(item)
                .width(Length::points(120.0))
                .height(Length::points(24.0));
            tx.edit::<Visual>(item).background(Rgba::rgb(30, 41, 59));
            if i % 4 == 0 {
                tx.edit::<Interaction>(item)
                    .listen(InteractionEvent::Press);
            }
            let lbl = tx.create();
            tx.edit::<TextContent>(lbl).text("Row Item");
            tx.set_children(item, &[lbl]);
            items.push(item);
        }
        tx.set_children(container, &items);
        tx.set_children(NodeId::ROOT, &[container]);
        tx.commit_into(&mut changes);
    }
    let mut enc = WireEncoder::default();
    let mut initial = Vec::new();
    enc.encode(&store, &changes, &mut initial);

    let mut delta_a = Vec::new();
    {
        let mut tx = store.tx();
        for &item in items.iter().step_by(8) {
            tx.edit::<Visual>(item).opacity(0.65);
        }
        tx.commit_into(&mut changes);
    }
    enc.encode(&store, &changes, &mut delta_a);

    let mut delta_b = Vec::new();
    {
        let mut tx = store.tx();
        for &item in items.iter().step_by(8) {
            tx.edit::<Visual>(item).opacity(0.95);
        }
        tx.commit_into(&mut changes);
    }
    enc.encode(&store, &changes, &mut delta_b);

    (initial, delta_a, delta_b)
}

fn bench_runtime(c: &mut Criterion) {
    let (frame, delta_a, delta_b) = build_tree_and_dirty_deltas(200);
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

    let mut toggle = false;
    group.bench_function("dirty_nve2_publish_400_nodes", |b| {
        b.iter(|| {
            toggle = !toggle;
            let payload = if toggle { &delta_a } else { &delta_b };
            rt.submit_nve2(black_box(payload)).expect("dirty submit");
            let guard = rt.acquire_scene().expect("acquire");
            black_box((guard.view().frame_id, guard.view().dirty_count));
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
