use crate::TaffyStoreLayout;
use nve_components::tables::{Display, FlexDirection, Layout, Position, Visual};
use nve_engine::Viewport;
use nve_store::{ChangeSet, Length, NodeId, NodeStore};

pub(super) const VP: Viewport = Viewport {
    width: 400.0,
    height: 300.0,
    scale_factor: 1.0,
};

pub(super) fn layout_once(
    store: &mut NodeStore,
    changes: &ChangeSet,
    layout: &mut TaffyStoreLayout,
) {
    layout.sync_changes(store, changes);
    layout.compute_layout(store, NodeId::ROOT, VP);
}

/// Incremental layout must match a from-scratch layout of the same store: every reachable rect and
/// every hoisted list, with consistent pool accounting.
pub(super) fn assert_matches_fresh_layout(store: &NodeStore, incremental: &TaffyStoreLayout) {
    assert_matches_layout_of(store, incremental, TaffyStoreLayout::new());
}

/// [`assert_matches_fresh_layout`] against a caller-configured `fresh` engine (e.g. a custom
/// text measurer) that has not laid out yet.
pub(super) fn assert_matches_layout_of(
    store: &NodeStore,
    incremental: &TaffyStoreLayout,
    mut fresh: TaffyStoreLayout,
) {
    assert!(fresh.compute_layout(store, NodeId::ROOT, VP));
    for idx in 0..store.capacity() {
        let Some(id) = store.id_at(idx) else {
            continue;
        };
        let expected = fresh.layout_of(id);
        assert_eq!(incremental.layout_of(id), expected, "rect of {id:?}");
        if expected.is_some() {
            assert_eq!(
                incremental.hoisted.get(idx),
                fresh.hoisted.get(idx),
                "hoisted list of {id:?}"
            );
        }
    }
    incremental.hoisted.assert_consistent();
}

#[test]
fn empty_tree_and_dead_root_hoist_nothing() {
    let mut store = NodeStore::new();
    let mut layout = TaffyStoreLayout::new();
    layout.compute_layout(&store, NodeId::ROOT, VP);
    let root = layout.layout_of(NodeId::ROOT).expect("root laid out");
    assert_eq!((root.width, root.height), (400.0, 300.0));
    assert!(layout.hoisted.get(NodeId::ROOT.index()).is_empty());
    assert_eq!(layout.hoisted.pool_len(), 0);
    assert_matches_fresh_layout(&store, &layout);

    // A dead root invalidates every rect without touching hoisted storage.
    let mut changes = ChangeSet::default();
    let orphan = {
        let mut tx = store.tx();
        let orphan = tx.create();
        tx.commit_into(&mut changes);
        orphan
    };
    layout.sync_changes(&store, &changes);
    let mut tx = store.tx();
    tx.remove(orphan);
    tx.commit_into(&mut changes);
    layout.sync_changes(&store, &changes);
    layout.compute_layout(&store, orphan, VP);
    assert_eq!(layout.layout_of(NodeId::ROOT), None);
    layout.compute_layout(&store, NodeId::ROOT, VP);
    assert!(
        layout.layout_of(NodeId::ROOT).is_some(),
        "live root recomputes after a dead root"
    );

    layout.reset();
    assert_eq!(layout.hoisted.pool_len(), 0);
    layout.compute_layout(&store, NodeId::ROOT, VP);
    assert_matches_fresh_layout(&store, &layout);
}

#[test]
fn multi_level_fixed_hoists_past_nested_containing_blocks_to_root() {
    let mut store = NodeStore::new();
    let mut layout = TaffyStoreLayout::new();
    let mut changes = ChangeSet::default();
    let (outer, mid, inner, abs_in_abs, root_abs, fixed) = {
        let mut tx = store.tx();
        let [outer, mid, inner, abs_in_abs, root_abs, fixed] = [(); 6].map(|_| tx.create());
        tx.set_children(NodeId::ROOT, &[outer, root_abs]);
        tx.set_children(outer, &[mid]);
        tx.set_children(mid, &[inner]);
        tx.set_children(inner, &[abs_in_abs, fixed]);
        for (id, size) in [(outer, 200.0), (mid, 150.0), (inner, 100.0)] {
            tx.edit::<Layout>(id)
                .padding_left(Length::points(10.0))
                .padding_top(Length::points(10.0))
                .width(Length::points(size))
                .height(Length::points(size));
        }
        tx.edit::<Layout>(inner)
            .position(Position::Absolute)
            .inset_left(Length::points(5.0));
        tx.edit::<Layout>(abs_in_abs)
            .position(Position::Absolute)
            .inset_left(Length::points(1.0))
            .inset_top(Length::points(2.0))
            .width(Length::points(4.0))
            .height(Length::points(4.0));
        tx.edit::<Layout>(root_abs)
            .position(Position::Absolute)
            .inset_right(Length::points(0.0))
            .width(Length::points(8.0))
            .height(Length::points(8.0));
        tx.edit::<Layout>(fixed)
            .position(Position::Fixed)
            .inset_right(Length::points(10.0))
            .inset_bottom(Length::points(20.0))
            .width(Length::points(30.0))
            .height(Length::points(40.0));
        tx.commit_into(&mut changes);
        (outer, mid, inner, abs_in_abs, root_abs, fixed)
    };
    layout_once(&mut store, &changes, &mut layout);

    // The root's list holds its own absolute child, then (appended by the root positioning pass)
    // the fixed box that bubbled past `inner`, `mid`, and `outer`.
    assert_eq!(layout.hoisted.get(NodeId::ROOT.index()), &[root_abs, fixed]);
    assert_eq!(layout.hoisted.get(mid.index()), &[inner]);
    assert_eq!(layout.hoisted.get(inner.index()), &[abs_in_abs]);
    assert!(layout.hoisted.get(outer.index()).is_empty());
    let r_fixed = layout.layout_of(fixed).expect("fixed laid out");
    assert_eq!(
        (r_fixed.x, r_fixed.y),
        (400.0 - 10.0 - 30.0, 300.0 - 20.0 - 40.0)
    );
    let r_inner = layout.layout_of(inner).expect("inner laid out");
    assert_eq!((r_inner.x, r_inner.y), (10.0 + 5.0, 20.0));
    let r_abs = layout.layout_of(abs_in_abs).expect("abs-in-abs laid out");
    assert_eq!((r_abs.x, r_abs.y), (r_inner.x + 1.0, r_inner.y + 2.0));
    assert_matches_fresh_layout(&store, &layout);

    // Relayout of only the inner subtree keeps the root's cached list (fixed box included).
    let mut tx = store.tx();
    tx.edit::<Layout>(abs_in_abs).inset_top(Length::points(6.0));
    tx.commit_into(&mut changes);
    layout_once(&mut store, &changes, &mut layout);
    assert_eq!(layout.hoisted.get(NodeId::ROOT.index()), &[root_abs, fixed]);
    assert_matches_fresh_layout(&store, &layout);

    // A layout edit on a detached node reaches no laid-out ancestor: the root list survives.
    let detached = {
        let mut tx = store.tx();
        let detached = tx.create();
        tx.edit::<Layout>(detached)
            .position(Position::Fixed)
            .width(Length::points(9.0));
        tx.commit_into(&mut changes);
        detached
    };
    layout_once(&mut store, &changes, &mut layout);
    let mut tx = store.tx();
    tx.edit::<Layout>(detached).width(Length::points(11.0));
    tx.commit_into(&mut changes);
    layout_once(&mut store, &changes, &mut layout);
    assert_eq!(
        layout.layout_of(detached),
        None,
        "detached nodes are never laid out"
    );
    assert_eq!(layout.hoisted.get(NodeId::ROOT.index()), &[root_abs, fixed]);
    assert_matches_fresh_layout(&store, &layout);

    // Dead-root round trip: every rect goes stale, then the live root re-records the full list.
    let dead = {
        let mut tx = store.tx();
        tx.remove(detached);
        tx.commit_into(&mut changes);
        detached
    };
    layout.sync_changes(&store, &changes);
    assert!(
        layout.compute_layout(&store, dead, VP),
        "a dead root reports a pass"
    );
    assert_eq!(layout.layout_of(fixed), None);
    assert!(
        layout.compute_layout(&store, NodeId::ROOT, VP),
        "live root recomputes"
    );
    assert_eq!(layout.hoisted.get(NodeId::ROOT.index()), &[root_abs, fixed]);
    assert_eq!(layout.hoisted.get(inner.index()), &[abs_in_abs]);
    assert_matches_fresh_layout(&store, &layout);
}

#[test]
fn dynamic_insert_remove_reparent_and_restyle_of_hoisted_children_match_fresh_layout() {
    let mut store = NodeStore::new();
    let mut layout = TaffyStoreLayout::new();
    let mut changes = ChangeSet::default();
    let (a, b) = {
        let mut tx = store.tx();
        let a = tx.create();
        let b = tx.create();
        tx.set_children(NodeId::ROOT, &[a, b]);
        for id in [a, b] {
            tx.edit::<Layout>(id)
                .flex_direction(FlexDirection::Row)
                .width(Length::points(180.0))
                .height(Length::points(60.0));
        }
        tx.commit_into(&mut changes);
        (a, b)
    };
    layout_once(&mut store, &changes, &mut layout);
    assert_matches_fresh_layout(&store, &layout);

    let spawn_abs = |store: &mut NodeStore, changes: &mut ChangeSet, parent: NodeId, at: f32| {
        let len = store.children(parent).len();
        let mut tx = store.tx();
        let id = tx.create();
        tx.edit::<Layout>(id)
            .position(Position::Absolute)
            .inset_left(Length::points(at))
            .width(Length::points(6.0))
            .height(Length::points(6.0));
        tx.insert_child(parent, len, id);
        tx.commit_into(changes);
        id
    };

    // Insert: a grows 0 -> 6 absolute children one frame at a time (in-place tail growth and
    // relocation once b's list sits behind a's).
    let mut a_abs = Vec::new();
    for i in 0..6 {
        a_abs.push(spawn_abs(&mut store, &mut changes, a, i as f32 * 10.0));
        layout_once(&mut store, &changes, &mut layout);
        spawn_abs(&mut store, &mut changes, b, i as f32 * 7.0);
        layout_once(&mut store, &changes, &mut layout);
        assert_matches_fresh_layout(&store, &layout);
        assert_eq!(layout.hoisted.get(a.index()), a_abs.as_slice());
    }

    // Remove one hoisted child and reparent another from a to b.
    let mut tx = store.tx();
    tx.remove(a_abs[1]);
    tx.commit_into(&mut changes);
    layout_once(&mut store, &changes, &mut layout);
    assert_matches_fresh_layout(&store, &layout);
    assert!(!layout.hoisted.get(a.index()).contains(&a_abs[1]));

    let b_len = store.children(b).len();
    let mut tx = store.tx();
    tx.insert_child(b, b_len, a_abs[2]);
    tx.commit_into(&mut changes);
    layout_once(&mut store, &changes, &mut layout);
    assert_matches_fresh_layout(&store, &layout);
    assert!(!layout.hoisted.get(a.index()).contains(&a_abs[2]));
    assert_eq!(layout.hoisted.get(b.index()).last(), Some(&a_abs[2]));

    // Restyle: absolute -> in-flow -> absolute, and display:none on the containing block.
    for position in [Position::Relative, Position::Absolute] {
        let mut tx = store.tx();
        tx.edit::<Layout>(a_abs[3]).position(position);
        tx.commit_into(&mut changes);
        layout_once(&mut store, &changes, &mut layout);
        assert_matches_fresh_layout(&store, &layout);
        assert_eq!(
            layout.hoisted.get(a.index()).contains(&a_abs[3]),
            position == Position::Absolute
        );
    }
    for display in [Display::None, Display::Flex] {
        let mut tx = store.tx();
        tx.edit::<Layout>(b).display(display);
        tx.commit_into(&mut changes);
        layout_once(&mut store, &changes, &mut layout);
        assert_matches_fresh_layout(&store, &layout);
        assert_eq!(
            layout.hoisted.get(b.index()).is_empty(),
            display == Display::None
        );
    }

    // Remove the whole subtree of a: its span is released, b's survives.
    let mut tx = store.tx();
    tx.remove(a);
    tx.commit_into(&mut changes);
    layout_once(&mut store, &changes, &mut layout);
    assert_matches_fresh_layout(&store, &layout);
    assert!(layout.hoisted.get(a.index()).is_empty());
    assert!(!layout.hoisted.get(b.index()).is_empty());
}

#[test]
fn repeated_relayouts_reach_a_fixed_point_and_skip_clean_frames() {
    let mut store = NodeStore::new();
    let mut layout = TaffyStoreLayout::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let cards: Vec<NodeId> = (0..32).map(|_| tx.create()).collect();
    let mut badges = Vec::new();
    for &card in &cards {
        tx.edit::<Layout>(card).height(Length::points(20.0));
        tx.edit::<Visual>(card).opacity(1.0);
        let kids: Vec<NodeId> = (0..3).map(|_| tx.create()).collect();
        for &kid in &kids {
            tx.edit::<Layout>(kid)
                .position(Position::Absolute)
                .width(Length::points(4.0))
                .height(Length::points(4.0));
        }
        tx.set_children(card, &kids);
        badges.extend(kids);
    }
    tx.set_children(NodeId::ROOT, &cards);
    tx.commit_into(&mut changes);
    layout_once(&mut store, &changes, &mut layout);

    let frame = |store: &mut NodeStore,
                 changes: &mut ChangeSet,
                 layout: &mut TaffyStoreLayout,
                 i: usize| {
        let mut tx = store.tx();
        let badge = badges[i % badges.len()];
        // Toggle one badge in and out of flow so hoisted lists shrink and regrow.
        let position = if (i / badges.len()).is_multiple_of(2) {
            Position::Relative
        } else {
            Position::Absolute
        };
        tx.edit::<Layout>(badge)
            .position(position)
            .inset_left(Length::points((i % 9) as f32));
        tx.commit_into(changes);
        layout_once(store, changes, layout);
    };
    for i in 0..badges.len() * 4 {
        frame(&mut store, &mut changes, &mut layout, i);
    }
    let warm_pool = layout.hoisted.pool_len();
    for i in 0..badges.len() * 8 {
        frame(&mut store, &mut changes, &mut layout, i);
    }
    assert_eq!(
        layout.hoisted.pool_len(),
        warm_pool,
        "hoisted pool must not grow across relayouts"
    );
    assert_matches_fresh_layout(&store, &layout);

    // Paint-only frame: the pass is skipped and every rect stays valid.
    let pass = layout.layout_gen;
    let mut tx = store.tx();
    tx.edit::<Visual>(cards[0]).opacity(0.5);
    tx.commit_into(&mut changes);
    layout_once(&mut store, &changes, &mut layout);
    assert_eq!(
        layout.layout_gen, pass,
        "clean frame must skip the layout pass"
    );
    assert_matches_fresh_layout(&store, &layout);
    // Viewport change forces a pass even with no store changes.
    layout.compute_layout(&store, NodeId::ROOT, Viewport { width: 320.0, ..VP });
    assert_ne!(layout.layout_gen, pass);
}
