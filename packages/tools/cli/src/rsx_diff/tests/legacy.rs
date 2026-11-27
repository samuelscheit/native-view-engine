//! Static text/string-style Tier-1 behavior carried over from the string-only differ.

use std::path::PathBuf;

use super::super::{DiffOutcome, RsxWatchCache};
use super::{diff_rsx_source, expect_patch, expect_tier1};

pub(super) const SAMPLE_COMPONENT: &str = r##"
use nve_dioxus::prelude::*;

#[component]
pub fn DemoCard() -> Element {
    let mut count = use_signal(|| 0i32);
    rsx! {
        view {
            test_id: "card-root",
            style: "width: 100%; padding: 12px; background-color: #0f172a;",
            text {
                test_id: "channel-title",
                style: "color: #ffffff; font-size: 16px;",
                "# engineering"
            }
            pressable {
                test_id: "cta-btn",
                background_color: "#2563eb",
                onpress: move |_| count += 1,
                text { "Count: {count}" }
                text { "Ship It" }
            }
        }
    }
}
"##;

fn assert_tier2(template_id: &str, old_src: &str, new_src: &str, generation: u64) {
    assert!(matches!(
        diff_rsx_source(template_id, old_src, new_src, generation),
        DiffOutcome::RequireTier2Rebuild { .. }
    ));
}

#[test]
fn tier1_text_only_edit_inside_rsx_emits_targeted_replacement() {
    let updated = SAMPLE_COMPONENT.replace("\"# engineering\"", "\"# ergoiherg\"");
    let patch = expect_patch(diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated, 2));
    assert_eq!(patch.generation, 2);
    assert_eq!(patch.template_id, "demo.rs");
    assert!(patch.attr_updates.is_empty());
    assert_eq!(
        patch.text_replacements,
        vec![("# engineering".into(), "# ergoiherg".into())]
    );
    assert!(patch.text_updates.is_empty());
    assert_eq!(patch.target_test_id.as_deref(), Some("channel-title"));
    assert_eq!(patch.node_path, vec![0, 0, 0]);
}

#[test]
fn tier1_style_and_attr_literal_edits_target_exact_test_id() {
    let updated = SAMPLE_COMPONENT.replace(
        "width: 100%; padding: 12px; background-color: #0f172a;",
        "width: 100%; padding: 24px; background-color: #1e293b;",
    );
    let patch = expect_patch(diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated, 3));
    assert_eq!(
        patch.attr_updates,
        vec![(
            "style".into(),
            "width: 100%; padding: 24px; background-color: #1e293b;".into()
        )]
    );
    assert!(patch.text_replacements.is_empty());
    assert_eq!(patch.target_test_id.as_deref(), Some("card-root"));
    assert_eq!(patch.node_path, vec![0]);

    // Direct typed attribute (`background_color` -> normalized `background-color`).
    let updated_attr = SAMPLE_COMPONENT.replace("\"#2563eb\"", "\"#16a34a\"");
    let attr_patch = expect_patch(diff_rsx_source(
        "demo.rs",
        SAMPLE_COMPONENT,
        &updated_attr,
        4,
    ));
    assert_eq!(
        attr_patch.attr_updates,
        vec![("background-color".into(), "#16a34a".into())]
    );
    assert_eq!(attr_patch.target_test_id.as_deref(), Some("cta-btn"));
}

#[test]
fn tier1_multiple_edits_in_one_save_split_text_and_element_messages() {
    let updated = SAMPLE_COMPONENT
        .replace(
            "color: #ffffff; font-size: 16px;",
            "color: #38bdf8; font-size: 18px;",
        )
        .replace("\"# engineering\"", "\"# platform\"")
        .replace("\"Ship It\"", "\"Deploy Now\"");

    let (patch, messages) = expect_tier1(diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated, 5));
    assert_eq!(patch.attr_changes.len(), 1);
    assert_eq!(patch.text_changes.len(), 2);
    assert_eq!(
        messages.len(),
        3,
        "one message per text node + one per element"
    );

    // Each text replacement is scoped to its own owner's subtree, never the `style` edit's.
    let owners: Vec<_> = messages[..2]
        .iter()
        .map(|m| (m.target_test_id.as_deref(), m.text_replacements.clone()))
        .collect();
    assert_eq!(
        owners,
        vec![
            (
                Some("channel-title"),
                vec![("# engineering".into(), "# platform".into())]
            ),
            (
                Some("cta-btn"),
                vec![("Ship It".into(), "Deploy Now".into())]
            ),
        ]
    );
    assert!(messages[..2].iter().all(|m| m.attr_updates.is_empty()));

    let attr = &messages[2];
    assert_eq!(
        attr.attr_updates,
        vec![("style".into(), "color: #38bdf8; font-size: 18px;".into())]
    );
    assert_eq!(attr.target_test_id.as_deref(), Some("channel-title"));
    assert!(attr.text_replacements.is_empty() && attr.text_updates.is_empty());
}

#[test]
fn interpolated_literal_structural_logic_handler_and_syntax_edits_fallback_to_tier2() {
    // 1. Format `{expr}` interpolation literal changed -> Tier-2.
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &SAMPLE_COMPONENT.replace("\"Count: {count}\"", "\"Total: {count}\""),
        2,
    );
    // 2. Adding interpolation braces to a previously static literal -> Tier-2.
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &SAMPLE_COMPONENT.replace("\"Ship It\"", "\"Ship {count}\""),
        2,
    );
    // 3. Event handler logic changed -> Tier-2.
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &SAMPLE_COMPONENT.replace("count += 1", "count += 2"),
        2,
    );
    // 4. Structural edit (adding a new element) -> Tier-2.
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &SAMPLE_COMPONENT.replace(
            "text { \"Ship It\" }",
            "text { \"Ship It\" } text { \"New\" }",
        ),
        2,
    );
    // 5. Editing `test_id` itself -> Tier-2.
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &SAMPLE_COMPONENT.replace("test_id: \"cta-btn\"", "test_id: \"cta-btn-2\""),
        2,
    );
    // 6. Syntax error (unclosed string / unbalanced brace) -> Tier-2.
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &SAMPLE_COMPONENT.replace("\"Ship It\"", "\"Ship It"),
        2,
    );
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &format!("{SAMPLE_COMPONENT}\n}}"),
        2,
    );
    // 7. String literal outside `rsx!` changed -> Tier-2.
    let with_rust_str = format!("const LABEL: &str = \"hello\";\n{SAMPLE_COMPONENT}");
    let with_rust_str_edited = format!("const LABEL: &str = \"world\";\n{SAMPLE_COMPONENT}");
    assert_tier2("demo.rs", &with_rust_str, &with_rust_str_edited, 2);
}

#[test]
fn attribute_edits_on_two_elements_emit_one_targeted_message_per_element() {
    let two_elems = SAMPLE_COMPONENT
        .replace("padding: 12px;", "padding: 20px;")
        .replace("\"#2563eb\"", "\"#ef4444\"");
    let (patch, messages) =
        expect_tier1(diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &two_elems, 2));
    assert_eq!(patch.attr_changes.len(), 2);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].target_test_id.as_deref(), Some("card-root"));
    assert_eq!(messages[0].node_path, vec![0]);
    assert_eq!(messages[0].attr_updates[0].0, "style");
    assert_eq!(messages[1].target_test_id.as_deref(), Some("cta-btn"));
    assert_eq!(messages[1].node_path, vec![0, 1]);
    assert_eq!(
        messages[1].attr_updates,
        vec![("background-color".into(), "#ef4444".into())]
    );
}

#[test]
fn comment_and_whitespace_only_edits_are_noop() {
    let commented = format!(
        "// top-level comment\n/* outer /* nested */ comment */\n{}\n   ",
        SAMPLE_COMPONENT.replace("rsx! {", "rsx! { // inline comment\n")
    );
    assert_eq!(
        diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &commented, 2),
        DiffOutcome::NoOp
    );
}

#[test]
fn raw_string_literals_in_rsx_text_and_attrs() {
    let base = r###"
pub fn RawDemo() -> Element {
    rsx! {
        view {
            test_id: "raw-card",
            style: r#"width: 100%; background-color: #0f172a;"#,
            text {
                test_id: "raw-quote",
                r#"Say "hello" with \slashes\ and {{braces}}"#
            }
        }
    }
}
"###;

    // 1. Editing raw string text containing quotes, backslashes, and escaped `{{}}` -> Tier-1.
    //    Values are the rendered text: `rsx!` format escapes `{{`/`}}` collapse to `{`/`}`.
    let edited_text = base.replace(
        r###"r#"Say "hello" with \slashes\ and {{braces}}"#"###,
        r###"r##"Say "world" with \paths\ and {{ok}}"##"###,
    );
    let patch = expect_patch(diff_rsx_source("raw.rs", base, &edited_text, 2));
    assert_eq!(patch.target_test_id.as_deref(), Some("raw-quote"));
    assert_eq!(
        patch.text_replacements,
        vec![(
            r#"Say "hello" with \slashes\ and {braces}"#.into(),
            r#"Say "world" with \paths\ and {ok}"#.into(),
        )]
    );

    // 2. Switching between `r#"..."#` and normal `"..."` with identical content is NoOp.
    let normalized = base.replace(
        r###"r#"width: 100%; background-color: #0f172a;"#"###,
        r#""width: 100%; background-color: #0f172a;""#,
    );
    assert_eq!(
        diff_rsx_source("raw.rs", base, &normalized, 3),
        DiffOutcome::NoOp
    );

    // 3. Adding unescaped `{expr}` interpolation inside a raw string -> Tier-2.
    assert_tier2("raw.rs", base, &base.replace("{{braces}}", "{count}"), 4);

    // 4. Unterminated raw string -> Tier-2.
    let unterminated = base.replace(
        r###"r#"width: 100%; background-color: #0f172a;"#"###,
        r###"r#"width: 100%;"###,
    );
    assert_tier2("raw.rs", base, &unterminated, 5);
}

#[test]
fn nested_rsx_macros_in_blocks_match_props_and_direct_children() {
    let base = r#"
pub fn NestedDemo(show: bool) -> Element {
    rsx! {
        view {
            test_id: "outer-root",
            header: rsx! {
                text { test_id: "prop-hdr", "Header Alpha" }
            },
            match show {
                true => rsx! {
                    view {
                        test_id: "match-card",
                        style: "padding: 8px;",
                        text { "Match On" }
                    }
                },
                false => rsx! { text { "Match Off" } },
            }
            {
                let sub = rsx! { text { test_id: "block-leaf", "Block Leaf" } };
                sub
            }
            rsx! {
                text { test_id: "direct-nested", "Direct Child" }
            }
        }
    }
}
"#;

    // 1. Edit text inside `header: rsx! { ... }` prop expression.
    let prop_patch = expect_patch(diff_rsx_source(
        "nested.rs",
        base,
        &base.replace("\"Header Alpha\"", "\"Header Beta\""),
        2,
    ));
    assert_eq!(prop_patch.target_test_id.as_deref(), Some("prop-hdr"));
    assert_eq!(
        prop_patch.text_replacements,
        vec![("Header Alpha".into(), "Header Beta".into())]
    );

    // 2. Edit style inside `match` arm's nested `rsx! { ... }`.
    let match_patch = expect_patch(diff_rsx_source(
        "nested.rs",
        base,
        &base.replace("\"padding: 8px;\"", "\"padding: 16px;\""),
        3,
    ));
    assert_eq!(match_patch.target_test_id.as_deref(), Some("match-card"));
    assert_eq!(
        match_patch.attr_updates,
        vec![("style".into(), "padding: 16px;".into())]
    );

    // 3. Edit text inside embedded `{ let sub = rsx! { ... }; sub }` Rust block.
    let block_patch = expect_patch(diff_rsx_source(
        "nested.rs",
        base,
        &base.replace("\"Block Leaf\"", "\"Updated Leaf\""),
        4,
    ));
    assert_eq!(block_patch.target_test_id.as_deref(), Some("block-leaf"));
    assert_eq!(
        block_patch.text_replacements,
        vec![("Block Leaf".into(), "Updated Leaf".into())]
    );

    // 4. Edit text inside a direct child `rsx! { ... }`.
    let direct_patch = expect_patch(diff_rsx_source(
        "nested.rs",
        base,
        &base.replace("\"Direct Child\"", "\"Direct Updated\""),
        5,
    ));
    assert_eq!(
        direct_patch.target_test_id.as_deref(),
        Some("direct-nested")
    );
    assert_eq!(
        direct_patch.text_replacements,
        vec![("Direct Child".into(), "Direct Updated".into())]
    );
}

#[test]
fn multiline_style_attribute_edits_with_newlines_and_line_continuations() {
    let base_newlines = r#"
pub fn MultiLineStyle() -> Element {
    rsx! {
        view {
            test_id: "panel",
            style: "
                width: 100%;
                padding: 12px;
                background-color: #0f172a;
            ",
            text { "Panel" }
        }
    }
}
"#;
    let patch = expect_patch(diff_rsx_source(
        "multiline.rs",
        base_newlines,
        &base_newlines.replace("padding: 12px;", "padding: 24px;"),
        2,
    ));
    assert_eq!(patch.target_test_id.as_deref(), Some("panel"));
    assert_eq!(patch.attr_updates.len(), 1);
    assert_eq!(patch.attr_updates[0].0, "style");
    assert!(patch.attr_updates[0].1.contains("padding: 24px;"));
    assert!(patch.attr_updates[0].1.contains('\n'));

    // Rust backslash line-continuation style string collapses indentation cleanly.
    let base_cont = "pub fn Cont() -> Element {\n    rsx! {\n        view {\n            test_id: \"hero\",\n            style: \"width: 100%; \\\n                padding: 12px; \\\n                background-color: #0f172a;\",\n        }\n    }\n}\n";
    let cont_patch = expect_patch(diff_rsx_source(
        "cont.rs",
        base_cont,
        &base_cont.replace("padding: 12px;", "padding: 20px;"),
        3,
    ));
    assert_eq!(cont_patch.target_test_id.as_deref(), Some("hero"));
    assert_eq!(
        cont_patch.attr_updates,
        vec![(
            "style".into(),
            "width: 100%; padding: 20px; background-color: #0f172a;".into()
        )]
    );

    // Re-indenting after `\` without changing the logical style value is a NoOp.
    let reindented = base_cont.replace("                padding:", "        padding:");
    assert_eq!(
        diff_rsx_source("cont.rs", base_cont, &reindented, 4),
        DiffOutcome::NoOp
    );
}

#[test]
fn watch_cache_chains_consecutive_tier1_patches_and_protects_baseline_on_error() {
    let mut cache = RsxWatchCache::new();
    let path = PathBuf::from("src/demo.rs");
    cache.update_file(&path, SAMPLE_COMPONENT.to_owned());

    let v1 = SAMPLE_COMPONENT.replace("\"# engineering\"", "\"# core\"");
    let p1 = expect_patch(cache.diff_file(&path, &v1, 2));
    assert_eq!(
        p1.text_replacements,
        vec![("# engineering".into(), "# core".into())]
    );

    let v2 = v1.replace("\"# core\"", "\"# runtime\"");
    let p2 = expect_patch(cache.diff_file(&path, &v2, 3));
    assert_eq!(
        p2.text_replacements,
        vec![("# core".into(), "# runtime".into())]
    );

    // Broken syntax edit triggers Tier-2 and preserves `v2` as baseline.
    let broken = v2.replace("\"# runtime\"", "\"# runtime");
    assert!(matches!(
        cache.diff_file(&path, &broken, 4),
        DiffOutcome::RequireTier2Rebuild { .. }
    ));
    assert_eq!(cache.cached_source(&path), Some(v2.as_str()));
}

#[test]
fn rsx_text_children_inside_components_and_nested_blocks_get_patched() {
    let base = "pub fn A() -> Element {\n    rsx! {\n        Card { variant: CardVariant::Outlined, \"Inside component\" }\n    }\n}\n";
    let patch = expect_patch(diff_rsx_source(
        "comp.rs",
        base,
        &base.replace("Inside component", "Inside card"),
        2,
    ));
    assert_eq!(
        patch.text_replacements,
        vec![("Inside component".into(), "Inside card".into())]
    );
    assert_eq!(patch.node_path, vec![0, 0]);
}
