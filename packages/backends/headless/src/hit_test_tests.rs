//! `display: contents` through `HeadlessBackend` hit testing, sticky geometry, and scene
//! publication: no box, so no rect, transform, scroll port, clip, or box paint, while events on
//! its descendants' boxes still bubble to it.

use nve_abi::{ColorRgba, EventMask};
use nve_components::interaction::{InteractionState, StateStyleRule};
use nve_components::style_patch::StylePatch;
use nve_components::tables::{
    Display, FlexDirection, Interaction, Layout, Overflow, Position, StateRules, Transform,
    Visibility, Visual, core_realization_order,
};
use nve_engine::{SceneRelocView, StoreBackend, VerifyMode, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba, Transaction};

use crate::HeadlessBackend;

const VP: Viewport = Viewport {
    width: 400.0,
    height: 300.0,
    scale_factor: 1.0,
};

/// `row` (at (20, 10), 300×40) → `wrapper` (contents, press handler, box paint, clip, transform)
/// → [`plain` (50×20, no handler), `pressable` (50×20, own handler)].
struct Fixture {
    store: NodeStore,
    changes: ChangeSet,
    backend: HeadlessBackend,
    row: NodeId,
    wrapper: NodeId,
    plain: NodeId,
    pressable: NodeId,
}

fn sized(tx: &mut Transaction<'_>, w: f32, h: f32) -> NodeId {
    let id = tx.create();
    tx.edit::<Layout>(id)
        .width(Length::points(w))
        .height(Length::points(h));
    id
}

fn press(tx: &mut Transaction<'_>, id: NodeId) {
    tx.edit::<Interaction>(id)
        .event_mask(EventMask::PRESS as u32);
}

fn fixture() -> Fixture {
    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let row = sized(&mut tx, 300.0, 40.0);
    tx.edit::<Layout>(row)
        .flex_direction(FlexDirection::Row)
        .margin_left(Length::points(20.0))
        .margin_top(Length::points(10.0));
    let wrapper = tx.create();
    let plain = sized(&mut tx, 50.0, 20.0);
    let pressable = sized(&mut tx, 50.0, 20.0);
    tx.edit::<Layout>(wrapper).display(Display::Contents);
    press(&mut tx, wrapper);
    press(&mut tx, pressable);
    tx.edit::<Visual>(wrapper)
        .background(Rgba::rgb(255, 0, 0))
        .border_width(3.0)
        .opacity(0.5)
        .overflow_x(Overflow::Hidden);
    tx.edit::<Transform>(wrapper).translate_x(1000.0);
    tx.set_children(NodeId::ROOT, &[row]);
    tx.set_children(row, &[wrapper]);
    tx.set_children(wrapper, &[plain, pressable]);
    tx.commit_into(&mut changes);
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(1, VP);
    assert!(backend.commit(&store, &changes).is_ok());
    Fixture {
        store,
        changes,
        backend,
        row,
        wrapper,
        plain,
        pressable,
    }
}

#[test]
fn events_on_descendant_boxes_bubble_to_a_contents_node() {
    let mut f = fixture();
    // `plain` (no handler) at (20, 10): the hit bubbles to the box-less wrapper.
    assert_eq!(
        f.backend.hit_test(&f.store, 30.0, 15.0),
        Some(f.wrapper.raw())
    );
    // `pressable` handles its own hits.
    assert_eq!(
        f.backend.hit_test(&f.store, 80.0, 15.0),
        Some(f.pressable.raw())
    );
    // Inside `row` but on no box of the wrapper's subtree: nothing (the wrapper has no box).
    assert_eq!(f.backend.hit_test(&f.store, 200.0, 15.0), None);
    // Interaction-state hit testing shares the walk.
    assert_eq!(
        f.backend.hit_test_interaction(&f.store, 30.0, 15.0),
        Some(f.wrapper.raw())
    );

    // A stale scroll offset on the wrapper is ignored (no scroll port).
    f.backend.set_scroll_offset(f.wrapper.raw(), 500.0, 500.0);
    assert_eq!(
        f.backend.hit_test(&f.store, 30.0, 15.0),
        Some(f.wrapper.raw())
    );

    // A hidden descendant box covers nothing, so nothing bubbles.
    let plain = f.plain;
    let mut tx = f.store.tx();
    tx.edit::<Visual>(plain).visibility(Visibility::Hidden);
    tx.commit_into(&mut f.changes);
    assert!(f.backend.commit(&f.store, &f.changes).is_ok());
    assert_eq!(f.backend.hit_test(&f.store, 30.0, 15.0), None);
}

#[test]
fn contents_nodes_publish_no_box_paint_or_clip() {
    let mut f = fixture();
    let view = f.backend.publish_scene_view(&f.store, &f.changes, 1);
    // SAFETY: `view` borrows `backend`'s published scene table with `node_count` valid nodes.
    let nodes = unsafe { std::slice::from_raw_parts(view.nodes, view.node_count as usize) };
    let wrapper = nodes
        .iter()
        .find(|n| n.node_id == f.wrapper.raw())
        .expect("wrapper published");
    assert_eq!(
        (wrapper.bg_rgba, wrapper.border_width, wrapper.opacity_u8),
        (0, 0.0, 255)
    );
    assert_eq!(wrapper.flags & nve_abi::NVE_NODE_CLIP_BOUNDS, 0);
    assert_eq!(
        (wrapper.x, wrapper.y, wrapper.width, wrapper.height),
        (20.0, 10.0, 0.0, 0.0)
    );

    let reloc = f
        .backend
        .export_scene_reloc(&f.store, 1, 1, "contents", ColorRgba::BLACK)
        .expect("reloc export");
    let parsed = SceneRelocView::from_bytes(&reloc, VerifyMode::Full).expect("valid reloc");
    let node = parsed
        .find_node_by_id(f.wrapper.raw())
        .expect("wrapper exported")
        .1;
    assert!(
        !node.clips_children(),
        "a box-less node must not clip its subtree"
    );
    let row = parsed.find_node_by_id(f.row.raw()).expect("row exported").1;
    assert!(!row.clips_children());
}

#[test]
fn sticky_geometry_skips_contents_ancestors() {
    let mut f = fixture();
    let (row, wrapper, plain) = (f.row, f.wrapper, f.plain);
    let mut tx = f.store.tx();
    tx.edit::<Visual>(row).overflow_y(Overflow::Scroll);
    tx.edit::<Layout>(plain)
        .position(Position::Sticky)
        .inset_top(Length::points(0.0));
    tx.edit::<Layout>(wrapper)
        .position(Position::Sticky)
        .inset_top(Length::points(0.0));
    tx.commit_into(&mut f.changes);
    assert!(f.backend.commit(&f.store, &f.changes).is_ok());
    f.backend.set_scroll_offset(row.raw(), 0.0, 15.0);

    // A contents node never sticks.
    assert_eq!(
        f.backend.compute_sticky_delta(&f.store, wrapper),
        (0.0, 0.0)
    );
    // `plain` sticks within `row` (its box parent and scroll container), not the 0×0 wrapper:
    // scrolled 15 down, it shifts 15 to stay pinned, inside the 40-tall containing block.
    assert_eq!(f.backend.compute_sticky_delta(&f.store, plain), (0.0, 15.0));
}

#[test]
fn hover_state_rules_and_transforms_never_give_a_contents_node_a_box() {
    let mut f = fixture();
    let wrapper = f.wrapper;
    let mut tx = f.store.tx();
    let hover_text = ColorRgba::rgb(220, 38, 38);
    let hover = StylePatch::new()
        .bg(ColorRgba::rgb(30, 64, 175))
        .opacity(0.0)
        .text_color(hover_text);
    tx.edit::<Interaction>(wrapper).state_rules(Some(StateRules(
        vec![StateStyleRule::new(InteractionState::HOVER, hover)].into(),
    )));
    tx.commit_into(&mut f.changes);
    assert!(f.backend.commit(&f.store, &f.changes).is_ok());

    // Hovering `plain` bubbles to the wrapper, whose `:hover` rule must not paint (or, with
    // `opacity: 0`, cull) a box it does not have.
    let _ = f.backend.dispatch_pointer_move(&f.store, 30.0, 15.0);
    assert!(
        f.backend
            .interaction_state(wrapper.raw())
            .contains(InteractionState::HOVER)
    );
    let visual = f.backend.effective_visual(&f.store, wrapper);
    // The rule applied (inherited text paint is kept) while box paint was stripped.
    assert_eq!(visual.text_color, hover_text);
    assert_eq!(
        (visual.background, visual.opacity),
        (ColorRgba::TRANSPARENT, 1.0)
    );
    let (_, both) = f.backend.effective_layout_and_visual(&f.store, wrapper);
    assert_eq!(both, visual);
    // The wrapper's `translate_x(1000)` transforms nothing.
    assert_eq!(f.backend.effective_transform(&f.store, wrapper), None);
}

#[test]
fn hits_bubble_through_hidden_contents_ancestors() {
    let mut f = fixture();
    let (wrapper, plain) = (f.wrapper, f.plain);
    let mut tx = f.store.tx();
    tx.edit::<Visual>(wrapper).visibility(Visibility::Hidden);
    tx.edit::<Visual>(plain).visibility(Visibility::Visible);
    tx.commit_into(&mut f.changes);
    assert!(f.backend.commit(&f.store, &f.changes).is_ok());
    // `plain` is visible again inside the hidden wrapper: its hit bubbles to the wrapper.
    assert_eq!(
        f.backend.hit_test(&f.store, 30.0, 15.0),
        Some(wrapper.raw())
    );
}

#[test]
fn hits_on_overflowing_children_bubble_to_unclipped_boxes_only() {
    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let parent = sized(&mut tx, 100.0, 20.0);
    let child = sized(&mut tx, 50.0, 20.0);
    tx.edit::<Layout>(child)
        .margin_top(Length::points(40.0))
        .flex_shrink(0.0);
    press(&mut tx, parent);
    tx.set_children(NodeId::ROOT, &[parent]);
    tx.set_children(parent, &[child]);
    tx.commit_into(&mut changes);
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(1, VP);
    assert!(backend.commit(&store, &changes).is_ok());
    assert_eq!(
        backend.layout_of(child.raw()).map(|r| (r.y, r.height)),
        Some((40.0, 20.0))
    );
    // `child` overflows `parent` (y 40..60): its hit bubbles to the parent's handler.
    assert_eq!(backend.hit_test(&store, 10.0, 50.0), Some(parent.raw()));
    assert_eq!(backend.hit_test(&store, 10.0, 10.0), Some(parent.raw()));
    assert_eq!(backend.hit_test(&store, 80.0, 50.0), None);

    let mut tx = store.tx();
    tx.edit::<Visual>(parent).overflow_y(Overflow::Hidden);
    tx.commit_into(&mut changes);
    assert!(backend.commit(&store, &changes).is_ok());
    assert_eq!(backend.hit_test(&store, 10.0, 50.0), None, "clipped away");
}

#[test]
fn a_handler_less_overlay_occludes_siblings_painted_below_it() {
    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    // `stack` (200×100) → [`under` (200×50, press handler), `overlay` (absolute, 100×100, no
    // handler, painted last)].
    let stack = sized(&mut tx, 200.0, 100.0);
    let under = sized(&mut tx, 200.0, 50.0);
    let overlay = sized(&mut tx, 100.0, 100.0);
    tx.edit::<Layout>(overlay)
        .position(Position::Absolute)
        .inset_left(Length::points(0.0))
        .inset_top(Length::points(0.0));
    tx.edit::<Visual>(overlay)
        .background(Rgba::new(0, 0, 0, 128));
    press(&mut tx, under);
    tx.set_children(NodeId::ROOT, &[stack]);
    tx.set_children(stack, &[under, overlay]);
    tx.commit_into(&mut changes);
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(1, VP);
    assert!(backend.commit(&store, &changes).is_ok());

    let hits = |b: &HeadlessBackend, store: &NodeStore, x, y| {
        (b.hit_test(store, x, y), b.hit_test_interaction(store, x, y))
    };
    // Covered by the overlay: nothing below it receives the hit.
    assert_eq!(hits(&backend, &store, 10.0, 10.0), (None, None));
    // Beside the overlay: `under` is exposed.
    let exposed = Some(under.raw());
    assert_eq!(hits(&backend, &store, 150.0, 10.0), (exposed, exposed));

    // The covered hit bubbles from the overlay to its ancestors, never sideways.
    let mut tx = store.tx();
    press(&mut tx, stack);
    tx.commit_into(&mut changes);
    assert!(backend.commit(&store, &changes).is_ok());
    let bubbled = Some(stack.raw());
    assert_eq!(hits(&backend, &store, 10.0, 10.0), (bubbled, bubbled));
    assert_eq!(hits(&backend, &store, 150.0, 10.0), (exposed, exposed));

    // `z-index` reorders painting, and so occlusion: raised above the overlay, `under` is hit
    // through it; a negative level drops the overlay below `under` the same way.
    for (node, z) in [(under, 1), (under, 0), (overlay, -1)] {
        let mut tx = store.tx();
        tx.edit::<Visual>(node).z_index(z);
        tx.commit_into(&mut changes);
        assert!(backend.commit(&store, &changes).is_ok());
        let expected = if z == 0 { bubbled } else { exposed };
        assert_eq!(
            hits(&backend, &store, 10.0, 10.0),
            (expected, expected),
            "{node:?} at z {z}"
        );
    }
}
