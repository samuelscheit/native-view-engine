//! Integration tests for zero-serialization `TypedStyle` interactive pseudo-state builders,
//! `CursorKind`, `NodeRecord` storage, and direct `RenderSink` streaming + `BatchSink` replay.

use dioxus_core::{AttributeValue, ElementId, IntoAttributeValue, WriteMutations};
use nve_abi::{
    ColorRgba, ComponentKind, FrameId, LayoutProps, NodeId, RawPropValueRef, StyleProps, SurfaceId,
};
use nve_dioxus::prelude::*;
use nve_dioxus::{DioxusSurface, NveMutationWriter};
use nve_engine::{replay_command_buffer_into, BackendApplyStats, BackendError, RenderSink};

#[derive(Default)]
struct InteractionSink {
    cursors: Vec<(NodeId, Option<CursorKind>)>,
    state_rules: Vec<(NodeId, Vec<StateStyleRule>)>,
    raw_prop_keys: Vec<String>,
}

impl RenderSink for InteractionSink {
    fn begin_frame(&mut self, _: SurfaceId, _: FrameId) {
        self.cursors.clear();
        self.state_rules.clear();
        self.raw_prop_keys.clear();
    }
    fn create_node(&mut self, _: NodeId, _: ComponentKind, _: Option<&str>) {}
    fn remove_node(&mut self, _: NodeId) {}
    fn set_children(&mut self, _: NodeId, _: &[NodeId]) {}
    fn set_text(&mut self, _: NodeId, _: &str) {}
    fn set_layout(&mut self, _: NodeId, _: &LayoutProps) {}
    fn set_style(&mut self, _: NodeId, _: &StyleProps) {}
    fn set_raw_prop(&mut self, _: NodeId, key: &str, _: RawPropValueRef<'_>) {
        self.raw_prop_keys.push(key.to_owned());
    }
    fn set_event_mask(&mut self, _: NodeId, _: u64) {}
    fn dispatch_command(&mut self, _: NodeId, _: &str, _: &[u8]) {}
    fn set_cursor(&mut self, node_id: NodeId, cursor: Option<CursorKind>) {
        self.cursors.push((node_id, cursor));
    }
    fn set_state_rules(&mut self, node_id: NodeId, rules: &[StateStyleRule]) {
        self.state_rules.push((node_id, rules.to_vec()));
    }
    fn commit_frame(&mut self, _: bool) -> Result<BackendApplyStats, BackendError> {
        Ok(BackendApplyStats::default())
    }
}

#[test]
fn dioxus_surface_streams_typed_cursors_and_state_rules_with_zero_serialization() {
    fn app() -> Element {
        let btn_style = TypedStyle::new()
            .bg(ColorRgba::rgb(37, 99, 235))
            .radius(8.0)
            .cursor(CursorKind::Pointer)
            .on_hover(StylePatch::new().bg(ColorRgba::rgb(59, 130, 246)))
            .on_active(
                StylePatch::new()
                    .bg(ColorRgba::rgb(29, 78, 216))
                    .cursor(CursorKind::Grabbing),
            )
            .on_focus(StylePatch::new().border_color(ColorRgba::rgb(99, 102, 241)))
            .on_focus_visible_with(|p| p.border(2.0, ColorRgba::rgb(56, 189, 248)))
            .on_disabled(StylePatch::new().opacity(0.5).cursor(CursorKind::NotAllowed))
            .on_state(
                InteractionState::HOVER | InteractionState::FOCUS_VISIBLE,
                StylePatch::new().radius(10.0),
            );

        let splitter_style = TypedStyle::new()
            .width_px(6.0)
            .cursor(CursorKind::ColResize);

        rsx! {
            pressable {
                nve_style: btn_style,
                text { "Submit" }
            }
            view {
                nve_style: splitter_style,
            }
        }
    }

    let mut surface = DioxusSurface::new(1, app);
    let mut sink = InteractionSink::default();
    surface.flush_into(1, &mut sink).unwrap();

    // Verify ZERO raw-prop string serialization for cursor/state_rules!
    assert!(!sink
        .raw_prop_keys
        .iter()
        .any(|k| matches!(k.as_str(), "cursor" | "nve-state" | "nve-states" | "state-rules")));

    sink.cursors.sort_by_key(|(id, _)| *id);
    assert_eq!(sink.cursors.len(), 2);
    assert_eq!(sink.cursors[0].1, Some(CursorKind::Pointer));
    assert_eq!(sink.cursors[1].1, Some(CursorKind::ColResize));
    assert_eq!(surface.node_cursor(sink.cursors[0].0), Some(CursorKind::Pointer));
    assert_eq!(surface.node_cursor(sink.cursors[1].0), Some(CursorKind::ColResize));

    assert_eq!(sink.state_rules.len(), 1);
    let (btn_id, ref rules) = sink.state_rules[0];
    assert_eq!(btn_id, sink.cursors[0].0);
    assert_eq!(rules.len(), 6);
    assert_eq!(rules[0].state, InteractionState::HOVER);
    assert_eq!(rules[1].state, InteractionState::ACTIVE);
    assert_eq!(rules[1].patch.cursor, Some(CursorKind::Grabbing));
    assert_eq!(rules[2].state, InteractionState::FOCUS);
    assert_eq!(rules[3].state, InteractionState::FOCUS_VISIBLE);
    assert_eq!(rules[3].patch.border_width, Some(2.0));
    assert_eq!(rules[4].state, InteractionState::DISABLED);
    assert_eq!(rules[4].patch.opacity, Some(0.5));
    assert_eq!(
        rules[5].state,
        InteractionState::HOVER | InteractionState::FOCUS_VISIBLE
    );
    assert_eq!(rules[5].patch.border_radius, Some(10.0));
    assert_eq!(surface.node_state_rules(btn_id), rules.as_slice());

    // Cross-boundary BatchSink + replay_command_buffer_into reproduces exact same rules/cursors!
    let mut wire_surface = DioxusSurface::new(1, app);
    let (_, _, bytes) = wire_surface.render_frame(1);
    let mut replayed = InteractionSink::default();
    replay_command_buffer_into(&bytes, &mut replayed).unwrap();
    replayed.cursors.sort_by_key(|(id, _)| *id);
    assert_eq!(replayed.cursors, sink.cursors);
    assert_eq!(replayed.state_rules, sink.state_rules);
}

#[test]
fn mutation_writer_updates_and_clears_cursor_and_state_rules() {
    let mut writer = NveMutationWriter::new(1);
    let mut sink = InteractionSink::default();

    let mut tx = writer.begin_tx(&mut sink, 1, 1);
    tx.create_placeholder(ElementId(7));
    let s1 = TypedStyle::new()
        .cursor(CursorKind::Grab)
        .on_hover_with(|p| p.opacity(0.85))
        .on_active_with(|p| p.cursor(CursorKind::Grabbing));
    tx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &s1.into_value(), ElementId(7));
    tx.finish_frame(true).unwrap();

    assert_eq!(sink.cursors.len(), 1);
    let (node_id, cursor) = sink.cursors[0];
    assert_eq!(cursor, Some(CursorKind::Grab));
    assert_eq!(writer.node_cursor(node_id), Some(CursorKind::Grab));
    assert_eq!(writer.node_state_rules(node_id).len(), 2);

    // Updating TypedStyle replaces cursor & state_rules cleanly.
    let mut tx2 = writer.begin_tx(&mut sink, 1, 2);
    let s2 = TypedStyle::new()
        .cursor(CursorKind::Move)
        .on_disabled_with(|p| p.opacity(0.4));
    tx2.set_attribute(NVE_TYPED_STYLE_ATTR, None, &s2.into_value(), ElementId(7));
    tx2.finish_frame(true).unwrap();
    assert_eq!(writer.node_cursor(node_id), Some(CursorKind::Move));
    assert_eq!(writer.node_state_rules(node_id).len(), 1);
    assert_eq!(writer.node_state_rules(node_id)[0].state, InteractionState::DISABLED);
    assert_eq!(sink.cursors, vec![(node_id, Some(CursorKind::Move))]);

    // Clearing TypedStyle emits None cursor and empty state_rules directly to RenderSink.
    let mut tx3 = writer.begin_tx(&mut sink, 1, 3);
    tx3.set_attribute(NVE_TYPED_STYLE_ATTR, None, &AttributeValue::None, ElementId(7));
    tx3.finish_frame(true).unwrap();
    assert_eq!(writer.node_cursor(node_id), None);
    assert!(writer.node_state_rules(node_id).is_empty());
    assert_eq!(sink.cursors, vec![(node_id, None)]);
    assert_eq!(sink.state_rules, vec![(node_id, vec![])]);
}
