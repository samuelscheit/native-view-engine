//! Static text, standalone numeric, and `nve_style: TypedStyle::...` Tier-1 binary `RowPatch` tests.

use std::path::PathBuf;

use nve_engine::hot_patch_template_key;
use nve_store::{Length, Rgba};
use nve_tables::tables::{Layout, Typography, Visual};

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
            padding: 12.0,
            nve_style: TypedStyle::col().bg(ColorRgba::rgb(15, 23, 42)),
            text {
                test_id: "channel-title",
                font_size: 16.0,
                nve_style: TypedStyle::new().color(ColorRgba::rgb(255, 255, 255)),
                "# engineering"
            }
            pressable {
                test_id: "cta-btn",
                nve_style: TypedStyle::new().bg(ColorRgba::rgb(37, 99, 235)),
                onpress: move |_| count += 1,
                text { "Count: {count}" }
                text { "Ship It" }
            }
        }
    }
}
"##;

fn assert_tier2(rel_file: &str, old_src: &str, new_src: &str, generation: u64) {
    assert!(matches!(
        diff_rsx_source(rel_file, old_src, new_src, generation),
        DiffOutcome::RequireTier2Rebuild { .. }
    ));
}

#[test]
fn tier1_text_only_edit_inside_rsx_emits_targeted_replacement() {
    let updated = SAMPLE_COMPONENT.replace("\"# engineering\"", "\"# ergoiherg\"");
    let patch = expect_patch(diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated, 2));
    assert_eq!(patch.generation, 2);
    assert_eq!(patch.template_key, hot_patch_template_key("demo.rs", 0));
    assert!(patch.row_patch_words.is_empty());
    assert_eq!(
        patch.text_replacements,
        vec![("# engineering".into(), "# ergoiherg".into())]
    );
    assert!(patch.text_updates.is_empty());
    assert_eq!(patch.target_test_id.as_deref(), Some("channel-title"));
    assert_eq!(patch.node_path, vec![0, 0, 0]);
}

#[test]
fn tier1_typed_attr_and_nve_style_literal_edits_emit_binary_row_patch_words() {
    let updated = SAMPLE_COMPONENT
        .replace("padding: 12.0,", "padding: 24.0,")
        .replace("ColorRgba::rgb(15, 23, 42)", "ColorRgba::rgb(30, 41, 59)");
    let patch = expect_patch(diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated, 3));
    assert_eq!(patch.prop(Layout::PADDING_TOP), Some(Length::points(24.0)));
    assert_eq!(patch.prop(Visual::BACKGROUND), Some(Rgba::rgb(30, 41, 59)));
    assert!(patch.text_replacements.is_empty());
    assert_eq!(patch.target_test_id.as_deref(), Some("card-root"));
    assert_eq!(patch.node_path, vec![0]);

    let updated_attr = SAMPLE_COMPONENT.replace(
        "ColorRgba::rgb(37, 99, 235)",
        "ColorRgba::rgb(22, 163, 74)",
    );
    let attr_patch = expect_patch(diff_rsx_source(
        "demo.rs",
        SAMPLE_COMPONENT,
        &updated_attr,
        4,
    ));
    assert_eq!(
        attr_patch.prop(Visual::BACKGROUND),
        Some(Rgba::rgb(22, 163, 74))
    );
    assert_eq!(attr_patch.target_test_id.as_deref(), Some("cta-btn"));
}

#[test]
fn tier1_multiple_edits_in_one_save_split_text_and_element_messages() {
    let updated = SAMPLE_COMPONENT
        .replace(
            "ColorRgba::rgb(255, 255, 255)",
            "ColorRgba::rgb(56, 189, 248)",
        )
        .replace("font_size: 16.0,", "font_size: 18.0,")
        .replace("\"# engineering\"", "\"# platform\"")
        .replace("\"Ship It\"", "\"Deploy Now\"");

    let (patch, messages) = expect_tier1(diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated, 5));
    assert_eq!(patch.attr_changes.len(), 2);
    assert_eq!(patch.text_changes.len(), 2);
    assert_eq!(
        messages.len(),
        3,
        "one message per text node + one merged RowPatch per element"
    );

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
    assert!(messages[..2].iter().all(|m| m.row_patch_words.is_empty()));

    let attr = &messages[2];
    assert_eq!(attr.prop(Typography::COLOR), Some(Rgba::rgb(56, 189, 248)));
    assert_eq!(attr.prop(Typography::FONT_SIZE), Some(18.0));
    assert_eq!(attr.target_test_id.as_deref(), Some("channel-title"));
    assert!(attr.text_replacements.is_empty() && attr.text_updates.is_empty());
}

#[test]
fn interpolated_literal_structural_logic_handler_and_syntax_edits_fallback_to_tier2() {
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &SAMPLE_COMPONENT.replace("\"Count: {count}\"", "\"Total: {count}\""),
        2,
    );
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &SAMPLE_COMPONENT.replace("\"Ship It\"", "\"Ship {count}\""),
        2,
    );
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &SAMPLE_COMPONENT.replace("count += 1", "count += 2"),
        2,
    );
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &SAMPLE_COMPONENT.replace(
            "text { \"Ship It\" }",
            "text { \"Ship It\" } text { \"New\" }",
        ),
        2,
    );
    assert_tier2(
        "demo.rs",
        SAMPLE_COMPONENT,
        &SAMPLE_COMPONENT.replace("test_id: \"cta-btn\"", "test_id: \"cta-btn-2\""),
        2,
    );
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
    let with_rust_str = format!("const LABEL: &str = \"hello\";\n{SAMPLE_COMPONENT}");
    let with_rust_str_edited = format!("const LABEL: &str = \"world\";\n{SAMPLE_COMPONENT}");
    assert_tier2("demo.rs", &with_rust_str, &with_rust_str_edited, 2);
}

#[test]
fn attribute_edits_on_two_elements_emit_one_targeted_message_per_element() {
    let two_elems = SAMPLE_COMPONENT
        .replace("padding: 12.0,", "padding: 20.0,")
        .replace(
            "ColorRgba::rgb(37, 99, 235)",
            "ColorRgba::rgb(239, 68, 68)",
        );
    let (patch, messages) =
        expect_tier1(diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &two_elems, 2));
    assert_eq!(patch.attr_changes.len(), 2);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].target_test_id.as_deref(), Some("card-root"));
    assert_eq!(messages[0].node_path, vec![0]);
    assert_eq!(messages[0].prop(Layout::PADDING_TOP), Some(Length::points(20.0)));
    assert_eq!(messages[1].target_test_id.as_deref(), Some("cta-btn"));
    assert_eq!(messages[1].node_path, vec![0, 1]);
    assert_eq!(
        messages[1].prop(Visual::BACKGROUND),
        Some(Rgba::rgb(239, 68, 68))
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
fn raw_string_literals_in_rsx_text_and_typed_style_str_helpers() {
    let base = r###"
pub fn RawDemo() -> Element {
    rsx! {
        view {
            test_id: "raw-card",
            nve_style: TypedStyle::new().flex_direction_str(r#"row"#),
            text {
                test_id: "raw-quote",
                r#"Say "hello" with \slashes\ and {{braces}}"#
            }
        }
    }
}
"###;

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

    let normalized = base.replace(r###"r#"row"#"###, r##""row""##);
    assert_eq!(
        diff_rsx_source("raw.rs", base, &normalized, 3),
        DiffOutcome::NoOp
    );

    assert_tier2("raw.rs", base, &base.replace("{{braces}}", "{count}"), 4);

    let unterminated = base.replace(r###"r#"row"#"###, r###"r#"row"###);
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
                        padding: 8.0,
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

    let match_patch = expect_patch(diff_rsx_source(
        "nested.rs",
        base,
        &base.replace("padding: 8.0,", "padding: 16.0,"),
        3,
    ));
    assert_eq!(match_patch.target_test_id.as_deref(), Some("match-card"));
    assert_eq!(
        match_patch.prop(Layout::PADDING_TOP),
        Some(Length::points(16.0))
    );

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
