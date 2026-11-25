use std::path::{Path, PathBuf};
use std::time::Instant;

use nve_engine::HotPatchMsg;

use super::{diff_rsx_source, DiffOutcome, RsxWatchCache};

const SAMPLE_COMPONENT: &str = r##"
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

struct PatchView {
    generation: u64,
    template_id: String,
    attr_updates: Vec<(String, String)>,
    text_updates: Vec<(usize, String)>,
    target_test_id: Option<String>,
    node_path: Vec<usize>,
    text_replacements: Vec<(String, String)>,
}

fn expect_patch(outcome: DiffOutcome) -> PatchView {
    match outcome {
        DiffOutcome::Tier1Patch(HotPatchMsg::TemplatePatch {
            generation,
            template_id,
            attr_updates,
            text_updates,
            target_test_id,
            node_path,
            text_replacements,
        }) => PatchView {
            generation,
            template_id,
            attr_updates,
            text_updates,
            target_test_id,
            node_path,
            text_replacements,
        },
        other => panic!("expected Tier1Patch, got {other:?}"),
    }
}

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
    assert_eq!(patch.text_updates, vec![(0, "# ergoiherg".into())]);
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
    let attr_patch = expect_patch(diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated_attr, 4));
    assert_eq!(
        attr_patch.attr_updates,
        vec![("background-color".into(), "#16a34a".into())]
    );
    assert_eq!(attr_patch.target_test_id.as_deref(), Some("cta-btn"));
}

#[test]
fn tier1_multiple_edits_in_one_save() {
    let updated = SAMPLE_COMPONENT
        .replace(
            "color: #ffffff; font-size: 16px;",
            "color: #38bdf8; font-size: 18px;",
        )
        .replace("\"# engineering\"", "\"# platform\"")
        .replace("\"Ship It\"", "\"Deploy Now\"");

    let patch = expect_patch(diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated, 5));
    assert_eq!(
        patch.attr_updates,
        vec![("style".into(), "color: #38bdf8; font-size: 18px;".into())]
    );
    assert_eq!(patch.target_test_id.as_deref(), Some("channel-title"));
    assert_eq!(
        patch.text_replacements,
        vec![
            ("# engineering".into(), "# platform".into()),
            ("Ship It".into(), "Deploy Now".into()),
        ]
    );
    assert_eq!(patch.text_updates.len(), 2);
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
        &SAMPLE_COMPONENT.replace("text { \"Ship It\" }", "text { \"Ship It\" } text { \"New\" }"),
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
    // 8. Attribute edits spanning two different elements -> Tier-2 (never mis-target).
    let two_elems = SAMPLE_COMPONENT
        .replace("padding: 12px;", "padding: 20px;")
        .replace("\"#2563eb\"", "\"#ef4444\"");
    assert_tier2("demo.rs", SAMPLE_COMPONENT, &two_elems, 2);
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
    let edited_text = base.replace(
        r###"r#"Say "hello" with \slashes\ and {{braces}}"#"###,
        r###"r##"Say "world" with \paths\ and {{ok}}"##"###,
    );
    let patch = expect_patch(diff_rsx_source("raw.rs", base, &edited_text, 2));
    assert_eq!(patch.target_test_id.as_deref(), Some("raw-quote"));
    assert_eq!(
        patch.text_replacements,
        vec![(
            r#"Say "hello" with \slashes\ and {{braces}}"#.into(),
            r#"Say "world" with \paths\ and {{ok}}"#.into(),
        )]
    );

    // 2. Switching between `r#"..."#` and normal `"..."` with identical content is NoOp.
    let normalized = base.replace(
        r###"r#"width: 100%; background-color: #0f172a;"#"###,
        r#""width: 100%; background-color: #0f172a;""#,
    );
    assert_eq!(diff_rsx_source("raw.rs", base, &normalized, 3), DiffOutcome::NoOp);

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
    assert_eq!(direct_patch.target_test_id.as_deref(), Some("direct-nested"));
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
fn diff_latency_on_real_studio_workspace_ui_rs_is_under_10ms() {
    let ui_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/studio-workspace/src/ui.rs");
    let original = std::fs::read_to_string(&ui_path)
        .unwrap_or_else(|e| panic!("failed reading {}: {e}", ui_path.display()));

    // 1. Static text edit inside `rsx!` ("QUICK REPLY" -> "FAST REPLY")
    let edited_text = original.replacen("\"QUICK REPLY\"", "\"FAST REPLY\"", 1);
    assert_ne!(original, edited_text);
    let t0 = Instant::now();
    let text_patch = expect_patch(diff_rsx_source(
        "examples/studio-workspace/src/ui.rs",
        &original,
        &edited_text,
        2,
    ));
    let elapsed = t0.elapsed();
    assert!(text_patch.attr_updates.is_empty());
    assert_eq!(
        text_patch.text_replacements,
        vec![("QUICK REPLY".into(), "FAST REPLY".into())]
    );
    assert!(
        elapsed.as_millis() < 10,
        "expected Tier-1 diff on real ui.rs ({} bytes) in < 10ms, took {:?}",
        original.len(),
        elapsed
    );

    // 2. Static style edit inside `rsx!` (`style: "width: 100%; flex: 1;"`)
    let edited_style = original.replacen(
        "style: \"width: 100%; flex: 1;\"",
        "style: \"width: 100%; flex: 1; padding: 8px;\"",
        1,
    );
    assert_ne!(original, edited_style);
    let t1 = Instant::now();
    let style_patch = expect_patch(diff_rsx_source(
        "examples/studio-workspace/src/ui.rs",
        &original,
        &edited_style,
        3,
    ));
    let style_elapsed = t1.elapsed();
    assert_eq!(
        style_patch.attr_updates,
        vec![("style".into(), "width: 100%; flex: 1; padding: 8px;".into())]
    );
    assert!(
        style_elapsed.as_millis() < 10,
        "expected Tier-1 style diff on ui.rs in < 10ms, took {:?}",
        style_elapsed
    );

    // 3. Rust logic / tuple data edit outside `rsx!` falls back cleanly to Tier-2.
    let edited_outside = original.replacen("\"# engineering\"", "\"# ergoiherg\"", 1);
    assert_tier2(
        "examples/studio-workspace/src/ui.rs",
        &original,
        &edited_outside,
        4,
    );
}
