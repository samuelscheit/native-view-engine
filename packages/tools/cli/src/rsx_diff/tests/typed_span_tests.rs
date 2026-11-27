//! Span-based typed attribute diffs: variable token counts, lowering, coordinates, registry.

use super::super::attrs::{PATCHABLE_ATTR_NAMES, lookup_patchable_attr};
use super::super::eval::TypedAttrValue as V;
use super::super::matcher::diff_rsx_typed;
use super::super::{DiffOutcome, diff_rsx_source};
use super::{assert_tier2_reason, element_with, expect_patch, expect_tier1};

fn rgba(r: u8, g: u8, b: u8, a: u8) -> V {
    V::Rgba { r, g, b, a }
}

fn enum_v(ty: &str, variant: &str) -> V {
    V::Enum {
        ty: Some(ty.into()),
        variant: variant.into(),
    }
}

#[test]
fn typed_value_edits_with_changing_token_counts_patch_and_lower_to_css() {
    let cases: Vec<(&str, &str, &str, &str, V)> = vec![
        (
            "margin: 8.0,",
            "margin: -8.0,",
            "margin",
            "-8px",
            V::Float(-8.0),
        ),
        (
            "width: px(8),",
            "width: pct(50.0),",
            "width",
            "50%",
            V::LengthPct(50.0),
        ),
        (
            "background_color: 12,",
            "background_color: Rgba::rgb(255, 0, 128),",
            "background-color",
            "#ff0080",
            rgba(255, 0, 128, 255),
        ),
        (
            "color: Rgba::rgb(10, 20, 30),",
            "color: hex(0x112233),",
            "color",
            "#112233",
            rgba(0x11, 0x22, 0x33, 255),
        ),
        (
            "flex_direction: FlexDirection::Row,",
            "flex_direction: FlexDirection::Column,",
            "flex-direction",
            "column",
            enum_v("FlexDirection", "Column"),
        ),
        (
            "justify_content: JustifyContent::Start,",
            "justify_content: JustifyContent::SpaceBetween,",
            "justify-content",
            "space-between",
            enum_v("JustifyContent", "SpaceBetween"),
        ),
        (
            "white_space: WhiteSpace::Normal,",
            "white_space: WhiteSpace::NoWrap,",
            "white-space",
            "nowrap",
            enum_v("WhiteSpace", "NoWrap"),
        ),
        (
            "cursor: CursorKind::Pointer,",
            "cursor: CursorKind::NotAllowed,",
            "cursor",
            "not-allowed",
            enum_v("CursorKind", "NotAllowed"),
        ),
        (
            "align_self: Center,",
            "align_self: FlexEnd,",
            "align-self",
            "flex-end",
            V::Enum {
                ty: None,
                variant: "FlexEnd".into(),
            },
        ),
        (
            "opacity: 1,",
            "opacity: 0.5,",
            "opacity",
            "0.5",
            V::Float(0.5),
        ),
        ("z_index: 1,", "z_index: -3,", "z-index", "-3", V::Int(-3)),
        (
            "font_weight: 400,",
            "font_weight: FontWeight::Bold,",
            "font-weight",
            "bold",
            enum_v("FontWeight", "Bold"),
        ),
        (
            "font_weight: 400,",
            "font_weight: 700,",
            "font-weight",
            "700",
            V::Int(700),
        ),
        (
            "line_height: 1.5,",
            "line_height: px(20.0),",
            "line-height",
            "20px",
            V::LengthPx(20.0),
        ),
        (
            "line_height: 1.5,",
            "line_height: 1.25,",
            "line-height",
            "1.25",
            V::Float(1.25),
        ),
        (
            "height: Length::Auto,",
            "height: Length::Px(120.0),",
            "height",
            "120px",
            V::LengthPx(120.0),
        ),
        (
            "max_width: Dimension::Points(10.0),",
            "max_width: Length::Auto,",
            "max-width",
            "auto",
            V::Auto,
        ),
        (
            "\"padding-top\": 4.0,",
            "\"padding-top\": 6.0,",
            "padding-top",
            "6px",
            V::Float(6.0),
        ),
        (
            "padding_left: 4,",
            "padding_left: 1_000,",
            "padding-left",
            "1000px",
            V::Int(1000),
        ),
        (
            "border_top_width: 1.0,",
            "border_top_width: 2.5,",
            "border-top-width",
            "2.5px",
            V::Float(2.5),
        ),
        (
            "font_family: \"Inter\",",
            "font_family: \"JetBrains Mono\",",
            "font-family",
            "JetBrains Mono",
            V::String("JetBrains Mono".into()),
        ),
        ("flex: 1,", "flex: 2.5,", "flex", "2.5", V::Float(2.5)),
        (
            "aspect_ratio: 1.5,",
            "aspect_ratio: 2,",
            "aspect-ratio",
            "2",
            V::Int(2),
        ),
        (
            "background: ColorRgba::WHITE,",
            "background: ColorRgba::rgba(0, 0, 0, 128),",
            "background",
            "#00000080",
            rgba(0, 0, 0, 128),
        ),
        (
            "border_color: rgb(1, 2, 3),",
            "border_color: rgba(1, 2, 3, 0),",
            "border-color",
            "#01020300",
            rgba(1, 2, 3, 0),
        ),
        ("gap: 4.0,", "gap: 8.0f32,", "gap", "8px", V::Float(8.0)),
        (
            "top: px(0.0),",
            "top: px(-1e-3),",
            "top",
            "-0.001px",
            V::LengthPx(-0.001),
        ),
        (
            "display: Display::Flex,",
            "display: Display::InlineBlock,",
            "display",
            "inline-block",
            enum_v("Display", "InlineBlock"),
        ),
        (
            "box_sizing: BoxSizing::BorderBox,",
            "box_sizing: \"content-box\",",
            "box-sizing",
            "content-box",
            V::String("content-box".into()),
        ),
    ];
    for (old_attr, new_attr, name, css, typed) in cases {
        let old = element_with(&format!("            {old_attr}"));
        let new = element_with(&format!("            {new_attr}"));
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
        assert!(msg.text_updates.is_empty() && msg.text_replacements.is_empty());
        let [change] = patch.attr_changes.as_slice() else {
            panic!("expected one typed change for {old_attr} -> {new_attr}: {patch:?}");
        };
        assert_eq!(change.attr.name, name);
        assert_eq!(change.new, typed, "{old_attr} -> {new_attr}");
    }
}

#[test]
fn semantically_identical_value_rewrites_are_noop() {
    let pairs = [
        ("padding: px( 8.0 ),", "padding: px(8.0),"),
        ("padding: 8.0,", "padding: 8.0f32,"),
        ("padding: 8.0,", "padding: 8.000 /* same */,"),
        ("z_index: 0x0F,", "z_index: 15,"),
        ("z_index: 1_000,", "z_index: 1000i64,"),
        ("color: \"#fff\",", "color: r#\"#fff\"#,"),
        ("color: hex(0xffffff),", "color: Rgba::rgb(255, 255, 255),"),
        (
            "color: ColorRgba::WHITE,",
            "color: rgba(255, 255, 255, 255),",
        ),
        ("width: Length::Percent(50.0),", "width: pct(50),"),
        (
            "flex_direction: FlexDirection::Row,",
            "flex_direction: nve::FlexDirection::Row,",
        ),
        ("margin: -0.0,", "margin: 0.0,"),
    ];
    for (old_attr, new_attr) in pairs {
        let old = element_with(&format!("            {old_attr}"));
        let new = element_with(&format!("            {new_attr}"));
        assert_eq!(
            diff_rsx_source("demo.rs", &old, &new, 2),
            DiffOutcome::NoOp,
            "{old_attr} -> {new_attr}"
        );
    }
}

#[test]
fn several_typed_edits_group_per_element_with_text_in_its_own_message() {
    let base = r#"pub fn Two() -> Element {
    rsx! {
        view {
            test_id: "outer",
            padding: 8.0,
            gap: 4.0,
            view {
                width: px(10.0),
                text { "Inner" }
            }
        }
    }
}
"#;
    let edited = base
        .replace("padding: 8.0", "padding: px(16.0)")
        .replace("gap: 4.0", "gap: 6.0")
        .replace("px(10.0)", "pct(25.0)")
        .replace("\"Inner\"", "\"Inner text\"");
    let (patch, messages) = expect_tier1(diff_rsx_source("two.rs", base, &edited, 2));
    assert_eq!(patch.attr_changes.len(), 3);
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0].text_replacements,
        vec![("Inner".into(), "Inner text".into())]
    );
    assert_eq!(
        messages[0].target_test_id, None,
        "inner view has no test_id"
    );
    assert_eq!(
        messages[0].node_path,
        vec![0, 0, 0, 0],
        "view > view > text > literal"
    );
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
        vec![("width".into(), "25%".into())]
    );
    assert_eq!(messages[2].target_test_id, None);
    assert_eq!(messages[2].node_path, vec![0, 0]);
}

#[test]
fn template_targets_carry_block_ordinal_line_column_anchor_path_and_test_id() {
    let base = "fn header() -> Element {\n    rsx! { text { \"Head\" } }\n}\n\nfn body() -> Element {\n    let inner = rsx! { view { test_id: \"leaf\", padding: 1.0 } };\n    rsx! {\n        view {\n            view { \"x\" }\n            pressable { test_id: \"go\", opacity: 1.0, \"Go\" }\n        }\n    }\n}\n";
    let edited = base
        .replace("opacity: 1.0", "opacity: 0.25")
        .replace("padding: 1.0", "padding: 2.0");
    let patch = diff_rsx_typed(base, &edited)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("expected a typed patch"));
    let [leaf, go] = patch.attr_changes.as_slice() else {
        panic!("expected two changes: {patch:?}");
    };
    assert_eq!(
        (
            leaf.target.block.ordinal,
            leaf.target.block.line,
            leaf.target.block.column
        ),
        (1, 6, 17)
    );
    assert_eq!(leaf.target.node_path, vec![0]);
    assert_eq!(leaf.target.test_id.as_deref(), Some("leaf"));
    assert_eq!(
        (
            go.target.block.ordinal,
            go.target.block.line,
            go.target.block.column
        ),
        (2, 7, 5)
    );
    assert_eq!(go.target.node_path, vec![0, 1]);
    assert_eq!(go.target.test_id.as_deref(), Some("go"));
    assert!(leaf.target.anchor_tok < go.target.anchor_tok);
    assert_eq!(go.old, V::Float(1.0));
    assert_eq!(go.new, V::Float(0.25));
    assert_eq!(
        go.to_string(),
        "opacity: 1.0 -> 0.25 (rsx#2 @7:5 path [0, 1] test_id \"go\")"
    );
}

#[test]
fn theme_token_edits_are_typed_but_need_runtime_lowering() {
    let old = element_with("            background: theme.surface.base,");
    let new = element_with("            background: theme.surface.raised,");
    let patch = diff_rsx_typed(&old, &new)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("expected a typed patch"));
    assert_eq!(
        patch.attr_changes[0].new,
        V::ThemeToken("theme.surface.raised".into())
    );
    assert_tier2_reason(&old, &new, "theme token `theme.surface.raised`");

    // Switching from a theme token to a literal color lowers fine.
    let literal = element_with("            background: hex(0x0f172a),");
    let msg = expect_patch(diff_rsx_source("demo.rs", &old, &literal, 2));
    assert_eq!(
        msg.attr_updates,
        vec![("background".into(), "#0f172a".into())]
    );
}

#[test]
fn patchable_attr_registry_covers_snake_and_kebab_and_excludes_identity_and_events() {
    let mut names = PATCHABLE_ATTR_NAMES.to_vec();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        PATCHABLE_ATTR_NAMES.len(),
        "duplicate canonical names"
    );
    for name in PATCHABLE_ATTR_NAMES {
        let spec = lookup_patchable_attr(name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(spec.name, *name);
        let snake = name.replace('-', "_");
        assert_eq!(
            lookup_patchable_attr(&snake).map(|s| s.name),
            Some(*name),
            "{snake}"
        );
    }
    for required in [
        "flex-wrap",
        "align-self",
        "overflow-x",
        "overflow-y",
        "font-family",
        "font-weight",
        "font-style",
        "line-height",
        "text-align",
        "text-decoration",
        "white-space",
        "overflow-wrap",
        "vertical-align",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
        "border",
        "display",
        "position",
        "cursor",
        "top",
        "right",
        "bottom",
        "left",
        "flex-basis",
        "aspect-ratio",
        "box-sizing",
        "style",
    ] {
        assert!(lookup_patchable_attr(required).is_some(), "{required}");
    }
    for excluded in [
        "test_id",
        "testID",
        "data-testid",
        "key",
        "onclick",
        "onpress",
        "on_press",
        "href",
        "src",
        "value",
        "id",
        "class",
        "nve_style",
        "nve_table",
        "disabled",
        "",
        "a_very_long_attribute_name_beyond_the_buffer",
    ] {
        assert!(lookup_patchable_attr(excluded).is_none(), "{excluded}");
    }
}
