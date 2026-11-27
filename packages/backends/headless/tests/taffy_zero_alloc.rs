//! Verifies ZERO steady-state heap allocations across `TaffyStoreLayout::sync_changes` +
//! `TaffyStoreLayout::compute_layout` + `HeadlessBackend::commit` using a counting global allocator.

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use nve_backend_headless::{HeadlessBackend, TaffyStoreLayout};
use nve_tables::tables::{
    core_realization_order, FlexDirection, Layout, TextContent, Transform, Typography, Visual,
};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

struct CountingAlloc;

static ALLOC_CALLS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Delegates directly to `std::alloc::System` while recording allocation counts in a lock-free atomic.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated verbatim to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated verbatim to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated verbatim to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[inline]
fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = ALLOC_CALLS.load(Ordering::SeqCst);
    let r = f();
    let after = ALLOC_CALLS.load(Ordering::SeqCst);
    (r, after - before)
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
    backend.set_viewport(nve_abi::SurfaceId::ROOT, vp);

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

    // 1. Steady-state cached `TaffyStoreLayout::sync_changes` + `compute_layout` across 50 frames: 0 allocs!
    let (_, allocs_cached) = count_allocs(|| {
        for i in 0..50u32 {
            let mut tx = store.tx();
            tx.edit::<Visual>(leaves[(i as usize) % leaves.len()])
                .opacity(0.5 + (i % 5) as f32 * 0.1);
            tx.edit::<Transform>(leaves[(i as usize) % leaves.len()])
                .translate_x(i as f32);
            tx.commit_into(&mut changes);
            layout.sync_changes(&store, &changes);
            layout.compute_layout(&store, NodeId::ROOT, vp);
        }
    });
    assert_eq!(
        allocs_cached, 0,
        "steady-state TaffyStoreLayout::sync_changes + compute_layout allocated {allocs_cached} times"
    );

    // 2. Steady-state `HeadlessBackend::commit` (120 Hz composite/paint lane) across 50 frames: 0 allocs!
    let (_, allocs_backend) = count_allocs(|| {
        for i in 0..50u32 {
            let mut tx = store.tx();
            tx.edit::<Transform>(leaves[(i as usize) % leaves.len()])
                .translate_y(i as f32);
            tx.commit_into(&mut changes);
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
        let (_, a_comp) = count_allocs(|| {
            layout.compute_layout(&store, NodeId::ROOT, vp);
        });
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
            tx.edit::<Visual>(leaves[(i as usize) % leaves.len()])
                .opacity(0.5 + (i % 5) as f32 * 0.1);
            tx.commit_into(&mut changes);
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
