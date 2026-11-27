//! `taffy_relayout_1k_hoisted`: on a 1,001-node tree with 200 absolute badges and 100
//! multi-level fixed toasts, steady-state `TaffyStoreLayout::sync_changes` + `compute_layout`
//! performs ZERO heap allocations, and dirty out-of-flow relayout allocates the same constant
//! (Taffy-internal scratch) every frame, so hoisted storage never grows.

#[path = "support/counting_alloc.rs"]
mod counting_alloc;
#[path = "support/hoisted_tree.rs"]
mod hoisted_tree;

use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use counting_alloc::count_allocs;
use hoisted_tree::{build_hoisted_tree, commit_badge_frame, commit_paint_frame, CARDS, VIEWPORT};
use nve_backend_headless::TaffyStoreLayout;
use nve_components::tables::core_realization_order;
use nve_store::{ChangeSet, NodeId, NodeStore};

#[test]
fn counting_alloc_probe_fires_on_this_thread_only() {
    let (_, own) = count_allocs(|| black_box(Vec::<u64>::with_capacity(32)));
    assert!(
        own >= 1,
        "allocator hook must count this thread's allocations"
    );

    let (_, foreign) = count_allocs(|| {
        std::thread::spawn(|| black_box(vec![0u8; 4096]).len())
            .join()
            .unwrap_or(0)
    });
    // `spawn` itself allocates on this thread; the child's 4 KiB buffer must not add a count.
    let (_, spawn_only) = count_allocs(|| std::thread::spawn(|| 0usize).join().unwrap_or(0));
    assert_eq!(
        foreign, spawn_only,
        "child-thread allocations leaked into this thread's count"
    );
}

#[test]
fn taffy_relayout_1k_hoisted() {
    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();
    let mut layout = TaffyStoreLayout::new();
    let tree = build_hoisted_tree(&mut store, &mut changes);
    let live = (0..store.capacity())
        .filter(|&i| store.id_at(i).is_some())
        .count();
    assert_eq!(live, 1 + CARDS * 10);

    layout.sync_changes(&store, &changes);
    layout.compute_layout(&store, NodeId::ROOT, VIEWPORT);

    // Hoisted geometry: badges sit in their card's corners, fixed toasts escape card + inner box.
    let card = layout.layout_of(tree.cards[3]).expect("card laid out");
    let left = layout
        .layout_of(tree.badges[6])
        .expect("left badge laid out");
    let right = layout
        .layout_of(tree.badges[7])
        .expect("right badge laid out");
    assert_eq!((left.x, left.y), (card.x + 2.0, card.y + 2.0));
    assert_eq!(right.x + right.width, card.x + card.width - 2.0);
    let toast = layout.layout_of(tree.toasts[3]).expect("toast laid out");
    assert_eq!(toast.x, VIEWPORT.width - 16.0 - 200.0);
    assert_eq!(toast.y, VIEWPORT.height - (16.0 + 3.0) - 48.0);

    let paint_frame = |store: &mut NodeStore,
                       changes: &mut ChangeSet,
                       layout: &mut TaffyStoreLayout,
                       frame: usize| {
        commit_paint_frame(store, changes, &tree, frame);
        layout.sync_changes(store, changes);
        layout.compute_layout(store, NodeId::ROOT, VIEWPORT);
    };
    let oof_frame = |store: &mut NodeStore,
                     changes: &mut ChangeSet,
                     layout: &mut TaffyStoreLayout,
                     frame: usize| {
        commit_badge_frame(store, changes, &tree, frame);
        layout.sync_changes(store, changes);
        layout.compute_layout(store, NodeId::ROOT, VIEWPORT);
    };

    // Warm every scratch buffer across both lanes (two passes over every badge).
    for frame in 0..tree.badges.len() * 2 {
        paint_frame(&mut store, &mut changes, &mut layout, frame);
        oof_frame(&mut store, &mut changes, &mut layout, frame);
    }

    // 1. Steady-state paint frames (sync + compute; the clean pass is skipped instead of letting
    //    Taffy re-run its allocating root positioning pass for the 100 fixed toasts) while a
    //    background thread allocates continuously: strictly 0 allocations.
    let stop = Arc::new(AtomicBool::new(false));
    let noise = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut churn = 0usize;
            while !stop.load(Ordering::Relaxed) {
                churn += black_box(vec![churn as u8; 256]).len();
            }
            churn
        })
    };
    let (_, steady) = count_allocs(|| {
        for frame in 0..200 {
            paint_frame(&mut store, &mut changes, &mut layout, frame);
        }
    });
    stop.store(true, Ordering::Relaxed);
    assert!(
        noise.join().is_ok_and(|churn| churn > 0),
        "noise thread must have allocated"
    );
    assert_eq!(
        steady, 0,
        "steady-state hoisted relayout allocated {steady} times over 200 frames"
    );

    // 2. Dirty out-of-flow relayout (badge inset change re-runs the card's and the root's
    //    positioning passes, rewriting both hoisted lists): only Taffy's per-pass scratch
    //    allocates (root flex over 100 cards + 100 fixed toasts), so each badge costs exactly the
    //    same in every round (no amortized hoisted-pool growth).
    let badge = tree.badges[6];
    let before = layout.layout_of(badge).expect("badge laid out");
    let mut round = |round: usize| -> Vec<usize> {
        let first = round * tree.badges.len();
        (first..first + tree.badges.len())
            .map(|frame| count_allocs(|| oof_frame(&mut store, &mut changes, &mut layout, frame)).1)
            .collect()
    };
    let (round_a, round_b) = (round(2), round(3));
    assert_eq!(
        round_a, round_b,
        "dirty hoisted relayout allocations drift between rounds"
    );
    assert!(
        round_a.iter().all(|&n| n > 0),
        "badge lane must run Taffy, not skip the pass"
    );
    assert!(
        round_a.iter().all(|&n| n <= 3 * CARDS),
        "dirty hoisted relayout exceeded Taffy's per-root-child scratch: {round_a:?}"
    );
    assert_ne!(
        layout.layout_of(badge),
        Some(before),
        "badge lane must really move the badge"
    );

    // 3. Geometry still matches after 1,000+ incremental frames.
    let toast = layout.layout_of(tree.toasts[3]).expect("toast laid out");
    assert_eq!(toast.x, VIEWPORT.width - 16.0 - 200.0);
}
