//! `flex-basis` has one typed source of truth, `LayoutExt::flex_basis`: inline `style` (`flex`
//! shorthand or `flex-basis`) and `flex` / `flex-basis` attributes set it, an `nve_style` basis is
//! the fallback, and changing or removing the inline declaration resets it, with zero `flex-basis`
//! raw props on the wire.

use std::collections::HashMap;

use nve_abi::{
    ComponentKind, Dimension, EventPayload, FrameId, LayoutProps, NativeEvent, NativeEventKind,
    NodeId, RawPropValueRef, StyleProps, SurfaceId,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{BackendApplyStats, BackendError, RenderSink};

/// Mirrors the last flushed `LayoutExt` (`None` when cleared), layout, style, and raw prop
/// values (`Some(s)` for String, `None` for Null) per node, plus per-frame write counters.
#[derive(Default)]
struct ExtSink {
    layout_ext: HashMap<NodeId, Option<LayoutExt>>,
    layout: HashMap<NodeId, LayoutProps>,
    style: HashMap<NodeId, StyleProps>,
    test_ids: HashMap<String, NodeId>,
    raw_keys: Vec<String>,
    raw_props: HashMap<NodeId, HashMap<String, Option<String>>>,
    frame_raw_writes: usize,
    frame_style_writes: HashMap<NodeId, usize>,
}

impl RenderSink for ExtSink {
    fn begin_frame(&mut self, _: SurfaceId, _: FrameId) {
        self.frame_raw_writes = 0;
        self.frame_style_writes.clear();
    }
    fn create_node(&mut self, _: NodeId, _: ComponentKind, _: Option<&str>) {}
    fn remove_node(&mut self, _: NodeId) {}
    fn set_children(&mut self, _: NodeId, _: &[NodeId]) {}
    fn set_text(&mut self, _: NodeId, _: &str) {}
    fn set_layout(&mut self, node_id: NodeId, layout: &LayoutProps) {
        self.layout.insert(node_id, *layout);
    }
    fn set_style(&mut self, node_id: NodeId, style: &StyleProps) {
        self.style.insert(node_id, *style);
        *self.frame_style_writes.entry(node_id).or_default() += 1;
    }
    fn set_raw_prop(&mut self, node_id: NodeId, key: &str, value: RawPropValueRef<'_>) {
        self.frame_raw_writes += 1;
        let recorded = match value {
            RawPropValueRef::String(s) => {
                if key == "testID" {
                    self.test_ids.insert(s.to_owned(), node_id);
                }
                Some(s.to_owned())
            }
            RawPropValueRef::Null => None,
            _ => Some("<non-string>".to_owned()),
        };
        self.raw_props
            .entry(node_id)
            .or_default()
            .insert(key.to_owned(), recorded);
        self.raw_keys.push(key.to_owned());
    }
    fn set_event_mask(&mut self, _: NodeId, _: u64) {}
    fn dispatch_command(&mut self, _: NodeId, _: &str, _: &[u8]) {}
    fn set_layout_ext(&mut self, node_id: NodeId, ext: Option<&LayoutExt>) {
        self.layout_ext.insert(node_id, ext.cloned());
    }
    fn commit_frame(&mut self, _: bool) -> Result<BackendApplyStats, BackendError> {
        Ok(BackendApplyStats::default())
    }
}

impl ExtSink {
    fn node(&self, tid: &str) -> NodeId {
        self.test_ids[tid]
    }

    /// Flushed basis of `tid` (`auto` when no `LayoutExt` was ever sent or it was cleared).
    fn basis(&self, tid: &str) -> Dimension {
        self.layout_ext
            .get(&self.node(tid))
            .and_then(Option::as_ref)
            .map_or(Dimension::Auto, |e| e.flex_basis)
    }

    fn flex_factors(&self, tid: &str) -> (f32, f32) {
        self.layout
            .get(&self.node(tid))
            .map_or((0.0, 1.0), |l| (l.flex_grow, l.flex_shrink))
    }

    fn style_of(&self, tid: &str) -> StyleProps {
        self.style.get(&self.node(tid)).copied().unwrap_or_default()
    }

    /// Returns `Some(Some(val))` if set, `Some(None)` if explicitly cleared via `Null`, or `None`
    /// if never emitted.
    fn raw_of(&self, tid: &str, key: &str) -> Option<Option<&str>> {
        self.raw_props
            .get(&self.node(tid))
            .and_then(|m| m.get(key))
            .map(|opt| opt.as_deref())
    }
}

/// Inline styles cycled by the `cycle` button: shorthand, `none`, longhand, removal.
const INLINE: [Option<&str>; 5] = [
    Some("flex: 1 1 280px; padding: 4px"),
    Some("flex: none"),
    Some("flex-basis: 12%; flex-grow: 2"),
    Some("padding: 4px"),
    None,
];

#[component]
fn FlexBasisApp() -> Element {
    let mut step = use_signal(|| 0usize);
    let inline = INLINE[step() % INLINE.len()].map(str::to_owned);
    let typed = TypedStyle::new().flex_basis_pct(50.0).aspect_ratio(2.0);
    rsx! {
        view {
            view { test_id: "plain", style: inline.clone() }
            view { test_id: "typed", nve_style: typed, style: inline }
            view { test_id: "attr", flex: if step() % 2 == 0 { "1 0 0" } else { "none" } }
            pressable { test_id: "cycle", onpress: move |_| step += 1 }
        }
    }
}

fn press(surface: &mut DioxusSurface, sink: &mut ExtSink, frame: u64) {
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: sink.node("cycle"),
        timestamp_us: frame * 1000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 1.0, y: 1.0 },
    }]));
    surface.flush_into(frame, sink).unwrap();
}

#[test]
fn inline_flex_basis_is_typed_layout_ext_and_resets_on_change_and_removal() {
    use Dimension::{Auto, Percent as Pct, Points as Px};
    let mut surface = DioxusSurface::new(1, FlexBasisApp);
    let mut sink = ExtSink::default();
    surface.flush_into(1, &mut sink).unwrap();

    // (plain basis, typed basis, attr basis) after each step; typed keeps aspect-ratio 2.
    let expected = [
        (Px(280.0), Px(280.0), Px(0.0)),
        (Auto, Auto, Auto),
        (Pct(12.0), Pct(12.0), Px(0.0)),
        (Auto, Pct(50.0), Auto),
        (Auto, Pct(50.0), Px(0.0)),
        (Px(280.0), Px(280.0), Auto),
    ];
    for (frame, (plain, typed, attr)) in (1u64..).zip(expected) {
        if frame > 1 {
            press(&mut surface, &mut sink, frame);
        }
        assert_eq!(sink.basis("plain"), plain, "plain @ frame {frame}");
        assert_eq!(sink.basis("typed"), typed, "typed @ frame {frame}");
        assert_eq!(sink.basis("attr"), attr, "attr @ frame {frame}");
        for tid in ["plain", "typed", "attr"] {
            let flushed = sink.layout_ext.get(&sink.node(tid)).cloned().flatten();
            assert_eq!(surface.node_layout_ext(sink.node(tid)).cloned(), flushed, "{tid} @ {frame}");
        }
        let typed_ext = surface.node_layout_ext(sink.node("typed")).unwrap();
        assert_eq!(typed_ext.aspect_ratio, Some(2.0), "typed fields survive @ frame {frame}");
        // A plain node with an `auto` basis carries no `LayoutExt` at all.
        assert_eq!(
            surface.node_layout_ext(sink.node("plain")).is_none(),
            plain == Auto,
            "frame {frame}"
        );
    }
    let plain = sink.layout[&sink.node("plain")];
    assert_eq!((plain.flex_grow, plain.flex_shrink), (1.0, 1.0));
    assert!(
        !sink.raw_keys.iter().any(|k| k.contains("flex")),
        "flex-basis never travels as a raw prop: {:?}",
        sink.raw_keys
    );
}

#[component]
fn MixedFlexPrecedenceApp() -> Element {
    let mut step = use_signal(|| 0usize);
    let s = step();
    let typed = TypedStyle::new()
        .flex_grow(3.0)
        .flex_shrink(2.0)
        .flex_basis_pct(50.0)
        .aspect_ratio(2.0);
    let attr_val: Option<String> = match s {
        0..=2 => Some("1 0 200px".to_owned()),
        _ => None,
    };
    let inline_val: Option<String> = match s {
        0 => Some("padding: 4px".to_owned()),
        1 | 3 => Some("flex-basis: 33%".to_owned()),
        2 => None,
        _ => Some("padding: 8px".to_owned()),
    };
    rsx! {
        view {
            // Opposite attribute declaration orders must produce identical basis + grow/shrink.
            view { test_id: "attr_first", flex: attr_val.clone(), style: inline_val.clone() }
            view { test_id: "style_first", style: inline_val.clone(), flex: attr_val.clone() }
            // Typed + standalone attr + inline style composition in both attribute orders.
            view { test_id: "all_three", nve_style: typed.clone(), flex: attr_val.clone(), style: inline_val.clone() }
            view { test_id: "flex_before_typed", flex: attr_val, nve_style: typed, style: inline_val }
            pressable { test_id: "cycle", onpress: move |_| step += 1 }
        }
    }
}

#[test]
fn mixed_flex_attr_and_style_precedence_is_order_independent_and_removal_emits_no_raw_prop() {
    use Dimension::{Auto, Percent as Pct, Points as Px};
    let mut surface = DioxusSurface::new(1, MixedFlexPrecedenceApp);
    let mut sink = ExtSink::default();
    surface.flush_into(1, &mut sink).unwrap();

    // Precedence: inline style > standalone flex attr > nve_style > defaults (0, 1, Auto).
    // step 0: attr="1 0 200px", style="padding:4px" -> plain (1,0,200px), all_three (1,0,200px)
    // step 1: attr="1 0 200px", style="flex-basis:33%" -> plain (1,0,33%), all_three (1,0,33%)
    // step 2: attr="1 0 200px", style=None -> plain (1,0,200px), all_three (1,0,200px) (grow/shrink preserved!)
    // step 3: attr=None, style="flex-basis:33%" -> plain (0,1,33%), all_three (3,2,33%) (attr removal resets grow/shrink!)
    // step 4: attr=None, style="padding:8px" -> plain (0,1,Auto), all_three (3,2,50%)
    let expected = [
        ((1.0, 0.0, Px(200.0)), (1.0, 0.0, Px(200.0))),
        ((1.0, 0.0, Pct(33.0)), (1.0, 0.0, Pct(33.0))),
        ((1.0, 0.0, Px(200.0)), (1.0, 0.0, Px(200.0))),
        ((0.0, 1.0, Pct(33.0)), (3.0, 2.0, Pct(33.0))),
        ((0.0, 1.0, Auto), (3.0, 2.0, Pct(50.0))),
    ];
    for (frame, ((pg, ps, pb), (tg, ts, tb))) in (1u64..).zip(expected) {
        if frame > 1 {
            press(&mut surface, &mut sink, frame);
        }
        assert_eq!(sink.basis("attr_first"), pb, "attr_first basis @ frame {frame}");
        assert_eq!(sink.flex_factors("attr_first"), (pg, ps), "attr_first factors @ frame {frame}");
        assert_eq!(sink.basis("style_first"), pb, "style_first basis @ frame {frame}");
        assert_eq!(sink.flex_factors("style_first"), (pg, ps), "style_first factors @ frame {frame}");
        assert_eq!(sink.basis("all_three"), tb, "all_three basis @ frame {frame}");
        assert_eq!(sink.flex_factors("all_three"), (tg, ts), "all_three factors @ frame {frame}");
        assert_eq!(sink.basis("flex_before_typed"), tb, "flex_before_typed basis @ frame {frame}");
        assert_eq!(
            sink.flex_factors("flex_before_typed"),
            (tg, ts),
            "flex_before_typed factors must match all_three @ frame {frame}"
        );
    }
    assert!(
        !sink.raw_keys.iter().any(|k| k.contains("flex")),
        "removing or updating standalone flex must never emit a raw prop: {:?}",
        sink.raw_keys
    );
}

#[component]
fn CurrentColorBorderApp() -> Element {
    let mut step = use_signal(|| 0usize);
    let s = step();
    let parent_color: Option<String> = match s {
        0 => Some("color: #ffffff".to_owned()),
        1 => Some("color: #22c55e".to_owned()),
        2 => None,
        _ => Some("color: #3b82f6".to_owned()),
    };
    let dynamic_color = if s % 2 == 0 { "color: #ef4444" } else { "color: #60a5fa" };
    rsx! {
        view {
            test_id: "parent",
            style: parent_color,
            // 1. Cross-attribute: `style="border: 1px solid"` + standalone `color` (both orders).
            view { test_id: "cross_a", style: "border: 1px solid", color: "red" }
            view { test_id: "cross_b", color: "red", style: "border: 1px solid" }
            // 2. Dynamic `style` update changing only `color` alongside standalone `border`.
            view { test_id: "dyn_color", "border": "2px solid", style: "{dynamic_color}" }
            // 3. Inherited parent `color` through an unstyled intermediate wrapper.
            view {
                view { test_id: "inherited", style: "border: 1px solid" }
            }
            // 4. Child with its own explicit black color shields itself from white parent.
            view { test_id: "shielded", style: "color: #000000; border: 1px solid" }
            // 5. Explicit `border-color` ignores ancestor color changes.
            view { test_id: "explicit_border", style: "border: 1px solid #f59e0b" }
            pressable { test_id: "cycle", onpress: move |_| step += 1 }
        }
    }
}

#[test]
fn current_color_resolves_across_attributes_dynamic_updates_and_ancestor_inheritance() {
    use nve_abi::ColorRgba;
    let red = ColorRgba::rgb(239, 68, 68);
    let blue = ColorRgba::rgb(96, 165, 250);
    let green = ColorRgba::rgb(34, 197, 94);
    let accent = ColorRgba::rgb(59, 130, 246);
    let amber = ColorRgba::rgb(245, 158, 11);

    let mut surface = DioxusSurface::new(1, CurrentColorBorderApp);
    let mut sink = ExtSink::default();
    surface.flush_into(1, &mut sink).unwrap();

    // Frame 1: parent white, dynamic red.
    assert_eq!(sink.style_of("cross_a").border_color, red);
    assert_eq!(sink.style_of("cross_b").border_color, red);
    assert_eq!(sink.style_of("dyn_color").border_color, red);
    assert_eq!(sink.style_of("inherited").border_color, ColorRgba::WHITE);
    assert_eq!(sink.style_of("shielded").border_color, ColorRgba::BLACK);
    assert_eq!(sink.style_of("explicit_border").border_color, amber);

    // Frame 2: parent -> green, dynamic -> blue.
    press(&mut surface, &mut sink, 2);
    assert_eq!(sink.style_of("dyn_color").border_color, blue);
    assert_eq!(sink.style_of("inherited").border_color, green);
    assert_eq!(sink.style_of("shielded").border_color, ColorRgba::BLACK);
    assert_eq!(sink.style_of("explicit_border").border_color, amber);

    // Frame 3: parent removes color -> inherited falls back to default black.
    press(&mut surface, &mut sink, 3);
    assert_eq!(sink.style_of("dyn_color").border_color, red);
    assert_eq!(sink.style_of("inherited").border_color, ColorRgba::BLACK);

    // Frame 4: parent sets accent blue -> inherited updates again.
    press(&mut surface, &mut sink, 4);
    assert_eq!(sink.style_of("inherited").border_color, accent);
}

#[component]
fn RawStyleLayerAndDiffApp() -> Element {
    let mut step = use_signal(|| 0usize);
    let s = step();
    let w = if s == 0 { "100px" } else { "200px" };
    let ov: Option<String> = (s < 2).then(|| "hidden".to_owned());
    let fw: Option<String> = (s < 2).then(|| "700".to_owned());
    let inline_raw: Option<String> = match s {
        0 => Some("display: none; white-space: nowrap; flex-wrap: wrap".to_owned()),
        1 => Some("display: none; white-space: nowrap; flex-wrap: wrap".to_owned()),
        2 => Some("padding: 8px".to_owned()),
        _ => None,
    };
    let parent_opacity = if s % 2 == 0 { "1" } else { "0.5" };
    let child_pad_style = if s == 0 {
        "border: 1px solid"
    } else {
        "border: 1px solid; padding: 12px"
    };
    rsx! {
        view {
            // Standalone raw-style attrs in BOTH attribute orders relative to typed attrs.
            view { test_id: "raw_first", "overflow": ov.clone(), width: w }
            view { test_id: "typed_first", width: w, "overflow": ov }
            text { test_id: "text_raw", "font-weight": fw, padding: "4px", "Bold" }
            // Inline raw-style attrs + dynamic typed attr update (must not re-send unchanged raw).
            view { test_id: "inline_raw", width: w, style: inline_raw }
            // Unrelated opacity animation on a colored parent must not rewrite currentColor child.
            view {
                test_id: "anim_parent",
                style: "color: #ffffff; opacity: {parent_opacity}",
                view { test_id: "anim_child", style: "{child_pad_style}" }
            }
            pressable { test_id: "cycle", onpress: move |_| step += 1 }
        }
    }
}

#[test]
fn standalone_raw_style_attrs_survive_typed_attrs_and_inline_raw_props_diff_and_clear_cleanly() {
    use nve_abi::ColorRgba;
    let mut surface = DioxusSurface::new(1, RawStyleLayerAndDiffApp);
    let mut sink = ExtSink::default();
    surface.flush_into(1, &mut sink).unwrap();

    // Frame 1: standalone raw-style attributes survive typed attributes in BOTH orders.
    assert_eq!(sink.raw_of("raw_first", "overflow"), Some(Some("hidden")));
    assert_eq!(sink.raw_of("typed_first", "overflow"), Some(Some("hidden")));
    assert_eq!(sink.raw_of("text_raw", "font-weight"), Some(Some("700")));
    assert_eq!(sink.raw_of("inline_raw", "display"), Some(Some("none")));
    assert_eq!(sink.raw_of("inline_raw", "white-space"), Some(Some("nowrap")));
    assert_eq!(sink.raw_of("inline_raw", "flex-wrap"), Some(Some("wrap")));
    assert_eq!(sink.style_of("anim_child").border_color, ColorRgba::WHITE);

    // Frame 2: only `width` + `opacity` + child `padding` changed -> ZERO redundant raw prop writes,
    // and child `border_color` stays WHITE (no revert to black, no extra SetStyle write on child!).
    press(&mut surface, &mut sink, 2);
    assert_eq!(
        sink.frame_raw_writes, 0,
        "width-only update must not re-send unchanged raw style props"
    );
    assert_eq!(sink.raw_of("raw_first", "overflow"), Some(Some("hidden")));
    assert_eq!(sink.raw_of("typed_first", "overflow"), Some(Some("hidden")));
    assert_eq!(sink.style_of("anim_child").border_color, ColorRgba::WHITE);
    assert_eq!(
        sink.frame_style_writes.get(&sink.node("anim_child")).copied().unwrap_or(0),
        0,
        "unrelated opacity animation + child padding change must not emit SetStyle for anim_child"
    );

    // Frame 3: remove standalone `"overflow"` / `"font-weight"` and drop inline raw declarations
    // (`display`, `white-space`, `flex-wrap`) -> all MUST emit `Null` on the wire!
    press(&mut surface, &mut sink, 3);
    assert_eq!(sink.raw_of("raw_first", "overflow"), Some(None));
    assert_eq!(sink.raw_of("typed_first", "overflow"), Some(None));
    assert_eq!(sink.raw_of("text_raw", "font-weight"), Some(None));
    assert_eq!(sink.raw_of("inline_raw", "display"), Some(None));
    assert_eq!(sink.raw_of("inline_raw", "white-space"), Some(None));
    assert_eq!(sink.raw_of("inline_raw", "flex-wrap"), Some(None));

    // Frame 4: removing inline `style` completely emits zero extra Nulls for already-cleared keys.
    press(&mut surface, &mut sink, 4);
    assert_eq!(sink.frame_raw_writes, 0);
}

#[component]
fn ShorthandLonghandPrecedenceApp() -> Element {
    let mut step = use_signal(|| 0usize);
    let s = step();
    // Dynamic shorthand paired with static longhand (Dioxus applies static template attrs BEFORE
    // dynamic attrs) AND `Some -> None -> Some` toggle cycle on `padding` + `padding_horizontal`.
    let dyn_flex = if s % 2 == 0 { "1 0 200px" } else { "2 0 150px" };
    let opt_pad: Option<String> = (s != 1).then(|| "16px".to_owned());
    let opt_bw: Option<String> = (s != 1).then(|| "5px".to_owned());
    rsx! {
        view {
            view { test_id: "flex_split", flex: "{dyn_flex}", flex_grow: "3" }
            view { test_id: "pad_toggle", padding: opt_pad, padding_horizontal: "4px" }
            view { test_id: "border_split", "border": "2px solid #ef4444", border_width: opt_bw }
            pressable { test_id: "cycle", onpress: move |_| step += 1 }
        }
    }
}

#[test]
fn layer2_shorthand_vs_longhand_precedence_survives_static_dynamic_split_and_toggles() {
    use nve_abi::{ColorRgba, EdgeInsets};
    let mut surface = DioxusSurface::new(1, ShorthandLonghandPrecedenceApp);
    let mut sink = ExtSink::default();
    surface.flush_into(1, &mut sink).unwrap();

    // Frame 1: static `flex_grow: "3"` wins over dynamic `flex: "1 0 200px"`;
    // `padding_horizontal: "4px"` wins over `padding: "16px"`; `border_width: "5px"` wins over `2px`.
    assert_eq!(sink.flex_factors("flex_split"), (3.0, 0.0));
    assert_eq!(sink.basis("flex_split"), Dimension::Points(200.0));
    assert_eq!(
        sink.layout[&sink.node("pad_toggle")].padding,
        EdgeInsets { top: 16.0, right: 4.0, bottom: 16.0, left: 4.0 }
    );
    assert_eq!(sink.style_of("border_split").border_width, 5.0);
    assert_eq!(sink.style_of("border_split").border_color, ColorRgba::rgb(239, 68, 68));

    // Frame 2: `padding` -> None leaves `padding_horizontal: 4px`; `border_width` -> None falls
    // back to `border` shorthand's `2px`; dynamic `flex` update keeps `flex_grow: 3.0`.
    press(&mut surface, &mut sink, 2);
    assert_eq!(sink.flex_factors("flex_split"), (3.0, 0.0));
    assert_eq!(sink.basis("flex_split"), Dimension::Points(150.0));
    assert_eq!(
        sink.layout[&sink.node("pad_toggle")].padding,
        EdgeInsets { top: 0.0, right: 4.0, bottom: 0.0, left: 4.0 }
    );
    assert_eq!(sink.style_of("border_split").border_width, 2.0);
    assert_eq!(sink.style_of("border_split").border_color, ColorRgba::rgb(239, 68, 68));

    // Frame 3: `padding` re-appears (`Some("16px")`) — MUST match Frame 1 identically (`4px` horiz).
    press(&mut surface, &mut sink, 3);
    assert_eq!(
        sink.layout[&sink.node("pad_toggle")].padding,
        EdgeInsets { top: 16.0, right: 4.0, bottom: 16.0, left: 4.0 }
    );
    assert_eq!(sink.style_of("border_split").border_width, 5.0);
}

#[component]
fn SameFrameTransitionAndFallbackApp() -> Element {
    let mut step = use_signal(|| 0usize);
    let s = step();
    // Same-frame removal of last layer paired with standalone typed attr insertion.
    let ov: Option<String> = (s == 0).then(|| "hidden".to_owned());
    let inline_bg: Option<String> = (s == 0).then(|| "background-color: #ef4444".to_owned());
    let added_w: Option<String> = (s >= 1).then(|| "240px".to_owned());
    let added_border: Option<String> = (s >= 1).then(|| "2px solid".to_owned());
    // Invalid-value update must clear stale value (direct path -> default 0, layered -> nve_style).
    let dyn_pad = if s == 0 { "8px" } else { "garbage" };
    let typed_pad = TypedStyle::new().padding(14.0);
    rsx! {
        view {
            style: "color: #ffffff",
            view { test_id: "ov_to_w", "overflow": ov, width: added_w.clone() }
            view { test_id: "style_to_wb", style: inline_bg, width: added_w, "border": added_border }
            view { test_id: "direct_invalid", padding: "{dyn_pad}" }
            view { test_id: "layered_invalid", nve_style: typed_pad, padding: "{dyn_pad}" }
            // Per-property Text inheritance across direct, inline `style`, and `TypedStyle` paths:
            // adding `opacity` / `font_size` / `color` must never clobber sibling inherited props,
            // and removing or invalidating an explicit prop must re-inherit from the Text parent.
            text {
                font_size: if s < 2 { "24px" } else { "30px" },
                color: if s < 2 { "#0000ff" } else { "#22c55e" },
                text {
                    test_id: "dyn_opacity",
                    opacity: match s { 0 => None, 1 | 2 => Some("0.5"), _ => Some("garbage") },
                    "op"
                }
                text {
                    test_id: "dyn_fs",
                    font_size: match s { 0 => None, 1 | 2 => Some("16px"), _ => Some("bad") },
                    "fs"
                }
                text {
                    test_id: "dyn_inline_fs",
                    style: match s {
                        0 => None,
                        1 | 2 => Some("font-size: 16px; opacity: 0.8"),
                        _ => Some("opacity: 0.8"),
                    },
                    "ifs"
                }
                text {
                    test_id: "dyn_inline_col",
                    style: match s {
                        0 => None,
                        1 | 2 => Some("color: #ef4444; opacity: 0.8"),
                        _ => Some("opacity: 0.8"),
                    },
                    "icol"
                }
                text {
                    test_id: "dyn_color",
                    color: match s { 0 => None, 1 | 2 => Some("#ef4444"), _ => None },
                    "col"
                }
                text {
                    test_id: "typed_op",
                    nve_style: TypedStyle::new().opacity(0.6),
                    "top"
                }
                text {
                    test_id: "typed_fs_col",
                    nve_style: match s {
                        1 | 2 => TypedStyle::new()
                            .font_size(16.0)
                            .color(nve_abi::ColorRgba::rgb(0xef, 0x44, 0x44)),
                        _ => TypedStyle::new().opacity(0.6),
                    },
                    "tfc"
                }
            }
            // Explicit default-valued `Text` children on mount (`16px` under `24px`, `#000000`
            // under `#0000ff`) MUST emit `set_style` so native backends (`headless`/`skia`) mark
            // them explicit instead of overwriting them with parent values.
            text {
                font_size: "24px",
                text { test_id: "default_fs_mount", font_size: "16px", "d1" }
                text { test_id: "default_inline_fs_mount", style: "font-size: 16px", "d2" }
                text {
                    test_id: "default_typed_fs_mount",
                    nve_style: TypedStyle::new().font_size(16.0),
                    "d3"
                }
            }
            text {
                color: "#0000ff",
                text { test_id: "default_col_mount", color: "#000000", "d4" }
            }
            pressable { test_id: "cycle", onpress: move |_| step += 1 }
        }
    }
}

#[test]
fn same_frame_layer_removal_default_style_explicit_and_invalid_attr_fallback() {
    use nve_abi::{ColorRgba, EdgeInsets};
    let blue = ColorRgba::rgb(0x00, 0x00, 0xff);
    let green = ColorRgba::rgb(0x22, 0xc5, 0x5e);
    let red = ColorRgba::rgb(0xef, 0x44, 0x44);

    let mut surface = DioxusSurface::new(1, SameFrameTransitionAndFallbackApp);
    let mut sink = ExtSink::default();
    surface.flush_into(1, &mut sink).unwrap();

    // Frame 1 (s=0): all children inherit (24px, blue) — including `TypedStyle::new().opacity(0.6)`.
    assert_eq!(sink.raw_of("ov_to_w", "overflow"), Some(Some("hidden")));
    assert_eq!(sink.layout[&sink.node("direct_invalid")].padding, EdgeInsets::all(8.0));
    assert_eq!(sink.layout[&sink.node("layered_invalid")].padding, EdgeInsets::all(8.0));
    for id in [
        "dyn_opacity",
        "dyn_fs",
        "dyn_inline_fs",
        "dyn_inline_col",
        "dyn_color",
        "typed_op",
        "typed_fs_col",
    ] {
        assert_eq!(sink.style_of(id).font_size, 24.0, "{id}");
        assert_eq!(sink.style_of(id).text_color, blue, "{id}");
    }
    assert_eq!(sink.style_of("typed_op").opacity, 0.6);

    // Explicit default-valued `Text` children MUST emit `set_style` directly (not be absent from `sink.style`).
    for id in ["default_fs_mount", "default_inline_fs_mount", "default_typed_fs_mount"] {
        assert_eq!(
            sink.style.get(&sink.node(id)).map(|s| s.font_size),
            Some(16.0),
            "{id} must emit set_style for native backends"
        );
    }
    assert_eq!(
        sink.style.get(&sink.node("default_col_mount")).map(|s| s.text_color),
        Some(ColorRgba::BLACK),
        "default_col_mount must emit set_style for native backends"
    );

    // Frame 2 (s=1): same-frame layer removal + typed attr insertion, invalid-value fallback, AND
    // dynamic per-property Text overrides (must preserve sibling inherited color / font_size!).
    press(&mut surface, &mut sink, 2);
    assert_eq!(sink.raw_of("ov_to_w", "overflow"), Some(None));
    assert_eq!(sink.layout[&sink.node("ov_to_w")].width, Dimension::Points(240.0));
    assert_eq!(sink.layout[&sink.node("style_to_wb")].width, Dimension::Points(240.0));
    assert_eq!(sink.style_of("style_to_wb").border_width, 2.0);
    assert_eq!(sink.style_of("style_to_wb").border_color, ColorRgba::WHITE);
    assert_eq!(sink.layout[&sink.node("direct_invalid")].padding, EdgeInsets::all(0.0));
    assert_eq!(sink.layout[&sink.node("layered_invalid")].padding, EdgeInsets::all(14.0));

    assert_eq!(sink.style_of("dyn_opacity").opacity, 0.5);
    assert_eq!(sink.style_of("dyn_opacity").font_size, 24.0);
    assert_eq!(sink.style_of("dyn_opacity").text_color, blue);

    assert_eq!(sink.style_of("dyn_fs").font_size, 16.0);
    assert_eq!(sink.style_of("dyn_fs").text_color, blue);

    assert_eq!(sink.style_of("dyn_inline_fs").font_size, 16.0);
    assert_eq!(sink.style_of("dyn_inline_fs").opacity, 0.8);
    assert_eq!(sink.style_of("dyn_inline_fs").text_color, blue);

    assert_eq!(sink.style_of("dyn_inline_col").text_color, red);
    assert_eq!(sink.style_of("dyn_inline_col").opacity, 0.8);
    assert_eq!(sink.style_of("dyn_inline_col").font_size, 24.0);

    assert_eq!(sink.style_of("dyn_color").text_color, red);
    assert_eq!(sink.style_of("dyn_color").font_size, 24.0);

    assert_eq!(sink.style_of("typed_fs_col").font_size, 16.0);
    assert_eq!(sink.style_of("typed_fs_col").text_color, red);

    // Frame 3 (s=2): parent updates to (30px, green); each child inherits ONLY its non-explicit props.
    press(&mut surface, &mut sink, 3);
    assert_eq!(sink.style_of("dyn_opacity").font_size, 30.0);
    assert_eq!(sink.style_of("dyn_opacity").text_color, green);
    assert_eq!(sink.style_of("dyn_fs").font_size, 16.0);
    assert_eq!(sink.style_of("dyn_fs").text_color, green);
    assert_eq!(sink.style_of("dyn_inline_fs").font_size, 16.0);
    assert_eq!(sink.style_of("dyn_inline_fs").text_color, green);
    assert_eq!(sink.style_of("dyn_inline_col").text_color, red);
    assert_eq!(sink.style_of("dyn_inline_col").font_size, 30.0);
    assert_eq!(sink.style_of("dyn_color").text_color, red);
    assert_eq!(sink.style_of("dyn_color").font_size, 30.0);
    assert_eq!(sink.style_of("typed_op").font_size, 30.0);
    assert_eq!(sink.style_of("typed_op").text_color, green);
    assert_eq!(sink.style_of("typed_fs_col").font_size, 16.0);
    assert_eq!(sink.style_of("typed_fs_col").text_color, red);

    // Frame 4 (s=3): removing / invalidating explicit overrides restores parent's (30px, green).
    press(&mut surface, &mut sink, 4);
    assert_eq!(sink.style_of("dyn_opacity").opacity, 1.0);
    assert_eq!(sink.style_of("dyn_opacity").font_size, 30.0);
    assert_eq!(sink.style_of("dyn_opacity").text_color, green);
    assert_eq!(sink.style_of("dyn_fs").font_size, 30.0);
    assert_eq!(sink.style_of("dyn_fs").text_color, green);
    assert_eq!(sink.style_of("dyn_inline_fs").font_size, 30.0);
    assert_eq!(sink.style_of("dyn_inline_fs").opacity, 0.8);
    assert_eq!(sink.style_of("dyn_inline_fs").text_color, green);
    assert_eq!(sink.style_of("dyn_inline_col").text_color, green);
    assert_eq!(sink.style_of("dyn_inline_col").opacity, 0.8);
    assert_eq!(sink.style_of("dyn_inline_col").font_size, 30.0);
    assert_eq!(sink.style_of("dyn_color").text_color, green);
    assert_eq!(sink.style_of("dyn_color").font_size, 30.0);
    assert_eq!(sink.style_of("typed_fs_col").font_size, 30.0);
    assert_eq!(sink.style_of("typed_fs_col").text_color, green);
    assert_eq!(sink.style_of("typed_fs_col").opacity, 0.6);
}
