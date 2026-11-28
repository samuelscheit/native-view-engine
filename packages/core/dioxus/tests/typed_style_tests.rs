//! Regression tests for `TypedStyle::merge` bitmask exactness (P0 #1), `TypedStyle::clear_diff_from`
//! layout/grid/inline-logical resets (P0 #2), and tag-default cursor restoration (P2 #1).

use std::rc::Rc;

use dioxus_core::{AttributeValue, ElementId, Template, TemplateNode, WriteMutations};
use nve_abi::{ColorRgba, SurfaceId};
use nve_dioxus::{NveMutationWriter, TypedStyle, NVE_TYPED_STYLE_ATTR};
use nve_store::{ChangeSet, GridLine, Length, NodeId, NodeStore};
use nve_tables::interaction::CursorKind;
use nve_tables::layout_ext::GridTrack;
use nve_tables::tables::{Display, GridItem, GridLayout, Interaction, Layout, Overflow, Position, Visual};

#[test]
fn merge_into_none_layout_ext_and_overflow_preserves_exact_masks_without_clobbering() {
    // 1. Merging `.flex_1()` into a style with `layout_ext: None` must NOT set `E_ALL`!
    //    Applying the merged style onto a node that already has `display: Grid`, `position: Absolute`,
    //    `inset_top: 12px`, and `aspect_ratio: 2.0` must touch ONLY `flex_grow`/`flex_shrink`/`flex_basis`.
    let merged = TypedStyle::new()
        .bg(ColorRgba::rgb(15, 23, 42))
        .merged(&TypedStyle::new().flex_1());

    let mut store = NodeStore::new();
    let node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Layout>(n)
            .display(Display::Grid)
            .position(Position::Absolute)
            .inset_top(Length::points(12.0))
            .aspect_ratio(Some(2.0));
        tx.edit::<Visual>(n).overflow_x(Overflow::Hidden);
        merged.apply_to_tx(&mut tx, n);
        tx.commit_into(&mut ChangeSet::default());
        n
    };
    let lay = store.try_row::<Layout>(node).expect("layout row");
    assert_eq!(lay.display(), Display::Grid, "merge(.flex_1()) must not clobber display");
    assert_eq!(lay.position(), Position::Absolute, "merge(.flex_1()) must not clobber position");
    assert_eq!(lay.inset_top(), Length::points(12.0), "merge(.flex_1()) must not clobber inset_top");
    assert_eq!(lay.aspect_ratio(), Some(2.0), "merge(.flex_1()) must not clobber aspect_ratio");
    assert_eq!(lay.flex_grow(), 1.0);
    assert_eq!(lay.flex_basis(), Length::percent(0.0));

    // 2. Chained merge: merging `bg.merged(position_absolute)` on top of `display_grid` must preserve `display: Grid`!
    let abs_badge = TypedStyle::new()
        .bg(ColorRgba::rgb(37, 99, 235))
        .merged(&TypedStyle::new().position_absolute());
    let combined = TypedStyle::new().display_grid().merged(&abs_badge);
    let node2 = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[node, n]);
        combined.apply_to_tx(&mut tx, n);
        tx.commit_into(&mut ChangeSet::default());
        n
    };
    let lay2 = store.try_row::<Layout>(node2).expect("layout2 row");
    assert_eq!(lay2.display(), Display::Grid, "chained merge must preserve display_grid");
    assert_eq!(lay2.position(), Position::Absolute);

    // 3. Merging single-axis `overflow_y(Auto)` into `overflow: None`, then merging onto `overflow_x(Hidden)`
    //    must preserve `overflow_x: Hidden` and NOT widen `overflow_axes` to `OV_BOTH`!
    let scroll_y = TypedStyle::new()
        .bg(ColorRgba::rgb(0, 0, 0))
        .merged(&TypedStyle::new().overflow_y(Overflow::Auto));
    let combined_ov = TypedStyle::new().overflow_x(Overflow::Hidden).merged(&scroll_y);
    assert_eq!(combined_ov.overflow_value(), Some((Overflow::Hidden, Overflow::Auto)));

    let node3 = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[node, node2, n]);
        tx.edit::<Visual>(n).overflow_x(Overflow::Scroll);
        scroll_y.apply_to_tx(&mut tx, n);
        tx.commit_into(&mut ChangeSet::default());
        n
    };
    let vis3 = store.try_row::<Visual>(node3).expect("vis3");
    assert_eq!(vis3.overflow_x(), Overflow::Scroll, "overflow_y merge must not clobber overflow_x");
    assert_eq!(vis3.overflow_y(), Overflow::Auto);

    // 4. Merging `padding_inline_start(8.0)` into `layout_ext: None` must NOT set `E_PAD_INLINE_END`.
    let pad_start = TypedStyle::new()
        .bg(ColorRgba::rgb(1, 2, 3))
        .merged(&TypedStyle::new().padding_inline_start(8.0));
    let pad_both = TypedStyle::new().padding_inline_end(20.0).merged(&pad_start);
    let node4 = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[node, node2, node3, n]);
        pad_both.apply_to_tx(&mut tx, n);
        tx.commit_into(&mut ChangeSet::default());
        n
    };
    let lay4 = store.try_row::<Layout>(node4).expect("lay4");
    assert!(lay4.padding_inline_logical());
    assert_eq!(lay4.padding_left(), Length::points(8.0));
    assert_eq!(lay4.padding_right(), Length::points(20.0));
}

#[test]
fn clear_layout_diff_resets_inline_logical_gaps_and_grid_tables_when_next_still_has_layout_ext() {
    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);
    let initial = TypedStyle::new()
        .position_absolute()
        .padding_inline_start(14.0)
        .padding_inline_end(18.0)
        .margin_inline_start(6.0)
        .margin_inline_end(10.0)
        .column_gap(16.0)
        .row_gap(12.0)
        .grid_cols(vec![GridTrack::fr(1.0), GridTrack::fr(1.0)])
        .grid_rows(vec![GridTrack::px(40.0)])
        .grid_column_span(2)
        .grid_row_span(3);

    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(initial)),
            ElementId(1),
        );
        mtx.append_children(ElementId(0), 1);
    });
    let node = writer.resolve_el(ElementId(1)).unwrap();
    let lay = writer.store().try_row::<Layout>(node).unwrap();
    assert!(lay.padding_inline_logical());
    assert_eq!(lay.padding_left(), Length::points(14.0));
    assert_eq!(lay.padding_right(), Length::points(18.0));
    assert!(lay.margin_inline_logical());
    assert_eq!(lay.margin_left(), Length::points(6.0));
    assert_eq!(lay.margin_right(), Length::points(10.0));
    assert_eq!(lay.column_gap(), Length::points(16.0));
    assert_eq!(lay.row_gap(), Length::points(12.0));
    assert!(writer.store().has::<GridLayout>(node));
    assert!(writer.store().has::<GridItem>(node));

    // Step 1: Drop one side of inline padding/margin, column_gap, grid_cols, and grid_column_span
    // while keeping `position_absolute()`, `padding_inline_start`, `margin_inline_end`, `row_gap`,
    // `grid_rows`, and `grid_row_span` (so `next.layout_ext.is_some()`!).
    let partial = TypedStyle::new()
        .position_absolute()
        .padding_inline_start(14.0)
        .margin_inline_end(10.0)
        .row_gap(12.0)
        .grid_rows(vec![GridTrack::px(40.0)])
        .grid_row_span(3);
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(partial)),
            ElementId(1),
        );
    });
    let lay = writer.store().try_row::<Layout>(node).unwrap();
    assert!(lay.padding_inline_logical());
    assert_eq!(lay.padding_left(), Length::points(14.0));
    assert_eq!(lay.padding_right(), Length::ZERO, "dropped padding_inline_end must reset padding_right");
    assert!(lay.margin_inline_logical());
    assert_eq!(lay.margin_left(), Length::ZERO, "dropped margin_inline_start must reset margin_left");
    assert_eq!(lay.margin_right(), Length::points(10.0));
    assert_eq!(lay.column_gap(), Length::ZERO, "dropped column_gap must reset to 0");
    assert_eq!(lay.row_gap(), Length::points(12.0));

    let gl = writer.store().try_row::<GridLayout>(node).expect("GridLayout kept for template_rows");
    assert!(gl.template_columns().is_none(), "dropped grid_cols must clear template_columns");
    assert!(!gl.template_rows().is_none(), "kept grid_rows must remain");

    let gi = writer.store().try_row::<GridItem>(node).expect("GridItem kept for row_span");
    assert_eq!(gi.column_start(), GridLine::Auto, "dropped grid_column must reset column_start");
    assert_eq!(gi.column_end(), GridLine::Auto, "dropped grid_column must reset column_end");
    assert_eq!(gi.row_start(), GridLine::Span(3));

    // Step 2: Drop ALL grid/inline/gap fields while STILL keeping `position_absolute()` (`layout_ext.is_some()`!).
    let only_pos = TypedStyle::new().position_absolute();
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(only_pos)),
            ElementId(1),
        );
    });
    let lay = writer.store().try_row::<Layout>(node).unwrap();
    assert!(!lay.padding_inline_logical());
    assert_eq!(lay.padding_left(), Length::ZERO);
    assert!(!lay.margin_inline_logical());
    assert_eq!(lay.margin_right(), Length::ZERO);
    assert_eq!(lay.row_gap(), Length::ZERO);
    assert!(!writer.store().has::<GridLayout>(node), "GridLayout must detach when both cols & rows dropped");
    assert!(!writer.store().has::<GridItem>(node), "GridItem must detach when both col & row items dropped");
}

static BUTTON_AND_LINK_TEMPLATE: Template = Template {
    roots: &[
        TemplateNode::Element {
            tag: "pressable",
            namespace: None,
            attrs: &[],
            children: &[],
        },
        TemplateNode::Element {
            tag: "a",
            namespace: None,
            attrs: &[],
            children: &[],
        },
        TemplateNode::Element {
            tag: "view",
            namespace: None,
            attrs: &[],
            children: &[],
        },
    ],
    node_paths: &[],
    attr_paths: &[],
};

#[test]
fn clearing_typed_style_cursor_restores_tag_default_pointer_on_pressable_and_link() {
    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);
    writer.with_transaction(|mtx| {
        mtx.load_template(BUTTON_AND_LINK_TEMPLATE, 0, ElementId(1));
        mtx.load_template(BUTTON_AND_LINK_TEMPLATE, 1, ElementId(2));
        mtx.load_template(BUTTON_AND_LINK_TEMPLATE, 2, ElementId(3));
        mtx.append_children(ElementId(0), 3);
    });
    let btn = writer.resolve_el(ElementId(1)).unwrap();
    let link = writer.resolve_el(ElementId(2)).unwrap();
    let view = writer.resolve_el(ElementId(3)).unwrap();
    assert_eq!(writer.store().try_row::<Interaction>(btn).unwrap().cursor(), CursorKind::Pointer);
    assert_eq!(writer.store().try_row::<Interaction>(link).unwrap().cursor(), CursorKind::Pointer);

    // Override cursor on all three, then drop cursor via TypedStyle update on btn/view and None on link.
    let disabled_style = TypedStyle::new().cursor(CursorKind::NotAllowed).opacity(0.5);
    writer.with_transaction(|mtx| {
        for el in [ElementId(1), ElementId(2), ElementId(3)] {
            mtx.set_attribute(
                NVE_TYPED_STYLE_ATTR,
                None,
                &AttributeValue::Any(Rc::new(disabled_style.clone())),
                el,
            );
        }
    });
    assert_eq!(writer.store().try_row::<Interaction>(btn).unwrap().cursor(), CursorKind::NotAllowed);
    assert_eq!(writer.store().try_row::<Interaction>(link).unwrap().cursor(), CursorKind::NotAllowed);
    assert_eq!(writer.store().try_row::<Interaction>(view).unwrap().cursor(), CursorKind::NotAllowed);

    let resting_style = TypedStyle::new().opacity(1.0);
    writer.with_transaction(|mtx| {
        mtx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &AttributeValue::Any(Rc::new(resting_style.clone())), ElementId(1));
        mtx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &AttributeValue::None, ElementId(2));
        mtx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &AttributeValue::Any(Rc::new(resting_style)), ElementId(3));
    });
    assert_eq!(
        writer.store().try_row::<Interaction>(btn).unwrap().cursor(),
        CursorKind::Pointer,
        "dropping cursor override on <pressable> must restore CursorKind::Pointer"
    );
    assert_eq!(
        writer.store().try_row::<Interaction>(link).unwrap().cursor(),
        CursorKind::Pointer,
        "clearing nve_style on <a> must restore CursorKind::Pointer"
    );
    assert_eq!(
        writer.store().try_row::<Interaction>(view).unwrap().cursor(),
        CursorKind::Auto,
        "dropping cursor override on plain <view> must restore CursorKind::Auto"
    );
}
