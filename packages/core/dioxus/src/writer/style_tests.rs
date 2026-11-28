//! Focused unit tests for `NveMutationWriter` canonical `TypedStyle` + numeric standalone
//! precedence, shorthand/longhand ranking, composite fast lane, clamping, and input length parsing.

use dioxus_core::{AttributeValue, ElementId, WriteMutations};
use nve_abi::SurfaceId;
use nve_store::{Length, NveTable};
use nve_tables::tables::{Layout, TextInput, Typography, Visual};

use super::NveMutationWriter;

#[test]
fn canonical_typed_and_numeric_precedence_and_composite_fast_lane() {
    use std::rc::Rc;

    use crate::typed_style::{TypedStyle, NVE_TYPED_STYLE_ATTR};
    use nve_abi::table::effect;
    use nve_abi::ColorRgba;

    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);

    // 1. Standalone numeric `width` beats `TypedStyle` width regardless of set order, and
    //    clearing standalone `width` restores `TypedStyle` width.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(TypedStyle::new().width_px(80.0).bg(ColorRgba::rgb(10, 20, 30)))),
            ElementId(1),
        );
        mtx.set_attribute("width", None, &AttributeValue::Float(240.0), ElementId(1));
        mtx.append_children(ElementId(0), 1);
    });
    let n1 = writer.resolve_el(ElementId(1)).unwrap();
    assert_eq!(writer.store().try_row::<Layout>(n1).unwrap().width(), Length::points(240.0));

    writer.with_transaction(|mtx| {
        mtx.set_attribute("width", None, &AttributeValue::None, ElementId(1));
    });
    assert_eq!(
        writer.store().try_row::<Layout>(n1).unwrap().width(),
        Length::points(80.0),
        "clearing standalone width must fall back to TypedStyle width"
    );

    // Clearing sole standalone width with no TypedStyle detaches Layout cleanly.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(2));
        mtx.set_attribute("width", None, &AttributeValue::Float(240.0), ElementId(2));
        mtx.append_children(ElementId(0), 1);
    });
    let n2 = writer.resolve_el(ElementId(2)).unwrap();
    writer.with_transaction(|mtx| {
        mtx.set_attribute("width", None, &AttributeValue::None, ElementId(2));
    });
    assert!(!writer.store().has::<Layout>(n2));

    // 2. Non-overlapping standalone `opacity` update emits strictly `effect::COMPOSITE` without
    //    replaying unrelated standalone attributes or dirtying Layout/Typography.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(3));
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(
                TypedStyle::new()
                    .width_px(80.0)
                    .bg(ColorRgba::rgb(0x11, 0x22, 0x33))
                    .color(ColorRgba::rgb(0xee, 0xee, 0xee)),
            )),
            ElementId(3),
        );
        mtx.set_attribute("opacity", None, &AttributeValue::Float(0.5), ElementId(3));
        mtx.set_attribute("borderRadius", None, &AttributeValue::Float(6.0), ElementId(3));
        mtx.append_children(ElementId(0), 1);
    });
    let n3 = writer.resolve_el(ElementId(3)).unwrap();
    writer.standalone_replay_count = 0;
    writer.with_transaction(|mtx| {
        mtx.set_attribute("opacity", None, &AttributeValue::Float(0.25), ElementId(3));
    });
    assert_eq!(
        writer.standalone_replay_count, 0,
        "non-overlapping standalone opacity update must perform zero trailing standalone replays"
    );
    let vis3 = writer.store().try_row::<Visual>(n3).unwrap();
    assert!((vis3.opacity() - 0.25).abs() < 1e-6);
    assert_eq!(vis3.background().0, ColorRgba::rgb(0x11, 0x22, 0x33).to_u32());
    assert!(writer.last_changes().table(Layout::slot()).is_none());
    assert!(writer.last_changes().table(Typography::slot()).is_none());
    let idx = writer
        .last_changes()
        .touched
        .iter()
        .position(|&id| id == n3)
        .expect("n3 must be in touched");
    assert_eq!(
        writer.last_changes().effects[idx],
        effect::COMPOSITE,
        "non-overlapping standalone opacity update must emit strictly effect::COMPOSITE"
    );
}

#[test]
fn numeric_shorthand_vs_longhand_ranking_survives_updates_and_clears() {
    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);

    // 1. `padding` < `paddingHorizontal` on in-place update and after clear+re-set.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute("padding", None, &AttributeValue::Float(16.0), ElementId(1));
        mtx.set_attribute("paddingHorizontal", None, &AttributeValue::Float(4.0), ElementId(1));
        mtx.append_children(ElementId(0), 1);
    });
    let n1 = writer.resolve_el(ElementId(1)).unwrap();
    writer.with_transaction(|mtx| {
        mtx.set_attribute("padding", None, &AttributeValue::Float(24.0), ElementId(1));
    });
    let lay = writer.store().try_row::<Layout>(n1).unwrap();
    assert_eq!(lay.padding_top(), Length::points(24.0));
    assert_eq!(lay.padding_bottom(), Length::points(24.0));
    assert_eq!(lay.padding_left(), Length::points(4.0));
    assert_eq!(lay.padding_right(), Length::points(4.0));

    writer.with_transaction(|mtx| {
        mtx.set_attribute("padding", None, &AttributeValue::None, ElementId(1));
    });
    writer.with_transaction(|mtx| {
        mtx.set_attribute("padding", None, &AttributeValue::Float(20.0), ElementId(1));
    });
    let lay = writer.store().try_row::<Layout>(n1).unwrap();
    assert_eq!(lay.padding_top(), Length::points(20.0));
    assert_eq!(lay.padding_left(), Length::points(4.0));

    // 2. `flex` < `flexGrow` and `gap` < `rowGap` survive clear+re-set.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(2));
        mtx.set_attribute("flex", None, &AttributeValue::Float(1.0), ElementId(2));
        mtx.set_attribute("flexGrow", None, &AttributeValue::Float(3.0), ElementId(2));
        mtx.set_attribute("gap", None, &AttributeValue::Float(8.0), ElementId(2));
        mtx.set_attribute("rowGap", None, &AttributeValue::Float(20.0), ElementId(2));
        mtx.append_children(ElementId(0), 1);
    });
    let n2 = writer.resolve_el(ElementId(2)).unwrap();
    writer.with_transaction(|mtx| {
        mtx.set_attribute("flex", None, &AttributeValue::None, ElementId(2));
        mtx.set_attribute("gap", None, &AttributeValue::None, ElementId(2));
    });
    writer.with_transaction(|mtx| {
        mtx.set_attribute("flex", None, &AttributeValue::Float(2.0), ElementId(2));
        mtx.set_attribute("gap", None, &AttributeValue::Float(12.0), ElementId(2));
    });
    let lay2 = writer.store().try_row::<Layout>(n2).unwrap();
    assert_eq!(lay2.flex_grow(), 3.0);
    assert_eq!(lay2.row_gap(), Length::points(20.0));
    assert_eq!(lay2.column_gap(), Length::points(12.0));
}

#[test]
fn typed_style_clamps_extreme_scalars_to_the_style_scalar_range() {
    use crate::typed_style::TypedStyle;
    use nve_abi::num::MAX_STYLE_SCALAR;
    use nve_abi::Dimension;

    let clamped = TypedStyle::new()
        .width(Dimension::Points(1e8))
        .height(Dimension::Percent(f32::INFINITY))
        .gap(2e7)
        .margin(-2e7)
        .font_size(1e8);
    assert_eq!(clamped.layout().width, Dimension::Points(MAX_STYLE_SCALAR));
    assert_eq!(clamped.layout().height, Dimension::Percent(0.0));
    assert_eq!(clamped.layout().gap, MAX_STYLE_SCALAR);
    assert_eq!(clamped.layout().margin.top, -MAX_STYLE_SCALAR);
    assert_eq!(clamped.style().font_size, MAX_STYLE_SCALAR);
}

#[test]
fn text_input_minlength_and_maxlength_preserve_zero_and_map_negative_to_none() {
    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.append_children(ElementId(0), 1);
    });
    let n1 = writer.resolve_el(ElementId(1)).unwrap();

    writer.with_transaction(|mtx| {
        mtx.set_attribute("minlength", None, &AttributeValue::Int(0), ElementId(1));
        mtx.set_attribute("maxlength", None, &AttributeValue::Int(0), ElementId(1));
    });
    let ti = writer.store().try_row::<TextInput>(n1).unwrap();
    assert_eq!(ti.min_length(), Some(0));
    assert_eq!(ti.max_length(), Some(0));

    writer.with_transaction(|mtx| {
        mtx.set_attribute("minlength", None, &AttributeValue::Int(100_000), ElementId(1));
        mtx.set_attribute("maxlength", None, &AttributeValue::Int(100_000), ElementId(1));
    });
    let ti = writer.store().try_row::<TextInput>(n1).unwrap();
    assert_eq!(ti.min_length(), Some(u16::MAX));
    assert_eq!(ti.max_length(), Some(u16::MAX));

    writer.with_transaction(|mtx| {
        mtx.set_attribute("minlength", None, &AttributeValue::Int(-5), ElementId(1));
        mtx.set_attribute("maxlength", None, &AttributeValue::Int(-1), ElementId(1));
    });
    let ti = writer.store().try_row::<TextInput>(n1).unwrap();
    assert_eq!(ti.min_length(), None);
    assert_eq!(ti.max_length(), None);

    writer.with_transaction(|mtx| {
        mtx.set_attribute("minlength", None, &AttributeValue::Text("0".into()), ElementId(1));
        mtx.set_attribute("maxlength", None, &AttributeValue::Text("42".into()), ElementId(1));
    });
    let ti = writer.store().try_row::<TextInput>(n1).unwrap();
    assert_eq!(ti.min_length(), Some(0));
    assert_eq!(ti.max_length(), Some(42));

    writer.with_transaction(|mtx| {
        mtx.set_attribute("minlength", None, &AttributeValue::Text("-3".into()), ElementId(1));
        mtx.set_attribute("maxlength", None, &AttributeValue::Text("-9".into()), ElementId(1));
    });
    let ti = writer.store().try_row::<TextInput>(n1).unwrap();
    assert_eq!(ti.min_length(), None);
    assert_eq!(ti.max_length(), None);
}
