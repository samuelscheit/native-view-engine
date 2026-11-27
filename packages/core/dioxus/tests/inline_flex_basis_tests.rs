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

/// Mirrors the last flushed `LayoutExt` (`None` when cleared), layout, style, and raw props per node.
#[derive(Default)]
struct ExtSink {
    layout_ext: HashMap<NodeId, Option<LayoutExt>>,
    layout: HashMap<NodeId, LayoutProps>,
    style: HashMap<NodeId, StyleProps>,
    test_ids: HashMap<String, NodeId>,
    raw_keys: Vec<String>,
}

impl RenderSink for ExtSink {
    fn begin_frame(&mut self, _: SurfaceId, _: FrameId) {}
    fn create_node(&mut self, _: NodeId, _: ComponentKind, _: Option<&str>) {}
    fn remove_node(&mut self, _: NodeId) {}
    fn set_children(&mut self, _: NodeId, _: &[NodeId]) {}
    fn set_text(&mut self, _: NodeId, _: &str) {}
    fn set_layout(&mut self, node_id: NodeId, layout: &LayoutProps) {
        self.layout.insert(node_id, *layout);
    }
    fn set_style(&mut self, node_id: NodeId, style: &StyleProps) {
        self.style.insert(node_id, *style);
    }
    fn set_raw_prop(&mut self, node_id: NodeId, key: &str, value: RawPropValueRef<'_>) {
        if key == "testID"
            && let RawPropValueRef::String(tid) = value
        {
            self.test_ids.insert(tid.to_owned(), node_id);
        }
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
