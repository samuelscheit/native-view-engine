//! `rsx_diff` tests: shared helpers plus one module per concern.

mod eval_tests;
mod fallback_tests;
mod legacy;
mod lexer_tests;
mod lowering_tests;
mod parser_tests;
mod real_world_tests;
mod surface_tests;
mod typed_span_tests;

use nve_engine::HotPatchMsg;
use nve_store::{NveTable, Prop, RowPatchView, ScalarField};

use super::DiffOutcome;
use super::cache::Baseline;
use super::eval::{EvalError, TypedAttrValue, evaluate};
use super::lexer::{Lex, tokenize};
use super::matcher::diff_rsx_typed;
use super::parser::RsxIndex;
use super::patch::TypedRsxPatch;

/// Cold diff of two sources (fresh baseline).
fn diff_rsx_source(
    rel_file: &str,
    old_src: &str,
    new_src: &str,
    generation: u64,
) -> DiffOutcome {
    Baseline::new(old_src.to_owned()).diff(rel_file, new_src, generation)
}

/// The typed patch of two sources, independent of binary `RowPatch` lowering.
fn typed_patch(old_src: &str, new_src: &str) -> Result<Option<TypedRsxPatch>, String> {
    let old_tokens = tokenize(old_src).map_err(|e| e.as_str().to_owned())?;
    let new_tokens = tokenize(new_src).map_err(|e| e.as_str().to_owned())?;
    let old = Lex::new(old_src, &old_tokens);
    diff_rsx_typed(old, &RsxIndex::build(old), Lex::new(new_src, &new_tokens))
}

/// Flattened `HotPatchMsg::TemplatePatch` fields for assertions.
#[derive(Debug)]
struct PatchView {
    generation: u64,
    template_key: u64,
    node_path: Vec<u16>,
    target_test_id: Option<String>,
    row_patch_words: Vec<u32>,
    text_updates: Vec<(usize, String)>,
    text_replacements: Vec<(String, String)>,
}

impl PatchView {
    fn row_patch(&self) -> RowPatchView<'_> {
        RowPatchView::from_words(&self.row_patch_words).expect("valid binary RowPatch words")
    }

    fn prop<T: NveTable, V: ScalarField>(&self, prop: Prop<T, V>) -> Option<V> {
        self.row_patch().get_prop(prop)
    }
}

fn view(msg: HotPatchMsg) -> PatchView {
    match msg {
        HotPatchMsg::TemplatePatch {
            generation,
            template_key,
            node_path,
            target_test_id,
            row_patch_words,
            text_updates,
            text_replacements,
        } => PatchView {
            generation,
            template_key,
            node_path,
            target_test_id,
            row_patch_words,
            text_updates,
            text_replacements,
        },
        other => panic!("expected TemplatePatch, got {other:?}"),
    }
}

/// The typed patch and every lowered message of a Tier-1 outcome.
fn expect_tier1(outcome: DiffOutcome) -> (TypedRsxPatch, Vec<PatchView>) {
    match outcome {
        DiffOutcome::Tier1Patch { patch, messages } => {
            assert!(
                messages.iter().all(|m| matches!(m, HotPatchMsg::TemplatePatch { text_updates, .. } if text_updates.is_empty())),
                "positional text_updates are never emitted"
            );
            (patch, messages.into_iter().map(view).collect())
        }
        other => panic!("expected Tier1Patch, got {other:?}"),
    }
}

/// The single lowered message of a Tier-1 outcome.
fn expect_patch(outcome: DiffOutcome) -> PatchView {
    let (_, mut messages) = expect_tier1(outcome);
    assert_eq!(
        messages.len(),
        1,
        "expected exactly one message: {messages:?}"
    );
    messages.remove(0)
}

/// Diffs and asserts a Tier-2 fallback, returning its reason.
fn assert_tier2(old_src: &str, new_src: &str) -> String {
    assert_ne!(old_src, new_src, "edit did not change the source");
    match diff_rsx_source("demo.rs", old_src, new_src, 2) {
        DiffOutcome::RequireTier2Rebuild { reason } => reason,
        other => panic!("expected RequireTier2Rebuild, got {other:?}"),
    }
}

/// Like [`assert_tier2`], also requiring `needle` in the reason.
fn assert_tier2_reason(old_src: &str, new_src: &str, needle: &str) {
    let reason = assert_tier2(old_src, new_src);
    assert!(
        reason.contains(needle),
        "reason {reason:?} lacks {needle:?}"
    );
}

/// Evaluates a standalone value snippet.
fn eval_src(src: &str) -> Result<TypedAttrValue, EvalError> {
    let tokens = tokenize(src).unwrap_or_else(|e| panic!("lex {src:?}: {}", e.as_str()));
    let lex = Lex::new(src, &tokens);
    evaluate(lex, 0, lex.len())
}

/// A one-element component whose `view` (test_id `box`) carries `attrs`.
fn element_with(attrs: &str) -> String {
    format!(
        "pub fn Demo() -> Element {{\n    let n = 1;\n    rsx! {{\n        view {{\n            test_id: \"box\",\n            {attrs}\n            text {{ \"Body\" }}\n        }}\n    }}\n}}\n"
    )
}
