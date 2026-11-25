//! Integration tests for `InteractionTracker`, `effective_style`, `active_cursor`,
//! disabled prop sync, and unmount cleanup inside `HeadlessBackend`.

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask, LayoutProps,
    RawPropValue, StyleProps, ROOT_NODE_ID,
};
use nve_backend_headless::{
    CursorKind, HeadlessBackend, InteractionState, StateStyleRule, StylePatch, TaffyLayoutEngine,
};
use nve_engine::{RenderBackend, Viewport};

fn build_scene() -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, 1);
    // Node 2: Pressable button at (10, 10, 120x40) containing child Text node 3.
    b.create_node(2, ComponentKind::Pressable, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(120.0),
            height: Dimension::Points(40.0),
            margin: EdgeInsets { left: 10.0, top: 10.0, ..Default::default() },
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(37, 99, 235),
            border_radius: 6.0,
            ..Default::default()
        },
    );
    b.set_event_mask(2, EventMask::PRESS);

    b.create_node(3, ComponentKind::Text, None);
    b.set_text(3, "Save");
    b.set_children(2, &[3]);

    // Node 4: Splitter View at (10, 60, 120x20) with no event_mask, only explicit cursor.
    b.create_node(4, ComponentKind::View, None);
    b.set_layout(
        4,
        &LayoutProps {
            width: Dimension::Points(120.0),
            height: Dimension::Points(20.0),
            margin: EdgeInsets { left: 10.0, top: 10.0, ..Default::default() },
            ..Default::default()
        },
    );

    // Node 5: TextInput at (10, 90, 120x30).
    b.create_node(5, ComponentKind::TextInput, None);
    b.set_layout(
        5,
        &LayoutProps {
            width: Dimension::Points(120.0),
            height: Dimension::Points(30.0),
            margin: EdgeInsets { left: 10.0, top: 10.0, ..Default::default() },
            ..Default::default()
        },
    );

    b.set_children(ROOT_NODE_ID, &[2, 4, 5]);
    b.commit(true);
    b.finish().to_vec()
}

#[test]
fn headless_backend_tracks_pointer_touch_focus_disabled_unmount_and_cursors() {
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(1, Viewport { width: 300.0, height: 200.0, scale_factor: 1.0 });
    backend.submit_frame(&build_scene()).unwrap();

    let btn_rules = vec![
        StateStyleRule::new(
            InteractionState::HOVER,
            StylePatch::new().bg(ColorRgba::rgb(59, 130, 246)),
        ),
        StateStyleRule::new(
            InteractionState::FOCUS_VISIBLE,
            StylePatch::new().border(2.0, ColorRgba::rgb(56, 189, 248)),
        ),
        StateStyleRule::new(
            InteractionState::ACTIVE,
            StylePatch::new()
                .bg(ColorRgba::rgb(29, 78, 216))
                .cursor(CursorKind::Grabbing),
        ),
        StateStyleRule::new(
            InteractionState::DISABLED,
            StylePatch::new()
                .opacity(0.4)
                .cursor(CursorKind::NotAllowed),
        ),
    ];
    backend.sync_interaction_from([(4, CursorKind::RowResize)], [(2, btn_rules.as_slice())]);

    // Idle state: base style & Auto cursor.
    assert_eq!(backend.active_cursor(), CursorKind::Auto);
    assert_eq!(backend.effective_style(2).unwrap().background, ColorRgba::rgb(37, 99, 235));

    // Moving pointer onto Pressable (even over its child Text) resolves to node 2.
    assert_eq!(backend.pointer_move(50.0, 25.0), vec![2]);
    assert_eq!(backend.interaction_state(2), InteractionState::HOVER);
    assert_eq!(backend.active_cursor(), CursorKind::Pointer);
    assert_eq!(backend.effective_style(2).unwrap().background, ColorRgba::rgb(59, 130, 246));

    // Pressing activates node 2 and switches cursor to Grabbing.
    assert_eq!(backend.pointer_down(50.0, 25.0), vec![2]);
    assert_eq!(backend.effective_style(2).unwrap().background, ColorRgba::rgb(29, 78, 216));
    assert_eq!(backend.active_cursor(), CursorKind::Grabbing);

    // Dragging onto splitter View (node 4, event_mask=0, explicit cursor) cancels press on 2 and hovers 4!
    assert_eq!(backend.pointer_move(50.0, 70.0), vec![2, 4]);
    assert_eq!(backend.interaction_state(2), InteractionState::NONE);
    assert_eq!(backend.interaction_state(4), InteractionState::HOVER);
    assert_eq!(backend.active_cursor(), CursorKind::RowResize);

    // Hovering TextInput defaults to CursorKind::Text.
    assert_eq!(backend.pointer_move(50.0, 105.0), vec![4, 5]);
    assert_eq!(backend.active_cursor(), CursorKind::Text);

    // Touch tap clears hover (no sticky hover on hybrid devices).
    assert_eq!(backend.touch_tap(50.0, 25.0), vec![5]);
    assert_eq!(backend.active_cursor(), CursorKind::Auto);

    // Focus pointer vs keyboard drives FOCUS vs FOCUS_VISIBLE.
    assert_eq!(backend.focus_pointer(Some(2)), vec![2]);
    assert_eq!(backend.effective_style(2).unwrap().border_width, 0.0);
    assert_eq!(backend.focus_keyboard(Some(2)), vec![2]);
    assert_eq!(backend.effective_style(2).unwrap().border_width, 2.0);
    assert_eq!(backend.effective_style(2).unwrap().border_color, ColorRgba::rgb(56, 189, 248));

    // Raw prop `disabled: true` automatically syncs InteractionTracker::DISABLED, blurs focus,
    // and updates active cursor on hover to NotAllowed.
    let mut b_dis = CommandBufferBuilder::new(1, 2);
    b_dis.set_raw_prop(2, "disabled", &RawPropValue::Bool(true));
    b_dis.commit(false);
    backend.submit_frame(b_dis.finish()).unwrap();
    assert_eq!(backend.interaction_state(2), InteractionState::DISABLED);
    assert_eq!(backend.effective_style(2).unwrap().opacity, 0.4);
    assert_eq!(backend.pointer_move(50.0, 25.0), vec![2]);
    assert_eq!(backend.active_cursor(), CursorKind::NotAllowed);

    // Layout engine swap preserves interaction state & rules.
    let mut backend = backend.swap_layout_engine(TaffyLayoutEngine::new());
    assert_eq!(backend.active_cursor(), CursorKind::NotAllowed);
    assert_eq!(backend.effective_style(2).unwrap().opacity, 0.4);

    // Unmounting hovered/disabled node 2 clears tracker state cleanly!
    let mut b_rm = CommandBufferBuilder::new(1, 3);
    b_rm.remove_node(2);
    b_rm.commit(true);
    backend.submit_frame(b_rm.finish()).unwrap();
    assert_eq!(backend.interaction_state(2), InteractionState::NONE);
    assert_eq!(backend.active_cursor(), CursorKind::Auto);
    assert!(backend.pointer_leave().is_empty());
}
