//! Unit tests for `NveMutationWriter`: string CSS forwarding, direct `TypedStyle` parity,
//! `nve_style` + `style` override merging (order-independent), and inherited text-prop updates.

use dioxus_core::{AttributeValue, ElementId, IntoAttributeValue, WriteMutations};
use nve_abi::{
    ColorRgba, Command, CommandDecoder, ComponentKind, Dimension, EdgeInsets, FlexAlign,
    FlexDirection, FlexJustify, LayoutProps, NodeId, RawPropValue, StyleProps,
};

use crate::typed_style::{TypedStyle, NVE_TYPED_STYLE_ATTR};

use super::NveMutationWriter;

fn decode_cmds(bytes: &[u8]) -> Vec<Command> {
    CommandDecoder::new(bytes)
        .expect("valid header")
        .decode_all()
        .expect("valid command buffer")
        .1
}

fn has_raw_str(cmds: &[Command], nid: NodeId, k: &str, expected: &str) -> bool {
    cmds.iter().any(|c| {
        matches!(
            c,
            Command::SetRawProp {
                node_id,
                key,
                value: RawPropValue::String(v)
            } if *node_id == nid && key == k && v == expected
        )
    })
}

fn last_layout(cmds: &[Command], nid: NodeId) -> Option<LayoutProps> {
    cmds.iter().rev().find_map(|c| match c {
        Command::SetLayout { node_id, layout } if *node_id == nid => Some(*layout),
        _ => None,
    })
}

fn last_style(cmds: &[Command], nid: NodeId) -> Option<StyleProps> {
    cmds.iter().rev().find_map(|c| match c {
        Command::SetStyle { node_id, style } if *node_id == nid => Some(*style),
        _ => None,
    })
}

#[test]
fn inline_style_forwards_font_weight_text_align_box_shadow_and_inherits_text_props() {
    let mut writer = NveMutationWriter::new(1);
    writer.begin_frame(1, 1);

    let card_id = writer.alloc_node(ComponentKind::View, None);
    writer.apply_str_attr(
        card_id,
        "style",
        "background-color: #1e293b; border-radius: 10px; box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35);",
    );

    let parent_text = writer.alloc_node(ComponentKind::Text, None);
    writer.apply_str_attr(
        parent_text,
        "style",
        "font-size: 18px; font-weight: 700; text-align: center; color: #ffffff;",
    );
    let leaf_text = writer.alloc_node(ComponentKind::Text, None);
    writer.record_text(leaf_text, "Title");
    writer.builder.set_text(leaf_text, "Title");

    writer.nodes.get_mut(&parent_text).unwrap().children.push(leaf_text);
    writer.nodes.get_mut(&leaf_text).unwrap().parent = Some(parent_text);
    writer.dirty_children.push(parent_text);

    let cmds = decode_cmds(writer.finish_frame(true));

    assert!(has_raw_str(&cmds, card_id, "box-shadow", "0 4px 12px rgba(0, 0, 0, 0.35)"));
    assert!(has_raw_str(&cmds, parent_text, "font-weight", "700"));
    assert!(has_raw_str(&cmds, parent_text, "text-align", "center"));
    assert!(has_raw_str(&cmds, leaf_text, "font-weight", "700"));
    assert!(has_raw_str(&cmds, leaf_text, "text-align", "center"));
}

#[test]
fn typed_style_matches_string_css_identically_including_leaf_text_inheritance() {
    // 1. Render via string CSS.
    let mut str_writer = NveMutationWriter::new(1);
    str_writer.begin_frame(1, 1);
    let s_card = str_writer.alloc_node(ComponentKind::View, None);
    str_writer.apply_str_attr(
        s_card,
        "style",
        "width: 100%; height: 48px; min-width: 120px; max-width: 560px; flex-direction: row; justify-content: space-between; align-items: center; flex-grow: 1; flex-shrink: 0; gap: 8px; padding-horizontal: 16px; padding-vertical: 10px; margin: 4px; background-color: #1e293b; border-width: 2px; border-color: #60a5fa; border-radius: 10px; opacity: 0.85; z-index: 900; box-shadow: 0 4px 16px rgba(0, 0, 0, 0.45); overflow: hidden;",
    );
    let s_parent = str_writer.alloc_node(ComponentKind::Text, None);
    str_writer.apply_str_attr(
        s_parent,
        "style",
        "color: #f8fafc; font-size: 18px; font-weight: 700; text-align: center; opacity: 0.9;",
    );
    let s_leaf = str_writer.alloc_node(ComponentKind::Text, None);
    str_writer.record_text(s_leaf, "Heading");
    str_writer.builder.set_text(s_leaf, "Heading");
    str_writer.nodes.get_mut(&s_parent).unwrap().children.push(s_leaf);
    str_writer.nodes.get_mut(&s_leaf).unwrap().parent = Some(s_parent);
    str_writer.dirty_children.push(s_parent);
    let str_cmds = decode_cmds(str_writer.finish_frame(true));

    // 2. Render via direct `TypedStyle` (zero CSS string formatting/parsing).
    let mut typed_writer = NveMutationWriter::new(1);
    typed_writer.begin_frame(1, 1);
    let t_card = typed_writer.alloc_node(ComponentKind::View, None);
    let card_el = ElementId(10);
    typed_writer.bind_element_id(t_card, card_el);
    let card_typed = TypedStyle::new()
        .width_pct(100.0)
        .height_px(48.0)
        .min_width_px(120.0)
        .max_width_px(560.0)
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Center)
        .flex_grow(1.0)
        .flex_shrink(0.0)
        .gap(8.0)
        .padding_xy(16.0, 10.0)
        .margin(4.0)
        .bg(ColorRgba::rgb(0x1e, 0x29, 0x3b))
        .border(2.0, ColorRgba::rgb(0x60, 0xa5, 0xfa))
        .radius(10.0)
        .opacity(0.85)
        .z_index(900)
        .box_shadow("0 4px 16px rgba(0, 0, 0, 0.45)")
        .overflow("hidden")
        .object_fit("cover");
    let card_attr_val = card_typed.into_value();
    typed_writer.set_attribute(NVE_TYPED_STYLE_ATTR, None, &card_attr_val, card_el);

    let t_parent = typed_writer.alloc_node(ComponentKind::Text, None);
    let parent_el = ElementId(11);
    typed_writer.bind_element_id(t_parent, parent_el);
    let text_typed = TypedStyle::new()
        .color(ColorRgba::rgb(0xf8, 0xfa, 0xfc))
        .font_size(18.0)
        .font_weight("700")
        .text_align("center")
        .opacity(0.9);
    let text_attr_val = text_typed.into_value();
    typed_writer.set_attribute(NVE_TYPED_STYLE_ATTR, None, &text_attr_val, parent_el);

    let t_leaf = typed_writer.alloc_node(ComponentKind::Text, None);
    typed_writer.record_text(t_leaf, "Heading");
    typed_writer.builder.set_text(t_leaf, "Heading");
    typed_writer.nodes.get_mut(&t_parent).unwrap().children.push(t_leaf);
    typed_writer.nodes.get_mut(&t_leaf).unwrap().parent = Some(t_parent);
    typed_writer.dirty_children.push(t_parent);
    let typed_cmds = decode_cmds(typed_writer.finish_frame(true));

    assert_eq!(last_layout(&typed_cmds, t_card), last_layout(&str_cmds, s_card));
    assert_eq!(last_style(&typed_cmds, t_card), last_style(&str_cmds, s_card));
    assert_eq!(last_style(&typed_cmds, t_parent), last_style(&str_cmds, s_parent));
    assert_eq!(last_style(&typed_cmds, t_leaf), last_style(&str_cmds, s_leaf));

    assert!(has_raw_str(&typed_cmds, t_card, "box-shadow", "0 4px 16px rgba(0, 0, 0, 0.45)"));
    assert!(has_raw_str(&typed_cmds, t_card, "overflow", "hidden"));
    assert!(has_raw_str(&typed_cmds, t_card, "object-fit", "cover"));
    assert!(has_raw_str(&typed_cmds, t_parent, "font-weight", "700"));
    assert!(has_raw_str(&typed_cmds, t_parent, "text-align", "center"));
    assert!(has_raw_str(&typed_cmds, t_leaf, "font-weight", "700"));
    assert!(has_raw_str(&typed_cmds, t_leaf, "text-align", "center"));

    // Frame 2: changing parent text weight & align propagates cleanly to unstyled leaf text.
    typed_writer.begin_frame(1, 2);
    let updated_text = TypedStyle::new()
        .color(ColorRgba::rgb(0x94, 0xa3, 0xb8))
        .font_size(14.0)
        .font_weight("400")
        .text_align("right")
        .opacity(1.0);
    let updated_val = updated_text.into_value();
    typed_writer.set_attribute(NVE_TYPED_STYLE_ATTR, None, &updated_val, parent_el);
    let cmds2 = decode_cmds(typed_writer.finish_frame(true));

    assert_eq!(
        last_style(&cmds2, t_leaf).unwrap().text_color,
        ColorRgba::rgb(0x94, 0xa3, 0xb8)
    );
    assert_eq!(last_style(&cmds2, t_leaf).unwrap().font_size, 14.0);
    assert!(has_raw_str(&cmds2, t_leaf, "font-weight", "400"));
    assert!(has_raw_str(&cmds2, t_leaf, "text-align", "right"));
}

#[test]
fn typed_style_merges_with_inline_style_override_deterministically_in_either_order() {
    let base = TypedStyle::new()
        .flex_row()
        .gap(8.0)
        .padding(12.0)
        .bg(ColorRgba::rgb(0x25, 0x63, 0xeb))
        .border(1.0, ColorRgba::rgb(0xcb, 0xd5, 0xe1))
        .radius(8.0)
        .box_shadow("none");
    let override_css = "padding: 24px; background-color: #ef4444; box-shadow: 0 2px 6px #00000040;";

    // Order A: nve_style first, style second.
    let mut w_a = NveMutationWriter::new(1);
    w_a.begin_frame(1, 1);
    let nid_a = w_a.alloc_node(ComponentKind::View, None);
    let el_a = ElementId(1);
    w_a.bind_element_id(nid_a, el_a);
    w_a.set_attribute(NVE_TYPED_STYLE_ATTR, None, &base.clone().into_value(), el_a);
    w_a.set_attribute("style", None, &AttributeValue::Text(override_css.into()), el_a);
    let cmds_a = decode_cmds(w_a.finish_frame(true));

    // Order B: style first, nve_style second.
    let mut w_b = NveMutationWriter::new(1);
    w_b.begin_frame(1, 1);
    let nid_b = w_b.alloc_node(ComponentKind::View, None);
    let el_b = ElementId(1);
    w_b.bind_element_id(nid_b, el_b);
    w_b.set_attribute("style", None, &AttributeValue::Text(override_css.into()), el_b);
    w_b.set_attribute(NVE_TYPED_STYLE_ATTR, None, &base.clone().into_value(), el_b);
    let cmds_b = decode_cmds(w_b.finish_frame(true));

    let lay_a = last_layout(&cmds_a, nid_a).unwrap();
    let lay_b = last_layout(&cmds_b, nid_b).unwrap();
    assert_eq!(lay_a, lay_b);
    assert_eq!(lay_a.flex_direction, FlexDirection::Row);
    assert_eq!(lay_a.gap, 8.0);
    assert_eq!(lay_a.padding, EdgeInsets::all(24.0));

    let st_a = last_style(&cmds_a, nid_a).unwrap();
    let st_b = last_style(&cmds_b, nid_b).unwrap();
    assert_eq!(st_a, st_b);
    assert_eq!(st_a.background, ColorRgba::rgb(0xef, 0x44, 0x44));
    assert_eq!(st_a.border_width, 1.0);
    assert_eq!(st_a.border_radius, 8.0);
    assert!(has_raw_str(&cmds_a, nid_a, "box-shadow", "0 2px 6px #00000040"));
    assert!(has_raw_str(&cmds_b, nid_b, "box-shadow", "0 2px 6px #00000040"));

    // Frame 2: update nve_style (e.g. focused border=2px) while style override stays untouched.
    w_a.begin_frame(1, 2);
    let focused = base.clone().border(2.0, ColorRgba::rgb(0x60, 0xa5, 0xfa));
    w_a.set_attribute(NVE_TYPED_STYLE_ATTR, None, &focused.into_value(), el_a);
    let cmds_a2 = decode_cmds(w_a.finish_frame(true));
    let st_a2 = last_style(&cmds_a2, nid_a).unwrap();
    // Focused border applies while user override background & padding stay intact.
    assert_eq!(st_a2.border_width, 2.0);
    assert_eq!(st_a2.border_color, ColorRgba::rgb(0x60, 0xa5, 0xfa));
    assert_eq!(st_a2.background, ColorRgba::rgb(0xef, 0x44, 0x44));
    assert_eq!(w_a.nodes[&nid_a].layout.padding, EdgeInsets::all(24.0));

    // Frame 3: remove inline style override (`AttributeValue::None`) -> cleanly restores base TypedStyle!
    w_a.begin_frame(1, 3);
    w_a.set_attribute("style", None, &AttributeValue::None, el_a);
    let cmds_a3 = decode_cmds(w_a.finish_frame(true));
    assert_eq!(last_layout(&cmds_a3, nid_a).unwrap().padding, EdgeInsets::all(12.0));
    assert_eq!(
        last_style(&cmds_a3, nid_a).unwrap().background,
        ColorRgba::rgb(0x25, 0x63, 0xeb)
    );
    assert!(has_raw_str(&cmds_a3, nid_a, "box-shadow", "none"));
    assert!(!cmds_a3.iter().any(|c| matches!(
        c,
        Command::SetRawProp { key, .. } if key == "style"
    )));
}

#[test]
fn typed_style_sanitizes_non_finite_values_and_none_style_is_noop() {
    let ts = TypedStyle::new()
        .width_px(f32::NAN)
        .height_pct(150.0)
        .gap(-5.0)
        .opacity(2.0);
    assert_eq!(ts.layout.width, Dimension::Points(0.0));
    assert_eq!(ts.layout.height, Dimension::Percent(100.0));
    assert_eq!(ts.layout.gap, 0.0);
    assert_eq!(ts.style.opacity, 1.0);

    let mut writer = NveMutationWriter::new(1);
    writer.begin_frame(1, 1);
    let nid = writer.alloc_node(ComponentKind::View, None);
    let el = ElementId(1);
    writer.bind_element_id(nid, el);
    writer.set_attribute(NVE_TYPED_STYLE_ATTR, None, &ts.into_value(), el);
    writer.set_attribute("style", None, &AttributeValue::None, el);
    let cmds = decode_cmds(writer.finish_frame(true));
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::SetRawProp { key, .. } if key == "style" || key == NVE_TYPED_STYLE_ATTR
    )));
}
