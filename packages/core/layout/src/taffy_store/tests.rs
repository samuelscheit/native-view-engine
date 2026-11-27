use super::*;
use crate::TextStyle;
use nve_components::layout_ext::{GridSingleTrack, GridTrack, GridTrackUnit};
use nve_components::tables::{
    AlignItems as NveAlignItems, BoxSizing as NveBoxSizing, Direction as NveDirection,
    Display as NveDisplay, FlexDirection as NveFlexDirection, GridItem, GridLayout,
    GridLine as NveGridLine, GridTracks, JustifyContent as NveJustifyContent, Layout as NveLayout,
    Position as NvePosition, TextContent, Typography, Visual,
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
            GridTrack::Repeat(
                1,
                GridSingleTrack::MinMax(GridTrackUnit::Px(60.0), GridTrackUnit::Fr(1.0)),
            ),
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
    assert_eq!(
        (r_root.x, r_root.y, r_root.width, r_root.height),
        (0.0, 0.0, 400.0, 600.0)
    );

    // Row respects root 10px padding.
    let r_row = layout.layout_of(row).unwrap();
    assert_eq!(
        (r_row.x, r_row.y, r_row.width, r_row.height),
        (10.0, 10.0, 380.0, 100.0)
    );

    // Text is measured via HeuristicTextMeasurer (10 chars * 20 * 0.58 = 116, height = 27).
    let r_text = layout.layout_of(text).unwrap();
    assert!(
        (r_text.width - 116.0).abs() <= 1.0,
        "text w={}",
        r_text.width
    );
    assert!(
        (r_text.height - 27.0).abs() <= 1.0,
        "text h={}",
        r_text.height
    );

    // Absolute child pinned to bottom-right of row (x = 10 + 380 - 8 - 40 = 342, y = 10 + 100 - 6 - 24 = 80).
    let r_abs = layout.layout_of(abs).unwrap();
    assert_eq!(
        (r_abs.x, r_abs.y, r_abs.width, r_abs.height),
        (342.0, 80.0, 40.0, 24.0)
    );

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
    let vp = Viewport {
        width: 400.0,
        height: 300.0,
        scale_factor: 1.0,
    };

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
    assert!(!layout.hoisted.get(host.index()).is_empty());

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
        layout.hoisted.get(host.index()).is_empty(),
        "leaf container must clear stale hoisted children"
    );
    let r_abs = layout.layout_of(abs_child).unwrap();
    assert_eq!((r_abs.x, r_abs.y), (12.0, 14.0));
}

/// Long text that wraps to many lines at 100–200 px.
const WRAPPED: &str = "word word word word word word word word word word word word word word word word \
word word word word word word word word word word word word word word word word";

/// Taffy 0.16 drops `aspect_ratio` for leaves sized under `SizingMode::ContentSize`
/// (DioxusLabs/taffy#804); `LayoutView::aspect_ratio_content_size` transfers the known axis.
#[test]
fn empty_leaf_aspect_ratio_with_flex_basis_and_flex_grow_resolves_the_cross_axis() {
    // (container direction, container + child edits, expected child border-box (width, height))
    type Edit = fn(&mut nve_store::Transaction<'_>, NodeId, NodeId);
    let cases: Vec<(NveFlexDirection, Edit, (f32, f32))> = vec![
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(2.0);
            },
            (100.0, 50.0),
        ),
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c).flex_grow(1.0).aspect_ratio(4.0);
            },
            (400.0, 100.0),
        ),
        (
            NveFlexDirection::Column,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(80.0))
                    .aspect_ratio(2.0);
            },
            (160.0, 80.0),
        ),
        // Padding + border: the ratio applies to the border box (Taffy's default `box-sizing`).
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(2.0)
                    .padding_left(Length::points(10.0))
                    .padding_top(Length::points(10.0))
                    .padding_bottom(Length::points(4.0));
                tx.edit::<Visual>(c).border_width(2.0);
            },
            (100.0, 50.0),
        ),
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .flex_grow(1.0)
                    .aspect_ratio(8.0)
                    .padding_top(Length::points(6.0));
                tx.edit::<Visual>(c).border_width(1.0);
            },
            (400.0, 50.0),
        ),
        // Padding + border larger than the ratio's height: the box never shrinks below them.
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(40.0))
                    .aspect_ratio(4.0)
                    .padding_top(Length::points(20.0));
            },
            (40.0, 20.0),
        ),
        // `box-sizing: content-box`: the ratio relates the content box (Taffy `leaf.rs`
        // InherentSize semantics); flex-basis 100 content + 20 padding = 120 border box.
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .box_sizing(NveBoxSizing::ContentBox)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(2.0)
                    .padding_left(Length::points(10.0))
                    .padding_right(Length::points(10.0))
                    .padding_top(Length::points(10.0))
                    .padding_bottom(Length::points(10.0));
            },
            (120.0, 70.0),
        ),
        // `%` padding resolves against the container width (5% of 400 = 20).
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .box_sizing(NveBoxSizing::ContentBox)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(2.0)
                    .padding_left(Length::percent(5.0));
            },
            (120.0, 50.0),
        ),
        // Column `flex-grow`: the grown main (height) axis transfers to the cross (width) axis.
        (
            NveFlexDirection::Column,
            |tx, container, c| {
                tx.edit::<NveLayout>(container)
                    .height(Length::points(300.0));
                tx.edit::<NveLayout>(c).flex_grow(1.0).aspect_ratio(0.5);
            },
            (150.0, 300.0),
        ),
        // `max-height` clamps the transferred cross size and, transferred back through the
        // ratio (CSS Sizing 4 transferred max size), the main size too.
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(1.0)
                    .max_height(Length::points(40.0));
            },
            (40.0, 40.0),
        ),
        // A transfer overflowing `f32` falls back to 0 instead of feeding Taffy `inf`.
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(f32::MIN_POSITIVE);
            },
            (100.0, 0.0),
        ),
        // Invalid ratios are no ratio at all.
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(f32::NAN);
            },
            (100.0, 0.0),
        ),
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(-1.0);
            },
            (100.0, 0.0),
        ),
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(f32::INFINITY);
            },
            (100.0, 0.0),
        ),
        // Short text: the ratio's transferred height is a floor (20 px text, 50 px ratio).
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<TextContent>(c).text("hi");
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(2.0);
            },
            (100.0, 50.0),
        ),
        // Wrapped text taller than the ratio's height keeps its own (measured) height.
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<TextContent>(c).text(WRAPPED);
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(8.0);
            },
            (100.0, -1.0),
        ),
        // Column: the height is known, so the width transferred through the ratio (200) bounds
        // the lines instead of the 400 px container.
        (
            NveFlexDirection::Column,
            |tx, _, c| {
                tx.edit::<TextContent>(c).text(WRAPPED);
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(2.0);
            },
            (200.0, 100.0),
        ),
        // An empty text leaf sizes like any empty leaf.
        (
            NveFlexDirection::Row,
            |tx, _, c| {
                tx.edit::<TextContent>(c).text("");
                tx.edit::<NveLayout>(c)
                    .flex_basis(Length::points(100.0))
                    .aspect_ratio(2.0);
            },
            (100.0, 50.0),
        ),
    ];
    for (i, (direction, edit, expected)) in cases.into_iter().enumerate() {
        let mut store = NodeStore::new();
        let mut layout = TaffyStoreLayout::new();
        let mut changes = ChangeSet::default();
        let mut tx = store.tx();
        let container = tx.create();
        let leaf = tx.create();
        tx.set_children(NodeId::ROOT, &[container]);
        tx.set_children(container, &[leaf]);
        tx.edit::<NveLayout>(container)
            .flex_direction(direction)
            .align_items(NveAlignItems::FlexStart)
            .width(Length::points(400.0));
        edit(&mut tx, container, leaf);
        tx.commit_into(&mut changes);
        layout.sync_changes(&store, &changes);
        layout.compute_layout(
            &store,
            NodeId::ROOT,
            Viewport {
                width: 400.0,
                height: 300.0,
                scale_factor: 1.0,
            },
        );
        let r = layout.layout_of(leaf).expect("leaf laid out");
        if expected.1 < 0.0 {
            // Content-sized: the text's height wrapped at the expected width (several lines,
            // far above the ratio's 12.5 px floor).
            let (_, h) =
                layout
                    .text_measurer()
                    .measure(WRAPPED, &TextStyle::sized(16.0), Some(expected.0));
            assert!(h > 2.0 * 16.0 * 1.25, "case {i}: wraps to several lines");
            assert_eq!((r.width, r.height), (expected.0, h.round()), "case {i}");
        } else {
            assert_eq!((r.width, r.height), expected, "case {i}");
        }
        let c = layout.layout_of(container).expect("container laid out");
        let (cross, leaf_cross) = if direction == NveFlexDirection::Row {
            (c.height, r.height)
        } else {
            (c.width, r.width)
        };
        assert_eq!(
            cross > 0.0,
            leaf_cross > 0.0,
            "case {i}: auto container must size to the leaf"
        );
    }
}

/// A ratio transfer that padding clamps to a 0-wide content box still bounds the text's lines:
/// the text wraps at 0 (one word per line) instead of falling back to the 400 px container.
#[test]
fn text_wraps_inside_a_ratio_box_whose_padding_consumes_the_transferred_width() {
    let mut store = NodeStore::new();
    let mut layout = TaffyStoreLayout::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let container = tx.create();
    let leaf = tx.create();
    tx.set_children(NodeId::ROOT, &[container]);
    tx.set_children(container, &[leaf]);
    tx.edit::<NveLayout>(container)
        .flex_direction(NveFlexDirection::Column)
        .align_items(NveAlignItems::FlexStart)
        .width(Length::points(400.0));
    tx.edit::<TextContent>(leaf).text(WRAPPED);
    // Border-box height 25 → width 50 through the ratio, all of it padding (`min-height: 0`
    // keeps the automatic minimum size from growing the item back to its content).
    tx.edit::<NveLayout>(leaf)
        .flex_basis(Length::points(25.0))
        .min_height(Length::points(0.0))
        .aspect_ratio(2.0)
        .padding_left(Length::points(50.0));
    tx.commit_into(&mut changes);
    layout.sync_changes(&store, &changes);
    let vp = Viewport {
        width: 400.0,
        height: 300.0,
        scale_factor: 1.0,
    };
    assert!(layout.compute_layout(&store, NodeId::ROOT, vp));
    let (word_w, _) = layout
        .text_measurer()
        .measure(WRAPPED, &TextStyle::sized(16.0), Some(0.0));
    let r = layout.layout_of(leaf).expect("leaf laid out");
    assert_eq!((r.width, r.height), ((50.0 + word_w).round(), 25.0));
    assert!(
        r.width < 100.0,
        "wrapped at 0, not at the container: {}",
        r.width
    );
}

/// A ratio whose transfer overflows `f32` (`100 × 1e38`) transfers nothing: the text wraps at
/// the container like a ratio-less leaf instead of at a width clamped to 0.
#[test]
fn an_overflowing_ratio_transfer_does_not_collapse_text_to_zero_width() {
    let mut store = NodeStore::new();
    let mut layout = TaffyStoreLayout::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let container = tx.create();
    let leaf = tx.create();
    tx.set_children(NodeId::ROOT, &[container]);
    tx.set_children(container, &[leaf]);
    tx.edit::<NveLayout>(container)
        .flex_direction(NveFlexDirection::Column)
        .align_items(NveAlignItems::FlexStart)
        .width(Length::points(400.0));
    tx.edit::<TextContent>(leaf).text(WRAPPED);
    tx.edit::<NveLayout>(leaf)
        .flex_basis(Length::points(100.0))
        .min_height(Length::points(0.0))
        .aspect_ratio(1e38);
    tx.commit_into(&mut changes);
    layout.sync_changes(&store, &changes);
    let vp = Viewport {
        width: 400.0,
        height: 300.0,
        scale_factor: 1.0,
    };
    assert!(layout.compute_layout(&store, NodeId::ROOT, vp));
    let (line_w, _) = layout
        .text_measurer()
        .measure(WRAPPED, &TextStyle::sized(16.0), Some(400.0));
    let r = layout.layout_of(leaf).expect("leaf laid out");
    assert!(r.width.is_finite() && r.height.is_finite(), "{r:?}");
    assert_eq!((r.width, r.height), (line_w.round(), 100.0));
}
