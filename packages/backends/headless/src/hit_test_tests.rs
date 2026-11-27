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

/// `stack` (200×100) holding `under` (200×50, press handler) and `overlay` (absolute at the
/// stack's origin, 100×100, translucent background, no handler), arranged by the caller.
struct OverlayFixture {
    store: NodeStore,
    changes: ChangeSet,
    backend: HeadlessBackend,
    stack: NodeId,
    under: NodeId,
    overlay: NodeId,
}

impl OverlayFixture {
    fn new(arrange: impl FnOnce(&mut Transaction<'_>, NodeId, NodeId, NodeId)) -> Self {
        let mut store = NodeStore::new();
        store.set_realization_order(core_realization_order());
        let mut changes = ChangeSet::default();
        let mut tx = store.tx();
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
        arrange(&mut tx, stack, under, overlay);
        tx.commit_into(&mut changes);
        let mut backend = HeadlessBackend::new();
        backend.set_viewport(1, VP);
        assert!(backend.commit(&store, &changes).is_ok());
        Self {
            store,
            changes,
            backend,
            stack,
            under,
            overlay,
        }
    }

    fn edit(&mut self, edit: impl FnOnce(&mut Transaction<'_>)) {
        let mut tx = self.store.tx();
        edit(&mut tx);
        tx.commit_into(&mut self.changes);
        assert!(self.backend.commit(&self.store, &self.changes).is_ok());
    }

    /// `(hit_test, hit_test_interaction)` at `(x, y)`, asserted equal (both targets subscribe).
    fn hit(&self, x: f32, y: f32) -> Option<nve_abi::NodeId> {
        let hit = self.backend.hit_test(&self.store, x, y);
        let interaction = self.backend.hit_test_interaction(&self.store, x, y);
        assert_eq!(
            hit, interaction,
            "hit_test vs hit_test_interaction at ({x}, {y})"
        );
        hit
    }
}

/// A `display: contents` wrapper with a press handler.
fn contents_wrapper(tx: &mut Transaction<'_>, child: NodeId) -> NodeId {
    let wrapper = tx.create();
    tx.edit::<Layout>(wrapper).display(Display::Contents);
    press(tx, wrapper);
    tx.set_children(wrapper, &[child]);
    wrapper
}

#[test]
fn a_handler_less_overlay_occludes_siblings_painted_below_it() {
    let mut f = OverlayFixture::new(|tx, stack, under, overlay| {
        tx.set_children(stack, &[under, overlay]);
    });
    let (stack, under, overlay) = (f.stack, f.under, f.overlay);
    // Covered by the overlay: nothing below it receives the hit.
    assert_eq!(f.hit(10.0, 10.0), None);
    // Beside the overlay: `under` is exposed.
    assert_eq!(f.hit(150.0, 10.0), Some(under.raw()));

    // The covered hit bubbles from the overlay to its ancestors, never sideways.
    f.edit(|tx| press(tx, stack));
    assert_eq!(f.hit(10.0, 10.0), Some(stack.raw()));
    assert_eq!(f.hit(150.0, 10.0), Some(under.raw()));

    // `z-index` reorders painting, and so occlusion: raised above the overlay, `under` is hit
    // through it; a negative level drops the overlay below `under` the same way.
    for (node, z) in [(under, 1), (under, 0), (overlay, -1)] {
        f.edit(|tx| {
            tx.edit::<Visual>(node).z_index(z);
        });
        let expected = if z == 0 { stack } else { under };
        assert_eq!(f.hit(10.0, 10.0), Some(expected.raw()), "{node:?} at z {z}");
    }
}

#[test]
fn an_invisible_overlay_occludes_nothing() {
    let mut f = OverlayFixture::new(|tx, stack, under, overlay| {
        tx.set_children(stack, &[under, overlay]);
    });
    let (under, overlay) = (f.under, f.overlay);
    f.edit(|tx| {
        tx.edit::<Visual>(overlay).visibility(Visibility::Hidden);
    });
    assert_eq!(f.hit(10.0, 10.0), Some(under.raw()));
}

#[test]
fn an_overlay_in_a_contents_wrapper_bubbles_to_the_wrapper_never_to_a_lower_sibling() {
    let mut wrapper = NodeId::ROOT;
    let mut f = OverlayFixture::new(|tx, stack, under, overlay| {
        wrapper = contents_wrapper(tx, overlay);
        tx.set_children(stack, &[under, wrapper]);
    });
    assert_eq!(f.hit(10.0, 10.0), Some(wrapper.raw()));
    assert_eq!(f.hit(150.0, 10.0), Some(f.under.raw()));

    // Without the wrapper's handler the hit still stops at the overlay.
    f.edit(|tx| {
        tx.edit::<Interaction>(wrapper).event_mask(0);
    });
    assert_eq!(f.hit(10.0, 10.0), None);
}

#[test]
fn a_contents_wrapper_z_index_never_reorders_its_children() {
    // Painted last at level 0, the wrapped overlay stays above `under` whatever the wrapper's
    // own `z_index` (it generates no box, so it does not stack).
    let mut wrapper = NodeId::ROOT;
    let mut f = OverlayFixture::new(|tx, stack, under, overlay| {
        wrapper = contents_wrapper(tx, overlay);
        tx.set_children(stack, &[under, wrapper]);
    });
    f.edit(|tx| {
        tx.edit::<Visual>(wrapper).z_index(-1);
    });
    assert_eq!(f.hit(10.0, 10.0), Some(wrapper.raw()));

    // Painted first, the wrapped overlay stays below `under` despite `z_index: 1`.
    let mut wrapper = NodeId::ROOT;
    let f = OverlayFixture::new(|tx, stack, under, overlay| {
        wrapper = contents_wrapper(tx, overlay);
        tx.edit::<Visual>(wrapper).z_index(1);
        tx.set_children(stack, &[wrapper, under]);
    });
    assert_eq!(f.hit(10.0, 10.0), Some(f.under.raw()));
    assert_eq!(
        f.hit(10.0, 70.0),
        Some(wrapper.raw()),
        "below `under`, the overlay is exposed"
    );
}

/// Absolute `w`×`h` box at `(x, y)` in its containing block.
fn placed(tx: &mut Transaction<'_>, x: f32, y: f32, w: f32, h: f32) -> NodeId {
    let id = sized(tx, w, h);
    tx.edit::<Layout>(id)
        .position(Position::Absolute)
        .inset_left(Length::points(x))
        .inset_top(Length::points(y));
    id
}

#[test]
fn long_mixed_z_sibling_lists_hit_the_topmost_box_through_nested_scratch() {
    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let stack = sized(&mut tx, 200.0, 200.0);
    // Visited first (highest `z`): a container away from the point whose own 100 mixed-`z`
    // children all miss, so its keys are appended after (and may reallocate) the stack's
    // before the stack's walk continues with its own keys.
    let container = placed(&mut tx, 100.0, 100.0, 10.0, 10.0);
    tx.edit::<Visual>(container).z_index(20);
    let inner: Vec<NodeId> = (0..100)
        .map(|j| {
            // Container-relative: x 100 + j % 10, y 100 in the stack.
            let id = placed(&mut tx, (j % 10) as f32, 0.0, 1.0, 1.0);
            tx.edit::<Visual>(id).z_index((j * 5) % 11);
            press(&mut tx, id);
            id
        })
        .collect();
    tx.set_children(container, &inner);
    // 100 overlapping pressable siblings with shuffled levels `(i * 7) % 13`: level 12 is
    // shared by i = 11, 24, …, 89, so the latest of them in DOM order (89, not the last
    // sibling) is topmost.
    let mut children = vec![container];
    for i in 0..100 {
        let id = placed(&mut tx, 0.0, 0.0, 20.0, 20.0);
        tx.edit::<Visual>(id).z_index((i * 7) % 13);
        press(&mut tx, id);
        children.push(id);
    }
    tx.set_children(NodeId::ROOT, &[stack]);
    tx.set_children(stack, &children);
    tx.commit_into(&mut changes);
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(1, VP);
    assert!(backend.commit(&store, &changes).is_ok());

    let topmost = Some(children[1 + 89].raw());
    for query in 0..2 {
        assert_eq!(backend.hit_test(&store, 5.0, 5.0), topmost, "query {query}");
        assert_eq!(backend.hit_test_interaction(&store, 5.0, 5.0), topmost);
        let (len, cap) = backend.scene().hit_scratch_stats();
        assert_eq!(len, 0, "every level truncated its keys");
        assert!(cap >= 201, "both levels' keys were held at once");
    }
    // The inner list orders correctly too: its own topmost child under its point.
    let inner_point = (100.5, 100.5);
    let inner_top = (0..100)
        .filter(|j| j % 10 == 0)
        .max_by_key(|&j| ((j * 5) % 11, j))
        .map(|j| inner[j].raw());
    assert_eq!(
        backend.hit_test(&store, inner_point.0, inner_point.1),
        inner_top
    );
}

/// Gives every node the same absolute rect without recursing, so a deep tree exercises only
/// the hit walk's stack.
struct UniformRects;

impl nve_layout::StoreLayoutEngine for UniformRects {
    fn name(&self) -> &'static str {
        "uniform-rects"
    }
    fn sync_changes(&mut self, _: &NodeStore, _: &ChangeSet) {}
    fn compute_layout(&mut self, _: &NodeStore, _: NodeId, _: Viewport) -> bool {
        true
    }
    fn layout_of(&self, _: NodeId) -> Option<nve_engine::LayoutRect> {
        Some(nve_engine::LayoutRect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        })
    }
    fn set_text_measurer(&mut self, _: std::sync::Arc<dyn nve_layout::TextMeasurer>) {}
    fn reset(&mut self) {}
}

#[test]
fn a_max_depth_chain_hit_tests_on_a_small_stack() {
    let deepest = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let mut store = NodeStore::new();
            store.set_realization_order(core_realization_order());
            let mut changes = ChangeSet::default();
            let mut tx = store.tx();
            let mut parent = NodeId::ROOT;
            for _ in 0..crate::MAX_TREE_DEPTH {
                let child = tx.create();
                tx.set_children(parent, &[child]);
                parent = child;
            }
            press(&mut tx, parent);
            tx.commit_into(&mut changes);
            let mut backend = HeadlessBackend::with_layout(UniformRects);
            backend.set_viewport(1, VP);
            assert!(backend.commit(&store, &changes).is_ok());
            (backend.hit_test(&store, 5.0, 5.0), parent.raw())
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");
    assert_eq!(
        deepest.0,
        Some(deepest.1),
        "the node at MAX_TREE_DEPTH is hit"
    );
}
