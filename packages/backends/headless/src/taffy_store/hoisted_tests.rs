use super::HoistedLists;
use crate::TaffyStoreLayout;
use nve_components::tables::{Display, FlexDirection, Layout, Position, Visual};
use nve_engine::Viewport;
use nve_store::{ChangeSet, Length, NodeId, NodeStore};

const VP: Viewport = Viewport {
    width: 400.0,
    height: 300.0,
    scale_factor: 1.0,
};

/// Deterministic SplitMix64 stream for model-based tests.
struct Rng(u64);

impl Rng {
    fn next(&mut self, bound: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % bound as u64) as usize
    }
}

fn ids(base: u32, n: usize) -> impl Iterator<Item = NodeId> {
    (0..n as u32).map(move |i| NodeId(base + i))
}

#[test]
fn empty_lists_and_out_of_range_reads() {
    let mut lists = HoistedLists::default();
    assert!(lists.get(0).is_empty());
    assert!(lists.get(usize::MAX).is_empty());
    lists.clear(7);
    lists.release(7);
    lists.extend(7, ids(1, 3));
    assert!(
        lists.get(7).is_empty(),
        "writes past `ensure_len` are ignored"
    );
    lists.compact_if_fragmented();
    lists.compact();
    lists.assert_consistent();

    lists.ensure_len(4);
    lists.extend(2, core::iter::empty());
    assert!(lists.get(2).is_empty());
    assert_eq!(lists.pool_len(), 0);
    lists.assert_consistent();
}

#[test]
fn model_based_random_ops_match_vec_of_vecs() {
    const NODES: usize = 48;
    let mut rng = Rng(0xC0FF_EE00);
    let mut lists = HoistedLists::default();
    let mut model: Vec<Vec<NodeId>> = vec![Vec::new(); NODES];
    lists.ensure_len(NODES);

    for step in 0..20_000u32 {
        let idx = rng.next(NODES);
        match rng.next(10) {
            0..=2 => {
                lists.clear(idx);
                model[idx].clear();
            }
            3..=6 => {
                let n = rng.next(9);
                let base = step * 16;
                model[idx].extend(ids(base, n));
                // Odd steps feed a filtered iterator whose upper bound (2n) overestimates its
                // length, like the store-lookup filter in `add_hoisted_children`.
                if step % 2 == 0 {
                    lists.extend(idx, ids(base, n));
                } else {
                    let doubled = (0..2 * n as u32).filter(|i| i % 2 == 0);
                    lists.extend(idx, doubled.map(|i| NodeId(base + i / 2)));
                }
            }
            7 => {
                lists.release(idx);
                model[idx].clear();
            }
            8 => lists.compact_if_fragmented(),
            _ => lists.compact(),
        }
        lists.assert_consistent();
        for (i, expected) in model.iter().enumerate() {
            assert_eq!(
                lists.get(i),
                expected.as_slice(),
                "node {i} after step {step}"
            );
        }
    }
}

#[test]
fn steady_state_rewrites_keep_pool_capacity_and_length() {
    const NODES: usize = 256;
    let mut lists = HoistedLists::default();
    lists.ensure_len(NODES);
    // Warm: every container records up to 6 boxes, then one compaction primes `scratch`.
    for round in 0..4usize {
        for idx in 0..NODES {
            lists.clear(idx);
            lists.extend(idx, ids(idx as u32 * 8, (idx + round) % 7));
        }
        lists.compact();
    }
    let (pool_cap, scratch_cap, pool_len) = (
        lists.pool.capacity(),
        lists.scratch.capacity(),
        lists.pool_len(),
    );

    // Steady state: any container re-records any list no longer than its peak, in any order.
    let mut rng = Rng(7);
    for _ in 0..10_000 {
        let idx = rng.next(NODES);
        lists.clear(idx);
        let peak = (idx..idx + 4).map(|r| r % 7).max().unwrap_or(0);
        lists.extend(idx, ids(idx as u32 * 8, rng.next(peak + 1)));
        lists.compact_if_fragmented();
    }
    lists.assert_consistent();
    assert_eq!(
        lists.pool_len(),
        pool_len,
        "in-place rewrites must not grow the pool"
    );
    assert_eq!(lists.pool.capacity(), pool_cap);
    assert_eq!(lists.scratch.capacity(), scratch_cap);
}

#[test]
fn growth_relocates_then_compaction_restores_index_order() {
    let mut lists = HoistedLists::default();
    lists.ensure_len(3);
    lists.extend(0, ids(10, 2));
    lists.extend(1, ids(20, 2));
    // Node 0 outgrows its capacity while not last: moves to the tail, leaving a 2-slot hole.
    lists.extend(0, ids(12, 1));
    assert_eq!(lists.get(0), &[NodeId(10), NodeId(11), NodeId(12)]);
    assert_eq!(lists.dead, 2);
    // Node 0 is now last: grows in place.
    lists.extend(0, ids(13, 1));
    assert_eq!(lists.pool_len(), 2 + 2 + 4);
    lists.compact();
    assert_eq!(lists.dead, 0);
    assert_eq!(lists.pool_len(), 6);
    assert_eq!(
        lists.spans[0].start, 0,
        "compaction packs spans in node-index order"
    );
    assert_eq!(lists.spans[1].start, 4);
    assert_eq!(lists.get(1), &[NodeId(20), NodeId(21)]);
    lists.release(0);
    assert_eq!(lists.dead, 4);
    lists.assert_consistent();
}

fn layout_once(store: &mut NodeStore, changes: &ChangeSet, layout: &mut TaffyStoreLayout) {
    layout.sync_changes(store, changes);
    layout.compute_layout(store, NodeId::ROOT, VP);
}

/// Incremental layout must match a from-scratch layout of the same store: every reachable rect and
/// every hoisted list, with consistent pool accounting.
fn assert_matches_fresh_layout(store: &NodeStore, incremental: &TaffyStoreLayout) {
    let mut fresh = TaffyStoreLayout::new();
    fresh.compute_layout(store, NodeId::ROOT, VP);
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
