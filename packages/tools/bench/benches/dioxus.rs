use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use nve_abi::{Command, CommandDecoder, EventMask, EventPayload, NativeEvent, NativeEventKind};
use nve_bench::create_bench_dioxus_surface;
use nve_engine::HotPatchMsg;

fn find_pressable_node_id(cmd_bytes: &[u8]) -> u32 {
    let (_, cmds) = CommandDecoder::new(cmd_bytes).unwrap().decode_all().unwrap();
    cmds.into_iter()
        .find_map(|cmd| match cmd {
            Command::SetEventMask { node_id, mask } if (mask & EventMask::PRESS) != 0 => {
                Some(node_id)
            }
            _ => None,
        })
        .expect("pressable node in BenchCounterApp")
}

fn bench_dioxus(c: &mut Criterion) {
    let mut group = c.benchmark_group("dioxus");

    group.bench_function("initial_frame_render", |b| {
        b.iter_batched(
            create_bench_dioxus_surface,
            |mut surface| black_box(surface.render_frame(1)),
            BatchSize::SmallInput,
        );
    });

    // Incremental state update diff (Press event -> signal increment -> VDOM diff + ABI encode)
    let mut surface = create_bench_dioxus_surface();
    let (_, _, init_bytes) = surface.render_frame(1);
    let btn_id = find_pressable_node_id(&init_bytes);
    let mut frame_id = 1u64;

    group.bench_function("incremental_state_update_diff", |b| {
        b.iter(|| {
            frame_id += 1;
            let handled = surface.dispatch_events(&[NativeEvent {
                surface_id: 1,
                node_id: btn_id,
                timestamp_us: frame_id * 16_000,
                kind: NativeEventKind::Press,
                payload: EventPayload::Point { x: 40.0, y: 20.0 },
            }]);
            debug_assert!(handled);
            black_box(surface.render_frame(frame_id))
        });
    });

    // Tier-1 hot patch application (queue TemplatePatch + render frame applying attr/text patches)
    let mut patch_surface = create_bench_dioxus_surface();
    let _ = patch_surface.render_frame(1);
    let mut generation = 1u64;
    let mut pframe = 1u64;

    group.bench_function("tier1_hot_patch_apply", |b| {
        b.iter(|| {
            generation += 1;
            pframe += 1;
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
            black_box(patch_surface.render_frame(pframe))
        });
    });

    group.finish();
}

criterion_group!(dioxus_benches, bench_dioxus);
criterion_main!(dioxus_benches);
