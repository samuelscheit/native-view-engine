//! Diffs against the real `studio-workspace` `ui_chrome.rs` and a synthetic 5,000-line file.
//! Latency and allocation budgets live in `benches/rsx_diff_bench.rs` (release build).

use std::path::Path;

use nve_store::{Length, Rgba};
use nve_tables::tables::{FlexDirection, Layout, Visual};

use super::super::DiffOutcome;
use super::super::cache::Baseline;
use super::super::fixture::{replace_last, synthetic_rsx_source};
use super::{assert_tier2, diff_rsx_source, expect_patch, expect_tier1};

const UI_CHROME: &str = "examples/studio-workspace/src/ui_chrome.rs";

fn ui_chrome_source() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(UI_CHROME);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed reading {}: {e}", path.display()))
}

#[test]
fn real_ui_chrome_text_edits_patch_and_logic_edits_rebuild() {
    let original = ui_chrome_source();

    // 1. Static text edit inside `rsx!`.
    let edited_text = original.replacen("\"LOCAL-FIRST | NVE\"", "\"LOCAL-FIRST | LIVE\"", 1);
    let text_patch = expect_patch(diff_rsx_source(UI_CHROME, &original, &edited_text, 2));
    assert!(text_patch.row_patch_words.is_empty());
    assert_eq!(
        text_patch.text_replacements,
        vec![("LOCAL-FIRST | NVE".into(), "LOCAL-FIRST | LIVE".into())]
    );

    // 2. String data outside `rsx!` is Rust logic.
    assert_tier2(&original, &original.replacen("\"Close\"", "\"Hide\"", 1));

    // 3. Component prop (`Card { padding: Some(8.0) }`) is not a native element attribute.
    assert_tier2(
        &original,
        &original.replacen("padding: Some(8.0),", "padding: Some(10.0),", 1),
    );

    // 4. Comment edits are no-ops.
    let commented = original.replacen("rsx! {", "rsx! { // hot reload probe\n", 1);
    assert_eq!(
        diff_rsx_source(UI_CHROME, &original, &commented, 4),
        DiffOutcome::NoOp
    );
}

#[test]
fn synthetic_5k_line_file_typed_edits_in_last_component_target_it_exactly() {
    let original = synthetic_rsx_source(5_000);
    assert!(original.lines().count() >= 5_000);
    let last = original.matches("pub fn Card").count() - 1;

    let edited = replace_last(&original, "padding: 8.0,", "padding: -12.5,");
    let (patch, messages) = expect_tier1(diff_rsx_source("synthetic.rs", &original, &edited, 2));
    let [change] = patch.attr_changes.as_slice() else {
        panic!("expected one change: {patch:?}");
    };
    assert_eq!(
        change.target.test_id.as_deref(),
        Some(format!("card-{last}").as_str())
    );
    // Three templates per card: its `rsx!`, the `if` body and the `for` body.
    assert_eq!(change.target.template.ordinal as usize, 3 * last);
    assert_eq!(change.target.node_path, vec![0]);
    assert_eq!(
        messages[0].prop(Layout::PADDING_TOP),
        Some(Length::points(0.0))
    );

    let multi = replace_last(
        &replace_last(
            &replace_last(&edited, "FlexDirection::Row", "FlexDirection::Column"),
            "ColorRgba::rgb(15, 23, 42)",
            "ColorRgba::rgb(15, 23, 204)",
        ),
        "\"Press ",
        "\"Tap ",
    );
    let (patch, messages) = expect_tier1(diff_rsx_source("synthetic.rs", &edited, &multi, 3));
    assert_eq!((patch.attr_changes.len(), patch.text_changes.len()), (2, 1));
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0].target_test_id.as_deref(),
        Some(format!("card-{last}-cta").as_str())
    );
    assert_eq!(
        messages[1].prop(Visual::BACKGROUND),
        Some(Rgba::rgb(15, 23, 204))
    );
    assert_eq!(
        messages[1].prop(Layout::FLEX_DIRECTION),
        Some(FlexDirection::Column)
    );

    // A logic edit at the very end reports its line.
    let logic = replace_last(&original, "count() * ", "count() + ");
    let reason = assert_tier2(&original, &logic);
    let at = original.rfind("count() * ").unwrap_or(0);
    let line = original[..at].bytes().filter(|&b| b == b'\n').count() + 1;
    assert!(reason.contains(&format!("(line {line})")), "{reason}");
}

#[test]
fn baseline_chains_saves_and_keeps_its_cached_index_across_token_identical_saves() {
    let original = synthetic_rsx_source(500);
    let mut baseline = Baseline::new(original.clone());
    let v1 = replace_last(&original, "padding: 8.0,", "padding: 9.0,");
    assert!(matches!(
        baseline.diff("s.rs", &v1, 2),
        DiffOutcome::Tier1Patch { .. }
    ));
    let v2 = replace_last(&v1, "padding: 9.0,", "padding: 10.0,");
    let (patch, _) = expect_tier1(baseline.diff("s.rs", &v2, 3));
    assert_eq!(patch.attr_changes[0].old.to_string(), "9.0");
    // Comment-only save: NoOp and the next diff still sees the chained baseline.
    let v3 = v2.replacen("rsx! {", "rsx! { // c\n", 1);
    assert_eq!(baseline.diff("s.rs", &v3, 4), DiffOutcome::NoOp);
    let v4 = replace_last(&v3, "padding: 10.0,", "padding: 11.0,");
    let (patch, _) = expect_tier1(baseline.diff("s.rs", &v4, 5));
    assert_eq!(patch.attr_changes[0].old.to_string(), "10.0");
    // A Tier-2 save keeps the baseline.
    let broken = v4.replace("padding: 11.0,", "padding: gap,");
    assert!(matches!(
        baseline.diff("s.rs", &broken, 6),
        DiffOutcome::RequireTier2Rebuild { .. }
    ));
    let v5 = replace_last(&v4, "padding: 11.0,", "padding: 12.0,");
    let (patch, _) = expect_tier1(baseline.diff("s.rs", &v5, 7));
    assert_eq!(patch.attr_changes[0].old.to_string(), "11.0");
}
