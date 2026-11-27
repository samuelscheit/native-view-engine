//! Criterion benchmarks for `nve-engine`: 64-byte-aligned `FrameArena` bump allocation & reset
//! and zero-copy `NVERELOC` scene archive encoding + verification.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::{ColorRgba, FrameId, NodeId, SurfaceId};
    use nve_components::{LayoutSnapshot, VisualSnapshot};
    use nve_engine::{
        encode_scene_reloc, FrameArena, LayoutRect, SceneRelocInput, SceneRelocNodeInput,
        SceneRelocView, VerifyMode, Viewport,
    };

    fn bench_frame_arena(c: &mut Criterion) {
        let mut arena = FrameArena::with_capacity(64 * 1024);
        c.bench_function("frame_arena_alloc_1000_rects_and_reset", |b| {
            b.iter(|| {
                arena.reset();
                let mut sum = 0.0f32;
                for i in 0..1000u32 {
                    let x = black_box(i as f32);
                    let r = arena.alloc_val(LayoutRect {
                        x,
                        y: x * 2.0,
                        width: 100.0,
                        height: 32.0,
                    });
                    sum += r.x;
                }
                black_box(sum)
            });
        });
    }

    fn bench_scene_reloc(c: &mut Criterion) {
        let child_indices: Vec<u32> = (1..64u32).collect();
        let nodes: Vec<SceneRelocNodeInput<'_>> = (0..64u32)
            .map(|idx| SceneRelocNodeInput {
                node_id: NodeId(idx + 1),
                parent_index: if idx == 0 { -1 } else { 0 },
                primary_slot: 0,
                flags: 0,
                text_align: 0,
                image_resize_mode: 0,
                font_weight: 500,
                event_mask: 0,
                computed_rect: LayoutRect {
                    x: (idx as f32) * 8.0,
                    y: (idx as f32) * 4.0,
                    width: 120.0,
                    height: 40.0,
                },
                style: VisualSnapshot::default(),
                layout: LayoutSnapshot::default(),
                text: "Bench Node",
                tag: "view",
                image_src: "",
                box_shadow: "",
                draw_list: &[],
                children: if idx == 0 { &child_indices } else { &[] },
            })
            .collect();
        let input = SceneRelocInput {
            schema_version: 1,
            surface_id: SurfaceId::ROOT,
            frame_id: FrameId(1),
            viewport: Viewport {
                width: 1280.0,
                height: 720.0,
                scale_factor: 2.0,
            },
            clear_color: ColorRgba::BLACK,
            root_node_index: 0,
            app_tag: "bench",
            nodes: &nodes,
        };
        let encoded = encode_scene_reloc(&input).expect("valid bench scene reloc");

        c.bench_function("scene_reloc_encode_64_nodes", |b| {
            b.iter(|| {
                let buf = encode_scene_reloc(black_box(&input)).expect("valid scene reloc encode");
                black_box(buf.as_slice().len())
            });
        });

        c.bench_function("scene_reloc_verify_full_64_nodes", |b| {
            b.iter(|| {
                let view =
                    SceneRelocView::from_bytes(black_box(encoded.as_slice()), VerifyMode::Full)
                        .expect("valid scene reloc verify");
                black_box(view.nodes().len())
            });
        });
    }

    criterion_group!(benches, bench_frame_arena, bench_scene_reloc);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
