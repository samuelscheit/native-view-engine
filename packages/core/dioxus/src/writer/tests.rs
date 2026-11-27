//! Unit tests for `NveMutationWriter` & `MutationTx`: string CSS forwarding, direct `TypedStyle`
//! parity, `nve_style` + `style` override merging (order-independent), inherited text-prop
//! updates, and direct `RenderSink` transition delivery.

use dioxus_core::{AttributeValue, ElementId, IntoAttributeValue, WriteMutations};
use nve_abi::{
    ColorRgba, Command, CommandDecoder, ComponentKind, Dimension, EdgeInsets, FlexAlign,
    FlexDirection, FlexJustify, LayoutProps, NodeId, RawPropValue, StyleProps,
};
use nve_components::motion::{EasingCurve, TransitionProperty, TransitionSpec};
use nve_engine::{BackendApplyStats, BackendError, BatchSink, RenderSink};

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

#[derive(Default)]
struct TransitionSink {
    transitions: Vec<(NodeId, Vec<TransitionSpec>)>,
}

impl RenderSink for TransitionSink {
    fn begin_frame(&mut self, _: nve_abi::SurfaceId, _: nve_abi::FrameId) {
        self.transitions.clear();
    }
    fn create_node(&mut self, _: NodeId, _: ComponentKind, _: Option<&str>) {}
    fn remove_node(&mut self, _: NodeId) {}
    fn set_children(&mut self, _: NodeId, _: &[NodeId]) {}
    fn set_text(&mut self, _: NodeId, _: &str) {}
    fn set_layout(&mut self, _: NodeId, _: &LayoutProps) {}
    fn set_style(&mut self, _: NodeId, _: &StyleProps) {}
    fn set_raw_prop(&mut self, _: NodeId, _: &str, _: nve_abi::RawPropValueRef<'_>) {}
    fn set_event_mask(&mut self, _: NodeId, _: u64) {}
    fn dispatch_command(&mut self, _: NodeId, _: &str, _: &[u8]) {}
    fn set_transitions(&mut self, node_id: NodeId, transitions: &[TransitionSpec]) {
        self.transitions.push((node_id, transitions.to_vec()));
    }
    fn commit_frame(&mut self, _: bool) -> Result<BackendApplyStats, BackendError> {
        Ok(BackendApplyStats::default())
    }
}

#[test]
fn inline_style_forwards_font_weight_text_align_box_shadow_and_inherits_text_props() {
    let mut writer = NveMutationWriter::new(1);
    let mut sink = BatchSink::new(1);
    let tx = writer.begin_tx(&mut sink, 1, 1);

    let card_id = tx.writer.alloc_node(tx.sink, ComponentKind::View, None);
    tx.writer.apply_str_attr(
        tx.sink,
        card_id,
        "style",
        "background-color: #1e293b; border-radius: 10px; box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35);",
    );

    let parent_text = tx.writer.alloc_node(tx.sink, ComponentKind::Text, None);
    tx.writer.apply_str_attr(
        tx.sink,
        parent_text,
        "style",
        "font-size: 18px; font-weight: 700; text-align: center; color: #ffffff;",
    );
    let leaf_text = tx.writer.alloc_node(tx.sink, ComponentKind::Text, None);
    tx.writer.record_text(leaf_text, "Title");
    tx.sink.set_text(leaf_text, "Title");

    tx.writer.nodes.get_mut(&parent_text).unwrap().children.push(leaf_text);
    tx.writer.nodes.get_mut(&leaf_text).unwrap().set_parent(Some(parent_text));
    tx.writer.dirty_children.push(parent_text);

    tx.finish_frame(true).unwrap();
    let cmds = decode_cmds(sink.bytes());

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
    let mut str_sink = BatchSink::new(1);
    let str_tx = str_writer.begin_tx(&mut str_sink, 1, 1);
    let s_card = str_tx.writer.alloc_node(str_tx.sink, ComponentKind::View, None);
    str_tx.writer.apply_str_attr(
        str_tx.sink,
        s_card,
        "style",
        "width: 100%; height: 48px; min-width: 120px; max-width: 560px; flex-direction: row; justify-content: space-between; align-items: center; flex-grow: 1; flex-shrink: 0; gap: 8px; padding-horizontal: 16px; padding-vertical: 10px; margin: 4px; background-color: #1e293b; border-width: 2px; border-color: #60a5fa; border-radius: 10px; opacity: 0.85; z-index: 900; box-shadow: 0 4px 16px rgba(0, 0, 0, 0.45); overflow: hidden;",
    );
    let s_parent = str_tx.writer.alloc_node(str_tx.sink, ComponentKind::Text, None);
    str_tx.writer.apply_str_attr(
        str_tx.sink,
        s_parent,
        "style",
        "color: #f8fafc; font-size: 18px; font-weight: 700; text-align: center; opacity: 0.9;",
    );
    let s_leaf = str_tx.writer.alloc_node(str_tx.sink, ComponentKind::Text, None);
    str_tx.writer.record_text(s_leaf, "Heading");
    str_tx.sink.set_text(s_leaf, "Heading");
    str_tx.writer.nodes.get_mut(&s_parent).unwrap().children.push(s_leaf);
    str_tx.writer.nodes.get_mut(&s_leaf).unwrap().set_parent(Some(s_parent));
    str_tx.writer.dirty_children.push(s_parent);
    str_tx.finish_frame(true).unwrap();
    let str_cmds = decode_cmds(str_sink.bytes());

    // 2. Render via direct `TypedStyle` (zero CSS string formatting/parsing).
    let mut typed_writer = NveMutationWriter::new(1);
    let mut typed_sink = BatchSink::new(1);
    let mut typed_tx = typed_writer.begin_tx(&mut typed_sink, 1, 1);
    let t_card = typed_tx.writer.alloc_node(typed_tx.sink, ComponentKind::View, None);
    let card_el = ElementId(10);
    typed_tx.writer.bind_element_id(t_card, card_el);
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
    typed_tx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &card_attr_val, card_el);

    let t_parent = typed_tx.writer.alloc_node(typed_tx.sink, ComponentKind::Text, None);
    let parent_el = ElementId(11);
    typed_tx.writer.bind_element_id(t_parent, parent_el);
    let text_typed = TypedStyle::new()
        .color(ColorRgba::rgb(0xf8, 0xfa, 0xfc))
        .font_size(18.0)
        .font_weight("700")
        .text_align("center")
        .opacity(0.9);
    let text_attr_val = text_typed.into_value();
    typed_tx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &text_attr_val, parent_el);

    let t_leaf = typed_tx.writer.alloc_node(typed_tx.sink, ComponentKind::Text, None);
    typed_tx.writer.record_text(t_leaf, "Heading");
    typed_tx.sink.set_text(t_leaf, "Heading");
    typed_tx.writer.nodes.get_mut(&t_parent).unwrap().children.push(t_leaf);
    typed_tx.writer.nodes.get_mut(&t_leaf).unwrap().set_parent(Some(t_parent));
    typed_tx.writer.dirty_children.push(t_parent);
    typed_tx.finish_frame(true).unwrap();
    let typed_cmds = decode_cmds(typed_sink.bytes());

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
    let mut typed_tx2 = typed_writer.begin_tx(&mut typed_sink, 1, 2);
    let updated_text = TypedStyle::new()
        .color(ColorRgba::rgb(0x94, 0xa3, 0xb8))
        .font_size(14.0)
        .font_weight("400")
        .text_align("right")
        .opacity(1.0);
    let updated_val = updated_text.into_value();
    typed_tx2.set_attribute(NVE_TYPED_STYLE_ATTR, None, &updated_val, parent_el);
    typed_tx2.finish_frame(true).unwrap();
    let cmds2 = decode_cmds(typed_sink.bytes());

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
    let mut s_a = BatchSink::new(1);
    let mut tx_a = w_a.begin_tx(&mut s_a, 1, 1);
    let nid_a = tx_a.writer.alloc_node(tx_a.sink, ComponentKind::View, None);
    let el_a = ElementId(1);
    tx_a.writer.bind_element_id(nid_a, el_a);
    tx_a.set_attribute(NVE_TYPED_STYLE_ATTR, None, &base.clone().into_value(), el_a);
    tx_a.set_attribute("style", None, &AttributeValue::Text(override_css.into()), el_a);
    tx_a.finish_frame(true).unwrap();
    let cmds_a = decode_cmds(s_a.bytes());

    // Order B: style first, nve_style second.
    let mut w_b = NveMutationWriter::new(1);
    let mut s_b = BatchSink::new(1);
    let mut tx_b = w_b.begin_tx(&mut s_b, 1, 1);
    let nid_b = tx_b.writer.alloc_node(tx_b.sink, ComponentKind::View, None);
    let el_b = ElementId(1);
    tx_b.writer.bind_element_id(nid_b, el_b);
    tx_b.set_attribute("style", None, &AttributeValue::Text(override_css.into()), el_b);
    tx_b.set_attribute(NVE_TYPED_STYLE_ATTR, None, &base.clone().into_value(), el_b);
    tx_b.finish_frame(true).unwrap();
    let cmds_b = decode_cmds(s_b.bytes());

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
    let mut tx_a2 = w_a.begin_tx(&mut s_a, 1, 2);
    let focused = base.clone().border(2.0, ColorRgba::rgb(0x60, 0xa5, 0xfa));
    tx_a2.set_attribute(NVE_TYPED_STYLE_ATTR, None, &focused.into_value(), el_a);
    tx_a2.finish_frame(true).unwrap();
    let cmds_a2 = decode_cmds(s_a.bytes());
    let st_a2 = last_style(&cmds_a2, nid_a).unwrap();
    assert_eq!(st_a2.border_width, 2.0);
    assert_eq!(st_a2.border_color, ColorRgba::rgb(0x60, 0xa5, 0xfa));
    assert_eq!(st_a2.background, ColorRgba::rgb(0xef, 0x44, 0x44));
    assert_eq!(w_a.nodes[&nid_a].layout.padding, EdgeInsets::all(24.0));

    // Frame 3: remove inline style override (`AttributeValue::None`) -> cleanly restores base TypedStyle!
    let mut tx_a3 = w_a.begin_tx(&mut s_a, 1, 3);
    tx_a3.set_attribute("style", None, &AttributeValue::None, el_a);
    tx_a3.finish_frame(true).unwrap();
    let cmds_a3 = decode_cmds(s_a.bytes());
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
    let mut sink = BatchSink::new(1);
    let mut tx = writer.begin_tx(&mut sink, 1, 1);
    let nid = tx.writer.alloc_node(tx.sink, ComponentKind::View, None);
    let el = ElementId(1);
    tx.writer.bind_element_id(nid, el);
    tx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &ts.into_value(), el);
    tx.set_attribute("style", None, &AttributeValue::None, el);
    tx.finish_frame(true).unwrap();
    let cmds = decode_cmds(sink.bytes());
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::SetRawProp { key, .. } if key == "style" || key == NVE_TYPED_STYLE_ATTR
    )));
}

#[test]
fn typed_style_streams_zero_serialization_transitions_into_render_sink() {
    let t1 = TransitionSpec::new(TransitionProperty::BackgroundColor, 150)
        .easing(EasingCurve::EaseOut);
    let t2 = TransitionSpec::new(TransitionProperty::Opacity, 220)
        .easing(EasingCurve::EaseInOut)
        .delay_ms(20);
    let ts = TypedStyle::new()
        .bg(ColorRgba::rgb(30, 41, 59))
        .transition(t1)
        .transitions(&[t2]);

    let mut writer = NveMutationWriter::new(1);
    let mut sink = TransitionSink::default();
    let mut tx = writer.begin_tx(&mut sink, 1, 1);
    let nid = tx.writer.alloc_node(tx.sink, ComponentKind::Pressable, None);
    let el = ElementId(1);
    tx.writer.bind_element_id(nid, el);
    tx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &ts.into_value(), el);
    tx.finish_frame(true).unwrap();

    assert_eq!(writer.node_transitions(nid), &[t1, t2]);
    assert_eq!(sink.transitions, vec![(nid, vec![t1, t2])]);

    // Clearing TypedStyle emits empty transitions slice directly into RenderSink.
    let mut tx2 = writer.begin_tx(&mut sink, 1, 2);
    tx2.set_attribute(NVE_TYPED_STYLE_ATTR, None, &AttributeValue::None, el);
    tx2.finish_frame(true).unwrap();
    assert!(writer.node_transitions(nid).is_empty());
    assert_eq!(sink.transitions, vec![(nid, vec![])]);
}

#[test]
fn node_record_size_budget() {
    use core::mem::size_of;
    let sz = size_of::<super::NodeRecord>();
    println!(
        "NodeRecord={sz} LayoutProps={} StyleProps={}",
        size_of::<LayoutProps>(),
        size_of::<StyleProps>(),
    );
    assert!(sz <= 200, "NodeRecord grew to {sz} bytes (budget <= 200)");
}

#[test]
fn dioxus_surface_new_with_props_and_new_with_context_render_cleanly() {
    use crate::prelude::*;
    use crate::DioxusSurface;
    use dioxus_core::consume_context;

    #[derive(Clone, PartialEq)]
    struct GreetingProps {
        title: String,
    }

    fn props_root(props: GreetingProps) -> Element {
        rsx! {
            text { "{props.title}" }
        }
    }

    #[derive(Clone)]
    struct ThemeLabel(&'static str);

    fn context_root() -> Element {
        let label = consume_context::<ThemeLabel>();
        rsx! {
            text { "{label.0}" }
        }
    }

    let mut props_surface = DioxusSurface::new_with_props(
        1,
        props_root,
        GreetingProps {
            title: "typed-props-ok".into(),
        },
    );
    let (_, _, f1) = props_surface.render_frame(1);
    let cmds1 = decode_cmds(&f1);
    assert!(cmds1
        .iter()
        .any(|c| matches!(c, Command::SetText { text, .. } if text == "typed-props-ok")));

    let mut ctx_surface =
        DioxusSurface::new_with_context(2, context_root, ThemeLabel("typed-ctx-ok"));
    let (_, _, f2) = ctx_surface.render_frame(1);
    let cmds2 = decode_cmds(&f2);
    assert!(cmds2
        .iter()
        .any(|c| matches!(c, Command::SetText { text, .. } if text == "typed-ctx-ok")));
}

#[test]
fn plain_standalone_typed_attrs_allocate_zero_node_extras_and_current_color_reparents_and_cleans_up() {
    let mut writer = NveMutationWriter::new(1);
    let mut sink = BatchSink::new(1);

    // 1. Mount 200 plain nodes with standalone typed attrs: ZERO `NodeExtras` allocations!
    let tx = writer.begin_tx(&mut sink, 1, 1);
    let mut plain_ids = Vec::with_capacity(200);
    for _ in 0..200 {
        let nid = tx.writer.alloc_node(tx.sink, ComponentKind::View, None);
        tx.writer.apply_str_attr(tx.sink, nid, "width", "100px");
        tx.writer.apply_str_attr(tx.sink, nid, "padding", "8px");
        tx.writer.apply_str_attr(tx.sink, nid, "height", "40px");
        plain_ids.push(nid);
    }
    tx.finish_frame(true).unwrap();
    for nid in plain_ids {
        let rec = &writer.nodes[&nid];
        assert!(rec.extras.is_none(), "plain typed-attr node must not allocate Box<NodeExtras>");
        assert_eq!(rec.layout.width, Dimension::Points(100.0));
        assert_eq!(rec.layout.padding, EdgeInsets::all(8.0));
        assert_eq!(rec.layout.height, Dimension::Points(40.0));
    }

    // 2. Reparent a `currentColor` border node between red and blue parents, then remove it.
    let tx2 = writer.begin_tx(&mut sink, 1, 2);
    let red_parent = tx2.writer.alloc_node(tx2.sink, ComponentKind::View, None);
    tx2.writer.apply_str_attr(tx2.sink, red_parent, "color", "#ef4444");
    let blue_parent = tx2.writer.alloc_node(tx2.sink, ComponentKind::View, None);
    tx2.writer.apply_str_attr(tx2.sink, blue_parent, "color", "#3b82f6");
    let child = tx2.writer.alloc_node(tx2.sink, ComponentKind::View, None);
    tx2.writer.apply_str_attr(tx2.sink, child, "border", "1px solid");
    tx2.writer.nodes.get_mut(&red_parent).unwrap().children.push(child);
    tx2.writer.nodes.get_mut(&child).unwrap().set_parent(Some(red_parent));
    tx2.writer.dirty_children.push(red_parent);
    tx2.finish_frame(true).unwrap();
    assert_eq!(writer.current_color_borders, vec![child]);
    assert_eq!(writer.nodes[&child].style.border_color, ColorRgba::rgb(0xef, 0x44, 0x44));

    // Reparent `child` under `blue_parent` without touching any style attributes.
    let tx3 = writer.begin_tx(&mut sink, 1, 3);
    tx3.writer.detach_from_parent(child);
    tx3.writer.nodes.get_mut(&blue_parent).unwrap().children.push(child);
    tx3.writer.nodes.get_mut(&child).unwrap().set_parent(Some(blue_parent));
    tx3.writer.dirty_children.push(blue_parent);
    tx3.finish_frame(true).unwrap();
    assert_eq!(writer.nodes[&child].style.border_color, ColorRgba::rgb(0x3b, 0x82, 0xf6));

    // Remove `blue_parent` subtree -> `current_color_borders` must drop to empty!
    let tx4 = writer.begin_tx(&mut sink, 1, 4);
    tx4.writer.remove_subtree(tx4.sink, blue_parent);
    tx4.finish_frame(true).unwrap();
    assert!(
        writer.current_color_borders.is_empty(),
        "removing a currentColor subtree must remove its entries from current_color_borders"
    );

    // 3. Direct-path removal guard: removing unset overlapping longhands/shorthands must NOT
    // clobber active fields or allocate `NodeExtras`, while removing active Color/Border/Flex
    // resets cleanly.
    let tx5 = writer.begin_tx(&mut sink, 1, 5);
    let pad_node = tx5.writer.alloc_node(tx5.sink, ComponentKind::View, None);
    let pad_el = ElementId(50);
    tx5.writer.bind_element_id(pad_node, pad_el);
    tx5.writer.apply_str_attr(tx5.sink, pad_node, "padding", "8px");
    let top_node = tx5.writer.alloc_node(tx5.sink, ComponentKind::View, None);
    let top_el = ElementId(51);
    tx5.writer.bind_element_id(top_node, top_el);
    tx5.writer.apply_str_attr(tx5.sink, top_node, "padding-top", "4px");
    let cbf_node = tx5.writer.alloc_node(tx5.sink, ComponentKind::View, None);
    let cbf_el = ElementId(52);
    tx5.writer.bind_element_id(cbf_node, cbf_el);
    tx5.writer.apply_str_attr(tx5.sink, cbf_node, "color", "#ef4444");
    tx5.writer.apply_str_attr(tx5.sink, cbf_node, "border", "2px solid");
    tx5.writer.apply_str_attr(tx5.sink, cbf_node, "flex", "2 0 120px");
    tx5.finish_frame(true).unwrap();
    assert!(writer.nodes[&pad_node].extras.is_none());
    assert!(writer.nodes[&top_node].extras.is_none());
    assert_eq!(writer.current_color_borders, vec![cbf_node]);

    let mut tx6 = writer.begin_tx(&mut sink, 1, 6);
    tx6.set_attribute("padding-left", None, &AttributeValue::None, pad_el);
    tx6.set_attribute("padding-horizontal", None, &AttributeValue::None, pad_el);
    tx6.set_attribute("padding", None, &AttributeValue::None, top_el);
    tx6.set_attribute("color", None, &AttributeValue::None, cbf_el);
    tx6.set_attribute("border", None, &AttributeValue::None, cbf_el);
    tx6.set_attribute("flex", None, &AttributeValue::None, cbf_el);
    tx6.finish_frame(true).unwrap();
    assert_eq!(writer.nodes[&pad_node].layout.padding, EdgeInsets::all(8.0));
    assert!(writer.nodes[&pad_node].extras.is_none());
    assert_eq!(writer.nodes[&top_node].layout.padding.top, 4.0);
    assert!(writer.nodes[&top_node].extras.is_none());
    assert_eq!(writer.nodes[&cbf_node].style.text_color, StyleProps::default().text_color);
    assert!(!writer.nodes[&cbf_node].has_explicit_text_color());
    assert_eq!(writer.nodes[&cbf_node].style.border_width, 0.0);
    assert!(writer.current_color_borders.is_empty());
    assert_eq!(
        (writer.nodes[&cbf_node].layout.flex_grow, writer.nodes[&cbf_node].layout.flex_shrink),
        (0.0, 1.0)
    );
    assert!(writer.node_layout_ext(cbf_node).is_none());

    // 4. Abandoned `MutationTx`: dropping a tx with pending `dirty_compose` entries must NOT leave
    // stale `FLAG_DIRTY_COMPOSE` set on the next `begin_tx`, or future style updates are skipped.
    let abort_node = {
        let tx7 = writer.begin_tx(&mut sink, 1, 7);
        let nid = tx7.writer.alloc_node(tx7.sink, ComponentKind::View, None);
        tx7.writer.bind_element_id(nid, ElementId(53));
        tx7.writer.apply_str_attr(tx7.sink, nid, "style", "width: 100px");
        assert!(tx7.writer.nodes[&nid].is_dirty_compose());
        nid
    };

    let tx8 = writer.begin_tx(&mut sink, 1, 8);
    assert!(!tx8.writer.nodes[&abort_node].is_dirty_compose());
    tx8.writer.apply_str_attr(tx8.sink, abort_node, "style", "width: 200px");
    tx8.finish_frame(true).unwrap();
    assert_eq!(writer.nodes[&abort_node].layout.width, Dimension::Points(200.0));
    assert!(!writer.nodes[&abort_node].is_dirty_compose());
}
