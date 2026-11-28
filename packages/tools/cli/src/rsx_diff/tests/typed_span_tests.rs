//! Span-based typed attribute & `nve_style: TypedStyle::...` builder chain diffs:
//! literal-leaf edits, shape guards, grouping, overlap protection, coordinates, and binary
//! `RowPatch` lowering (including untagged elements without `test_id`).

use nve_engine::hot_patch_template_key;
use nve_store::{Length, Rgba};
use nve_tables::tables::{
    AlignItems, CursorKind, Display, FlexDirection, Interaction, JustifyContent, Layout, Overflow,
    Position, TextAlign, TextDecoration, Typography, Visual, WhiteSpace,
};

use super::super::DiffOutcome;
use super::super::eval::TypedAttrValue as V;
use super::{
    assert_tier2_reason, diff_rsx_source, element_with, expect_patch, expect_tier1, typed_patch,
};

fn enum_v(ty: &str, variant: &str) -> V {
    V::Enum {
        ty: Some(ty.into()),
        variant: variant.into(),
    }
}

#[test]
fn standalone_numeric_and_nve_style_chain_edits_lower_to_binary_row_patch() {
    let check = |old_attr: &str, new_attr: &str, name: &str, typed: V| {
        let (old, new) = (element_with(old_attr), element_with(new_attr));
        let (patch, mut messages) = expect_tier1(diff_rsx_source("demo.rs", &old, &new, 7));
        assert_eq!(messages.len(), 1, "{old_attr} -> {new_attr}");
        let msg = messages.remove(0);
        assert_eq!(msg.target_test_id.as_deref(), Some("box"));
        assert_eq!(msg.node_path, vec![0]);
        assert_eq!(msg.generation, 7);
        assert_eq!(msg.template_key, hot_patch_template_key("demo.rs", 0));
        assert!(msg.text_replacements.is_empty());
        let [change] = patch.attr_changes.as_slice() else {
            panic!("expected one typed change for {old_attr} -> {new_attr}: {patch:?}");
        };
        assert_eq!(change.attr.name, name);
        assert_eq!(change.new, typed, "{old_attr} -> {new_attr}");
        msg
    };

    // Standalone numeric attrs handled directly by `NveMutationWriter`.
    assert_eq!(
        check("margin: 8.0,", "margin: -8.0,", "margin", V::Float(-8.0)).prop(Layout::MARGIN_TOP),
        Some(Length::points(-8.0))
    );
    assert_eq!(
        check("padding: 8.0,", "padding: -4.5,", "padding", V::Float(-4.5))
            .prop(Layout::PADDING_TOP),
        Some(Length::points(0.0))
    );
    assert_eq!(
        check("width: 8.0,", "width: 120.5,", "width", V::Float(120.5)).prop(Layout::WIDTH),
        Some(Length::points(120.5))
    );
    assert_eq!(
        check(
            "\"min-height\": 4.0,",
            "\"min-height\": 6.0,",
            "min-height",
            V::Float(6.0)
        )
        .prop(Layout::MIN_HEIGHT),
        Some(Length::points(6.0))
    );
    assert_eq!(
        check("font_size: 14.0,", "font_size: 0.25,", "font-size", V::Float(0.25))
            .prop(Typography::FONT_SIZE),
        Some(1.0)
    );
    assert_eq!(
        check("opacity: 1.0,", "opacity: 1.5,", "opacity", V::Float(1.5)).prop(Visual::OPACITY),
        Some(1.0)
    );
    assert_eq!(
        check("flex: 1.0,", "flex: 2.5,", "flex", V::Float(2.5)).prop(Layout::FLEX_GROW),
        Some(2.5)
    );
    assert_eq!(
        check("flex_shrink: 1.0,", "flex_shrink: -1.0,", "flex-shrink", V::Float(-1.0))
            .prop(Layout::FLEX_SHRINK),
        Some(0.0)
    );
    assert_eq!(
        check("z_index: 1,", "z_index: -3,", "z-index", V::Int(-3)).prop(Visual::Z_INDEX),
        Some(-3)
    );
    assert_eq!(
        check("z_index: 1.0,", "z_index: 2.9,", "z-index", V::Float(2.9)).prop(Visual::Z_INDEX),
        Some(2)
    );
    assert_eq!(
        check(
            "border_radius: 4.0,",
            "border_radius: 6.25,",
            "border-radius",
            V::Float(6.25)
        )
        .prop(Visual::BORDER_RADIUS),
        Some(6.25)
    );
    assert_eq!(
        check("gap: 4.0e0,", "gap: 1.5e1,", "gap", V::Float(15.0)).prop(Layout::ROW_GAP),
        Some(Length::points(15.0))
    );

    // `nve_style: TypedStyle::...` literal builder chains.
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().width(Dimension::Points(8.0)),",
            "nve_style: TypedStyle::new().width(Dimension::Points(12.0)),",
            "width",
            V::LengthPx(12.0)
        )
        .prop(Layout::WIDTH),
        Some(Length::points(12.0))
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().width_pct(25.0),",
            "nve_style: TypedStyle::new().width_pct(50.0),",
            "width",
            V::Float(50.0)
        )
        .prop(Layout::WIDTH),
        Some(Length::percent(50.0))
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().padding_xy(10.0, 6.0),",
            "nve_style: TypedStyle::new().padding_xy(18.0, 6.0),",
            "padding-horizontal",
            V::Float(18.0)
        )
        .prop(Layout::PADDING_LEFT),
        Some(Length::points(18.0))
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().margin_xy(4.0, 8.0),",
            "nve_style: TypedStyle::new().margin_xy(4.0, -2.0),",
            "margin-vertical",
            V::Float(-2.0)
        )
        .prop(Layout::MARGIN_TOP),
        Some(Length::points(-2.0))
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().bg(ColorRgba::rgb(10, 20, 30)),",
            "nve_style: TypedStyle::new().bg(ColorRgba::rgb(255, 0, 128)),",
            "background-color",
            V::Rgba {
                r: 255,
                g: 0,
                b: 128,
                a: 255
            }
        )
        .prop(Visual::BACKGROUND),
        Some(Rgba::rgb(255, 0, 128))
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().border(1.0, ColorRgba::rgb(1, 2, 3)),",
            "nve_style: TypedStyle::new().border(1.0, ColorRgba::rgb(4, 5, 6)),",
            "border-color",
            V::Rgba { r: 4, g: 5, b: 6, a: 255 }
        )
        .prop(Visual::BORDER_COLOR),
        Some(Rgba::rgb(4, 5, 6))
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().color(ColorRgba::rgba(0, 0, 0, 0)),",
            "nve_style: TypedStyle::new().color(ColorRgba::rgba(0, 0, 0, 128)),",
            "color",
            V::Rgba { r: 0, g: 0, b: 0, a: 128 }
        )
        .prop(Typography::COLOR),
        Some(Rgba::new(0, 0, 0, 128))
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().font_weight(400),",
            "nve_style: TypedStyle::new().font_weight(600),",
            "font-weight",
            V::Int(600)
        )
        .prop(Typography::FONT_WEIGHT),
        Some(600)
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().flex_direction(FlexDirection::Row),",
            "nve_style: TypedStyle::new().flex_direction(FlexDirection::Column),",
            "flex-direction",
            enum_v("FlexDirection", "Column")
        )
        .prop(Layout::FLEX_DIRECTION),
        Some(FlexDirection::Column)
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().justify(FlexJustify::FlexStart),",
            "nve_style: TypedStyle::new().justify(FlexJustify::SpaceBetween),",
            "justify-content",
            enum_v("FlexJustify", "SpaceBetween")
        )
        .prop(Layout::JUSTIFY_CONTENT),
        Some(JustifyContent::SpaceBetween)
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().align(FlexAlign::Center),",
            "nve_style: TypedStyle::new().align(FlexAlign::FlexEnd),",
            "align-items",
            enum_v("FlexAlign", "FlexEnd")
        )
        .prop(Layout::ALIGN_ITEMS),
        Some(AlignItems::FlexEnd)
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().position(PositionMode::Relative),",
            "nve_style: TypedStyle::new().position(PositionMode::Absolute),",
            "position",
            enum_v("PositionMode", "Absolute")
        )
        .prop(Layout::POSITION),
        Some(Position::Absolute)
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().display(Display::Flex),",
            "nve_style: TypedStyle::new().display(Display::None),",
            "display",
            enum_v("Display", "None")
        )
        .prop(Layout::DISPLAY),
        Some(Display::None)
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().white_space(WhiteSpace::Normal),",
            "nve_style: TypedStyle::new().white_space(WhiteSpace::NoWrap),",
            "white-space",
            enum_v("WhiteSpace", "NoWrap")
        )
        .prop(Typography::WHITE_SPACE),
        Some(WhiteSpace::NoWrap)
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().overflow_x(Overflow::Visible),",
            "nve_style: TypedStyle::new().overflow_x(Overflow::Hidden),",
            "overflow-x",
            enum_v("Overflow", "Hidden")
        )
        .prop(Visual::OVERFLOW_X),
        Some(Overflow::Hidden)
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().text_decoration(TextDecoration::EMPTY),",
            "nve_style: TypedStyle::new().text_decoration(TextDecoration::UNDERLINE),",
            "text-decoration",
            enum_v("TextDecoration", "UNDERLINE")
        )
        .prop(Typography::TEXT_DECORATION),
        Some(TextDecoration::UNDERLINE)
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().inset_top_px(4.0),",
            "nve_style: TypedStyle::new().inset_top_px(-6.5),",
            "top",
            V::Float(-6.5)
        )
        .prop(Layout::INSET_TOP),
        Some(Length::points(-6.5))
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().cursor(CursorKind::Pointer),",
            "nve_style: TypedStyle::new().cursor(CursorKind::NotAllowed),",
            "cursor",
            enum_v("CursorKind", "NotAllowed")
        )
        .prop(Interaction::CURSOR),
        Some(CursorKind::NotAllowed)
    );
    assert_eq!(
        check(
            "nve_style: TypedStyle::new().text_align(TextAlign::Left),",
            "nve_style: TypedStyle::new().text_align(TextAlign::Start),",
            "text-align",
            enum_v("TextAlign", "Start")
        )
        .prop(Typography::TEXT_ALIGN),
        Some(TextAlign::Start)
    );
}

#[test]
fn nve_style_chain_patches_literals_beside_theme_tokens_and_guards_merges_and_overrides() {
    // 1. Editing `.padding(8.0)` -> `.padding(16.0)` in a chain that ALSO uses `theme.surface.base`
    //    succeeds at Tier-1 because the `theme.*` argument is untouched!
    let base = element_with(
        "nve_style: TypedStyle::col().bg(theme.surface.base).padding(8.0).radius(6.0),",
    );
    let edited = base.replace(".padding(8.0)", ".padding(16.0)");
    let msg = expect_patch(diff_rsx_source("demo.rs", &base, &edited, 2));
    assert_eq!(msg.prop(Layout::PADDING_TOP), Some(Length::points(16.0)));

    // 2. Editing the `theme.*` argument itself cleanly falls back to Tier-2!
    let theme_edit = base.replace("theme.surface.base", "theme.surface.raised");
    assert_tier2_reason(&base, &theme_edit, "theme token `theme.surface.raised`");

    // 3. Editing a call BEFORE `.merge_opt(nve_style.as_ref())` must NOT falsely Tier-1 patch
    //    (because caller's `nve_style` may override it -> falls back to Tier-2).
    let merged = element_with(
        "nve_style: TypedStyle::col().padding(8.0).merge_opt(nve_style.as_ref()),",
    );
    assert_tier2_reason(
        &merged,
        &merged.replace(".padding(8.0)", ".padding(12.0)"),
        "structural or logic edit",
    );

    // 4. Editing a shorthand `.padding(8.0)` followed by `.padding_left(4.0)` in the same chain
    //    must NOT clobber `.padding_left(4.0)` -> falls back to Tier-2; editing `.padding_left(4.0)`
    //    itself IS Tier-1!
    let shadowed_chain = element_with(
        "nve_style: TypedStyle::new().padding(8.0).padding_left(4.0),",
    );
    assert_tier2_reason(
        &shadowed_chain,
        &shadowed_chain.replace(".padding(8.0)", ".padding(12.0)"),
        "structural or logic edit",
    );
    let leaf_msg = expect_patch(diff_rsx_source(
        "demo.rs",
        &shadowed_chain,
        &shadowed_chain.replace(".padding_left(4.0)", ".padding_left(10.0)"),
        3,
    ));
    assert_eq!(leaf_msg.prop(Layout::PADDING_LEFT), Some(Length::points(10.0)));

    // 5. Standalone `width: 240.0` shadows `nve_style: TypedStyle::new().width_px(100.0)` ->
    //    editing `.width_px(100.0)` falls back to Tier-2 while `.radius(4.0)` & `width: 240.0` stay Tier-1!
    let with_standalone = element_with(
        "nve_style: TypedStyle::new().width_px(100.0).radius(4.0),\n            width: 240.0,",
    );
    assert_tier2_reason(
        &with_standalone,
        &with_standalone.replace(".width_px(100.0)", ".width_px(120.0)"),
        "structural or logic edit",
    );
    let rad_msg = expect_patch(diff_rsx_source(
        "demo.rs",
        &with_standalone,
        &with_standalone.replace(".radius(4.0)", ".radius(8.0)"),
        4,
    ));
    assert_eq!(rad_msg.prop(Visual::BORDER_RADIUS), Some(8.0));
}

#[test]
fn dead_standalone_attrs_are_rejected_as_tier2() {
    // Standalone attrs that `dioxus_elements` / `NveMutationWriter` do NOT handle must NOT match Tier-1.
    for (old_attr, new_attr) in [
        ("background_color: \"#fff\",", "background_color: \"#000\","),
        ("color: Rgba::WHITE,", "color: Rgba::BLACK,"),
        ("flex_direction: FlexDirection::Row,", "flex_direction: FlexDirection::Column,"),
        ("cursor: CursorKind::Pointer,", "cursor: CursorKind::Grab,"),
        ("display: Display::Flex,", "display: Display::None,"),
        ("padding_top: 4.0,", "padding_top: 8.0,"),
    ] {
        assert_tier2_reason(
            &element_with(old_attr),
            &element_with(new_attr),
            "structural or logic edit",
        );
    }
}

#[test]
fn shape_changing_edits_rebuild_and_keep_the_baseline() {
    let cases = [
        (
            "nve_style: TypedStyle::new().font_weight(400),",
            "nve_style: TypedStyle::new().font_weight(FontWeight::Bold),",
        ),
        (
            "nve_style: TypedStyle::new().flex_direction(FlexDirection::Row),",
            "nve_style: TypedStyle::new().flex_direction(Hidden),",
        ),
        (
            "nve_style: TypedStyle::new().flex_direction(FlexDirection::Row),",
            "nve_style: TypedStyle::new().flex_direction(Display::Row),",
        ),
        ("padding: 8.0,", "padding: 8,"),
        ("padding: 8.0,", "padding: 8.0f32,"),
        ("z_index: 1,", "z_index: 1i32,"),
        (
            "nve_style: TypedStyle::new().color(Rgba::rgb(1, 2, 3)),",
            "nve_style: TypedStyle::new().color(ColorRgba::rgb(1, 2, 3)),",
        ),
        (
            "nve_style: TypedStyle::new().color(ColorRgba::WHITE),",
            "nve_style: TypedStyle::new().color(ColorRgba::rgb(255, 255, 255)),",
        ),
        (
            "nve_style: TypedStyle::new().width(Dimension::Points(8.0)),",
            "nve_style: TypedStyle::new().width(Dimension::Percent(8.0)),",
        ),
        (
            "nve_style: TypedStyle::new().bg(theme.surface.base),",
            "nve_style: TypedStyle::new().bg(theme.radius),",
        ),
    ];
    for (old_attr, new_attr) in cases {
        assert_tier2_reason(
            &element_with(old_attr),
            &element_with(new_attr),
            "value shape changed",
        );
    }
}

#[test]
fn semantically_identical_rewrites_are_noop() {
    let pairs = [
        (
            "nve_style: TypedStyle::new().width(Dimension::Points( 8.0 )),",
            "nve_style: TypedStyle::new().width(Dimension::Points(8.0)),",
        ),
        ("padding: 8.0,", "padding: 8.000 /* same */,"),
        ("padding: /* a */ 8.0,", "padding: 8.0 // b\n,"),
        ("z_index: 0x0F,", "z_index: 15,"),
        ("z_index: 1_000,", "z_index: 1000,"),
        (
            "nve_style: TypedStyle::new().color(ColorRgba::rgb(0xff, 0, 0)),",
            "nve_style: TypedStyle::new().color(ColorRgba::rgb(255, 0, 0)),",
        ),
        ("margin: -0.0,", "margin: 0.0,"),
    ];
    for (old_attr, new_attr) in pairs {
        assert_eq!(
            diff_rsx_source(
                "demo.rs",
                &element_with(old_attr),
                &element_with(new_attr),
                2
            ),
            DiffOutcome::NoOp,
            "{old_attr} -> {new_attr}"
        );
    }
}

#[test]
fn edits_group_per_element_in_first_appearance_order_with_text_messages_first() {
    let base = r#"pub fn Two() -> Element {
    rsx! {
        view {
            test_id: "outer",
            padding: 8.0,
            view {
                test_id: "inner",
                width: 10.0,
                text { "Inner" }
            }
            gap: 4.0,
        }
    }
}
"#;
    let edited = base
        .replace("padding: 8.0", "padding: 16.0")
        .replace("gap: 4.0", "gap: 6.0")
        .replace("width: 10.0", "width: 25.0")
        .replace("\"Inner\"", "\"Inner text\"");
    let (patch, messages) = expect_tier1(diff_rsx_source("two.rs", base, &edited, 2));
    assert_eq!(patch.attr_changes.len(), 3);
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0].text_replacements,
        vec![("Inner".into(), "Inner text".into())]
    );
    assert_eq!(messages[0].target_test_id.as_deref(), Some("inner"));
    assert_eq!(messages[0].node_path, vec![0, 0, 0, 0]);
    assert_eq!(
        messages[1].prop(Layout::PADDING_TOP),
        Some(Length::points(16.0))
    );
    assert_eq!(messages[1].prop(Layout::ROW_GAP), Some(Length::points(6.0)));
    assert_eq!(messages[1].target_test_id.as_deref(), Some("outer"));
    assert_eq!(messages[2].prop(Layout::WIDTH), Some(Length::points(25.0)));
    assert_eq!(messages[2].target_test_id.as_deref(), Some("inner"));
    assert_eq!(messages[2].node_path, vec![0, 0]);
}

#[test]
fn template_targets_and_untagged_elements_carry_exact_coordinates() {
    let base = "fn header() -> Element {\n    rsx! { text { \"Head\" } }\n}\n\nfn body(on: bool) -> Element {\n    let inner = rsx! { view { test_id: \"leaf\", padding: 1.0 } };\n    rsx! {\n        view {\n            view { \"x\" }\n            if on {\n                pressable { test_id: \"go\", opacity: 1.0, \"Go\" }\n            }\n        }\n    }\n}\n";
    let edited = base
        .replace("opacity: 1.0", "opacity: 0.25")
        .replace("padding: 1.0", "padding: 2.0");
    let patch = typed_patch(base, &edited)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("expected a typed patch"));
    let [leaf, go] = patch.attr_changes.as_slice() else {
        panic!("expected two changes: {patch:?}");
    };
    let coord = |c: &super::super::patch::TypedAttrChange| {
        (
            c.target.template.ordinal,
            c.target.template.line,
            c.target.template.column,
        )
    };
    assert_eq!(coord(leaf), (1, 6, 17));
    assert_eq!(leaf.target.node_path, vec![0]);
    assert_eq!(leaf.target.test_id.as_deref(), Some("leaf"));
    assert_eq!(coord(go), (3, 10, 19));
    assert_eq!(go.target.node_path, vec![0]);
    assert_eq!(go.target.test_id.as_deref(), Some("go"));
    assert_eq!(
        go.to_string(),
        "opacity: 1.0 -> 0.25 (template#3 @10:19 path [0] test_id \"go\")"
    );

    // Untagged elements without `test_id` lower cleanly to Tier-1 via `(template_key, node_path)`.
    let no_id = "fn a() -> Element {\n    rsx! { view { nve_style: TypedStyle::new().padding(1.0) } }\n}\n";
    let no_id_msg = expect_patch(diff_rsx_source(
        "a.rs",
        no_id,
        &no_id.replace("1.0", "2.0"),
        2,
    ));
    assert_eq!(no_id_msg.target_test_id, None);
    assert_eq!(no_id_msg.template_key, hot_patch_template_key("a.rs", 0));
    assert_eq!(no_id_msg.node_path, vec![0]);
    assert_eq!(
        no_id_msg.prop(Layout::PADDING_TOP),
        Some(Length::points(2.0))
    );
}

#[test]
fn ill_typed_values_and_unhonoured_integers_rebuild() {
    let cases = [
        ("padding: 12,", "padding: 16,", "`padding` does not accept 16"),
        ("opacity: true,", "opacity: false,", "`opacity` does not accept false"),
        (
            "nve_style: TypedStyle::new().flex_direction(FlexDirection::Row),",
            "nve_style: TypedStyle::new().flex_direction(FlexDirection::Diagonal),",
            "does not accept FlexDirection::Diagonal",
        ),
        (
            "nve_style: TypedStyle::new().display(Overflow::Hidden),",
            "nve_style: TypedStyle::new().display(Overflow::Auto),",
            "does not accept Overflow::Auto",
        ),
    ];
    for (old_attr, new_attr, needle) in cases {
        assert_tier2_reason(&element_with(old_attr), &element_with(new_attr), needle);
    }
}
