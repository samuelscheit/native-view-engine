use super::*;
use nve_components::layout_ext::{GridSingleTrack, GridTrack, GridTrackUnit};
use nve_components::tables::{
    AlignItems as NveAlignItems, Direction as NveDirection, Display as NveDisplay,
    FlexDirection as NveFlexDirection, GridItem, GridLayout, GridLine as NveGridLine, GridTracks,
    JustifyContent as NveJustifyContent, Layout as NveLayout, Position as NvePosition,
    TextContent, Typography,
};
use nve_engine::Viewport;
use nve_store::{ChangeSet, Length, NodeId, NodeStore};

#[test]
fn taffy_store_layout_flex_grid_rtl_oof_hidden_text_and_incremental() {
    let mut store = NodeStore::new();
    let mut layout = TaffyStoreLayout::new();
    let mut changes = ChangeSet::default();

    let mut tx = store.tx();
    let row = tx.create();
    let text = tx.create();
    let abs = tx.create();
    let hidden = tx.create();
    let grid = tx.create();
    let c1 = tx.create();
    let c2 = tx.create();
    let rtl_box = tx.create();
    let rtl_child = tx.create();

    tx.set_children(NodeId::ROOT, &[row, grid, rtl_box]);
    tx.set_children(row, &[text, abs, hidden]);
    tx.set_children(grid, &[c1, c2]);
    tx.set_children(rtl_box, &[rtl_child]);

    tx.edit::<NveLayout>(NodeId::ROOT)
        .padding_top(Length::points(10.0))
        .padding_right(Length::points(10.0))
        .padding_bottom(Length::points(10.0))
        .padding_left(Length::points(10.0));
    tx.edit::<NveLayout>(row)
        .flex_direction(NveFlexDirection::Row)
        .height(Length::points(100.0))
        .align_items(NveAlignItems::Center);
    tx.edit::<TextContent>(text).text("Hello NVE2");
    tx.edit::<Typography>(text).font_size(20.0);
    tx.edit::<NveLayout>(abs)
        .position(NvePosition::Absolute)
        .inset_right(Length::points(8.0))
        .inset_bottom(Length::points(6.0))
        .width(Length::points(40.0))
        .height(Length::points(24.0));
    tx.edit::<NveLayout>(hidden)
        .display(NveDisplay::None)
        .width(Length::points(500.0))
        .height(Length::points(500.0));

    tx.edit::<NveLayout>(grid)
        .display(NveDisplay::Grid)
        .height(Length::points(80.0))
        .column_gap(Length::points(10.0));
    tx.edit::<GridLayout>(grid)
        .template_columns(Some(GridTracks::new(vec![
            GridTrack::Px(120.0),
            GridTrack::Fr(1.0),
            GridTrack::Repeat(1, GridSingleTrack::MinMax(GridTrackUnit::Px(60.0), GridTrackUnit::Fr(1.0))),
        ])));
    tx.edit::<GridItem>(c1)
        .column_start(NveGridLine::Line(1))
        .column_end(NveGridLine::Span(1));
    tx.edit::<GridItem>(c2)
        .column_start(NveGridLine::Line(2))
        .column_end(NveGridLine::Span(2));

    tx.edit::<NveLayout>(rtl_box)
        .direction(NveDirection::Rtl)
        .flex_direction(NveFlexDirection::Row)
        .justify_content(NveJustifyContent::FlexStart)
        .height(Length::points(50.0))
        .padding_left(Length::points(24.0)) // inline-start -> physical right in RTL!
        .padding_inline_logical(true);
    tx.edit::<NveLayout>(rtl_child)
        .width(Length::points(80.0))
        .height(Length::points(30.0));

    tx.commit_into(&mut changes);
    let vp = Viewport {
        width: 400.0,
        height: 600.0,
        scale_factor: 1.0,
    };
    layout.sync_changes(&store, &changes);
    layout.compute_layout(&store, NodeId::ROOT, vp);

    // Root fills viewport.
    let r_root = layout.layout_of(NodeId::ROOT).unwrap();
    assert_eq!((r_root.x, r_root.y, r_root.width, r_root.height), (0.0, 0.0, 400.0, 600.0));

    // Row respects root 10px padding.
    let r_row = layout.layout_of(row).unwrap();
    assert_eq!((r_row.x, r_row.y, r_row.width, r_row.height), (10.0, 10.0, 380.0, 100.0));

    // Text is measured via HeuristicTextMeasurer (10 chars * 20 * 0.58 = 116, height = 27).
    let r_text = layout.layout_of(text).unwrap();
    assert!((r_text.width - 116.0).abs() <= 1.0, "text w={}", r_text.width);
    assert!((r_text.height - 27.0).abs() <= 1.0, "text h={}", r_text.height);

    // Absolute child pinned to bottom-right of row (x = 10 + 380 - 8 - 40 = 342, y = 10 + 100 - 6 - 24 = 80).
    let r_abs = layout.layout_of(abs).unwrap();
    assert_eq!((r_abs.x, r_abs.y, r_abs.width, r_abs.height), (342.0, 80.0, 40.0, 24.0));

    // Display::None collapses to 0x0.
    let r_hid = layout.layout_of(hidden).unwrap();
    assert_eq!((r_hid.width, r_hid.height), (0.0, 0.0));

    // Grid column 1 = 120px, c2 spans cols 2..4 (380 - 120 - 10 = 250px).
    let r_c1 = layout.layout_of(c1).unwrap();
    let r_c2 = layout.layout_of(c2).unwrap();
    assert_eq!(r_c1.width, 120.0);
    assert_eq!(r_c2.x, r_c1.x + 120.0 + 10.0);
    assert_eq!(r_c2.width, 250.0);

    // RTL logical inline-start padding pushes child 24px from right edge of rtl_box (10 + 380 - 24 - 80 = 286).
    assert!(layout.resolved_rtl(rtl_box));
    assert!(layout.resolved_rtl(rtl_child));
    let r_rtl_child = layout.layout_of(rtl_child).unwrap();
    assert_eq!(r_rtl_child.x, 286.0);

    // Incremental update: resize row height -> shifts grid & rtl_box downward.
    let mut tx = store.tx();
    tx.edit::<NveLayout>(row).height(Length::points(150.0));
    tx.commit_into(&mut changes);
    layout.sync_changes(&store, &changes);
    layout.compute_layout(&store, NodeId::ROOT, vp);
    assert_eq!(layout.layout_of(row).unwrap().height, 150.0);
    assert_eq!(layout.layout_of(grid).unwrap().y, 160.0);
}

#[test]
fn taffy_clears_stale_hoisted_children_and_propagates_rtl_detach() {
    let mut store = NodeStore::new();
    let mut layout = TaffyStoreLayout::new();
    let mut changes = ChangeSet::default();
    let vp = Viewport { width: 400.0, height: 300.0, scale_factor: 1.0 };

    let (rtl_parent, child, host, abs_child) = {
        let mut tx = store.tx();
        let rtl_parent = tx.create();
        let child = tx.create();
        let host = tx.create();
        let abs_child = tx.create();
        tx.set_children(NodeId::ROOT, &[rtl_parent, host]);
        tx.set_children(rtl_parent, &[child]);
        tx.set_children(host, &[abs_child]);

        tx.edit::<NveLayout>(rtl_parent)
            .direction(NveDirection::Rtl)
            .height(Length::points(40.0));
        tx.edit::<NveLayout>(child)
            .width(Length::points(50.0))
            .height(Length::points(20.0));
        tx.edit::<NveLayout>(host)
            .position(NvePosition::Relative)
            .width(Length::points(200.0))
            .height(Length::points(100.0));
        tx.edit::<NveLayout>(abs_child)
            .position(NvePosition::Absolute)
            .inset_left(Length::points(12.0))
            .inset_top(Length::points(14.0))
            .width(Length::points(30.0))
            .height(Length::points(20.0));
        tx.commit_into(&mut changes);
        (rtl_parent, child, host, abs_child)
    };

    layout.sync_changes(&store, &changes);
    layout.compute_layout(&store, NodeId::ROOT, vp);
    assert!(layout.resolved_rtl(rtl_parent));
    assert!(layout.resolved_rtl(child));
    assert!(!layout.hoisted[host.index()].is_empty());

    // 1) Detach Layout from `rtl_parent`: both `rtl_parent` and `child` must revert to LTR.
    // 2) Reparent `abs_child` away so `host` becomes a leaf: `host`'s hoisted list must clear
    //    so `abs_child` is not rounded twice under `host`'s stale origin.
    {
        let mut tx = store.tx();
        tx.detach::<NveLayout>(rtl_parent);
        tx.set_children(host, &[]);
        tx.set_children(NodeId::ROOT, &[rtl_parent, host, abs_child]);
        tx.commit_into(&mut changes);
    }
    layout.sync_changes(&store, &changes);
    layout.compute_layout(&store, NodeId::ROOT, vp);
    assert!(!layout.resolved_rtl(rtl_parent));
    assert!(!layout.resolved_rtl(child));
    assert!(
        layout.hoisted[host.index()].is_empty(),
        "leaf container must clear stale hoisted children"
    );
    let r_abs = layout.layout_of(abs_child).unwrap();
    assert_eq!((r_abs.x, r_abs.y), (12.0, 14.0));
}

#[test]
fn taffy_inherited_typography_explicit_defaults_override_bold_ancestor() {
    let mut store = NodeStore::new();
    let mut layout = TaffyStoreLayout::new();
    let mut changes = ChangeSet::default();
    let vp = Viewport {
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
    };

    let (parent, c_inh, c_reset) = {
        let mut tx = store.tx();
        let p = tx.create();
        let c1 = tx.create();
        let c2 = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[c1, c2]);
        tx.edit::<Typography>(p)
            .color(Rgba::WHITE)
            .font_size(24.0)
            .font_weight(700);
        tx.edit::<TextContent>(c1).text("bold");
        tx.edit::<TextContent>(c2).text("regular");
        tx.edit::<Typography>(c2)
            .color(Rgba::BLACK)
            .font_weight(400);
        tx.commit_into(&mut changes);
        (p, c1, c2)
    };

    layout.sync_changes(&store, &changes);
    layout.compute_layout(&store, NodeId::ROOT, vp);

    let p_t = layout.resolved_text(parent);
    let c1_t = layout.resolved_text(c_inh);
    let c2_t = layout.resolved_text(c_reset);
    assert_eq!((p_t.color, p_t.font_size, p_t.font_weight), (Rgba::WHITE, 24.0, 700));
    assert_eq!((c1_t.color, c1_t.font_size, c1_t.font_weight), (Rgba::WHITE, 24.0, 700));
    assert_eq!((c2_t.color, c2_t.font_size, c2_t.font_weight), (Rgba::BLACK, 24.0, 400));
}
