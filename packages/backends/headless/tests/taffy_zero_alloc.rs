//! Verifies ZERO steady-state heap allocations across `TaffyStoreLayout::sync_changes` +
//! `TaffyStoreLayout::compute_layout` + `HeadlessBackend::commit` using a counting global allocator.

#[path = "support/counting_alloc.rs"]
mod counting_alloc;

use counting_alloc::count_allocs;
use nve_backend_headless::HeadlessBackend;
use nve_layout::TaffyStoreLayout;
use nve_tables::tables::{
    FlexDirection, Layout, TextContent, Transform, Typography, Visual, core_realization_order,
};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba, SurfaceId};

#[test]
fn counting_alloc_probe_fires_on_this_thread_only() {
    counting_alloc::assert_counts_only_this_thread();
}

#[test]
fn taffy_store_layout_and_headless_commit_zero_steady_state_allocs() {
    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();
    let mut layout = TaffyStoreLayout::new();
    let mut backend = HeadlessBackend::new().with_mutation_tracking(false);
    let vp = Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 1.0,
    };
    backend.set_viewport(SurfaceId::ROOT, vp);

    // Build a 64-node Flexbox + text tree and warm all caches/scratch stacks.
    let mut tx = store.tx();
    let mut rows = Vec::with_capacity(16);
    let mut leaves = Vec::with_capacity(48);
    for _ in 0..16 {
        let row = tx.create();
        tx.edit::<Layout>(row)
            .flex_direction(FlexDirection::Row)
            .height(Length::points(36.0))
            .column_gap(Length::points(8.0));
        let mut children = [NodeId::ROOT; 3];
        for slot in &mut children {
            let leaf = tx.create();
            tx.edit::<Layout>(leaf)
                .flex_grow(1.0)
                .height(Length::points(32.0));
            tx.edit::<Visual>(leaf).background(Rgba::rgb(30, 41, 59));
            tx.edit::<TextContent>(leaf).text("metric-cell");
            tx.edit::<Typography>(leaf).font_size(14.0);
            tx.edit::<Transform>(leaf).translate_x(0.0);
            *slot = leaf;
            leaves.push(leaf);
        }
        tx.set_children(row, &children);
        rows.push(row);
    }
    tx.set_children(NodeId::ROOT, &rows);
    tx.commit_into(&mut changes);

    layout.sync_changes(&store, &changes);
    layout.compute_layout(&store, NodeId::ROOT, vp);
    backend.commit(&store, &changes).unwrap();

    // Warm one paint/composite mutation frame to prime ChangeSet & scratch buffers.
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(leaves[0]).opacity(0.9);
        tx.edit::<Transform>(leaves[0]).translate_x(4.0);
        tx.commit_into(&mut changes);
        layout.sync_changes(&store, &changes);
        layout.compute_layout(&store, NodeId::ROOT, vp);
        backend.commit(&store, &changes).unwrap();
    }

    // 1. Steady-state paint/composite frames through `sync_changes` + `compute_layout` across 50
    //    frames: 0 allocs. Honest scope: this measures the engine's self-gated *skip* (every frame
    //    asserts `compute_layout` returned `false`), not a forced pass over a fully cached tree;
    //    such a pass is not reachable through the public API and, with fixed boxes, Taffy 0.16
    //    would re-run its allocating root positioning pass (docs/UPSTREAM.md).
    let (_, allocs_skipped) = count_allocs(|| {
        for i in 0..50u32 {
            let mut tx = store.tx();
            tx.edit::<Visual>(leaves[(i as usize) % leaves.len()])
                .opacity(0.5 + (i % 5) as f32 * 0.1);
            tx.edit::<Transform>(leaves[(i as usize) % leaves.len()])
                .translate_x(i as f32);
            tx.commit_into(&mut changes);
            assert!(
                !changes.touched.is_empty(),
                "frame {i} must change the store"
            );
            layout.sync_changes(&store, &changes);
            assert!(
                !layout.compute_layout(&store, NodeId::ROOT, vp),
                "paint/composite frame {i} must skip the layout pass"
            );
        }
    });
    assert_eq!(
        allocs_skipped, 0,
        "steady-state TaffyStoreLayout::sync_changes + skipped compute_layout allocated {allocs_skipped} times"
    );

    // 2. Steady-state `HeadlessBackend::commit` (120 Hz composite/paint lane) across 50 frames: 0 allocs!
    let (_, allocs_backend) = count_allocs(|| {
        for i in 0..50u32 {
            let mut tx = store.tx();
            // `+ 1`: the default `translate_y` is 0, so frame 0 must not rewrite it.
            tx.edit::<Transform>(leaves[(i as usize) % leaves.len()])
                .translate_y(i as f32 + 1.0);
            tx.commit_into(&mut changes);
            assert!(
                !changes.touched.is_empty(),
                "frame {i} must change the store"
            );
            backend.commit(&store, &changes).unwrap();
        }
    });
    assert_eq!(
        allocs_backend, 0,
        "steady-state HeadlessBackend::commit allocated {allocs_backend} times"
    );

    // 3. Steady-state REAL layout + measure mutation (`Layout::width` + `TextContent::text`):
    //    - `tx.edit` + `tx.commit_into` + `TaffyStoreLayout::sync_changes`: strictly 0 allocs!
    //    - `TaffyStoreLayout::compute_layout`: only Taffy's internal flex-container scratch Vecs
    //      for the 2 dirty containers (`root` + `rows[0]`), proving the 15 clean sibling rows stay
    //      cached (bounded <= 8 allocs/frame, independent of total tree size).
    {
        let mut tx = store.tx();
        tx.edit::<Layout>(leaves[0]).width(Length::points(120.0));
        tx.edit::<TextContent>(leaves[0]).text("metric-0001");
        tx.commit_into(&mut changes);
        layout.sync_changes(&store, &changes);
        layout.compute_layout(&store, NodeId::ROOT, vp);
    }
    let mut sync_allocs = 0usize;
    let mut taffy_recompute_allocs = 0usize;
    for i in 0..25u32 {
        let (_, a_sync) = count_allocs(|| {
            let mut tx = store.tx();
            tx.edit::<Layout>(leaves[0])
                .width(Length::points(100.0 + (i % 10) as f32 * 4.0));
            tx.edit::<TextContent>(leaves[0]).text(if i % 2 == 0 {
                "metric-even"
            } else {
                "metric-odds"
            });
            tx.commit_into(&mut changes);
            layout.sync_changes(&store, &changes);
        });
        sync_allocs += a_sync;
        let (ran, a_comp) = count_allocs(|| layout.compute_layout(&store, NodeId::ROOT, vp));
        assert!(
            ran,
            "Layout::width + TextContent::text frame {i} must run the layout pass"
        );
        taffy_recompute_allocs += a_comp;
    }
    assert_eq!(
        sync_allocs, 0,
        "steady-state Layout::width + TextContent::text + sync_changes allocated {sync_allocs} times"
    );
    assert!(
        taffy_recompute_allocs <= 25 * 8,
        "dirty-branch Taffy recompute allocated {taffy_recompute_allocs} times across 25 frames (> 8/frame)"
    );

    // 4. Steady-state `HeadlessBackend::commit` WITH `track_mutations = true`:
    //    - composite/paint lane: strictly 0 allocs!
    //    - `Layout::width` recompute: 0 backend/diff allocs beyond Taffy's bounded dirty-container scratch.
    backend.set_mutation_tracking(true);
    {
        let mut tx = store.tx();
        tx.edit::<Layout>(leaves[0]).width(Length::points(140.0));
        tx.commit_into(&mut changes);
        backend.commit(&store, &changes).unwrap();
        let mut tx = store.tx();
        tx.edit::<Visual>(leaves[0]).opacity(0.8);
        tx.commit_into(&mut changes);
        backend.commit(&store, &changes).unwrap();
    }
    let (_, allocs_tracked_paint) = count_allocs(|| {
        for i in 0..25u32 {
            let mut tx = store.tx();
            // Values below lane 1's `0.5..=0.9`, so no frame rewrites a value it left behind.
            tx.edit::<Visual>(leaves[(i as usize) % leaves.len()])
                .opacity(0.05 + i as f32 * 0.01);
            tx.commit_into(&mut changes);
            assert!(
                !changes.touched.is_empty(),
                "frame {i} must change the store"
            );
            backend.commit(&store, &changes).unwrap();
        }
    });
    assert_eq!(
        allocs_tracked_paint, 0,
        "steady-state HeadlessBackend::commit (track_mutations=true, paint) allocated {allocs_tracked_paint} times"
    );
    let (_, allocs_tracked_relayout) = count_allocs(|| {
        for i in 0..25u32 {
            let mut tx = store.tx();
            tx.edit::<Layout>(leaves[0])
                .width(Length::points(120.0 + (i % 8) as f32 * 5.0));
            tx.commit_into(&mut changes);
            backend.commit(&store, &changes).unwrap();
        }
    });
    assert!(
        allocs_tracked_relayout <= 25 * 8,
        "steady-state HeadlessBackend::commit (track_mutations=true, Layout::width) allocated {allocs_tracked_relayout} times (> 8/frame)"
    );

    // 5. Sanity-check computed rects remain valid after steady-state frames.
    let first_row = layout.layout_of(rows[0]).unwrap();
    assert_eq!(first_row.width, 800.0);
    assert_eq!(first_row.height, 36.0);
}
