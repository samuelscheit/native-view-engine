//! Span-based typed attribute diffs: literal-leaf edits, shape guards, grouping, coordinates,
//! and binary `RowPatch` lowering (including untagged elements without `test_id`).

use nve_engine::hot_patch_template_key;
use nve_store::{Length, Rgba};
use nve_tables::tables::{
    AlignItems, CursorKind, Display, FlexDirection, Interaction, JustifyContent, Layout, Overflow,
    TextAlign, Typography, Visual, WhiteSpace, WordBreak,
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
fn literal_leaf_edits_patch_and_lower_to_binary_row_patch() {
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
    assert_eq!(
        check(
            "background_color: \"#fff\",",
            "background_color: r#\"#000\"#,",
            "background-color",
            V::String("#000".into())
        )
        .prop(Visual::BACKGROUND),
        Some(Rgba::BLACK)
    );
    assert_eq!(
        check(
            "\"font-weight\": \"400\",",
            "\"font-weight\": \"semibold\",",
            "font-weight",
            V::String("semibold".into())
        )
        .prop(Typography::FONT_WEIGHT),
        Some(600)
    );
    assert_eq!(
        check(
            "width: Length::points(8.0),",
            "width: Length::points(12.0),",
            "width",
            V::LengthPx(12.0)
        )
        .prop(Layout::WIDTH),
        Some(Length::points(12.0))
    );
    assert_eq!(
        check(
            "width: Length::percent(25.0),",
            "width: Length::percent(50.0),",
            "width",
            V::LengthPct(50.0)
        )
        .prop(Layout::WIDTH),
        Some(Length::percent(50.0))
    );
    assert_eq!(
        check(
            "height: nve_store::Length::points(1.0),",
            "height: nve_store::Length::points(2.0),",
            "height",
            V::LengthPx(2.0)
        )
        .prop(Layout::HEIGHT),
        Some(Length::points(2.0))
    );
    assert_eq!(
        check(
            "max_width: Dimension::Points(10.0),",
            "max_width: Dimension::Points(11.0),",
            "max-width",
            V::LengthPx(11.0)
        )
        .prop(Layout::MAX_WIDTH),
        Some(Length::points(11.0))
    );
    assert_eq!(
        check(
            "background_color: Rgba::rgb(10, 20, 30),",
            "background_color: Rgba::rgb(255, 0, 128),",
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
            "color: ColorRgba::rgba(0, 0, 0, 0),",
            "color: ColorRgba::rgba(0, 0, 0, 128),",
            "color",
            V::Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 128
            }
        )
        .prop(Typography::COLOR),
        Some(Rgba::new(0, 0, 0, 128))
    );
    assert_eq!(
        check(
            "flex_direction: FlexDirection::Row,",
            "flex_direction: FlexDirection::Column,",
            "flex-direction",
            enum_v("FlexDirection", "Column")
        )
        .prop(Layout::FLEX_DIRECTION),
        Some(FlexDirection::Column)
    );
    assert_eq!(
        check(
            "justify_content: JustifyContent::FlexStart,",
            "justify_content: JustifyContent::SpaceBetween,",
            "justify-content",
            enum_v("JustifyContent", "SpaceBetween")
        )
        .prop(Layout::JUSTIFY_CONTENT),
        Some(JustifyContent::SpaceBetween)
    );
    assert_eq!(
        check(
            "display: Display::Flex,",
            "display: Display::None,",
            "display",
            enum_v("Display", "None")
        )
        .prop(Layout::DISPLAY),
        Some(Display::None)
    );
    assert_eq!(
        check(
            "\"white-space\": WhiteSpace::Normal,",
            "\"white-space\": WhiteSpace::NoWrap,",
            "white-space",
            enum_v("WhiteSpace", "NoWrap")
        )
        .prop(Typography::WHITE_SPACE),
        Some(WhiteSpace::NoWrap)
    );
    assert_eq!(
        check(
            "\"overflow-x\": Overflow::Visible,",
            "\"overflow-x\": Overflow::Hidden,",
            "overflow-x",
            enum_v("Overflow", "Hidden")
        )
        .prop(Visual::OVERFLOW_X),
        Some(Overflow::Hidden)
    );
    assert_eq!(
        check(
            "\"word-break\": WordBreak::Normal,",
            "\"word-break\": WordBreak::BreakAll,",
            "word-break",
            enum_v("WordBreak", "BreakAll")
        )
        .prop(Typography::WORD_BREAK),
        Some(WordBreak::BreakAll)
    );
    assert_eq!(
        check(
            "align_items: Center,",
            "align_items: FlexEnd,",
            "align-items",
            V::Enum {
                ty: None,
                variant: "FlexEnd".into()
            }
        )
        .prop(Layout::ALIGN_ITEMS),
        Some(AlignItems::FlexEnd)
    );
}

#[test]
fn shape_changing_edits_rebuild_and_keep_the_baseline() {
    let cases = [
        ("font_weight: 400,", "font_weight: FontWeight::Bold,"),
        (
            "box_sizing: BoxSizing::BorderBox,",
            "box_sizing: \"content-box\",",
        ),
        (
            "flex_direction: FlexDirection::Row,",
            "flex_direction: Hidden,",
        ),
        (
            "flex_direction: FlexDirection::Row,",
            "flex_direction: Display::Row,",
        ),
        ("padding: 8.0,", "padding: 8,"),
        ("padding: 8.0,", "padding: 8.0f32,"),
        ("z_index: 1,", "z_index: 1i32,"),
        ("padding: 12,", "padding: Length::points(16.0),"),
        (
            "color: Rgba::rgb(1, 2, 3),",
            "color: ColorRgba::rgb(1, 2, 3),",
        ),
        ("color: Rgba::WHITE,", "color: Rgba::rgb(255, 255, 255),"),
        (
            "width: Length::points(8.0),",
            "width: Length::percent(8.0),",
        ),
        (
            "width: Length::points(8.0),",
            "width: nve_store::Length::points(8.0),",
        ),
        (
            "background: theme.surface.base,",
            "background: theme.radius,",
        ),
        ("background: \"#fff\",", "background: Rgba::WHITE,"),
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
            "padding: Length::points( 8.0 ),",
            "padding: Length::points(8.0),",
        ),
        ("padding: 8.0,", "padding: 8.000 /* same */,"),
        ("padding: /* a */ 8.0,", "padding: 8.0 // b\n,"),
        ("z_index: 0x0F,", "z_index: 15,"),
        ("z_index: 1_000,", "z_index: 1000,"),
        ("color: \"#fff\",", "color: r#\"#fff\"#,"),
        (
            "color: Rgba::rgb(0xff, 0, 0),",
            "color: Rgba::rgb(255, 0, 0),",
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
    assert_eq!(
        messages[0].node_path,
        vec![0, 0, 0, 0],
        "view > view > text > literal"
    );
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
fn template_targets_carry_ordinal_line_column_path_and_test_id() {
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
        (go.old.clone(), go.new.clone()),
        (V::Float(1.0), V::Float(0.25))
    );
    assert_eq!(
        go.to_string(),
        "opacity: 1.0 -> 0.25 (template#3 @10:19 path [0] test_id \"go\")"
    );
}

#[test]
fn typed_only_attributes_and_untagged_elements_lower_to_tier1_while_theme_tokens_rebuild() {
    // Previously CSS-only refused attributes now lower straight into binary `RowPatch`!
    let top_msg = expect_patch(diff_rsx_source(
        "demo.rs",
        &element_with("\"top\": 4.0,"),
        &element_with("\"top\": 6.0,"),
        2,
    ));
    assert_eq!(top_msg.prop(Layout::INSET_TOP), Some(Length::points(6.0)));

    let cursor_msg = expect_patch(diff_rsx_source(
        "demo.rs",
        &element_with("cursor: CursorKind::Pointer,"),
        &element_with("cursor: CursorKind::NotAllowed,"),
        2,
    ));
    assert_eq!(
        cursor_msg.prop(Interaction::CURSOR),
        Some(CursorKind::NotAllowed)
    );

    let ws_msg = expect_patch(diff_rsx_source(
        "demo.rs",
        &element_with("\"white-space\": WhiteSpace::Normal,"),
        &element_with("\"white-space\": WhiteSpace::BreakSpaces,"),
        2,
    ));
    assert_eq!(
        ws_msg.prop(Typography::WHITE_SPACE),
        Some(WhiteSpace::BreakSpaces)
    );

    let align_msg = expect_patch(diff_rsx_source(
        "demo.rs",
        &element_with("text_align: TextAlign::Left,"),
        &element_with("text_align: TextAlign::Start,"),
        2,
    ));
    assert_eq!(align_msg.prop(Typography::TEXT_ALIGN), Some(TextAlign::Start));

    // Untagged elements without `test_id` now lower cleanly to Tier-1 via `(template_key, node_path)`!
    let no_id = "fn a() -> Element {\n    rsx! { view { padding: 1.0 } }\n}\n";
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

    // Theme tokens still require the running app's theme -> Tier-2 rebuild.
    assert_tier2_reason(
        &element_with("background: theme.surface.base,"),
        &element_with("background: theme.surface.raised,"),
        "theme token `theme.surface.raised`",
    );
    let (old, new) = (
        element_with("padding: 1.0, background: theme.surface.base,"),
        element_with("padding: 2.0, background: theme.surface.raised,"),
    );
    let typed = typed_patch(&old, &new).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(typed.map(|p| p.attr_changes.len()), Some(2));
    assert_tier2_reason(&old, &new, "theme token");

    // Text edits need no test_id.
    let text = "fn a() -> Element {\n    rsx! { view { \"Hi\" } }\n}\n";
    let msg = expect_patch(diff_rsx_source("a.rs", text, &text.replace("Hi", "Ho"), 2));
    assert_eq!(msg.target_test_id, None);
}

#[test]
fn ill_typed_values_and_unhonoured_integers_rebuild() {
    let cases = [
        (
            "padding: 12,",
            "padding: 16,",
            "`padding` does not accept 16",
        ),
        (
            "opacity: true,",
            "opacity: false,",
            "`opacity` does not accept false",
        ),
        (
            "flex_direction: FlexDirection::Row,",
            "flex_direction: FlexDirection::Diagonal,",
            "does not accept FlexDirection::Diagonal",
        ),
        (
            "display: Overflow::Hidden,",
            "display: Overflow::Auto,",
            "does not accept Overflow::Auto",
        ),
    ];
    for (old_attr, new_attr, needle) in cases {
        assert_tier2_reason(&element_with(old_attr), &element_with(new_attr), needle);
    }
}
