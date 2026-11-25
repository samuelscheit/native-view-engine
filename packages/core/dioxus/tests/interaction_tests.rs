//! Integration tests for zero-serialization `TypedStyle` interactive pseudo-state builders,
//! `CursorKind`, `NodeRecord` storage, and `DioxusSurface` / `NveMutationWriter` iterators.

use dioxus_core::{AttributeValue, ElementId, IntoAttributeValue, WriteMutations};
use nve_abi::{ColorRgba, Command, CommandDecoder, RawPropValue};
use nve_dioxus::prelude::*;
use nve_dioxus::{DioxusSurface, NveMutationWriter};

fn decode_cmds(bytes: &[u8]) -> Vec<Command> {
    CommandDecoder::new(bytes)
        .expect("valid header")
        .decode_all()
        .expect("valid command buffer")
        .1
}

#[test]
fn dioxus_surface_stores_typed_cursors_and_state_rules_with_zero_serialization() {
    fn app() -> Element {
        let btn_style = TypedStyle::new()
            .bg(ColorRgba::rgb(37, 99, 235))
            .radius(8.0)
            .cursor(CursorKind::Pointer)
            .on_hover(StylePatch::new().bg(ColorRgba::rgb(59, 130, 246)))
            .on_active(StylePatch::new().bg(ColorRgba::rgb(29, 78, 216)).cursor(CursorKind::Grabbing))
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
    let (_, _, frame_bytes) = surface.render_frame(1);
    let cmds = decode_cmds(&frame_bytes);

    // Verify ZERO byte or string serialization in CommandBuffer for cursor/state_rules!
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::SetRawProp { key, .. }
            if matches!(key.as_str(), "cursor" | "nve-state" | "nve-states" | "state-rules")
    )));
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::SetRawProp {
            value: RawPropValue::Bytes(_),
            ..
        }
    )));

    let mut cursors: Vec<_> = surface.iter_cursors().collect();
    cursors.sort_by_key(|(id, _)| *id);
    assert_eq!(cursors.len(), 2);
    assert_eq!(cursors[0].1, CursorKind::Pointer);
    assert_eq!(cursors[1].1, CursorKind::ColResize);
    assert_eq!(surface.node_cursor(cursors[0].0), Some(CursorKind::Pointer));
    assert_eq!(surface.node_cursor(cursors[1].0), Some(CursorKind::ColResize));

    let state_entries: Vec<_> = surface.iter_state_rules().collect();
    assert_eq!(state_entries.len(), 1);
    let (btn_id, rules) = state_entries[0];
    assert_eq!(btn_id, cursors[0].0);
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
    assert_eq!(surface.node_state_rules(btn_id), rules);
}

#[test]
fn mutation_writer_updates_and_clears_cursor_and_state_rules() {
    let mut writer = NveMutationWriter::new(1);
    writer.begin_frame(1, 1);
    writer.create_placeholder(ElementId(7));

    let s1 = TypedStyle::new()
        .cursor(CursorKind::Grab)
        .on_hover_with(|p| p.opacity(0.85))
        .on_active_with(|p| p.cursor(CursorKind::Grabbing));
    writer.set_attribute(NVE_TYPED_STYLE_ATTR, None, &s1.into_value(), ElementId(7));
    let _ = writer.finish_frame(true);

    let (node_id, cursor) = writer.iter_cursors().next().expect("cursor stored");
    assert_eq!(cursor, CursorKind::Grab);
    assert_eq!(writer.node_cursor(node_id), Some(CursorKind::Grab));
    assert_eq!(writer.node_state_rules(node_id).len(), 2);

    // Updating TypedStyle replaces cursor & state_rules cleanly.
    writer.begin_frame(1, 2);
    let s2 = TypedStyle::new()
        .cursor(CursorKind::Move)
        .on_disabled_with(|p| p.opacity(0.4));
    writer.set_attribute(NVE_TYPED_STYLE_ATTR, None, &s2.into_value(), ElementId(7));
    let _ = writer.finish_frame(true);
    assert_eq!(writer.node_cursor(node_id), Some(CursorKind::Move));
    assert_eq!(writer.node_state_rules(node_id).len(), 1);
    assert_eq!(writer.node_state_rules(node_id)[0].state, InteractionState::DISABLED);

    // Clearing TypedStyle clears both cursor and state_rules.
    writer.begin_frame(1, 3);
    writer.set_attribute(NVE_TYPED_STYLE_ATTR, None, &AttributeValue::None, ElementId(7));
    let _ = writer.finish_frame(true);
    assert_eq!(writer.node_cursor(node_id), None);
    assert!(writer.node_state_rules(node_id).is_empty());
    assert_eq!(writer.iter_cursors().count(), 0);
    assert_eq!(writer.iter_state_rules().count(), 0);
}
