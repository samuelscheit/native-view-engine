//! Deep unit & edge-case tests for `nve_tables::interaction` and table-generic `RowPatch`.

use std::mem::{align_of, size_of};

use nve_abi::ColorRgba;
use nve_tables::interaction::{
    resolve_cursor_from_view, resolve_layout_and_style_from_view, resolve_patch_from_view,
    resolve_style_from_view, CursorKind, InteractionState,
    StateStyleRule,
};
use nve_tables::snapshots::{LayoutSnapshot, VisualSnapshot};
use nve_tables::tables::{Display, StateRules, StateRulesView, Visual};
use nve_tables::{RowPatch, StylePatchExt};
use nve_store::{FlatResource, Rgba, ValidatedWords};

fn flat(rules: &[StateStyleRule]) -> Vec<u32> {
    let mut words = Vec::new();
    assert!(StateRules::new(rules.to_vec()).write_words(&mut words));
    words
}

fn view(words: &[u32]) -> StateRulesView<'_> {
    StateRules::view(ValidatedWords::try_new(words).expect("valid StateRules"))
}

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
        assert_eq!(CursorKind::try_from_word(u32::from(raw)), Some(kind));
        assert_eq!(CursorKind::parse_css(kw), Some(kind));
        assert_eq!(CursorKind::parse_css(&format!("  {}  ", kw.to_ascii_uppercase())), Some(kind));
    }
    assert_eq!(CursorKind::try_from_word(13), None);
    assert_eq!(CursorKind::try_from_word(255), None);
    assert_eq!(CursorKind::parse_css("invalid-cursor"), None);
}

#[test]
fn unified_row_patch_resolves_cascade_priority_compounds_and_large_rule_lists() {
    let _ = RowPatch::new().display(Display::None).cursor(CursorKind::Wait);

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
            RowPatch::new()
                .bg(ColorRgba::rgb(100, 100, 100))
                .opacity(0.4)
                .cursor(CursorKind::NotAllowed),
        ),
        StateStyleRule::new(
            InteractionState::ACTIVE,
            RowPatch::new()
                .bg(ColorRgba::rgb(30, 64, 175))
                .cursor(CursorKind::Grabbing),
        ),
        StateStyleRule::new(
            InteractionState::HOVER | InteractionState::FOCUS_VISIBLE,
            RowPatch::new()
                .bg(ColorRgba::rgb(14, 165, 233))
                .border(3.0, ColorRgba::rgb(250, 204, 21)),
        ),
        StateStyleRule::new(
            InteractionState::FOCUS_VISIBLE,
            RowPatch::new()
                .bg(ColorRgba::rgb(56, 189, 248))
                .border(2.0, ColorRgba::rgb(56, 189, 248)),
        ),
        StateStyleRule::new(
            InteractionState::FOCUS,
            RowPatch::new().border_color(ColorRgba::rgb(99, 102, 241)),
        ),
        StateStyleRule::new(
            InteractionState::HOVER,
            RowPatch::new()
                .bg(ColorRgba::rgb(59, 130, 246))
                .cursor(CursorKind::Pointer),
        ),
    ];
    let rules_words = flat(&rules);
    let rules = view(&rules_words);

    // No state -> unchanged base.
    assert_eq!(resolve_style_from_view(&base, rules, InteractionState::NONE), base);
    assert_eq!(resolve_cursor_from_view(CursorKind::Auto, rules, InteractionState::NONE), CursorKind::Auto);

    // Hover only.
    let s_hover = resolve_style_from_view(&base, rules, InteractionState::HOVER);
    assert_eq!(s_hover.background, ColorRgba::rgb(59, 130, 246));
    assert_eq!(resolve_cursor_from_view(CursorKind::Auto, rules, InteractionState::HOVER), CursorKind::Pointer);

    // Hover + Focus: Focus border color layers on top of Hover background.
    let s_hf = resolve_style_from_view(&base, rules, InteractionState::HOVER | InteractionState::FOCUS);
    assert_eq!(s_hf.background, ColorRgba::rgb(59, 130, 246));
    assert_eq!(s_hf.border_color, ColorRgba::rgb(99, 102, 241));

    // Hover + Focus + FocusVisible: compound `HOVER | FOCUS_VISIBLE` beats both single HOVER and single FOCUS_VISIBLE.
    let s_hfv = resolve_style_from_view(
        &base,
        rules,
        InteractionState::HOVER | InteractionState::FOCUS | InteractionState::FOCUS_VISIBLE,
    );
    assert_eq!(s_hfv.background, ColorRgba::rgb(14, 165, 233));
    assert_eq!(s_hfv.border_width, 3.0);
    assert_eq!(s_hfv.border_color, ColorRgba::rgb(250, 204, 21));

    // Active beats Hover | FocusVisible on overlapping fields (background + cursor), preserving border.
    let s_act = resolve_style_from_view(
        &base,
        rules,
        InteractionState::HOVER | InteractionState::FOCUS_VISIBLE | InteractionState::ACTIVE,
    );
    assert_eq!(s_act.background, ColorRgba::rgb(30, 64, 175));
    assert_eq!(s_act.border_width, 3.0);
    assert_eq!(
        resolve_cursor_from_view(
            CursorKind::Auto,
            rules,
            InteractionState::HOVER | InteractionState::ACTIVE
        ),
        CursorKind::Grabbing
    );

    // Disabled beats everything on overlapping fields.
    let s_dis = resolve_style_from_view(&base, rules, InteractionState::ALL);
    assert_eq!(s_dis.background, ColorRgba::rgb(100, 100, 100));
    assert_eq!(s_dis.opacity, 0.4);
    assert_eq!(resolve_cursor_from_view(CursorKind::Pointer, rules, InteractionState::ALL), CursorKind::NotAllowed);

    // Layout + style resolution helper applies layout fields too.
    let layout_rules = [StateStyleRule::new(
        InteractionState::HOVER,
        RowPatch::new().gap(12.0).padding(8.0).opacity(0.9),
    )];
    let (l_res, s_res) =
        resolve_layout_and_style_from_view(&LayoutSnapshot::default(), &base, view(&flat(&layout_rules)), InteractionState::HOVER);
    assert_eq!(l_res.gap, 12.0);
    assert_eq!(l_res.padding.top, 8.0);
    assert_eq!(s_res.opacity, 0.9);

    // >16 rules exercises the Vec fallback path and preserves tie-break declaration order.
    let mut many = Vec::new();
    for i in 0..20u8 {
        many.push(StateStyleRule::new(
            InteractionState::HOVER,
            RowPatch::new().bg(ColorRgba::rgb(i, i, i)),
        ));
    }
    let patch = resolve_patch_from_view(view(&flat(&many)), InteractionState::HOVER);
    assert_eq!(patch.get_prop(Visual::BACKGROUND), Some(Rgba::rgb(19, 19, 19)));
}
