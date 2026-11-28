//! Global-allocator (`CountingAlloc`) zero-allocation hot-path guard for `nve-tables`:
//! CSS/color/dim parsing, 2D affine transforms, normalized gradient sampling, zero-copy
//! `RowPatchView` snapshot application, and steady-state `NodeStore` resource view iteration.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use nve_abi::ColorRgba;
use nve_store::{ChangeSet, FlatResource, NodeId, NodeStore, Rgba, RowPatch, RowPatchView};
use nve_tables::layout_ext::GridTrack;
use nve_tables::tables::{
    BackgroundFillRes, BoxShadow, BoxShadows, GridLayout, GridTracks, Layout as LayoutTable,
    Visual,
};
use nve_tables::{
    affine_concat, affine_inverse_map_point, affine_map_point, apply_inline_style_layers_to_tx,
    apply_patch_view_to_layout, apply_patch_view_to_layout_ext, apply_patch_view_to_visual,
    parse_color, parse_dim, parse_flex_align, parse_flex_direction, parse_flex_justify,
    parse_font_weight, sample_normalized_stops, transform_from_patch_view, BackgroundFill,
    GradientStop, LayoutExt, LayoutSnapshot, StylePatchExt, Transform2D, VisualSnapshot,
    STYLE_LAYER_ALL,
};

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

// SAFETY: Delegates directly to `std::alloc::System` while counting per-thread allocations.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let r = f();
    let after = THREAD_ALLOCS.with(Cell::get);
    (after.wrapping_sub(before), r)
}

#[test]
fn css_parsers_transforms_gradients_and_patch_views_allocate_zero_bytes() {
    let colors = [
        "#2563eb",
        "#fff",
        "rgba(37, 99, 235, 0.75)",
        "rgb(15, 23, 42)",
        "transparent",
        "#10b981cc",
    ];
    let dims = ["auto", "100%", "24px", "12.5px", "320", "50%"];
    let stops = [
        GradientStop::new(0.0, ColorRgba::rgb(37, 99, 235)),
        GradientStop::new(0.5, ColorRgba::rgb(139, 92, 246)),
        GradientStop::new(1.0, ColorRgba::rgb(16, 185, 129)),
    ];
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

    let patch = RowPatch::new()
        .bg(ColorRgba::rgb(15, 23, 42))
        .opacity(0.85)
        .flex_row()
        .width_px(240.0)
        .padding(16.0)
        .rotate(12.0);
    let mut patch_words = Vec::new();
    assert!(patch.write_words(&mut patch_words));
    let patch_view = RowPatchView::from_words(&patch_words).expect("valid patch view");

    // Pre-populate a NodeStore with flat resources & warm layout/visual columns.
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<LayoutTable>(n);
        tx.edit::<Visual>(n)
            .box_shadows(Some(BoxShadows::new(vec![BoxShadow::new(
                0.0,
                4.0,
                12.0,
                Rgba::new(0, 0, 0, 64),
            )])))
            .background_fill(Some(BackgroundFillRes(BackgroundFill::linear(
                90.0,
                stops.to_vec(),
            ))));
        tx.edit::<GridLayout>(n)
            .template_columns(Some(GridTracks::new(vec![
                GridTrack::Px(120.0),
                GridTrack::Fr(1.0),
            ])));
        tx.commit_into(&mut changes);
        n
    };

    // Warm inline-style write path once so table rows & dirty lists have steady-state capacity.
    {
        let mut tx = store.tx();
        apply_inline_style_layers_to_tx(
            &mut tx,
            node,
            "width: 240px; height: 48px; flex-direction: row; opacity: 0.9; background-color: #0f172a;",
            STYLE_LAYER_ALL,
        );
        tx.commit_into(&mut changes);
    }

    let (allocs, acc) = measure_allocs(|| {
        let mut sum = 0u64;
        for _ in 0..32 {
            for &raw in black_box(&colors) {
                if let Some(c) = parse_color(raw) {
                    sum = sum.wrapping_add(u64::from(c.to_u32()));
                }
            }
            for &raw in black_box(&dims) {
                let _ = black_box(parse_dim(raw));
            }
            let _ = black_box(parse_flex_direction("row-reverse"));
            let _ = black_box(parse_flex_justify("space-between"));
            let _ = black_box(parse_flex_align("center"));
            sum = sum.wrapping_add(u64::from(parse_font_weight("bold")));

            let m1 = black_box(&t1).to_matrix_2d(100.0, 50.0);
            let m2 = black_box(&t2).to_matrix_2d(100.0, 50.0);
            let m = affine_concat(&m1, &m2);
            let p = affine_map_point(&m, 50.0, 25.0);
            let _ = black_box(affine_inverse_map_point(&m, p.0, p.1));

            for i in 0..8 {
                let c = sample_normalized_stops(black_box(&stops), i as f32 / 7.0);
                sum = sum.wrapping_add(u64::from(c.to_u32()));
            }

            let mut vis = VisualSnapshot::default();
            let mut lay = LayoutSnapshot::default();
            let mut ext = LayoutExt::default();
            apply_patch_view_to_visual(patch_view, &mut vis);
            apply_patch_view_to_layout(patch_view, &mut lay);
            apply_patch_view_to_layout_ext(patch_view, &mut ext);
            let tf = transform_from_patch_view(patch_view);
            sum = sum.wrapping_add(u64::from(vis.background.to_u32()) + u64::from(tf.is_some()));

            // Zero-alloc flat resource views from NodeStore.
            let v = store.row::<Visual>(node);
            if let Some(sh) = store.resource::<BoxShadows>(v.box_shadows()) {
                for s in sh.iter() {
                    sum = sum.wrapping_add(u64::from(s.color.0));
                }
            }
            if let Some(gl) = store.try_row::<GridLayout>(node)
                && let Some(tracks) = store.resource::<GridTracks>(gl.template_columns())
            {
                sum = sum.wrapping_add(tracks.iter().count() as u64);
            }

            // Steady-state inline CSS application directly onto warm NodeStore rows.
            let mut tx = store.tx();
            apply_inline_style_layers_to_tx(
                &mut tx,
                node,
                "width: 260px; height: 52px; flex-direction: column; opacity: 0.75; background-color: #1e293b;",
                STYLE_LAYER_ALL,
            );
            tx.commit_into(&mut changes);
        }
        sum
    });

    black_box(acc);
    assert_eq!(
        allocs, 0,
        "steady-state nve-tables hot paths (parsers, transforms, gradients, patch views, flat resources, inline style tx) must allocate 0 bytes"
    );
}
