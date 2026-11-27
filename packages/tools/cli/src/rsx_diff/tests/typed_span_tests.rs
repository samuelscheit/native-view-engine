//! Span-based typed attribute diffs: literal-leaf edits, shape guards, grouping, coordinates.

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
fn literal_leaf_edits_patch_and_lower_like_the_typed_attribute_path() {
    let cases: Vec<(&str, &str, &str, &str, V)> = vec![
        // Float literals mirror `apply_numeric_attr` (f32, clamps).
        (
            "margin: 8.0,",
            "margin: -8.0,",
            "margin",
            "-8px",
            V::Float(-8.0),
        ),
        (
            "padding: 8.0,",
            "padding: -4.5,",
            "padding",
            "0px",
            V::Float(-4.5),
        ),
        (
            "width: 8.0,",
            "width: 120.5,",
            "width",
            "120.5px",
            V::Float(120.5),
        ),
        (
            "\"min-height\": 4.0,",
            "\"min-height\": 6.0,",
            "min-height",
            "6px",
            V::Float(6.0),
        ),
        (
            "font_size: 14.0,",
            "font_size: 0.25,",
            "font-size",
            "1px",
            V::Float(0.25),
        ),
        (
            "opacity: 1.0,",
            "opacity: 1.5,",
            "opacity",
            "1",
            V::Float(1.5),
        ),
        ("flex: 1.0,", "flex: 2.5,", "flex", "2.5", V::Float(2.5)),
        (
            "flex_shrink: 1.0,",
            "flex_shrink: -1.0,",
            "flex-shrink",
            "0",
            V::Float(-1.0),
        ),
        ("z_index: 1,", "z_index: -3,", "z-index", "-3", V::Int(-3)),
        (
            "z_index: 1.0,",
            "z_index: 2.9,",
            "z-index",
            "2",
            V::Float(2.9),
        ),
        (
            "border_radius: 4.0,",
            "border_radius: 6.25,",
            "border-radius",
            "6.25px",
            V::Float(6.25),
        ),
        ("gap: 4.0e0,", "gap: 1.5e1,", "gap", "15px", V::Float(15.0)),
        // Strings pass through to the same CSS parser the compiled app uses.
        (
            "background_color: \"#fff\",",
            "background_color: r#\"#000\"#,",
            "background-color",
            "#000",
            V::String("#000".into()),
        ),
        (
            "\"font-weight\": \"400\",",
            "\"font-weight\": \"semibold\",",
            "font-weight",
            "semibold",
            V::String("semibold".into()),
        ),
        (
            "style: \"gap: {{1}}px;\",",
            "style: \"gap: 2px;\",",
            "style",
            "gap: 2px;",
            V::String("gap: 2px;".into()),
        ),
        // Typed constructors and keywords (the Track 2 attribute forms).
        (
            "width: Length::points(8.0),",
            "width: Length::points(12.0),",
            "width",
            "12px",
            V::LengthPx(12.0),
        ),
        (
            "width: Length::percent(25.0),",
            "width: Length::percent(50.0),",
            "width",
            "50%",
            V::LengthPct(50.0),
        ),
        (
            "height: nve_store::Length::points(1.0),",
            "height: nve_store::Length::points(2.0),",
            "height",
            "2px",
            V::LengthPx(2.0),
        ),
        (
            "max_width: Dimension::Points(10.0),",
            "max_width: Dimension::Points(11.0),",
            "max-width",
            "11px",
            V::LengthPx(11.0),
        ),
        (
            "padding: Length::points(1.0),",
            "padding: Length::points(-2.0),",
            "padding",
            "-2px",
            V::LengthPx(-2.0),
        ),
        (
            "background_color: Rgba::rgb(10, 20, 30),",
            "background_color: Rgba::rgb(255, 0, 128),",
            "background-color",
            "#ff0080",
            V::Rgba {
                r: 255,
                g: 0,
                b: 128,
                a: 255,
            },
        ),
        (
            "color: ColorRgba::rgba(0, 0, 0, 0),",
            "color: ColorRgba::rgba(0, 0, 0, 128),",
            "color",
            "#00000080",
            V::Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 128,
            },
        ),
        (
            "flex_direction: FlexDirection::Row,",
            "flex_direction: FlexDirection::Column,",
            "flex-direction",
            "column",
            enum_v("FlexDirection", "Column"),
        ),
        (
            "justify_content: JustifyContent::FlexStart,",
            "justify_content: JustifyContent::SpaceBetween,",
            "justify-content",
            "space-between",
            enum_v("JustifyContent", "SpaceBetween"),
        ),
        (
            "display: Display::Flex,",
            "display: Display::None,",
            "display",
            "none",
            enum_v("Display", "None"),
        ),
        (
            "\"white-space\": WhiteSpace::Normal,",
            "\"white-space\": WhiteSpace::NoWrap,",
            "white-space",
            "nowrap",
            enum_v("WhiteSpace", "NoWrap"),
        ),
        (
            "\"overflow-x\": Overflow::Visible,",
            "\"overflow-x\": Overflow::Hidden,",
            "overflow-x",
            "hidden",
            enum_v("Overflow", "Hidden"),
        ),
        (
            "\"word-break\": WordBreak::Normal,",
            "\"word-break\": WordBreak::BreakAll,",
            "word-break",
            "break-all",
            enum_v("WordBreak", "BreakAll"),
        ),
        (
            "align_items: Center,",
            "align_items: FlexEnd,",
            "align-items",
            "flex-end",
            V::Enum {
                ty: None,
                variant: "FlexEnd".into(),
            },
        ),
    ];
    for (old_attr, new_attr, name, css, typed) in cases {
        let (old, new) = (element_with(old_attr), element_with(new_attr));
        let (patch, messages) = expect_tier1(diff_rsx_source("demo.rs", &old, &new, 7));
        assert_eq!(messages.len(), 1, "{old_attr} -> {new_attr}");
        let msg = &messages[0];
        assert_eq!(
            msg.attr_updates,
            vec![(name.to_owned(), css.to_owned())],
            "{old_attr} -> {new_attr}"
        );
        assert_eq!(msg.target_test_id.as_deref(), Some("box"));
        assert_eq!(msg.node_path, vec![0]);
        assert_eq!((msg.generation, msg.template_id.as_str()), (7, "demo.rs"));
        assert!(msg.text_replacements.is_empty());
        let [change] = patch.attr_changes.as_slice() else {
            panic!("expected one typed change for {old_attr} -> {new_attr}: {patch:?}");
        };
        assert_eq!(change.attr.name, name);
        assert_eq!(change.new, typed, "{old_attr} -> {new_attr}");
    }
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
    // Parent / child / parent edits: the outer element's two edits merge into one message.
    assert_eq!(
        messages[1].attr_updates,
        vec![
            ("padding".into(), "16px".into()),
            ("gap".into(), "6px".into())
        ]
    );
    assert_eq!(messages[1].target_test_id.as_deref(), Some("outer"));
    assert_eq!(
        messages[2].attr_updates,
        vec![("width".into(), "25px".into())]
    );
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
    // The `if` body is its own template (ordinal 3, anchored at its `{`).
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
fn typed_only_attributes_theme_tokens_and_untargetable_elements_stay_typed_but_rebuild() {
    let cases = [
        (
            "\"margin-top\": 4.0,",
            "\"margin-top\": 6.0,",
            "`margin-top` has no string TemplatePatch form",
        ),
        (
            "cursor: CursorKind::Pointer,",
            "cursor: CursorKind::NotAllowed,",
            "`cursor` has no string",
        ),
        (
            "background: theme.surface.base,",
            "background: theme.surface.raised,",
            "theme token `theme.surface.raised`",
        ),
        (
            "\"white-space\": WhiteSpace::Normal,",
            "\"white-space\": WhiteSpace::BreakSpaces,",
            "cannot carry WhiteSpace::BreakSpaces",
        ),
        (
            "text_align: TextAlign::Left,",
            "text_align: TextAlign::Start,",
            "cannot carry TextAlign::Start",
        ),
    ];
    for (old_attr, new_attr, needle) in cases {
        let (old, new) = (element_with(old_attr), element_with(new_attr));
        let typed = typed_patch(&old, &new).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            typed.is_some_and(|p| p.attr_changes.len() == 1),
            "{old_attr}"
        );
        assert_tier2_reason(&old, &new, needle);
    }

    // A save mixing a lowerable literal with a theme token keeps both typed, then rebuilds.
    let (old, new) = (
        element_with("padding: 1.0, background: theme.surface.base,"),
        element_with("padding: 2.0, background: theme.surface.raised,"),
    );
    let typed = typed_patch(&old, &new).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(typed.map(|p| p.attr_changes.len()), Some(2));
    assert_tier2_reason(&old, &new, "theme token");

    let no_id = "fn a() -> Element {\n    rsx! { view { padding: 1.0 } }\n}\n";
    assert_tier2_reason(
        no_id,
        &no_id.replace("1.0", "2.0"),
        "without a static `test_id`",
    );
    // Text edits need no test_id: exact old-text replacement.
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
            "\"font-weight\": 400,",
            "\"font-weight\": 700,",
            "`font-weight` does not accept 700",
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
        ("style: 1.0,", "style: 2.0,", "`style` does not accept 2.0"),
    ];
    for (old_attr, new_attr, needle) in cases {
        assert_tier2_reason(&element_with(old_attr), &element_with(new_attr), needle);
    }
}
