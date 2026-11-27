//! `taffy_relayout_1k_hoisted`: on a 1,001-node tree with 200 absolute badges and 100
//! multi-level fixed toasts, steady-state `TaffyStoreLayout::sync_changes` + `compute_layout`
//! performs ZERO heap allocations, and dirty out-of-flow relayout allocates the same constant
//! (Taffy-internal scratch) every frame, so hoisted storage never grows.

#[path = "support/counting_alloc.rs"]
mod counting_alloc;
#[path = "support/hoisted_tree.rs"]
mod hoisted_tree;

use counting_alloc::count_allocs;
use hoisted_tree::{CARDS, VIEWPORT, build_hoisted_tree, commit_badge_frame, commit_paint_frame};
use nve_layout::TaffyStoreLayout;
use nve_components::tables::core_realization_order;
use nve_store::{ChangeSet, NodeId, NodeStore};

#[test]
fn counting_alloc_probe_fires_on_this_thread_only() {
    counting_alloc::assert_counts_only_this_thread();
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

    // Each lane keeps its own monotonic frame counter so no measured frame revisits a value its
    // warm-up already wrote (the store would elide such an edit and the frame measure nothing).
    // Every frame asserts it really changed the store and whether the engine ran a pass.
    let mut paint_at = 0usize;
    let mut paint_frame =
        |store: &mut NodeStore, changes: &mut ChangeSet, layout: &mut TaffyStoreLayout| {
            commit_paint_frame(store, changes, &tree, paint_at);
            paint_at += 1;
            assert!(
                !changes.touched.is_empty(),
                "paint frame must change the store"
            );
            layout.sync_changes(store, changes);
            assert!(
                !layout.compute_layout(store, NodeId::ROOT, VIEWPORT),
                "paint-only frame must skip the layout pass"
            );
        };
    let mut oof_at = 0usize;
    let mut oof_frame =
        |store: &mut NodeStore, changes: &mut ChangeSet, layout: &mut TaffyStoreLayout| {
            commit_badge_frame(store, changes, &tree, oof_at);
            oof_at += 1;
            assert!(
                !changes.touched.is_empty(),
                "badge frame must change the store"
            );
            layout.sync_changes(store, changes);
            assert!(
                layout.compute_layout(store, NodeId::ROOT, VIEWPORT),
                "badge frame must run the layout pass"
            );
        };

    // Warm every scratch buffer across both lanes (two passes over every badge).
    for _ in 0..tree.badges.len() * 2 {
        paint_frame(&mut store, &mut changes, &mut layout);
        oof_frame(&mut store, &mut changes, &mut layout);
    }

    // 1. Steady-state paint frames (sync + self-gated compute; the engine skips the clean pass
    //    instead of letting Taffy re-run its allocating root positioning pass for the 100 fixed
    //    toasts): strictly 0 allocations over 200 frames that each really edit the store.
    let (_, steady) = count_allocs(|| {
        for _ in 0..200 {
            paint_frame(&mut store, &mut changes, &mut layout);
        }
    });
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
    let mut round = || -> Vec<usize> {
        (0..tree.badges.len())
            .map(|_| count_allocs(|| oof_frame(&mut store, &mut changes, &mut layout)).1)
            .collect()
    };
    let (round_a, round_b) = (round(), round());
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
