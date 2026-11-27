//! Diffs against the real `studio-workspace` `ui_chrome.rs` and a synthetic 5,000-line file.

use std::path::Path;
use std::time::Instant;

use super::super::fixture::{replace_last, synthetic_rsx_source};
use super::super::{DiffOutcome, diff_rsx_source};
use super::{assert_tier2, assert_tier2_reason, expect_patch, expect_tier1};

const UI_CHROME: &str = "examples/studio-workspace/src/ui_chrome.rs";

fn ui_chrome_source() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(UI_CHROME);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed reading {}: {e}", path.display()))
}

#[test]
fn real_ui_chrome_text_and_style_edits_patch_under_10ms_and_logic_edits_rebuild() {
    let original = ui_chrome_source();

    // 1. Static text edit inside `rsx!`.
    let edited_text = original.replacen("\"LOCAL-FIRST | NVE\"", "\"LOCAL-FIRST | LIVE\"", 1);
    assert_ne!(original, edited_text);
    let t0 = Instant::now();
    let text_patch = expect_patch(diff_rsx_source(UI_CHROME, &original, &edited_text, 2));
    let elapsed = t0.elapsed();
    assert!(text_patch.attr_updates.is_empty());
    assert_eq!(
        text_patch.text_replacements,
        vec![("LOCAL-FIRST | NVE".into(), "LOCAL-FIRST | LIVE".into())]
    );
    assert!(
        elapsed.as_millis() < 10,
        "Tier-1 diff on ui_chrome.rs ({} bytes) took {elapsed:?} (debug build)",
        original.len()
    );

    // 2. Static style string edit on a native `view` inside `rsx!`.
    let edited_style = original.replacen(
        "style: \"flex-direction: row; align-items: center; gap: 8px;\"",
        "style: \"flex-direction: row; align-items: center; gap: 12px;\"",
        1,
    );
    assert_ne!(original, edited_style);
    let style_patch = expect_patch(diff_rsx_source(UI_CHROME, &original, &edited_style, 3));
    assert_eq!(
        style_patch.attr_updates,
        vec![(
            "style".into(),
            "flex-direction: row; align-items: center; gap: 12px;".into()
        )]
    );

    // 3. String data outside `rsx!` is Rust logic.
    assert_tier2(&original, &original.replacen("\"Close\"", "\"Hide\"", 1));

    // 4. Interpolated `style` (`{s.t.text.link.hex()}`) edits need Rust.
    assert_tier2_reason(
        &original,
        &original.replacen(
            "font-size: 15px;\", \"AETHER\"",
            "font-size: 16px;\", \"AETHER\"",
            1,
        ),
        "interpolated",
    );

    // 5. Component prop (`Card { padding: Some(8.0) }`) is not a native element attribute.
    assert_tier2(
        &original,
        &original.replacen("padding: Some(8.0),", "padding: Some(10.0),", 1),
    );

    // 6. Comment and re-indentation edits are no-ops.
    let commented = original.replacen("rsx! {", "rsx! { // hot reload probe\n", 1);
    assert_eq!(
        diff_rsx_source(UI_CHROME, &original, &commented, 4),
        DiffOutcome::NoOp
    );
}

#[test]
fn synthetic_5k_line_file_typed_edit_in_last_component_targets_it_exactly() {
    let original = synthetic_rsx_source(5_000);
    assert!(original.lines().count() >= 5_000);
    let last = original.matches("pub fn Card").count() - 1;

    let edited = replace_last(&original, "padding: 8.0,", "padding: px(-12.5),");
    let (patch, messages) = expect_tier1(diff_rsx_source("synthetic.rs", &original, &edited, 2));
    assert_eq!(patch.attr_changes.len(), 1);
    let change = &patch.attr_changes[0];
    assert_eq!(
        change.target.test_id.as_deref(),
        Some(format!("card-{last}").as_str())
    );
    assert_eq!(change.target.block.ordinal as usize, last);
    assert_eq!(change.target.node_path, vec![0]);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].attr_updates,
        vec![("padding".into(), "-12.5px".into())]
    );

    // Every typed form of the fixture round-trips through one save in the last component.
    let multi = replace_last(
        &replace_last(
            &replace_last(&edited, "FlexDirection::Row", "FlexDirection::Column"),
            "Rgba::rgb(15, 23, 42)",
            "hex(0x0f172aCC)",
        ),
        "\"Press ",
        "\"Tap ",
    );
    let (patch, messages) = expect_tier1(diff_rsx_source("synthetic.rs", &edited, &multi, 3));
    assert_eq!(patch.attr_changes.len(), 2);
    assert_eq!(patch.text_changes.len(), 1);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[1].attr_updates,
        vec![
            ("background-color".into(), "#0f172acc".into()),
            ("flex-direction".into(), "column".into()),
        ]
    );
    assert_eq!(
        messages[0].target_test_id.as_deref(),
        Some(format!("card-{last}-cta").as_str())
    );

    // A logic edit at the very end reports its line.
    let total_lines = original.lines().count();
    let logic = replace_last(&original, "count() * ", "count() + ");
    let reason = assert_tier2(&original, &logic);
    let at = original.rfind("count() * ").unwrap_or(0);
    let line = original[..at].bytes().filter(|&b| b == b'\n').count() + 1;
    assert!(
        reason.contains(&format!("(line {line})")),
        "{reason} (file has {total_lines} lines)"
    );
}
