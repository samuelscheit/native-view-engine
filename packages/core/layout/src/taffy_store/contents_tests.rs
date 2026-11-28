//! `display: contents` layout, hoisting, and incremental-parity tests.

use super::hoisted_tests::{assert_matches_fresh_layout, layout_once};
use crate::TaffyStoreLayout;
use nve_tables::tables::{
    Display, FlexDirection, InputKind, Layout, Overflow, Position, TextInput, Visual,
};
use nve_engine::LayoutRect;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Transaction};

fn rect(layout: &TaffyStoreLayout, id: NodeId) -> (f32, f32, f32, f32) {
    let r: LayoutRect = layout.layout_of(id).expect("laid out");
    (r.x, r.y, r.width, r.height)
}

/// `row`'s border-box origin: every expected rect is relative to it, so a contents node's
/// "zero rect at the parent box origin" is distinguishable from `(0, 0)`.
const ORIGIN: (f32, f32) = (5.0, 3.0);

fn at(x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
    (ORIGIN.0 + x, ORIGIN.1 + y, w, h)
}

fn sized(tx: &mut Transaction<'_>, w: f32, h: f32) -> NodeId {
    let id = tx.create();
    tx.edit::<Layout>(id)
        .width(Length::points(w))
        .height(Length::points(h));
    id
}

/// `row` (at `ORIGIN`, padding 10) → [a, c1 (contents) → [b, c2 (contents) → [d]], e].
struct Fixture {
    store: NodeStore,
    changes: ChangeSet,
    layout: TaffyStoreLayout,
    row: NodeId,
    c1: NodeId,
    c2: NodeId,
    leaves: [NodeId; 4],
}

fn fixture() -> Fixture {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let row = tx.create();
    tx.edit::<Layout>(row)
        .flex_direction(FlexDirection::Row)
        .width(Length::points(300.0))
        .height(Length::points(50.0))
        .padding_left(Length::points(10.0))
        .padding_top(Length::points(10.0))
        .margin_left(Length::points(ORIGIN.0))
        .margin_top(Length::points(ORIGIN.1));
    let [a, b, d, e] = [50.0, 60.0, 70.0, 40.0].map(|w| sized(&mut tx, w, 20.0));
    let c1 = tx.create();
    let c2 = tx.create();
    for c in [c1, c2] {
        tx.edit::<Layout>(c)
            .display(Display::Contents)
            .width(Length::points(999.0));
    }
    tx.set_children(NodeId::ROOT, &[row]);
    tx.set_children(row, &[a, c1, e]);
    tx.set_children(c1, &[b, c2]);
    tx.set_children(c2, &[d]);
    tx.commit_into(&mut changes);
    let mut layout = TaffyStoreLayout::new();
    layout_once(&mut store, &changes, &mut layout);
    Fixture {
        store,
        changes,
        layout,
        row,
        c1,
        c2,
        leaves: [a, b, d, e],
    }
}

#[test]
fn nested_contents_children_flow_as_children_of_the_parent_box() {
    let f = fixture();
    let [a, b, d, e] = f.leaves;
    assert_eq!(rect(&f.layout, a), at(10.0, 10.0, 50.0, 20.0));
    assert_eq!(rect(&f.layout, b), at(60.0, 10.0, 60.0, 20.0));
    assert_eq!(rect(&f.layout, d), at(120.0, 10.0, 70.0, 20.0));
    assert_eq!(rect(&f.layout, e), at(190.0, 10.0, 40.0, 20.0));
    // No box: zero-size at the parent box's origin, its own `width` ignored.
    assert_eq!(rect(&f.layout, f.c1), at(0.0, 0.0, 0.0, 0.0));
    assert_eq!(rect(&f.layout, f.c2), at(0.0, 0.0, 0.0, 0.0));
    assert_eq!(f.layout.box_children.get(&f.store, f.row), &[a, b, d, e]);
    // Only the nearest box ancestor keeps a flattened list; contents nodes own none.
    assert!(!f.layout.box_children.is_flattened(f.c1));
    assert!(!f.layout.box_children.is_flattened(f.c2));
    assert_matches_fresh_layout(&f.store, &f.layout);
}

#[test]
fn contents_never_clips_and_a_contents_root_still_generates_the_viewport_box() {
    let mut f = fixture();
    let mut tx = f.store.tx();
    tx.edit::<Visual>(f.c1).overflow_x(Overflow::Hidden);
    tx.edit::<Visual>(f.row).overflow_x(Overflow::Hidden);
    tx.edit::<Layout>(NodeId::ROOT).display(Display::Contents);
    tx.commit_into(&mut f.changes);
    layout_once(&mut f.store, &f.changes, &mut f.layout);
    assert!(!nve_tables::tables::node_clips_overflow(&f.store, f.c1));
    assert!(nve_tables::tables::node_clips_overflow(&f.store, f.row));
    assert_eq!(rect(&f.layout, NodeId::ROOT), (0.0, 0.0, 400.0, 300.0));
    assert_eq!(rect(&f.layout, f.leaves[2]), at(120.0, 10.0, 70.0, 20.0));
    assert_matches_fresh_layout(&f.store, &f.layout);
}

#[test]
fn absolute_and_fixed_boxes_inside_contents_hoist_past_it() {
    let mut f = fixture();
    let (abs, fixed) = {
        let mut tx = f.store.tx();
        let abs = sized(&mut tx, 8.0, 8.0);
        let fixed = sized(&mut tx, 30.0, 10.0);
        tx.edit::<Layout>(abs)
            .position(Position::Absolute)
            .inset_left(Length::points(5.0))
            .inset_top(Length::points(6.0));
        tx.edit::<Layout>(fixed)
            .position(Position::Fixed)
            .inset_right(Length::points(0.0))
            .inset_bottom(Length::points(0.0));
        let d = f.leaves[2];
        tx.set_children(f.c2, &[d, abs, fixed]);
        tx.commit_into(&mut f.changes);
        (abs, fixed)
    };
    layout_once(&mut f.store, &f.changes, &mut f.layout);
    // `row` is the nearest box ancestor (all positioned): it claims `abs`; `fixed` bubbles to root.
    assert_eq!(f.layout.hoisted.get(f.row.index()), &[abs]);
    assert!(f.layout.hoisted.get(f.c1.index()).is_empty());
    assert!(f.layout.hoisted.get(f.c2.index()).is_empty());
    assert_eq!(f.layout.hoisted.get(NodeId::ROOT.index()), &[fixed]);
    assert_eq!(rect(&f.layout, abs), at(5.0, 6.0, 8.0, 8.0));
    assert_eq!(rect(&f.layout, fixed), (370.0, 290.0, 30.0, 10.0));
    // In-flow siblings are unaffected by the out-of-flow boxes.
    assert_eq!(rect(&f.layout, f.leaves[3]), at(190.0, 10.0, 40.0, 20.0));
    assert_matches_fresh_layout(&f.store, &f.layout);
}

#[test]
fn toggling_contents_and_editing_inside_it_match_fresh_layout() {
    let mut f = fixture();
    let [a, b, d, e] = f.leaves;
    let abs = {
        let mut tx = f.store.tx();
        let abs = sized(&mut tx, 4.0, 4.0);
        tx.edit::<Layout>(abs)
            .position(Position::Absolute)
            .inset_left(Length::points(1.0));
        tx.set_children(f.c1, &[b, f.c2, abs]);
        tx.commit_into(&mut f.changes);
        abs
    };
    layout_once(&mut f.store, &f.changes, &mut f.layout);
    assert_eq!(f.layout.hoisted.get(f.row.index()), &[abs]);
    assert_matches_fresh_layout(&f.store, &f.layout);

    // c1 becomes a real box (`width: 999`, row): it claims `abs`, and `row`'s stale entry clears.
    for (display, c1_claims) in [
        (Display::Flex, true),
        (Display::Contents, false),
        (Display::Flex, true),
    ] {
        let mut tx = f.store.tx();
        tx.edit::<Layout>(f.c1).display(display);
        tx.commit_into(&mut f.changes);
        layout_once(&mut f.store, &f.changes, &mut f.layout);
        assert_matches_fresh_layout(&f.store, &f.layout);
        assert_eq!(f.layout.hoisted.get(f.c1.index()) == [abs], c1_claims);
        assert_eq!(f.layout.hoisted.get(f.row.index()) == [abs], !c1_claims);
    }
    let mut tx = f.store.tx();
    tx.edit::<Layout>(f.c1).display(Display::Contents);
    tx.commit_into(&mut f.changes);
    layout_once(&mut f.store, &f.changes, &mut f.layout);

    // Resize, insert, and remove inside the nested contents node.
    let mut tx = f.store.tx();
    tx.edit::<Layout>(d).width(Length::points(20.0));
    tx.commit_into(&mut f.changes);
    layout_once(&mut f.store, &f.changes, &mut f.layout);
    assert_eq!(rect(&f.layout, e), at(140.0, 10.0, 40.0, 20.0));
    assert_matches_fresh_layout(&f.store, &f.layout);

    let extra = {
        let mut tx = f.store.tx();
        let extra = sized(&mut tx, 15.0, 20.0);
        tx.insert_child(f.c2, 0, extra);
        tx.commit_into(&mut f.changes);
        extra
    };
    layout_once(&mut f.store, &f.changes, &mut f.layout);
    assert_eq!(
        f.layout.box_children.get(&f.store, f.row),
        &[a, b, extra, d, abs, e]
    );
    assert_eq!(rect(&f.layout, extra), at(120.0, 10.0, 15.0, 20.0));
    assert_matches_fresh_layout(&f.store, &f.layout);

    let mut tx = f.store.tx();
    tx.remove(f.c2);
    tx.commit_into(&mut f.changes);
    layout_once(&mut f.store, &f.changes, &mut f.layout);
    assert_eq!(f.layout.box_children.get(&f.store, f.row), &[a, b, abs, e]);
    assert_eq!(rect(&f.layout, e), at(120.0, 10.0, 40.0, 20.0));
    assert_matches_fresh_layout(&f.store, &f.layout);

    // Contents → none hides the whole subtree; back to contents restores it.
    for display in [Display::None, Display::Contents] {
        let mut tx = f.store.tx();
        tx.edit::<Layout>(f.c1).display(display);
        tx.commit_into(&mut f.changes);
        layout_once(&mut f.store, &f.changes, &mut f.layout);
        assert_matches_fresh_layout(&f.store, &f.layout);
    }
    assert_eq!(rect(&f.layout, b), at(60.0, 10.0, 60.0, 20.0));

    // Once `b` has a `Visual` row (attaching one may relayout), a paint-only edit inside contents
    // is a clean frame.
    for opacity in [0.5, 0.25] {
        let pass = f.layout.layout_gen;
        let mut tx = f.store.tx();
        tx.edit::<Visual>(b).opacity(opacity);
        tx.commit_into(&mut f.changes);
        layout_once(&mut f.store, &f.changes, &mut f.layout);
        if opacity < 0.5 {
            assert_eq!(
                f.layout.layout_gen, pass,
                "paint-only frame must skip the pass"
            );
        }
    }
}

fn commit_frame(f: &mut Fixture, edit: impl FnOnce(&mut Transaction<'_>)) {
    let mut tx = f.store.tx();
    edit(&mut tx);
    tx.commit_into(&mut f.changes);
    layout_once(&mut f.store, &f.changes, &mut f.layout);
    assert_matches_fresh_layout(&f.store, &f.layout);
}

#[test]
fn absolute_or_fixed_position_on_a_contents_node_keeps_its_subtree_in_flow() {
    let mut f = fixture();
    let [a, b, d, e] = f.leaves;
    for position in [Position::Absolute, Position::Fixed, Position::Relative] {
        let c1 = f.c1;
        commit_frame(&mut f, |tx| {
            tx.edit::<Layout>(c1)
                .position(position)
                .inset_left(Length::points(50.0));
        });
        // No box, so nothing to position: children stay in the row's flow.
        assert_eq!(rect(&f.layout, b), at(60.0, 10.0, 60.0, 20.0));
        assert_eq!(rect(&f.layout, d), at(120.0, 10.0, 70.0, 20.0));
        assert_eq!(rect(&f.layout, f.c1), at(0.0, 0.0, 0.0, 0.0));
        assert_eq!(f.layout.box_children.get(&f.store, f.row), &[a, b, d, e]);
        assert!(f.layout.hoisted.get(f.row.index()).is_empty());
        assert!(f.layout.hoisted.get(NodeId::ROOT.index()).is_empty());
    }
}

#[test]
fn a_hidden_text_input_is_display_none_not_contents() {
    let mut f = fixture();
    let [a, b, d, e] = f.leaves;
    let c2 = f.c2;
    commit_frame(&mut f, |tx| {
        tx.edit::<TextInput>(c2).input_kind(InputKind::Hidden);
    });
    assert!(!nve_tables::tables::is_display_contents(&f.store, c2));
    // `c2` is now a (hidden) box child itself; its child `d` is never flattened into `row`.
    assert_eq!(f.layout.box_children.get(&f.store, f.row), &[a, b, c2, e]);
    assert_eq!(rect(&f.layout, e), at(120.0, 10.0, 40.0, 20.0));

    commit_frame(&mut f, |tx| {
        tx.edit::<TextInput>(c2).input_kind(InputKind::Text);
    });
    assert_eq!(f.layout.box_children.get(&f.store, f.row), &[a, b, d, e]);
    assert_eq!(rect(&f.layout, d), at(120.0, 10.0, 70.0, 20.0));
}

#[test]
fn display_none_toggles_walk_inheritance_only_where_contents_is_involved() {
    let mut f = fixture();
    let (row, a, c1) = (f.row, f.leaves[0], f.c1);
    let sync = |f: &mut Fixture, edit: &dyn Fn(&mut Transaction<'_>)| -> bool {
        let mut tx = f.store.tx();
        edit(&mut tx);
        tx.commit_into(&mut f.changes);
        f.layout.sync_changes(&f.store, &f.changes);
        let walked = f.layout.inherit_dirty;
        f.layout
            .compute_layout(&f.store, NodeId::ROOT, super::hoisted_tests::VP);
        assert_matches_fresh_layout(&f.store, &f.layout);
        walked
    };
    // `row`'s parent (root) has no contents child: no box-children list can change.
    assert!(!sync(&mut f, &|tx| {
        tx.edit::<Layout>(row).display(Display::None);
    }));
    assert!(!sync(&mut f, &|tx| {
        tx.edit::<Layout>(row).display(Display::Flex);
    }));
    // `a` sits in `row`'s flattened list, but stays a box either way: the list is unchanged.
    assert!(!sync(&mut f, &|tx| {
        tx.edit::<Layout>(a).display(Display::None);
    }));
    assert!(!sync(&mut f, &|tx| {
        tx.edit::<Layout>(a).display(Display::Flex);
    }));
    // A contents node changing `display` always reshapes.
    assert!(sync(&mut f, &|tx| {
        tx.edit::<Layout>(c1).display(Display::Flex);
    }));
    assert!(sync(&mut f, &|tx| {
        tx.edit::<Layout>(c1).display(Display::Contents);
    }));
}

#[test]
fn a_parent_that_stops_flattening_returns_its_list_capacity() {
    let mut f = fixture();
    let (c1, c2) = (f.c1, f.c2);
    assert!(f.layout.box_children.is_flattened(f.row));
    let row_cap = f.layout.box_children.list_capacity(f.row);
    assert!(row_cap >= 4);
    let dead_before = f.layout.box_children.dead_slots();
    commit_frame(&mut f, |tx| {
        tx.edit::<Layout>(c1).display(Display::Flex);
        tx.edit::<Layout>(c2).display(Display::Flex);
    });
    assert!(!f.layout.box_children.is_flattened(f.row));
    assert!(!f.layout.box_children.is_flattened(c1));
    // The capacity really went back to the pool (dead slots, until compaction reclaims them).
    assert_eq!(f.layout.box_children.list_capacity(f.row), 0);
    assert_eq!(f.layout.box_children.dead_slots(), dead_before + row_cap);
    assert_eq!(
        f.layout.box_children.get(&f.store, f.row),
        f.store.children(f.row)
    );
}

#[test]
fn contents_under_grid_text_and_hidden_ancestors_match_fresh_layout() {
    let mut f = fixture();
    let (row, c1, b) = (f.row, f.c1, f.leaves[1]);
    commit_frame(&mut f, |tx| {
        tx.edit::<Layout>(row).display(Display::Grid);
    });
    commit_frame(&mut f, |tx| {
        tx.edit::<nve_tables::tables::TextContent>(b)
            .text("flattened text");
    });
    // A `display: none` ancestor hides the contents subtree; restoring it restores the flow.
    commit_frame(&mut f, |tx| {
        tx.edit::<Layout>(row).display(Display::None);
    });
    assert!(!f.layout.box_children.is_flattened(c1));
    commit_frame(&mut f, |tx| {
        tx.edit::<Layout>(row).display(Display::Flex);
    });
    assert_eq!(rect(&f.layout, f.leaves[3]), at(190.0, 10.0, 40.0, 20.0));
}
