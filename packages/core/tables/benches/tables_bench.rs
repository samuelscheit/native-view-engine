//! Criterion micro-benchmarks for `nve-tables` hot paths: zero-copy `RowPatchView` snapshot
//! application, direct `NodeStore` patch commits, 2D affine transforms, and gradient sampling.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::ColorRgba;
    use nve_store::{ChangeSet, FlatResource, NodeId, NodeStore, RowPatch, RowPatchView};
    use nve_tables::{
        affine_concat, affine_map_point, apply_patch_view_to_layout,
        apply_patch_view_to_layout_ext, apply_patch_view_to_visual, sample_gradient_stops,
        transform_from_patch_view, GradientStop, LayoutExt, LayoutSnapshot, StylePatchExt,
        Transform2D, VisualSnapshot,
    };

    fn bench_row_patch_apply(c: &mut Criterion) {
        nve_tables::tables::register_core_tables();
        let patch_a = RowPatch::new()
            .bg(ColorRgba::rgb(15, 23, 42))
            .opacity(0.85)
            .radius(8.0)
            .flex_row()
            .width_px(240.0)
            .padding(16.0)
            .rotate(12.0);
        let patch_b = RowPatch::new()
            .bg(ColorRgba::rgb(37, 99, 235))
            .opacity(0.95)
            .radius(12.0)
            .flex_col()
            .width_px(320.0)
            .padding(12.0)
            .rotate(0.0);
        let mut words_a = Vec::new();
        assert!(patch_a.write_words(&mut words_a));
        let view_a = RowPatchView::from_words(&words_a).expect("valid patch view");
        let mut words_b = Vec::new();
        assert!(patch_b.write_words(&mut words_b));
        let view_b = RowPatchView::from_words(&words_b).expect("valid patch view");

        c.bench_function("tables_row_patch_view_snapshots", |b| {
            b.iter(|| {
                let mut vis = VisualSnapshot::default();
                let mut lay = LayoutSnapshot::default();
                let mut ext = LayoutExt::default();
                apply_patch_view_to_visual(black_box(view_a), &mut vis);
                apply_patch_view_to_layout(black_box(view_a), &mut lay);
                apply_patch_view_to_layout_ext(black_box(view_a), &mut ext);
                let tf = transform_from_patch_view(black_box(view_a));
                black_box((vis.background.to_u32(), lay.gap, ext.aspect_ratio, tf.is_some()))
            });
        });

        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let node = {
            let mut tx = store.tx();
            let n = tx.create();
            tx.set_children(NodeId::ROOT, &[n]);
            tx.apply_row_patch(n, view_a);
            tx.commit_into(&mut changes);
            n
        };
        let mut toggle = false;
        c.bench_function("tables_row_patch_apply_tx", |b| {
            b.iter(|| {
                toggle = !toggle;
                let p = if toggle { view_b } else { view_a };
                let mut tx = store.tx();
                tx.apply_row_patch(node, black_box(p));
                tx.commit_into(&mut changes);
                black_box(changes.touched.len())
            });
        });
    }

    fn bench_transform_and_paint(c: &mut Criterion) {
        let stops = [
            GradientStop {
                offset: 0.0,
                color: ColorRgba::rgb(37, 99, 235),
            },
            GradientStop {
                offset: 0.5,
                color: ColorRgba::rgb(139, 92, 246),
            },
            GradientStop {
                offset: 1.0,
                color: ColorRgba::rgb(16, 185, 129),
            },
        ];

        c.bench_function("tables_transform2d_concat_map", |b| {
            let t1 = Transform2D {
                translate_x: 24.0,
                translate_y: 12.0,
                scale_x: 1.25,
                scale_y: 1.25,
                rotate_deg: 15.0,
                ..Transform2D::IDENTITY
            };
            let t2 = Transform2D {
                translate_x: -8.0,
                translate_y: 32.0,
                scale_x: 0.9,
                scale_y: 0.9,
                rotate_deg: -5.0,
                ..Transform2D::IDENTITY
            };
            b.iter(|| {
                let m1 = black_box(&t1).to_matrix_2d(100.0, 50.0);
                let m2 = black_box(&t2).to_matrix_2d(100.0, 50.0);
                let m = affine_concat(black_box(&m1), black_box(&m2));
                black_box(affine_map_point(&m, 50.0, 25.0))
            });
        });

        c.bench_function("tables_sample_gradient_16", |b| {
            b.iter(|| {
                let mut acc = 0u32;
                for i in 0..16 {
                    let t = i as f32 / 15.0;
                    acc = acc.wrapping_add(sample_gradient_stops(black_box(&stops), t).to_u32());
                }
                black_box(acc)
            });
        });
    }

    criterion_group!(benches, bench_row_patch_apply, bench_transform_and_paint);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
