use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use nve_abi::{EventBatchBuf, FrameId, InteractionEvent, NodeId, SurfaceId, TimestampUs};
use nve_bench::create_bench_dioxus_surface;
use nve_tables::tables::Interaction;
use nve_engine::HotPatchMsg;
use nve_store::NodeStore;

fn find_pressable_node_id(store: &NodeStore) -> NodeId {
    store
        .live_nodes()
        .find_map(|id| {
            store
                .try_row::<Interaction>(id)?
                .listeners()
                .contains(InteractionEvent::Press)
                .then_some(id)
        })
        .expect("pressable node in BenchCounterApp")
}

fn bench_dioxus(c: &mut Criterion) {
    let mut group = c.benchmark_group("dioxus");

    group.bench_function("initial_frame_render", |b| {
        b.iter_batched(
            create_bench_dioxus_surface,
            |mut surface| black_box(surface.render_nve2_slice(FrameId(1)).len()),
            BatchSize::SmallInput,
        );
    });

    // Incremental state update diff (Press event -> signal increment -> VDOM diff + NVE2 encode)
    let mut surface = create_bench_dioxus_surface();
    let _ = surface.render_nve2_slice(FrameId(1));
    let btn_id = find_pressable_node_id(surface.store());
    let mut frame_id = FrameId(1);

    let mut ev_buf = EventBatchBuf::with_capacity(1, 0);
    group.bench_function("incremental_state_update_diff", |b| {
        b.iter(|| {
            frame_id = frame_id.next();
            ev_buf.clear();
            ev_buf.push_press(
                SurfaceId::ROOT,
                btn_id,
                TimestampUs(frame_id.raw() * 16_000),
                40.0,
                20.0,
            );
            let handled = surface.dispatch_events(ev_buf.as_validated());
            debug_assert!(handled);
            black_box(surface.render_nve2_slice(frame_id).len())
        });
    });

    // Tier-1 hot patch application (queue TemplatePatch + render frame applying attr/text patches)
    let mut patch_surface = create_bench_dioxus_surface();
    let _ = patch_surface.render_nve2_slice(FrameId(1));
    let mut generation = 1u64;
    let mut pframe = FrameId(1);

    group.bench_function("tier1_hot_patch_apply", |b| {
        b.iter(|| {
            generation += 1;
            pframe = pframe.next();
            let opacity = if generation.is_multiple_of(2) {
                "0.85"
            } else {
                "0.95"
            };
            let patch = HotPatchMsg::TemplatePatch {
                generation,
                template_id: "bench_counter".into(),
                target_test_id: None,
                node_path: vec![],
                attr_updates: vec![("opacity".into(), opacity.into())],
                text_updates: vec![(0, format!("Hot Patch Gen {generation}"))],
                text_replacements: vec![],
            };
            let accepted = patch_surface.apply_hot_patch(&patch);
            debug_assert!(accepted);
            black_box(patch_surface.render_nve2_slice(pframe).len())
        });
    });

    group.finish();
}

criterion_group!(dioxus_benches, bench_dioxus);
criterion_main!(dioxus_benches);
