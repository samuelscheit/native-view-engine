//! Criterion benchmarks for `nve-engine`: zero-copy `NVERELOC` scene archive encoding +
//! verification.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::ColorRgba;
    use nve_components::{LayoutSnapshot, VisualSnapshot};
    use nve_engine::{
        encode_scene_reloc, LayoutRect, SceneRelocInput, SceneRelocNodeInput, SceneRelocView,
        VerifyMode, Viewport,
    };

    fn bench_scene_reloc(c: &mut Criterion) {
        let child_indices: Vec<u32> = (1..64u32).collect();
        let nodes: Vec<SceneRelocNodeInput<'_>> = (0..64u32)
            .map(|idx| SceneRelocNodeInput {
                node_id: idx + 1,
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
            surface_id: 1,
            frame_id: 1,
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

    criterion_group!(benches, bench_scene_reloc);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
