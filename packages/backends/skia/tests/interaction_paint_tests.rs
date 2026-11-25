//! End-to-end Skia DisplayList & PixelBuffer rasterization tests proving interactive
//! pseudo-states (`:hover`, `:active`, `:focus`, `:focus-visible`, `:disabled`) actually paint
//! resolved colors, borders, radii, opacity, and report the expected `CursorKind`.

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask, LayoutProps,
    RawPropValue, StyleProps, ROOT_NODE_ID,
};
use nve_backend_skia::{
    CursorKind, Display, InteractionState, PaintOp, PixelBuffer, SkiaBackend, StateStyleRule,
    StylePatch,
};
use nve_engine::{RenderBackend, Viewport};

fn build_button_frame() -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Pressable, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(120.0),
            height: Dimension::Points(48.0),
            margin: EdgeInsets { left: 20.0, top: 20.0, ..Default::default() },
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(30, 41, 59),
            border_width: 1.0,
            border_color: ColorRgba::rgb(71, 85, 105),
            border_radius: 4.0,
            opacity: 1.0,
            ..Default::default()
        },
    );
    b.set_event_mask(2, EventMask::PRESS);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    b.finish().to_vec()
}

#[test]
fn skia_paints_hover_active_focus_visible_and_disabled_patches_into_display_list_and_pixels() {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(1, Viewport { width: 200.0, height: 100.0, scale_factor: 1.0 });
    skia.submit_frame(&build_button_frame()).unwrap();

    let hover_bg = ColorRgba::rgb(59, 130, 246);
    let active_bg = ColorRgba::rgb(29, 78, 216);
    let fv_border = ColorRgba::rgb(250, 204, 21);
    let disabled_bg = ColorRgba::rgb(100, 116, 139);

    skia.set_node_state_rules(
        2,
        vec![
            StateStyleRule::new(
                InteractionState::HOVER,
                StylePatch::new().bg(hover_bg).radius(10.0),
            ),
            StateStyleRule::new(
                InteractionState::FOCUS_VISIBLE,
                StylePatch::new().border(3.0, fv_border),
            ),
            StateStyleRule::new(
                InteractionState::ACTIVE,
                StylePatch::new()
                    .bg(active_bg)
                    .cursor(CursorKind::Grabbing),
            ),
            StateStyleRule::new(
                InteractionState::DISABLED,
                StylePatch::new()
                    .bg(disabled_bg)
                    .opacity(0.5)
                    .cursor(CursorKind::NotAllowed),
            ),
        ],
    );

    let mut buf = PixelBuffer::new(200, 100);

    // 1. Base state paints base background `(30, 41, 59)`.
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    assert_eq!(buf.get_rgb(80, 44), Some((30, 41, 59)));
    assert_eq!(skia.active_cursor(), CursorKind::Auto);

    // 2. Hover paints `hover_bg` + `radius = 10.0` and reports `CursorKind::Pointer`.
    assert_eq!(skia.pointer_move(80.0, 44.0), vec![2]);
    assert_eq!(skia.active_cursor(), CursorKind::Pointer);
    assert!(skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::FillRoundedRect { color, radius, .. }
            if *color == hover_bg && (*radius - 10.0).abs() < 1e-3
    )));
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    assert_eq!(buf.get_rgb(80, 44), Some((59, 130, 246)));

    // 3. Active press overrides hover color with `active_bg` and switches cursor to `Grabbing`.
    assert_eq!(skia.pointer_down(80.0, 44.0), vec![2]);
    assert_eq!(skia.active_cursor(), CursorKind::Grabbing);
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    assert_eq!(buf.get_rgb(80, 44), Some((29, 78, 216)));

    // 4. Dragging off cancels press and reverts painted pixels to base.
    assert_eq!(skia.pointer_move(5.0, 5.0), vec![2]);
    assert_eq!(skia.active_cursor(), CursorKind::Auto);
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    assert_eq!(buf.get_rgb(80, 44), Some((30, 41, 59)));

    // 5. Touch tap clears sticky hover immediately.
    assert_eq!(skia.pointer_move(80.0, 44.0), vec![2]);
    assert_eq!(skia.touch_tap(80.0, 44.0), vec![2]);
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    assert_eq!(buf.get_rgb(80, 44), Some((30, 41, 59)));

    // 6. Focus pointer does NOT paint `:focus-visible` border; focus keyboard DOES!
    assert_eq!(skia.focus_pointer(Some(2)), vec![2]);
    assert!(!skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::StrokeRoundedRect { color, width, .. }
            if *color == fv_border && (*width - 3.0).abs() < 1e-3
    )));
    assert_eq!(skia.focus_keyboard(Some(2)), vec![2]);
    assert!(skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::StrokeRoundedRect { color, width, .. }
            if *color == fv_border && (*width - 3.0).abs() < 1e-3
    )));

    // 7. Disabled prop paints `SaveOpacity(0.5)` + `disabled_bg` (beating hover) and `NotAllowed` cursor.
    let mut b_dis = CommandBufferBuilder::new(1, 2);
    b_dis.set_raw_prop(2, "disabled", &RawPropValue::Bool(true));
    b_dis.commit(false);
    skia.submit_frame(b_dis.finish()).unwrap();
    assert_eq!(skia.pointer_move(80.0, 44.0), vec![2]);
    assert_eq!(skia.active_cursor(), CursorKind::NotAllowed);
    assert!(skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::SaveOpacity(op_val) if (*op_val - 0.5).abs() < 1e-3
    )));
    assert!(skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::FillRoundedRect { color, .. } if *color == disabled_bg
    )));
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    // 50% opacity of (100, 116, 139) over black -> ~(50, 58, 70)
    let (r, g, b) = buf.get_rgb(80, 44).unwrap();
    assert!((48..=52).contains(&r) && (56..=60).contains(&g) && (68..=72).contains(&b), "got ({r},{g},{b})");

    // 8. `Display::None` state patch suppresses node painting completely.
    skia.set_node_state_rules(
        2,
        vec![StateStyleRule::new(
            InteractionState::DISABLED,
            StylePatch::new().display(Display::None),
        )],
    );
    assert!(skia.display_list().ops.is_empty());

    // 9. Removing the node clears tracker state and resets cursor.
    let mut b_rm = CommandBufferBuilder::new(1, 3);
    b_rm.remove_node(2);
    b_rm.commit(true);
    skia.submit_frame(b_rm.finish()).unwrap();
    assert_eq!(skia.active_cursor(), CursorKind::Auto);
    assert_eq!(skia.interaction_state(2), InteractionState::NONE);
}
