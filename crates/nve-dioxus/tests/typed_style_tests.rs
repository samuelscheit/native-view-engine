//! End-to-end `DioxusSurface` integration tests for zero-string-parse `TypedStyle` (`nve_style`),
//! deterministic `nve_style` + `style: Option<String>` merging across re-renders, and multi-level
//! `Text` style/raw-prop inheritance.

use std::collections::HashMap;

use nve_abi::{
    ColorRgba, Command, CommandDecoder, ComponentKind, EdgeInsets, EventPayload, FlexAlign,
    FlexDirection, FlexJustify, LayoutProps, NativeEvent, NativeEventKind, NodeId, RawPropValue,
    StyleProps, ROOT_NODE_ID,
};
use nve_dioxus::prelude::*;
use nve_dioxus::{DioxusSurface, NVE_TYPED_STYLE_ATTR};

#[derive(Debug, Clone)]
struct MirrorNode {
    #[allow(dead_code)]
    kind: ComponentKind,
    text: Option<String>,
    layout: LayoutProps,
    style: StyleProps,
    raw_props: HashMap<String, RawPropValue>,
}

struct WireMirror {
    nodes: HashMap<NodeId, MirrorNode>,
}

impl WireMirror {
    fn new() -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(
            ROOT_NODE_ID,
            MirrorNode {
                kind: ComponentKind::Root,
                text: None,
                layout: LayoutProps::default(),
                style: StyleProps::default(),
                raw_props: HashMap::new(),
            },
        );
        Self { nodes }
    }

    fn apply(&mut self, bytes: &[u8]) -> Vec<Command> {
        let (_, cmds) = CommandDecoder::new(bytes)
            .expect("valid header")
            .decode_all()
            .expect("valid commands");
        for cmd in &cmds {
            match cmd {
                Command::CreateNode { node_id, kind, .. } => {
                    self.nodes.insert(
                        *node_id,
                        MirrorNode {
                            kind: *kind,
                            text: None,
                            layout: LayoutProps::default(),
                            style: StyleProps::default(),
                            raw_props: HashMap::new(),
                        },
                    );
                }
                Command::RemoveNode { node_id } => {
                    self.nodes.remove(node_id);
                }
                Command::SetText { node_id, text } => {
                    if let Some(n) = self.nodes.get_mut(node_id) {
                        n.text = Some(text.clone());
                    }
                }
                Command::SetLayout { node_id, layout } => {
                    if let Some(n) = self.nodes.get_mut(node_id) {
                        n.layout = *layout;
                    }
                }
                Command::SetStyle { node_id, style } => {
                    if let Some(n) = self.nodes.get_mut(node_id) {
                        n.style = *style;
                    }
                }
                Command::SetRawProp {
                    node_id,
                    key,
                    value,
                } => {
                    if let Some(n) = self.nodes.get_mut(node_id) {
                        n.raw_props.insert(key.clone(), value.clone());
                    }
                }
                _ => {}
            }
        }
        cmds
    }

    fn by_test_id(&self, tid: &str) -> (NodeId, &MirrorNode) {
        self.nodes
            .iter()
            .find(|(_, n)| {
                matches!(n.raw_props.get("testID"), Some(RawPropValue::String(s)) if s == tid)
            })
            .map(|(&id, n)| (id, n))
            .unwrap_or_else(|| panic!("missing testID='{tid}'"))
    }

    fn by_text(&self, needle: &str) -> (NodeId, &MirrorNode) {
        self.nodes
            .iter()
            .find(|(_, n)| n.text.as_deref() == Some(needle))
            .map(|(&id, n)| (id, n))
            .unwrap_or_else(|| panic!("missing text='{needle}'"))
    }

    fn raw_str(&self, id: NodeId, key: &str) -> Option<&str> {
        match self.nodes.get(&id)?.raw_props.get(key) {
            Some(RawPropValue::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }
}

#[component]
fn ParityApp() -> Element {
    let card_typed = TypedStyle::new()
        .width_pct(100.0)
        .height_px(64.0)
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Center)
        .gap(12.0)
        .padding_xy(16.0, 8.0)
        .margin(4.0)
        .bg(ColorRgba::rgb(0x1e, 0x29, 0x3b))
        .border(2.0, ColorRgba::rgb(0x60, 0xa5, 0xfa))
        .radius(12.0)
        .opacity(0.9)
        .z_index(100)
        .box_shadow("0 4px 12px rgba(0, 0, 0, 0.35)")
        .overflow("hidden");

    let text_typed = TypedStyle::new()
        .color(ColorRgba::rgb(0xf8, 0xfa, 0xfc))
        .font_size(18.0)
        .font_weight("700")
        .text_align("center")
        .opacity(0.95);

    rsx! {
        view {
            view {
                test_id: "str-card",
                style: "width: 100%; height: 64px; flex-direction: row; justify-content: space-between; align-items: center; gap: 12px; padding-horizontal: 16px; padding-vertical: 8px; margin: 4px; background-color: #1e293b; border-width: 2px; border-color: #60a5fa; border-radius: 12px; opacity: 0.9; z-index: 100; box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35); overflow: hidden;",
                text {
                    test_id: "str-text",
                    style: "color: #f8fafc; font-size: 18px; font-weight: 700; text-align: center; opacity: 0.95;",
                    "StrLeaf"
                }
            }
            view {
                test_id: "typed-card",
                nve_style: card_typed,
                text {
                    test_id: "typed-text",
                    nve_style: text_typed,
                    "TypedLeaf"
                }
            }
        }
    }
}

#[test]
fn rsx_typed_style_matches_css_string_parity_across_layout_style_raw_and_leaf_inheritance() {
    let mut surface = DioxusSurface::new(1, ParityApp);
    let mut mirror = WireMirror::new();

    let (_, _, frame) = surface.render_frame(1);
    let cmds = mirror.apply(&frame);

    let (str_card_id, str_card) = mirror.by_test_id("str-card");
    let (typed_card_id, typed_card) = mirror.by_test_id("typed-card");
    assert_eq!(typed_card.layout, str_card.layout);
    assert_eq!(typed_card.style, str_card.style);
    assert_eq!(
        mirror.raw_str(typed_card_id, "box-shadow"),
        mirror.raw_str(str_card_id, "box-shadow")
    );
    assert_eq!(
        mirror.raw_str(typed_card_id, "overflow"),
        mirror.raw_str(str_card_id, "overflow")
    );

    let (str_leaf_id, str_leaf) = mirror.by_text("StrLeaf");
    let (typed_leaf_id, typed_leaf) = mirror.by_text("TypedLeaf");
    assert_eq!(typed_leaf.style, str_leaf.style);
    assert_eq!(
        mirror.raw_str(typed_leaf_id, "font-weight"),
        mirror.raw_str(str_leaf_id, "font-weight")
    );
    assert_eq!(
        mirror.raw_str(typed_leaf_id, "text-align"),
        mirror.raw_str(str_leaf_id, "text-align")
    );

    // Ensure `nve_style` is never leaked as a raw prop on the wire.
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::SetRawProp { key, .. } if key == NVE_TYPED_STYLE_ATTR
    )));
}

#[component]
fn DynamicMergeAndInheritanceApp() -> Element {
    let mut focused = use_signal(|| false);
    let mut override_mode = use_signal(|| 1u8); // 1 = pad+bg, 2 = radius only, 0 = None
    let mut bold = use_signal(|| true);

    let box_style = TypedStyle::new()
        .flex_row()
        .gap(10.0)
        .padding(12.0)
        .bg(ColorRgba::rgb(0x25, 0x63, 0xeb))
        .border(
            if focused() { 2.0 } else { 1.0 },
            if focused() {
                ColorRgba::rgb(0x60, 0xa5, 0xfa)
            } else {
                ColorRgba::rgb(0xcb, 0xd5, 0xe1)
            },
        )
        .radius(8.0)
        .opacity(if focused() { 0.8 } else { 1.0 })
        .box_shadow("none");

    let extra_style: Option<String> = match override_mode() {
        1 => Some("padding: 24px; background-color: #ef4444;".into()),
        2 => Some("border-radius: 20px;".into()),
        _ => None,
    };

    let heading_style = TypedStyle::new()
        .color(if bold() {
            ColorRgba::rgb(0xff, 0xff, 0xff)
        } else {
            ColorRgba::rgb(0x94, 0xa3, 0xb8)
        })
        .font_size(if bold() { 20.0 } else { 14.0 })
        .font_weight(if bold() { "700" } else { "400" })
        .text_align(if bold() { "center" } else { "left" });

    rsx! {
        view {
            view {
                test_id: "merge-target",
                nve_style: box_style,
                style: extra_style,
                text {
                    nve_style: heading_style,
                    "InheritedHeading"
                }
            }
            pressable {
                test_id: "toggle-focus",
                onpress: move |_| focused.toggle(),
            }
            pressable {
                test_id: "cycle-override",
                onpress: move |_| override_mode.set((override_mode() + 1) % 3),
            }
            pressable {
                test_id: "toggle-bold",
                onpress: move |_| bold.toggle(),
            }
        }
    }
}

fn press_btn(surface: &mut DioxusSurface, mirror: &mut WireMirror, tid: &str, frame_id: u64) -> Vec<Command> {
    let (nid, _) = mirror.by_test_id(tid);
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: nid,
        timestamp_us: frame_id * 1000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 2.0, y: 2.0 },
    }]));
    let (_, _, bytes) = surface.render_frame(frame_id);
    mirror.apply(&bytes)
}

#[test]
fn rsx_typed_style_merges_user_style_override_and_updates_inherited_text_cleanly() {
    let mut surface = DioxusSurface::new(1, DynamicMergeAndInheritanceApp);
    let mut mirror = WireMirror::new();

    let (_, _, f1) = surface.render_frame(1);
    mirror.apply(&f1);

    // Frame 1: user override wins on padding & background; TypedStyle wins on gap, radius, border.
    let (_, target) = mirror.by_test_id("merge-target");
    assert_eq!(target.layout.flex_direction, FlexDirection::Row);
    assert_eq!(target.layout.gap, 10.0);
    assert_eq!(target.layout.padding, EdgeInsets::all(24.0));
    assert_eq!(target.style.background, ColorRgba::rgb(0xef, 0x44, 0x44));
    assert_eq!(target.style.border_width, 1.0);
    assert_eq!(target.style.border_radius, 8.0);

    let (leaf_id, leaf) = mirror.by_text("InheritedHeading");
    assert_eq!(leaf.style.text_color, ColorRgba::WHITE);
    assert_eq!(leaf.style.font_size, 20.0);
    assert_eq!(mirror.raw_str(leaf_id, "font-weight"), Some("700"));
    assert_eq!(mirror.raw_str(leaf_id, "text-align"), Some("center"));

    // Frame 2: toggle focus -> TypedStyle border/opacity update while user override persists!
    press_btn(&mut surface, &mut mirror, "toggle-focus", 2);
    let (_, target) = mirror.by_test_id("merge-target");
    assert_eq!(target.style.border_width, 2.0);
    assert_eq!(target.style.border_color, ColorRgba::rgb(0x60, 0xa5, 0xfa));
    assert!((target.style.opacity - 0.8).abs() < 1e-4);
    assert_eq!(target.layout.padding, EdgeInsets::all(24.0));
    assert_eq!(target.style.background, ColorRgba::rgb(0xef, 0x44, 0x44));

    // Frame 3: change override to radius-only -> padding & background restore to TypedStyle!
    press_btn(&mut surface, &mut mirror, "cycle-override", 3);
    let (_, target) = mirror.by_test_id("merge-target");
    assert_eq!(target.layout.padding, EdgeInsets::all(12.0));
    assert_eq!(target.style.background, ColorRgba::rgb(0x25, 0x63, 0xeb));
    assert_eq!(target.style.border_radius, 20.0);

    // Frame 4: remove override (`None`) -> border_radius restores to TypedStyle (8.0) with no bogus raw prop!
    let cmds4 = press_btn(&mut surface, &mut mirror, "cycle-override", 4);
    let (_, target) = mirror.by_test_id("merge-target");
    assert_eq!(target.style.border_radius, 8.0);
    assert!(!cmds4.iter().any(|c| matches!(
        c,
        Command::SetRawProp { key, .. } if key == "style"
    )));

    // Frame 5: toggle parent text style -> leaf text inherits updated color, font_size, font_weight, text_align!
    press_btn(&mut surface, &mut mirror, "toggle-bold", 5);
    let (leaf_id, leaf) = mirror.by_text("InheritedHeading");
    assert_eq!(leaf.style.text_color, ColorRgba::rgb(0x94, 0xa3, 0xb8));
    assert_eq!(leaf.style.font_size, 14.0);
    assert_eq!(mirror.raw_str(leaf_id, "font-weight"), Some("400"));
    assert_eq!(mirror.raw_str(leaf_id, "text-align"), Some("left"));
}
