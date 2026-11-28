//! Global-allocator (`CountingAlloc`) zero-allocation hot-path guard for `nve-tables`:
//! 2D affine transforms, normalized gradient sampling, zero-copy `RowPatchView` snapshot
//! application, steady-state `RowPatch` transactions, `effective_text_color`, and flat
//! `NodeStore` resource view iteration.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use nve_abi::ColorRgba;
use nve_store::{ChangeSet, FlatResource, NodeId, NodeStore, Rgba, RowPatch, RowPatchView};
use nve_tables::layout_ext::GridTrack;
use nve_tables::tables::{
    register_core_tables, BackgroundFillRes, BoxShadow, BoxShadows, GridLayout, GridTracks,
    Layout as LayoutTable, Typography, Visual,
};
use nve_tables::{
    affine_concat, affine_inverse_map_point, affine_map_point, apply_patch_view_to_layout,
    apply_patch_view_to_layout_ext, apply_patch_view_to_visual, effective_text_color,
    sample_normalized_stops, transform_from_patch_view, BackgroundFill, GradientStop, LayoutExt,
    LayoutSnapshot, StylePatchExt, Transform2D, VisualSnapshot,
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
fn transforms_gradients_patch_views_and_row_patches_allocate_zero_bytes() {
    register_core_tables();
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
    let patch_alt = RowPatch::new()
        .bg(ColorRgba::rgb(30, 41, 59))
        .opacity(0.75)
        .flex_col()
        .width_px(260.0)
        .padding(12.0)
        .rotate(0.0);
    let mut patch_words = Vec::new();
    assert!(patch.write_words(&mut patch_words));
    let patch_view = RowPatchView::from_words(&patch_words).expect("valid patch view");
    let mut patch_alt_words = Vec::new();
    assert!(patch_alt.write_words(&mut patch_alt_words));
    let patch_alt_view = RowPatchView::from_words(&patch_alt_words).expect("valid alt patch view");

    // Pre-populate a NodeStore with flat resources & warm layout/visual/typography columns.
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (node, leaf) = {
        let mut tx = store.tx();
        let n = tx.create();
        let child = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.set_children(n, &[child]);
        tx.edit::<LayoutTable>(n);
        tx.edit::<Typography>(n).color(Rgba::rgb(226, 232, 240));
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
        (n, child)
    };

    // Warm typed RowPatch write path once so table rows & dirty lists have steady-state capacity.
    for p in [patch_view, patch_alt_view] {
        let mut tx = store.tx();
        tx.apply_row_patch(node, p);
        tx.commit_into(&mut changes);
    }

    let (allocs, acc) = measure_allocs(|| {
        let mut sum = 0u64;
        for step in 0..32 {
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

            // Zero-alloc inherited text color walk + flat resource views from NodeStore.
            sum = sum.wrapping_add(u64::from(effective_text_color(&store, leaf).0));
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

            // Steady-state typed RowPatch application directly onto warm NodeStore rows.
            let active = if step % 2 == 0 { patch_view } else { patch_alt_view };
            let mut tx = store.tx();
            tx.apply_row_patch(node, active);
            tx.commit_into(&mut changes);
        }
        sum
    });

    black_box(acc);
    assert_eq!(
        allocs, 0,
        "steady-state nve-tables hot paths (transforms, gradients, patch views, flat resources, typed RowPatch tx) must allocate 0 bytes"
    );
}
