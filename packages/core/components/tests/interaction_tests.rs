//! Deep unit & edge-case tests for `nve_components::interaction` and unified `StylePatch`.

use std::mem::{align_of, size_of};

use nve_abi::{ColorRgba, NodeId};
use nve_components::interaction::{
    resolve_cursor, resolve_layout_and_style, resolve_patch, resolve_style, CursorKind,
    FocusModality, InteractionState, InteractionTracker, StateStyleRule,
};
use nve_components::media::ResponsiveStylePatch;
use nve_components::style_patch::{Display, LayoutSnapshot, StylePatch, VisualSnapshot};

#[test]
fn interaction_state_layout_bitops_and_cascade_ordering() {
    assert_eq!(size_of::<InteractionState>(), 1);
    assert_eq!(align_of::<InteractionState>(), 1);

    assert!(InteractionState::NONE.is_empty());
    assert!(!InteractionState::NONE.matches(InteractionState::ALL));
    assert!(!InteractionState::NONE.matches(InteractionState::NONE));
    assert_eq!(InteractionState::from_bits_truncate(0xFF), InteractionState::ALL);

    // Strict base cascade priority: Hover < Focus < FocusVisible < Active < Disabled.
    let ordered = [
        InteractionState::HOVER,
        InteractionState::FOCUS,
        InteractionState::FOCUS_VISIBLE,
        InteractionState::ACTIVE,
        InteractionState::DISABLED,
    ];
    for pair in ordered.windows(2) {
        assert!(
            pair[0].cascade_order() < pair[1].cascade_order(),
            "{:?} must precede {:?}",
            pair[0],
            pair[1]
        );
    }
    assert_eq!(InteractionState::NONE.cascade_tier(), None);
    assert_eq!(InteractionState::HOVER.cascade_tier(), Some(0));
    assert_eq!(InteractionState::FOCUS.cascade_tier(), Some(1));
    assert_eq!(InteractionState::FOCUS_VISIBLE.cascade_tier(), Some(2));
    assert_eq!(InteractionState::ACTIVE.cascade_tier(), Some(3));
    assert_eq!(InteractionState::DISABLED.cascade_tier(), Some(4));

    // Compound matching & cascade priority over constituents.
    let hf = InteractionState::HOVER | InteractionState::FOCUS_VISIBLE;
    assert!(hf.contains(InteractionState::HOVER));
    assert!(hf.contains(InteractionState::FOCUS_VISIBLE));
    assert!(!hf.contains(InteractionState::ACTIVE));
    assert!(hf.matches(InteractionState::HOVER | InteractionState::FOCUS_VISIBLE | InteractionState::FOCUS));
    assert!(!hf.matches(InteractionState::HOVER));
    assert!(!hf.matches(InteractionState::FOCUS_VISIBLE));
    assert!(hf.cascade_order() > InteractionState::HOVER.cascade_order());
    assert!(hf.cascade_order() > InteractionState::FOCUS_VISIBLE.cascade_order());
    assert!(hf.cascade_order() < InteractionState::ACTIVE.cascade_order());
    assert_eq!(hf.cascade_tier(), Some(2));

    // Bitwise operators round-trip cleanly.
    let mut s = InteractionState::HOVER;
    s |= InteractionState::ACTIVE;
    assert_eq!(s & InteractionState::ACTIVE, InteractionState::ACTIVE);
    assert_eq!(s ^ InteractionState::HOVER, InteractionState::ACTIVE);
    assert_eq!((!InteractionState::NONE), InteractionState::ALL);
    s &= InteractionState::HOVER;
    assert_eq!(s, InteractionState::HOVER);
}

#[test]
fn cursor_kind_roundtrips_u8_and_css_keywords() {
    let expected = [
        (0u8, CursorKind::Auto, "auto"),
        (1, CursorKind::Default, "default"),
        (2, CursorKind::Pointer, "pointer"),
        (3, CursorKind::Text, "text"),
        (4, CursorKind::Grab, "grab"),
        (5, CursorKind::Grabbing, "grabbing"),
        (6, CursorKind::NotAllowed, "not-allowed"),
        (7, CursorKind::ColResize, "col-resize"),
        (8, CursorKind::RowResize, "row-resize"),
        (9, CursorKind::Move, "move"),
        (10, CursorKind::Crosshair, "crosshair"),
        (11, CursorKind::Progress, "progress"),
        (12, CursorKind::Wait, "wait"),
    ];
    for (raw, kind, kw) in expected {
        assert_eq!(kind as u8, raw);
        assert_eq!(CursorKind::from_u8(raw), Some(kind));
        assert_eq!(kind.to_css_keyword(), kw);
        assert_eq!(CursorKind::parse_css(kw), Some(kind));
        assert_eq!(CursorKind::parse_css(&format!("  {}  ", kw.to_ascii_uppercase())), Some(kind));
    }
    assert_eq!(CursorKind::from_u8(13), None);
    assert_eq!(CursorKind::from_u8(255), None);
    assert_eq!(CursorKind::parse_css("invalid-cursor"), None);
}

#[test]
fn unified_style_patch_resolves_cascade_priority_compounds_and_large_rule_lists() {
    // ResponsiveStylePatch is a direct alias for StylePatch.
    let _: StylePatch = ResponsiveStylePatch::new().display(Display::None).cursor(CursorKind::Wait);

    let base = VisualSnapshot {
        background: ColorRgba::rgb(10, 10, 10),
        text_color: ColorRgba::WHITE,
        border_width: 1.0,
        border_color: ColorRgba::rgb(50, 50, 50),
        border_radius: 4.0,
        opacity: 1.0,
        font_size: 14.0,
        z_index: 0,
    };

    // Deliberately registered in reverse priority order to prove cascade priority wins over order.
    let rules = [
        StateStyleRule::new(
            InteractionState::DISABLED,
            StylePatch::new()
                .bg(ColorRgba::rgb(100, 100, 100))
                .opacity(0.4)
                .cursor(CursorKind::NotAllowed),
        ),
        StateStyleRule::new(
            InteractionState::ACTIVE,
            StylePatch::new()
                .bg(ColorRgba::rgb(30, 64, 175))
                .cursor(CursorKind::Grabbing),
        ),
        StateStyleRule::new(
            InteractionState::HOVER | InteractionState::FOCUS_VISIBLE,
            StylePatch::new()
                .bg(ColorRgba::rgb(14, 165, 233))
                .border(3.0, ColorRgba::rgb(250, 204, 21)),
        ),
        StateStyleRule::new(
            InteractionState::FOCUS_VISIBLE,
            StylePatch::new()
                .bg(ColorRgba::rgb(56, 189, 248))
                .border(2.0, ColorRgba::rgb(56, 189, 248)),
        ),
        StateStyleRule::new(
            InteractionState::FOCUS,
            StylePatch::new().border_color(ColorRgba::rgb(99, 102, 241)),
        ),
        StateStyleRule::new(
            InteractionState::HOVER,
            StylePatch::new()
                .bg(ColorRgba::rgb(59, 130, 246))
                .cursor(CursorKind::Pointer),
        ),
    ];

    // No state -> unchanged base.
    assert_eq!(resolve_style(&base, &rules, InteractionState::NONE), base);
    assert_eq!(resolve_cursor(CursorKind::Auto, &rules, InteractionState::NONE), CursorKind::Auto);

    // Hover only.
    let s_hover = resolve_style(&base, &rules, InteractionState::HOVER);
    assert_eq!(s_hover.background, ColorRgba::rgb(59, 130, 246));
    assert_eq!(resolve_cursor(CursorKind::Auto, &rules, InteractionState::HOVER), CursorKind::Pointer);

    // Hover + Focus: Focus border color layers on top of Hover background.
    let s_hf = resolve_style(&base, &rules, InteractionState::HOVER | InteractionState::FOCUS);
    assert_eq!(s_hf.background, ColorRgba::rgb(59, 130, 246));
    assert_eq!(s_hf.border_color, ColorRgba::rgb(99, 102, 241));

    // Hover + Focus + FocusVisible: compound `HOVER | FOCUS_VISIBLE` beats both single HOVER and single FOCUS_VISIBLE.
    let s_hfv = resolve_style(
        &base,
        &rules,
        InteractionState::HOVER | InteractionState::FOCUS | InteractionState::FOCUS_VISIBLE,
    );
    assert_eq!(s_hfv.background, ColorRgba::rgb(14, 165, 233));
    assert_eq!(s_hfv.border_width, 3.0);
    assert_eq!(s_hfv.border_color, ColorRgba::rgb(250, 204, 21));

    // Active beats Hover | FocusVisible on overlapping fields (background + cursor), preserving border.
    let s_act = resolve_style(
        &base,
        &rules,
        InteractionState::HOVER | InteractionState::FOCUS_VISIBLE | InteractionState::ACTIVE,
    );
    assert_eq!(s_act.background, ColorRgba::rgb(30, 64, 175));
    assert_eq!(s_act.border_width, 3.0);
    assert_eq!(
        resolve_cursor(
            CursorKind::Auto,
            &rules,
            InteractionState::HOVER | InteractionState::ACTIVE
        ),
        CursorKind::Grabbing
    );

    // Disabled beats everything on overlapping fields.
    let s_dis = resolve_style(&base, &rules, InteractionState::ALL);
    assert_eq!(s_dis.background, ColorRgba::rgb(100, 100, 100));
    assert_eq!(s_dis.opacity, 0.4);
    assert_eq!(resolve_cursor(CursorKind::Pointer, &rules, InteractionState::ALL), CursorKind::NotAllowed);

    // Layout + style resolution helper applies layout fields too.
    let layout_rules = [StateStyleRule::new(
        InteractionState::HOVER,
        StylePatch::new().gap(12.0).padding(8.0).opacity(0.9),
    )];
    let (l_res, s_res) =
        resolve_layout_and_style(&LayoutSnapshot::default(), &base, &layout_rules, InteractionState::HOVER);
    assert_eq!(l_res.gap, 12.0);
    assert_eq!(l_res.padding.top, 8.0);
    assert_eq!(s_res.opacity, 0.9);

    // >16 rules exercises the Vec fallback path and preserves tie-break declaration order.
    let mut many = Vec::new();
    for i in 0..20u8 {
        many.push(StateStyleRule::new(
            InteractionState::HOVER,
            StylePatch::new().bg(ColorRgba::rgb(i, i, i)),
        ));
    }
    let patch = resolve_patch(&many, InteractionState::HOVER);
    assert_eq!(patch.background, Some(ColorRgba::rgb(19, 19, 19)));
}

#[test]
fn interaction_tracker_handles_hover_drag_cancel_touch_focus_modality_disabled_and_unmount() {
    let mut t = InteractionTracker::new();
    let n10 = NodeId::new(10, 0);
    let n20 = NodeId::new(20, 0);
    let n30 = NodeId::new(30, 0);

    // 1. Hover enter, repeat (no dirty), switch, and leave.
    assert_eq!(t.pointer_move(Some(n10)), vec![n10]);
    assert_eq!(t.state(n10), InteractionState::HOVER);
    assert!(t.pointer_move(Some(n10)).is_empty());
    assert_eq!(t.pointer_move(Some(n20)), vec![n10, n20]);
    assert_eq!(t.state(n10), InteractionState::NONE);
    assert_eq!(t.state(n20), InteractionState::HOVER);

    // 2. Press + click completion on same node.
    assert_eq!(t.pointer_down(Some(n20)), vec![n20]);
    assert_eq!(t.state(n20), InteractionState::HOVER | InteractionState::ACTIVE);
    assert_eq!(t.active_cursor_node(), Some(n20));
    let (clicked, dirty) = t.pointer_up_clicked(Some(n20));
    assert_eq!(clicked, Some(n20));
    assert_eq!(dirty, vec![n20]);
    assert_eq!(t.state(n20), InteractionState::HOVER);

    // 3. Press-cancel on drag-off.
    assert_eq!(t.pointer_down(Some(n20)), vec![n20]);
    assert_eq!(t.active(), Some(n20));
    assert_eq!(t.pointer_move(Some(n30)), vec![n20, n30]);
    assert_eq!(t.active(), None, "dragging off node 20 cancels active press");
    assert_eq!(t.state(n20), InteractionState::NONE);
    assert_eq!(t.state(n30), InteractionState::HOVER);
    let (clicked_after_drag, dirty_up) = t.pointer_up_clicked(Some(n30));
    assert_eq!(clicked_after_drag, None);
    assert!(dirty_up.is_empty());

    // 4. Touch sticky-hover prevention on hybrid devices.
    assert_eq!(t.hovered(), Some(n30));
    assert_eq!(t.touch_tap(Some(n30)), vec![n30]);
    assert_eq!(t.hovered(), None, "touch tap must clear hovered so no sticky hover remains");
    assert_eq!(t.active(), None);

    assert_eq!(t.pointer_move(Some(n10)), vec![n10]);
    assert_eq!(t.touch_start(Some(n20)), vec![n10, n20]);
    assert_eq!(t.hovered(), None);
    assert_eq!(t.active(), Some(n20));
    assert_eq!(t.touch_end(), vec![n20]);
    assert_eq!(t.state(n20), InteractionState::NONE);

    // 5. Focus modality: Pointer -> FOCUS only; Keyboard -> FOCUS | FOCUS_VISIBLE.
    assert_eq!(t.focus_pointer(Some(n10)), vec![n10]);
    assert_eq!(t.focus_modality(), FocusModality::Pointer);
    assert_eq!(t.state(n10), InteractionState::FOCUS);
    assert_eq!(t.focus_keyboard(Some(n10)), vec![n10]);
    assert_eq!(t.focus_modality(), FocusModality::Keyboard);
    assert_eq!(t.state(n10), InteractionState::FOCUS | InteractionState::FOCUS_VISIBLE);
    assert_eq!(t.focus_pointer(Some(n10)), vec![n10]);
    assert_eq!(t.state(n10), InteractionState::FOCUS);
    assert_eq!(t.focus_keyboard(Some(n20)), vec![n10, n20]);
    assert_eq!(t.state(n10), InteractionState::NONE);
    assert_eq!(t.state(n20), InteractionState::FOCUS | InteractionState::FOCUS_VISIBLE);

    // 6. Disabled clears active & focus and rejects new press/focus while keeping hover cursor target.
    assert_eq!(t.pointer_down(Some(n20)), vec![n20]);
    assert_eq!(t.set_disabled(n20, true), vec![n20]);
    assert_eq!(t.active(), None);
    assert_eq!(t.focused(), None);
    assert_eq!(t.state(n20), InteractionState::HOVER | InteractionState::DISABLED);

    assert!(t.pointer_down(Some(n20)).is_empty(), "disabled node cannot become active");
    assert_eq!(t.active(), None);
    assert_eq!(t.focus_keyboard(Some(n10)), vec![n10]);
    assert!(t.focus_keyboard(Some(n20)).is_empty(), "focusing disabled node must not steal focus");
    assert_eq!(t.focused(), Some(n10));

    // 7. Unmount cleanly clears hovered, active, focused, and disabled state.
    assert_eq!(t.remove_node(n10), vec![n10]);
    assert_eq!(t.focused(), None);
    assert_eq!(t.state(n10), InteractionState::NONE);

    assert_eq!(t.remove_node(n20), vec![n20]);
    assert_eq!(t.hovered(), None);
    assert!(!t.is_disabled(n20));
    assert_eq!(t.state(n20), InteractionState::NONE);
    assert!(t.remove_node(NodeId::new(999, 0)).is_empty());
}
