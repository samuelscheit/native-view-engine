//! Focused unit parity and edge-case tests for [`YogaLayoutEngine`] against [`TaffyStoreLayout`]:
//! - `row_gap != column_gap`, `flex_wrap`, `align_content`, `align_self`
//! - `BoxSizing::ContentBox`, `Visual::overflow`, `Visual::border_width`
//! - Inherited `Typography` mutation & invalid `<= 0`/`NaN` `font_size` fallback parity
//! - `<scrollview>` vertical vs horizontal `Scroll::axes()` + `Overflow::Scroll`
//! - Clean-frame self-gating (`compute_layout == false` on empty / paint-only / `Typography::color`)
//! - Text node temporarily gaining children and returning to 0 children re-acquires measure hook
//! - Root switch restores previous root's own dimensions and re-propagates typography
//! - Caught host `TextMeasurer` panic leaves `layout_dirty = true` for next-frame retry
//! - Non-finite / NaN / negative inputs produce finite non-NaN rects

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nve_components::tables::{
    AlignContent, AlignItems, AlignSelf, BoxSizing, FlexDirection, FlexWrap, Layout, Overflow,
    Scroll, ScrollAxes, TextContent, Typography, Visual,
};
use nve_engine::{LayoutRect, Viewport};
use nve_layout::{StoreLayoutEngine, TaffyStoreLayout, TextMeasurer};
use nve_layout_yoga::YogaLayoutEngine;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

const TOL: f32 = 1.0;

fn require_yoga_or_skip() -> bool {
    if YogaLayoutEngine::is_available() {
        return true;
    }
    assert!(
        std::env::var_os("NVE_REQUIRE_YOGA").is_none(),
        "NVE_REQUIRE_YOGA is set but real React Native Yoga was not compiled in (set RN_REACT_COMMON_DIR)"
    );
    false
}

fn assert_parity(label: &str, id: NodeId, taffy: LayoutRect, yoga: LayoutRect) {
    assert!(
        (taffy.x - yoga.x).abs() <= TOL
            && (taffy.y - yoga.y).abs() <= TOL
            && (taffy.width - yoga.width).abs() <= TOL
            && (taffy.height - yoga.height).abs() <= TOL,
        "[{label}] node #{} diverged:\n  taffy: {taffy:?}\n  yoga : {yoga:?}",
        id.raw()
    );
}

fn run_pair(label: &str, vp: Viewport, store: &NodeStore, changes: &ChangeSet, ids: &[NodeId]) {
    let mut taffy = TaffyStoreLayout::new();
    let mut yoga = YogaLayoutEngine::new().expect("real RN Yoga");
    taffy.sync_changes(store, changes);
    yoga.sync_changes(store, changes);
    assert!(taffy.compute_layout(store, NodeId::ROOT, vp));
    assert!(yoga.compute_layout(store, NodeId::ROOT, vp));
    for &id in ids {
        assert_parity(
            label,
            id,
            taffy.layout_of(id).expect("taffy rect"),
            yoga.layout_of(id).expect("yoga rect"),
        );
    }
}

#[test]
fn parity_gaps_wrap_align_content_align_self_content_box_border_overflow() {
    if !require_yoga_or_skip() {
        return;
    }
    let vp = Viewport {
        width: 320.0,
        height: 400.0,
        scale_factor: 2.0,
    };
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let ids = {
        let mut tx = store.tx();
        let wrap = tx.create();
        tx.edit::<Layout>(wrap)
            .width(Length::points(220.0))
            .height(Length::points(200.0))
            .flex_direction(FlexDirection::Row)
            .flex_wrap(FlexWrap::Wrap)
            .align_content(AlignContent::SpaceBetween)
            .align_items(AlignItems::FlexStart)
            .row_gap(Length::points(14.0))
            .column_gap(Length::points(6.0))
            .padding_left(Length::points(8.0))
            .padding_right(Length::points(8.0));
        tx.edit::<Visual>(wrap)
            .border_width(2.0)
            .overflow_x(Overflow::Hidden);

        let a = tx.create();
        tx.edit::<Layout>(a)
            .width(Length::points(90.0))
            .height(Length::points(40.0))
            .align_self(AlignSelf::Center);

        let b = tx.create();
        tx.edit::<Layout>(b)
            .width(Length::points(90.0))
            .height(Length::points(50.0))
            .box_sizing(BoxSizing::ContentBox)
            .padding_left(Length::points(5.0))
            .padding_right(Length::points(5.0))
            .padding_top(Length::points(4.0))
            .padding_bottom(Length::points(4.0));
        tx.edit::<Visual>(b).border_width(3.0);

        let c = tx.create();
        tx.edit::<Layout>(c)
            .width(Length::points(120.0))
            .height(Length::points(36.0))
            .align_self(AlignSelf::FlexEnd);

        tx.set_children(wrap, &[a, b, c]);
        tx.set_children(NodeId::ROOT, &[wrap]);
        tx.commit_into(&mut changes);
        [NodeId::ROOT, wrap, a, b, c]
    };

    run_pair("wrap_gaps_box_sizing_border_overflow", vp, &store, &changes, &ids);
}

#[test]
fn parity_scroll_axes_and_inherited_typography_mutation() {
    if !require_yoga_or_skip() {
        return;
    }
    let vp = Viewport {
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
    };
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (vscroll, vchild, hscroll, hchild, typo_parent, text_node) = {
        let mut tx = store.tx();
        let vscroll = tx.create();
        tx.edit::<Layout>(vscroll)
            .width(Length::points(300.0))
            .height(Length::points(100.0))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Scroll>(vscroll).axes(ScrollAxes::Vertical);

        let vchild = tx.create();
        tx.edit::<Layout>(vchild).flex_direction(FlexDirection::Column);
        let tall_a = tx.create();
        tx.edit::<Layout>(tall_a).height(Length::points(90.0));
        let tall_b = tx.create();
        tx.edit::<Layout>(tall_b).height(Length::points(90.0));
        tx.set_children(vchild, &[tall_a, tall_b]);
        tx.set_children(vscroll, &[vchild]);

        let hscroll = tx.create();
        tx.edit::<Layout>(hscroll)
            .width(Length::points(200.0))
            .height(Length::points(60.0))
            .flex_direction(FlexDirection::Row);
        tx.edit::<Scroll>(hscroll).axes(ScrollAxes::Horizontal);
        let hchild = tx.create();
        tx.edit::<Layout>(hchild).flex_direction(FlexDirection::Row);
        let wide_a = tx.create();
        tx.edit::<Layout>(wide_a)
            .width(Length::points(180.0))
            .height(Length::points(40.0));
        let wide_b = tx.create();
        tx.edit::<Layout>(wide_b)
            .width(Length::points(180.0))
            .height(Length::points(40.0));
        tx.set_children(hchild, &[wide_a, wide_b]);
        tx.set_children(hscroll, &[hchild]);

        let typo_parent = tx.create();
        tx.edit::<Typography>(typo_parent).font_size(14.0).font_weight(400);
        let text_node = tx.create();
        tx.edit::<TextContent>(text_node).text("Inherited text");
        tx.set_children(typo_parent, &[text_node]);

        tx.set_children(NodeId::ROOT, &[vscroll, hscroll, typo_parent]);
        tx.commit_into(&mut changes);
        (vscroll, vchild, hscroll, hchild, typo_parent, text_node)
    };

    let mut taffy = TaffyStoreLayout::new();
    let mut yoga = YogaLayoutEngine::new().unwrap();
    taffy.sync_changes(&store, &changes);
    yoga.sync_changes(&store, &changes);
    assert!(taffy.compute_layout(&store, NodeId::ROOT, vp));
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    for id in [vscroll, vchild, hscroll, hchild, typo_parent, text_node] {
        assert_parity("scroll_typo_f1", id, taffy.layout_of(id).unwrap(), yoga.layout_of(id).unwrap());
    }
    assert!(yoga.layout_of(vchild).unwrap().height >= 179.0);
    assert!(yoga.layout_of(hchild).unwrap().width >= 359.0);
    let h14 = yoga.layout_of(text_node).unwrap().height;

    // Mutate inherited font_size + verify non-finite fallback AND CSS `font-size: 0` parity.
    {
        let mut tx = store.tx();
        tx.edit::<Typography>(typo_parent).font_size(24.0);
        tx.commit_into(&mut changes);
    }
    taffy.sync_changes(&store, &changes);
    yoga.sync_changes(&store, &changes);
    assert!(taffy.compute_layout(&store, NodeId::ROOT, vp));
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    let h24 = yoga.layout_of(text_node).unwrap().height;
    assert!(
        h24 > h14 + 5.0,
        "inherited font_size 14 -> 24 must increase text height: h14={h14}, h24={h24}"
    );
    assert_parity("typo_24", text_node, taffy.layout_of(text_node).unwrap(), yoga.layout_of(text_node).unwrap());

    {
        let mut tx = store.tx();
        tx.edit::<Typography>(text_node).font_size(f32::NAN);
        tx.commit_into(&mut changes);
    }
    taffy.sync_changes(&store, &changes);
    yoga.sync_changes(&store, &changes);
    let _ = taffy.compute_layout(&store, NodeId::ROOT, vp);
    let _ = yoga.compute_layout(&store, NodeId::ROOT, vp);
    let h_nan = yoga.layout_of(text_node).unwrap().height;
    assert!(
        (h_nan - h24).abs() <= TOL,
        "NaN font_size must inherit parent 24px size: h24={h24}, h_nan={h_nan}"
    );
    assert_parity("typo_nan_fallback", text_node, taffy.layout_of(text_node).unwrap(), yoga.layout_of(text_node).unwrap());

    // CSS allows `font-size: 0` (must collapse to 0x0 instead of falling back to parent 24px).
    {
        let mut tx = store.tx();
        tx.edit::<Typography>(text_node).font_size(0.0);
        tx.commit_into(&mut changes);
    }
    taffy.sync_changes(&store, &changes);
    yoga.sync_changes(&store, &changes);
    assert!(taffy.compute_layout(&store, NodeId::ROOT, vp));
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    let r0_taffy = taffy.layout_of(text_node).unwrap();
    let r0_yoga = yoga.layout_of(text_node).unwrap();
    assert!(
        r0_taffy.height <= TOL && r0_yoga.height <= TOL,
        "font_size 0 must measure zero height instead of inheriting parent: taffy={r0_taffy:?} yoga={r0_yoga:?}"
    );
    assert_parity("typo_zero", text_node, r0_taffy, r0_yoga);
}

#[test]
fn yoga_clean_frame_self_gating_and_paint_only_skip() {
    if !require_yoga_or_skip() {
        return;
    }
    let vp = Viewport {
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
    };
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (box_id, label) = {
        let mut tx = store.tx();
        let box_id = tx.create();
        tx.edit::<Layout>(box_id)
            .width(Length::points(120.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(box_id).opacity(1.0);
        let label = tx.create();
        tx.edit::<TextContent>(label).text("Clean frame");
        tx.edit::<Typography>(label).color(Rgba::BLACK);
        tx.set_children(NodeId::ROOT, &[box_id, label]);
        tx.commit_into(&mut changes);
        (box_id, label)
    };

    let mut yoga = YogaLayoutEngine::new().unwrap();
    yoga.sync_changes(&store, &changes);
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    // Clean empty frame skips.
    yoga.sync_changes(&store, &ChangeSet::default());
    assert!(!yoga.compute_layout(&store, NodeId::ROOT, vp));

    // Paint-only Visual::opacity + Typography::color must NOT trigger a Yoga layout pass!
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(box_id).opacity(0.4);
        tx.edit::<Typography>(label).color(Rgba::WHITE);
        tx.commit_into(&mut changes);
    }
    yoga.sync_changes(&store, &changes);
    assert!(
        !yoga.compute_layout(&store, NodeId::ROOT, vp),
        "paint-only Visual::opacity and Typography::color must skip Yoga relayout"
    );

    // Changing font_size MUST reopen the pass.
    {
        let mut tx = store.tx();
        tx.edit::<Typography>(label).font_size(28.0);
        tx.commit_into(&mut changes);
    }
    yoga.sync_changes(&store, &changes);
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    assert!(!yoga.compute_layout(&store, NodeId::ROOT, vp));
}

#[test]
fn yoga_text_reacquires_measure_hook_root_switch_and_panic_retry() {
    if !require_yoga_or_skip() {
        return;
    }
    let vp = Viewport {
        width: 300.0,
        height: 240.0,
        scale_factor: 1.0,
    };
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (panel, text_id, temp_child) = {
        let mut tx = store.tx();
        let panel = tx.create();
        tx.edit::<Layout>(panel)
            .width(Length::points(180.0))
            .height(Length::points(120.0));
        tx.edit::<Typography>(NodeId::ROOT).font_size(20.0);
        let text_id = tx.create();
        tx.edit::<TextContent>(text_id).text("Hello Yoga");
        let temp_child = tx.create();
        tx.edit::<Layout>(temp_child)
            .width(Length::points(20.0))
            .height(Length::points(10.0));
        tx.set_children(panel, &[text_id]);
        tx.set_children(NodeId::ROOT, &[panel]);
        tx.commit_into(&mut changes);
        (panel, text_id, temp_child)
    };

    let mut yoga = YogaLayoutEngine::new().unwrap();
    yoga.sync_changes(&store, &changes);
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    let initial_text_rect = yoga.layout_of(text_id).unwrap();
    assert!(initial_text_rect.height > 20.0);

    // Temporarily attach a child to `text_id`, then remove it: measure hook MUST re-attach!
    {
        let mut tx = store.tx();
        tx.set_children(text_id, &[temp_child]);
        tx.commit_into(&mut changes);
    }
    yoga.sync_changes(&store, &changes);
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));

    {
        let mut tx = store.tx();
        tx.set_children(text_id, &[]);
        tx.commit_into(&mut changes);
    }
    yoga.sync_changes(&store, &changes);
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    assert_parity("reacquired_measure", text_id, initial_text_rect, yoga.layout_of(text_id).unwrap());

    // Root switch `ROOT -> panel -> ROOT` must recompute default 16px while `root == panel`,
    // then restore `panel`'s 180x120 size & inherited 20px typography!
    assert!(yoga.compute_layout(&store, panel, vp));
    assert_eq!(yoga.layout_of(panel).unwrap().width, 300.0);
    let panel_root_text_h = yoga.layout_of(text_id).unwrap().height;
    assert!(
        panel_root_text_h < initial_text_rect.height - 3.0,
        "switching root to `panel` must drop ROOT's inherited 20px font_size back to default 16px: initial={}, panel_root={panel_root_text_h}",
        initial_text_rect.height
    );
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    let restored_panel = yoga.layout_of(panel).unwrap();
    assert_eq!((restored_panel.width, restored_panel.height), (180.0, 120.0));
    assert_parity("root_switch_text", text_id, initial_text_rect, yoga.layout_of(text_id).unwrap());

    // Caught panic in `TextMeasurer` must keep `layout_dirty = true` so next frame retries cleanly!
    struct PanicOnce(AtomicBool);
    impl TextMeasurer for PanicOnce {
        fn measure(
            &self,
            text: &str,
            style: &nve_layout::TextStyle<&str>,
            max_width: Option<f32>,
        ) -> (f32, f32) {
            assert!(!self.0.load(Ordering::Acquire), "simulated measurer panic");
            nve_layout::HeuristicTextMeasurer.measure(text, style, max_width)
        }
    }
    let panic_measurer = Arc::new(PanicOnce(AtomicBool::new(true)));
    yoga.set_text_measurer(panic_measurer.clone());
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    panic_measurer.0.store(false, Ordering::Release);
    assert!(
        yoga.compute_layout(&store, NodeId::ROOT, vp),
        "frame after caught measurer panic must stay dirty and retry"
    );
    assert_parity("post_panic_retry", text_id, initial_text_rect, yoga.layout_of(text_id).unwrap());

    // Non-finite viewport dimensions must stay finite and non-NaN.
    let _ = yoga.compute_layout(
        &store,
        NodeId::ROOT,
        Viewport {
            width: f32::NAN,
            height: f32::INFINITY,
            scale_factor: -1.0,
        },
    );
    let r = yoga.layout_of(NodeId::ROOT).unwrap();
    assert!(r.x.is_finite() && r.y.is_finite() && r.width.is_finite() && r.height.is_finite());
}

#[test]
fn scroll_shrink_preservation_negative_and_incremental_flip_cases() {
    if !require_yoga_or_skip() {
        return;
    }
    let vp = Viewport {
        width: 300.0,
        height: 300.0,
        scale_factor: 1.0,
    };

    // 1. Negative cases:
    //    - `ScrollAxes::Vertical` on `FlexDirection::Row` -> child DOES shrink (`width < 180`)
    //    - `ScrollAxes::None` on `Column` -> child DOES shrink (`height < 180`)
    //    - `TextContent` inside horizontal `Row` scroll -> wraps/shrinks cleanly (does not expand)
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (vrow_child, none_child, htext, dyn_parent, dyn_child) = {
        let mut tx = store.tx();

        let vrow = tx.create();
        tx.edit::<Layout>(vrow)
            .width(Length::points(120.0))
            .height(Length::points(60.0))
            .flex_direction(FlexDirection::Row);
        tx.edit::<Scroll>(vrow).axes(ScrollAxes::Vertical);
        let vrow_child = tx.create();
        tx.edit::<Layout>(vrow_child)
            .flex_basis(Length::points(200.0))
            .height(Length::points(40.0));
        tx.set_children(vrow, &[vrow_child]);

        let none_scroll = tx.create();
        tx.edit::<Layout>(none_scroll)
            .width(Length::points(120.0))
            .height(Length::points(80.0))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Scroll>(none_scroll).axes(ScrollAxes::None);
        let none_child = tx.create();
        tx.edit::<Layout>(none_child).flex_basis(Length::points(200.0));
        tx.set_children(none_scroll, &[none_child]);

        let hscroll_text = tx.create();
        tx.edit::<Layout>(hscroll_text)
            .width(Length::points(120.0))
            .height(Length::points(60.0))
            .flex_direction(FlexDirection::Row);
        tx.edit::<Scroll>(hscroll_text).axes(ScrollAxes::Horizontal);
        let htext = tx.create();
        tx.edit::<TextContent>(htext)
            .text("Long wrapping horizontal text inside scroll row");
        tx.set_children(hscroll_text, &[htext]);

        // Dynamic parent starts non-scrolling (`ScrollAxes::None`) with auto-height child wrapping
        // two 90px blocks (180px total inside 80px viewport).
        let dyn_parent = tx.create();
        tx.edit::<Layout>(dyn_parent)
            .width(Length::points(120.0))
            .height(Length::points(80.0))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Scroll>(dyn_parent).axes(ScrollAxes::None);
        let dyn_child = tx.create();
        tx.edit::<Layout>(dyn_child).flex_direction(FlexDirection::Column);
        let b1 = tx.create();
        tx.edit::<Layout>(b1).height(Length::points(90.0));
        let b2 = tx.create();
        tx.edit::<Layout>(b2).height(Length::points(90.0));
        tx.set_children(dyn_child, &[b1, b2]);
        tx.set_children(dyn_parent, &[dyn_child]);

        tx.set_children(NodeId::ROOT, &[vrow, none_scroll, hscroll_text, dyn_parent]);
        tx.commit_into(&mut changes);
        (vrow_child, none_child, htext, dyn_parent, dyn_child)
    };

    let mut taffy = TaffyStoreLayout::new();
    let mut yoga = YogaLayoutEngine::new().unwrap();
    taffy.sync_changes(&store, &changes);
    yoga.sync_changes(&store, &changes);
    assert!(taffy.compute_layout(&store, NodeId::ROOT, vp));
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));

    assert!(
        yoga.layout_of(vrow_child).unwrap().width <= 121.0,
        "Vertical scroll on Row must allow child to shrink: {:?}",
        yoga.layout_of(vrow_child)
    );
    assert!(
        yoga.layout_of(none_child).unwrap().height <= 81.0,
        "ScrollAxes::None must allow child to shrink: {:?}",
        yoga.layout_of(none_child)
    );
    assert!(
        yoga.layout_of(htext).unwrap().width <= 121.0,
        "TextContent inside horizontal scroll row must shrink/wrap within container: {:?}",
        yoga.layout_of(htext)
    );
    assert_parity("vrow_child", vrow_child, taffy.layout_of(vrow_child).unwrap(), yoga.layout_of(vrow_child).unwrap());
    assert_parity("none_child", none_child, taffy.layout_of(none_child).unwrap(), yoga.layout_of(none_child).unwrap());
    assert_parity("htext", htext, taffy.layout_of(htext).unwrap(), yoga.layout_of(htext).unwrap());

    // 2. Incremental flip: `ScrollAxes::None` -> `ScrollAxes::Vertical` must expand `dyn_child` to >= 179px.
    {
        let mut tx = store.tx();
        tx.edit::<Scroll>(dyn_parent).axes(ScrollAxes::Vertical);
        tx.commit_into(&mut changes);
    }
    taffy.sync_changes(&store, &changes);
    yoga.sync_changes(&store, &changes);
    assert!(taffy.compute_layout(&store, NodeId::ROOT, vp));
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    assert!(
        yoga.layout_of(dyn_child).unwrap().height >= 179.0,
        "enabling ScrollAxes::Vertical incrementally must preserve child intrinsic height: {:?}",
        yoga.layout_of(dyn_child)
    );
    assert_parity("dyn_scroll_on", dyn_child, taffy.layout_of(dyn_child).unwrap(), yoga.layout_of(dyn_child).unwrap());

    // 3. Detach `Scroll` and toggle `Visual::overflow_y(Overflow::Scroll)` incrementally.
    {
        let mut tx = store.tx();
        tx.detach::<Scroll>(dyn_parent);
        tx.edit::<Visual>(dyn_parent).overflow_y(Overflow::Hidden);
        tx.commit_into(&mut changes);
    }
    taffy.sync_changes(&store, &changes);
    yoga.sync_changes(&store, &changes);
    let _ = taffy.compute_layout(&store, NodeId::ROOT, vp);
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    assert!(
        yoga.layout_of(dyn_child).unwrap().height <= 81.0,
        "detaching Scroll + setting overflow_y=Hidden must shrink child back to 80px: {:?}",
        yoga.layout_of(dyn_child)
    );

    {
        let mut tx = store.tx();
        tx.edit::<Visual>(dyn_parent).overflow_y(Overflow::Scroll);
        tx.commit_into(&mut changes);
    }
    taffy.sync_changes(&store, &changes);
    yoga.sync_changes(&store, &changes);
    let _ = taffy.compute_layout(&store, NodeId::ROOT, vp);
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));
    assert!(
        yoga.layout_of(dyn_child).unwrap().height >= 179.0,
        "setting Visual::overflow_y=Scroll incrementally must preserve child height: {:?}",
        yoga.layout_of(dyn_child)
    );
}
