//! Span-by-span structural diff of watched `.rs` sources and Tier-1 typed patch classification.
//!
//! Tokens outside the baseline's patchable spans must match token for token (comments and
//! whitespace are not tokens). Inside a patchable attribute value span the token count may change
//! freely (`8.0` -> `-8.0`, `12` -> `px(16.0)`); both sides must evaluate to typed constants.

use super::DiffOutcome;
use super::attrs::AttrSpec;
use super::eval::{TypedAttrValue, evaluate};
use super::lexer::{
    LexResult, TokenKind, has_interpolation_braces, line_col, tokenize, unescape_format_braces,
    unescape_string_literal,
};
use super::parser::{RsxIndex, SpanKind, value_span_end};
use super::patch::{RsxBlockCoord, TemplateTarget, TextChange, TypedAttrChange, TypedRsxPatch};

/// Diffs `old_src` vs `new_src` for a single `.rs` file and classifies the edit as `NoOp`,
/// `Tier1Patch`, or `RequireTier2Rebuild`.
pub fn diff_rsx_source(
    template_id: &str,
    old_src: &str,
    new_src: &str,
    generation: u64,
) -> DiffOutcome {
    let patch = match diff_rsx_typed(old_src, new_src) {
        Ok(Some(patch)) => patch,
        Ok(None) => return DiffOutcome::NoOp,
        Err(reason) => return DiffOutcome::RequireTier2Rebuild { reason },
    };
    match patch.to_template_patches(template_id, generation) {
        Ok(messages) => DiffOutcome::Tier1Patch { patch, messages },
        Err(reason) => DiffOutcome::RequireTier2Rebuild { reason },
    }
}

/// Typed span diff: `Ok(None)` when nothing changed semantically, `Err(reason)` when the edit
/// needs a Tier-2 rebuild.
pub(super) fn diff_rsx_typed(
    old_src: &str,
    new_src: &str,
) -> Result<Option<TypedRsxPatch>, String> {
    if old_src == new_src {
        return Ok(None);
    }
    let old = tokenize(old_src).map_err(|e| format!("baseline lex error: {}", e.as_str()))?;
    let new = tokenize(new_src).map_err(|e| format!("syntax error: {}", e.as_str()))?;
    let index = RsxIndex::build(&old);
    let mut diff = SpanDiff {
        old_src,
        new_src,
        old: &old,
        new: &new,
        index: &index,
        patch: TypedRsxPatch::default(),
    };

    let (mut i, mut j) = (0usize, 0usize);
    for span in &index.spans {
        let (start, end) = (span.start as usize, span.end as usize);
        j = diff.match_structure(i, start, j)?;
        let new_end = match span.kind {
            SpanKind::Text { .. } => match new.tokens.get(j) {
                Some(TokenKind::StringLit { .. }) => j + 1,
                _ => return Err(diff.structural_reason(start, end, j)),
            },
            SpanKind::Attr(_) => value_span_end(&new, j, new.tokens.len()),
        };
        if old.tokens[start..end] != new.tokens[j..new_end] {
            match span.kind {
                SpanKind::Text { text_index } => {
                    diff.record_text(span.node, text_index, start, j)?;
                }
                SpanKind::Attr(spec) => {
                    diff.record_attr(span.node, spec, start..end, j..new_end)?;
                }
            }
        }
        (i, j) = (end, new_end);
    }
    let old_len = old.tokens.len();
    let tail = diff.match_structure(i, old_len, j)?;
    if tail != new.tokens.len() {
        return Err(diff.structural_reason(old_len, old_len, tail));
    }
    Ok((!diff.patch.is_empty()).then_some(diff.patch))
}

struct SpanDiff<'s, 'l> {
    old_src: &'s str,
    new_src: &'s str,
    old: &'l LexResult<'s>,
    new: &'l LexResult<'s>,
    index: &'l RsxIndex,
    patch: TypedRsxPatch,
}

impl SpanDiff<'_, '_> {
    /// Requires `old[i..i_end]` to equal the new tokens from `j`; returns the new index after them.
    fn match_structure(&self, i: usize, i_end: usize, j: usize) -> Result<usize, String> {
        let len = i_end - i;
        match self.new.tokens.get(j..j + len) {
            Some(new) if new == &self.old.tokens[i..i_end] => Ok(j + len),
            _ => Err(self.structural_reason(i, i_end, j)),
        }
    }

    /// Diagnostic for a token mismatch, pointing at the first differing token's line in the
    /// edited source.
    fn structural_reason(&self, i: usize, i_end: usize, j: usize) -> String {
        let (old, new) = (&self.old.tokens, &self.new.tokens);
        let first_diff = (0..)
            .find(|&k| i + k >= i_end || new.get(j + k) != old.get(i + k))
            .map_or(j, |k| j + k);
        format!(
            "structural or logic edit outside patchable rsx! values (line {})",
            self.new_line(first_diff)
        )
    }

    fn new_line(&self, tok: usize) -> u32 {
        let offset = self
            .new
            .starts
            .get(tok)
            .map_or(self.new_src.len(), |&s| s as usize);
        line_col(self.new_src, offset).0
    }

    fn target(&self, node: u32) -> TemplateTarget {
        let node = &self.index.nodes[node as usize];
        let macro_tok = self.index.blocks[node.block as usize] as usize;
        let (line, column) = line_col(self.old_src, self.old.starts[macro_tok] as usize);
        TemplateTarget {
            block: RsxBlockCoord {
                ordinal: node.block,
                line,
                column,
            },
            anchor_tok: node.anchor_tok,
            node_path: self.index.node_path(node),
            test_id: self.index.test_id(self.old, node),
        }
    }

    fn record_attr(
        &mut self,
        node: u32,
        spec: &'static AttrSpec,
        old_span: std::ops::Range<usize>,
        new_span: std::ops::Range<usize>,
    ) -> Result<(), String> {
        let (name, line) = (spec.name, self.new_line(new_span.start));
        let old = evaluate(self.old, old_span.start, old_span.end).map_err(|e| {
            format!(
                "`{name}` baseline value is not a Tier-1 constant: {} (line {line})",
                e.as_str()
            )
        })?;
        let new = evaluate(self.new, new_span.start, new_span.end).map_err(|e| {
            format!(
                "`{name}` value is not a Tier-1 constant: {} (line {line})",
                e.as_str()
            )
        })?;
        if old == new {
            return Ok(());
        }
        if !spec.accepts(&new) {
            return Err(format!("`{name}` does not accept {new} (line {line})"));
        }
        if let (
            TypedAttrValue::Enum {
                ty: Some(old_ty), ..
            },
            TypedAttrValue::Enum {
                ty: Some(new_ty), ..
            },
        ) = (&old, &new)
            && old_ty != new_ty
        {
            return Err(format!(
                "`{name}` enum type changed from `{old_ty}` to `{new_ty}` (line {line})"
            ));
        }
        let target = self.target(node);
        self.patch.attr_changes.push(TypedAttrChange {
            target,
            attr: spec,
            old,
            new,
        });
        Ok(())
    }

    fn record_text(
        &mut self,
        node: u32,
        text_index: u32,
        old_tok: usize,
        new_tok: usize,
    ) -> Result<(), String> {
        let line = self.new_line(new_tok);
        let (
            TokenKind::StringLit {
                raw: old_raw,
                is_raw: old_is_raw,
            },
            TokenKind::StringLit {
                raw: new_raw,
                is_raw: new_is_raw,
            },
        ) = (&self.old.tokens[old_tok], &self.new.tokens[new_tok])
        else {
            return Err(self.structural_reason(old_tok, old_tok + 1, new_tok));
        };
        let unescape = |raw: &str, is_raw: bool, side: &str| {
            unescape_string_literal(raw, is_raw).map_err(|e| {
                format!(
                    "invalid {side} string literal: {} (line {line})",
                    e.as_str()
                )
            })
        };
        let old = unescape(old_raw, *old_is_raw, "baseline")?;
        let new = unescape(new_raw, *new_is_raw, "new")?;
        // Semantically identical after raw/escaped normalization (e.g., `"hi"` vs `r#"hi"#`).
        if old == new {
            return Ok(());
        }
        if has_interpolation_braces(&old) || has_interpolation_braces(&new) {
            return Err(format!(
                "interpolated `{{expr}}` text literal changed (line {line})"
            ));
        }
        if old.is_empty() {
            return Err(format!(
                "empty static text literal cannot be matched uniquely (line {line})"
            ));
        }
        let target = self.target(node);
        self.patch.text_changes.push(TextChange {
            target,
            text_index: text_index as usize,
            old: unescape_format_braces(old),
            new: unescape_format_braces(new),
        });
        Ok(())
    }
}
