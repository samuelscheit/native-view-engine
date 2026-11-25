//! Token-stream diffing and Tier-1 hot-patch classification for watched `.rs` sources.

use nve_engine::HotPatchMsg;

use super::lexer::{has_interpolation_braces, tokenize, unescape_string_literal, TokenKind};
use super::parser::{RsxIndex, RsxLitContext};
use super::DiffOutcome;

/// Diffs `old_src` vs `new_src` for a single `.rs` file and classifies the edit as `NoOp`,
/// `Tier1Patch`, or `RequireTier2Rebuild`.
pub fn diff_rsx_source(
    template_id: &str,
    old_src: &str,
    new_src: &str,
    generation: u64,
) -> DiffOutcome {
    if old_src == new_src {
        return DiffOutcome::NoOp;
    }

    let old_lex = match tokenize(old_src) {
        Ok(l) => l,
        Err(e) => {
            return DiffOutcome::RequireTier2Rebuild {
                reason: format!("baseline lex error: {}", e.as_str()),
            };
        }
    };
    let new_lex = match tokenize(new_src) {
        Ok(l) => l,
        Err(e) => {
            return DiffOutcome::RequireTier2Rebuild {
                reason: format!("syntax error: {}", e.as_str()),
            };
        }
    };

    if old_lex.tokens.len() != new_lex.tokens.len() {
        return DiffOutcome::RequireTier2Rebuild {
            reason: "token count changed (structural or logic edit)".into(),
        };
    }

    let mut diff_indices = Vec::new();
    for i in 0..old_lex.tokens.len() {
        if old_lex.tokens[i] == new_lex.tokens[i] {
            continue;
        }
        match (&old_lex.tokens[i], &new_lex.tokens[i]) {
            (TokenKind::StringLit { .. }, TokenKind::StringLit { .. }) => {
                diff_indices.push(i);
            }
            _ => {
                return DiffOutcome::RequireTier2Rebuild {
                    reason: "non-string Rust token changed".into(),
                };
            }
        }
    }

    if diff_indices.is_empty() {
        return DiffOutcome::NoOp;
    }

    // Only string literals changed; verify EVERY changed literal lives inside `rsx! { ... }`
    // as a pure static (non-interpolated) text child or patchable style/layout attribute.
    let rsx_index = RsxIndex::build(&old_lex);

    let mut attr_updates: Vec<(String, String)> = Vec::new();
    let mut text_updates: Vec<(usize, String)> = Vec::new();
    let mut text_replacements: Vec<(String, String)> = Vec::new();
    let mut target_test_id: Option<String> = None;
    let mut node_path: Vec<usize> = Vec::new();
    let mut attr_owner_elem: Option<usize> = None;
    let mut text_target_test_id: Option<String> = None;
    let mut text_target_path: Vec<usize> = Vec::new();
    let mut text_owners_uniform = true;
    let mut seen_text_change = false;

    for idx in diff_indices {
        let (old_raw, old_is_raw) = match old_lex.tokens[idx] {
            TokenKind::StringLit { raw, is_raw } => (raw, is_raw),
            _ => unreachable!(),
        };
        let (new_raw, new_is_raw) = match new_lex.tokens[idx] {
            TokenKind::StringLit { raw, is_raw } => (raw, is_raw),
            _ => unreachable!(),
        };

        let old_val = match unescape_string_literal(old_raw, old_is_raw) {
            Ok(v) => v,
            Err(e) => {
                return DiffOutcome::RequireTier2Rebuild {
                    reason: format!("invalid old string literal: {}", e.as_str()),
                };
            }
        };
        let new_val = match unescape_string_literal(new_raw, new_is_raw) {
            Ok(v) => v,
            Err(e) => {
                return DiffOutcome::RequireTier2Rebuild {
                    reason: format!("invalid new string literal: {}", e.as_str()),
                };
            }
        };

        // Semantically identical after raw/escaped normalization (e.g., `"hi"` vs `r#"hi"#`).
        if old_val == new_val {
            continue;
        }

        // Any format-string `{expr}` interpolation brace requires Rust recompilation.
        if has_interpolation_braces(&old_val) || has_interpolation_braces(&new_val) {
            return DiffOutcome::RequireTier2Rebuild {
                reason: "interpolated `{expr}` string literal changed".into(),
            };
        }

        let Some(ctx) = rsx_index.literals.get(&idx) else {
            return DiffOutcome::RequireTier2Rebuild {
                reason: "string literal outside patchable rsx! text/style context changed".into(),
            };
        };

        match ctx {
            RsxLitContext::Attr {
                element_open_idx,
                attr_name,
                element_test_id,
                node_path: elem_path,
            } => {
                if let Some(prev_elem) = attr_owner_elem {
                    if prev_elem != *element_open_idx {
                        return DiffOutcome::RequireTier2Rebuild {
                            reason: "attribute edits span multiple RSX elements".into(),
                        };
                    }
                } else {
                    attr_owner_elem = Some(*element_open_idx);
                    target_test_id = element_test_id.clone();
                    node_path = elem_path.clone();
                }
                attr_updates.push((attr_name.clone(), new_val));
            }
            RsxLitContext::Text {
                text_index,
                parent_test_id,
                node_path: tpath,
            } => {
                if old_val.is_empty() {
                    return DiffOutcome::RequireTier2Rebuild {
                        reason: "empty static text literal cannot be matched uniquely".into(),
                    };
                }
                if !seen_text_change {
                    seen_text_change = true;
                    text_target_test_id = parent_test_id.clone();
                    text_target_path = tpath.clone();
                } else if text_target_test_id != *parent_test_id {
                    text_owners_uniform = false;
                }
                text_replacements.push((old_val, new_val.clone()));
                text_updates.push((*text_index, new_val));
            }
        }
    }

    if attr_updates.is_empty() && text_replacements.is_empty() {
        return DiffOutcome::NoOp;
    }

    // If no attribute edit claimed targeting, carry the single text element's targeting metadata.
    if attr_owner_elem.is_none() && seen_text_change {
        if text_owners_uniform {
            target_test_id = text_target_test_id;
        }
        if text_replacements.len() == 1 {
            node_path = text_target_path;
        }
    }

    DiffOutcome::Tier1Patch(HotPatchMsg::TemplatePatch {
        generation,
        template_id: template_id.to_owned(),
        attr_updates,
        text_updates,
        target_test_id,
        node_path,
        text_replacements,
    })
}
