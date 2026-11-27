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

/// Mirrors the last flushed `LayoutExt` (`None` when cleared), layout, and raw props per node.
#[derive(Default)]
struct ExtSink {
    layout_ext: HashMap<NodeId, Option<LayoutExt>>,
    layout: HashMap<NodeId, LayoutProps>,
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
    fn set_style(&mut self, _: NodeId, _: &StyleProps) {}
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
    let typed = TypedStyle::new().flex_basis_pct(50.0).aspect_ratio(2.0);
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
            // Opposite attribute declaration orders must produce identical basis at every step.
            view { test_id: "attr_first", flex: attr_val.clone(), style: inline_val.clone() }
            view { test_id: "style_first", style: inline_val.clone(), flex: attr_val.clone() }
            // Typed + standalone attr + inline style composition.
            view { test_id: "all_three", nve_style: typed, flex: attr_val, style: inline_val }
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

    // Precedence: inline style > standalone flex attr > nve_style > auto.
    // step 0: attr=200px, style="padding:4px" -> plain 200px, all_three 200px
    // step 1: attr=200px, style="flex-basis:33%" -> plain 33%, all_three 33%
    // step 2: attr=200px, style=None -> plain 200px, all_three 200px
    // step 3: attr=None, style="flex-basis:33%" -> plain 33%, all_three 33%
    // step 4: attr=None, style="padding:8px" -> plain Auto, all_three 50% (typed fallback)
    let expected = [
        (Px(200.0), Px(200.0)),
        (Pct(33.0), Pct(33.0)),
        (Px(200.0), Px(200.0)),
        (Pct(33.0), Pct(33.0)),
        (Auto, Pct(50.0)),
    ];
    for (frame, (plain_exp, typed_exp)) in (1u64..).zip(expected) {
        if frame > 1 {
            press(&mut surface, &mut sink, frame);
        }
        assert_eq!(sink.basis("attr_first"), plain_exp, "attr_first @ frame {frame}");
        assert_eq!(
            sink.basis("style_first"),
            plain_exp,
            "style_first must match attr_first @ frame {frame}"
        );
        assert_eq!(sink.basis("all_three"), typed_exp, "all_three @ frame {frame}");
    }
    assert!(
        !sink.raw_keys.iter().any(|k| k.contains("flex")),
        "removing or updating standalone flex must never emit a raw prop: {:?}",
        sink.raw_keys
    );
}
