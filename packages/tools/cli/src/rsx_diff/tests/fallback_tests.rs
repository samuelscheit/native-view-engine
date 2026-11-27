//! Every Tier-2 fallback guard of the span-based typed differ.

use super::{assert_tier2, assert_tier2_reason, element_with};

fn el(attrs: &str) -> String {
    element_with(&format!("            {attrs}"))
}

const HANDLERS: &str = r#"pub fn Demo(show: bool) -> Element {
    let mut count = use_signal(|| 0);
    rsx! {
        view {
            test_id: "root",
            key: "row-1",
            onclick: move |_| count += 1,
            padding: 8.0,
            if show { text { "On" } } else { text { "Off" } }
            for i in 0..3 { text { key: "{i}", "Item" } }
            Card { gap: 4.0, padding: Some(8.0), "In card" }
            text { "Hello" }
        }
    }
}
"#;

#[test]
fn logic_control_flow_handler_identity_and_component_edits_rebuild() {
    let line_of = |needle: &str| {
        let at = HANDLERS.find(needle).unwrap_or(0);
        HANDLERS[..at].bytes().filter(|&b| b == b'\n').count() + 1
    };
    let cases = [
        ("use_signal(|| 0)", "use_signal(|| 1)"),
        ("if show", "if !show"),
        ("0..3", "0..4"),
        ("count += 1", "count += 2"),
        ("key: \"row-1\"", "key: \"row-2\""),
        ("test_id: \"root\"", "test_id: \"root-2\""),
        ("key: \"{i}\"", "key: \"{i}-x\""),
        ("gap: 4.0", "gap: 5.0"),
        ("Some(8.0)", "Some(9.0)"),
    ];
    for (from, to) in cases {
        assert_tier2_reason(
            HANDLERS,
            &HANDLERS.replacen(from, to, 1),
            &format!(
                "structural or logic edit outside patchable rsx! values (line {})",
                line_of(from)
            ),
        );
    }
    // Control-flow text stays patchable; adding an element or a branch does not.
    assert_tier2(
        HANDLERS,
        &HANDLERS.replace("text { \"Hello\" }", "text { \"Hello\" } text { \"New\" }"),
    );
    assert_tier2(HANDLERS, &HANDLERS.replace("else { text { \"Off\" } }", ""));
}

#[test]
fn attribute_added_removed_renamed_or_unknown_rebuilds() {
    let base = el("padding: 8.0,");
    assert_tier2(&base, &el("padding: 8.0,\n            gap: 4.0,"));
    assert_tier2(&base, &el(""));
    assert_tier2(&base, &el("margin: 8.0,"));
    // Trailing comma added after the last attribute value is structural punctuation.
    let last = "pub fn D() -> Element {\n    rsx! { view { padding: 8.0 } }\n}\n";
    assert_tier2(last, &last.replace("8.0 }", "8.0, }"));
    // Unknown / content attributes are not patchable even with constant values.
    assert_tier2(&el("href: \"/a\","), &el("href: \"/b\","));
    assert_tier2(&el("foo_bar: 1.0,"), &el("foo_bar: 2.0,"));
    assert_tier2(&el("disabled: false,"), &el("disabled: true,"));
    assert_tier2(&el("nve_style: s1,"), &el("nve_style: s2,"));
}

#[test]
fn non_constant_or_ill_typed_values_rebuild_with_a_reason() {
    let cases = [
        (
            "background_color: \"{c}\",",
            "background_color: \"{d}\",",
            "baseline value is not a Tier-1 constant: interpolated",
        ),
        (
            "background_color: \"#fff\",",
            "background_color: \"{d}\",",
            "value is not a Tier-1 constant: interpolated",
        ),
        (
            "padding: 8.0,",
            "padding: compute_px(),",
            "call outside the px/pct/rgb/rgba/hex helper allowlist",
        ),
        (
            "padding: compute_px(),",
            "padding: 8.0,",
            "baseline value is not a Tier-1 constant",
        ),
        ("padding: 8.0,", "padding: 8.0 9.0,", "trailing tokens"),
        ("padding: 8.0,", "padding: 8.0 * 2.0,", "trailing tokens"),
        ("padding: 8.0,", "padding: n,", "not a constant"),
        ("padding: 8.0,", "padding: +8.0,", "unary `+`"),
        ("padding: 8.0,", "padding: 1e39,", "not finite"),
        ("padding: 8.0,", "padding: ,", "empty value"),
        ("color: rgb(1, 2, 3),", "color: rgb(1, 2, 300),", "0..=255"),
        (
            "color: hex(0x112233),",
            "color: hex(0x1122),",
            "6- or 8-digit",
        ),
        (
            "padding: 8.0,",
            "padding: FlexDirection::Row,",
            "`padding` does not accept FlexDirection::Row",
        ),
        (
            "opacity: 0.5,",
            "opacity: true,",
            "`opacity` does not accept true",
        ),
        (
            "opacity: 0.5,",
            "opacity: px(1.0),",
            "`opacity` does not accept px(1.0)",
        ),
        (
            "style: \"gap: 1px;\",",
            "style: 8.0,",
            "`style` does not accept 8.0",
        ),
        (
            "font_family: \"Inter\",",
            "font_family: FontFamily::Mono,",
            "`font-family` does not accept",
        ),
        (
            "color: hex(0x112233),",
            "color: 4.0,",
            "`color` does not accept 4.0",
        ),
        (
            "flex_direction: FlexDirection::Row,",
            "flex_direction: AlignItems::Center,",
            "enum type changed from `FlexDirection` to `AlignItems`",
        ),
    ];
    for (from, to, needle) in cases {
        assert_tier2_reason(&el(from), &el(to), needle);
    }
}

#[test]
fn text_interpolation_emptiness_and_kind_changes_rebuild() {
    let text = |t: &str| {
        format!("pub fn T() -> Element {{\n    rsx! {{ view {{ text {{ {t} }} }} }}\n}}\n")
    };
    assert_tier2_reason(
        &text("\"Count {n}\""),
        &text("\"Total {n}\""),
        "interpolated",
    );
    assert_tier2_reason(&text("\"Count\""), &text("\"Count {n}\""), "interpolated");
    assert_tier2_reason(&text("\"\""), &text("\"x\""), "empty static text literal");
    assert_tier2_reason(&text("\"Hi\""), &text("{label}"), "structural");
    assert_tier2_reason(&text("\"Hi\""), &text("\"Hi\" \"there\""), "structural");
}

#[test]
fn syntax_errors_and_edits_outside_rsx_rebuild() {
    let base = el("padding: 8.0,");
    assert_tier2_reason(
        &base,
        &base.replace("8.0,", "8.0, {"),
        "unbalanced delimiter",
    );
    assert_tier2_reason(
        &base,
        &base.replace("\"Body\"", "\"Body"),
        "unterminated string",
    );
    let with_const = format!("const PAD: f32 = 8.0;\n{base}");
    assert_tier2_reason(
        &with_const,
        &with_const.replace("PAD: f32 = 8.0", "PAD: f32 = 9.0"),
        "(line 1)",
    );
    let tail = format!("{base}\nfn helper() -> u8 {{ 1 }}\n");
    assert_tier2(&tail, &tail.replace("{ 1 }", "{ 2 }"));
    assert_tier2(&base, &format!("{base}\nfn extra() {{}}\n"));
    assert_tier2(&format!("{base}\nfn extra() {{}}\n"), &base);
}
