//! Span-by-span structural diff of watched `.rs` sources and Tier-1 typed patch classification.
//!
//! Tokens outside the baseline's patchable spans must match token for token (comments and
//! whitespace are not tokens). Inside a patchable attribute value span only literal leaves may
//! change (`8.0` -> `-12.5`, `Rgba::rgb(1, 2, 3)` -> `Rgba::rgb(4, 5, 6)`,
//! `FlexDirection::Row` -> `FlexDirection::Column`): same constructor, same literal kinds.

use super::DiffOutcome;
use super::attrs::AttrSpec;
use super::eval::{TypedAttrValue, evaluate, same_shape};
use super::lexer::Lex;
use super::parser::{RsxIndex, SpanKind, value_span_end};
use super::patch::{TemplateCoord, TemplateTarget, TextChange, TypedAttrChange, TypedRsxPatch};
use super::strings::{
    has_interpolation_braces, line_col, unescape_format_braces, unescape_string_literal,
};

fn tier2(reason: String) -> DiffOutcome {
    DiffOutcome::RequireTier2Rebuild { reason }
}

/// Classifies lexed sources, building the baseline index into `index` only when tokens differ.
pub(super) fn classify(
    template_id: &str,
    old: Lex<'_>,
    index: &mut Option<RsxIndex>,
    new: Lex<'_>,
    generation: u64,
) -> DiffOutcome {
    if old.len() == new.len() && old.same_range(0, old.len(), new, 0) {
        return DiffOutcome::NoOp;
    }
    let index = index.get_or_insert_with(|| RsxIndex::build(old));
    let patch = match diff_rsx_typed(old, index, new) {
        Ok(Some(patch)) => patch,
        Ok(None) => return DiffOutcome::NoOp,
        Err(reason) => return tier2(reason),
    };
    match patch.to_template_patches(template_id, generation) {
        Ok(messages) => DiffOutcome::Tier1Patch { patch, messages },
        Err(reason) => tier2(reason),
    }
}

/// Typed span diff: `Ok(None)` when nothing changed semantically, `Err(reason)` when the edit
/// needs a Tier-2 rebuild.
pub(super) fn diff_rsx_typed(
    old: Lex<'_>,
    index: &RsxIndex,
    new: Lex<'_>,
) -> Result<Option<TypedRsxPatch>, String> {
    let (old_len, new_len) = (old.len(), new.len());
    let prefix = (0..old_len.min(new_len))
        .find(|&k| !old.same(k, new, k))
        .unwrap_or(old_len.min(new_len));
    let suffix = (0..old_len.min(new_len) - prefix)
        .find(|&k| !old.same(old_len - 1 - k, new, new_len - 1 - k))
        .unwrap_or(old_len.min(new_len) - prefix);
    let suffix_start = old_len - suffix;
    let delta = new_len as isize - old_len as isize;

    // Spans ending before the first difference are untouched: start at the first span that can
    // contain or abut it.
    let first = index.spans.partition_point(|s| (s.end as usize) < prefix);
    let mut diff = SpanDiff {
        old,
        new,
        index,
        patch: TypedRsxPatch::default(),
    };
    let mut i = index
        .spans
        .get(first)
        .map_or(prefix, |s| (s.start as usize).min(prefix));
    let mut j = i;
    for span in &index.spans[first..] {
        let (start, end) = (span.start as usize, span.end as usize);
        // Past the changed region with offsets realigned: the rest is the common suffix.
        if i >= suffix_start && j as isize - i as isize == delta {
            return Ok(diff.finish());
        }
        j = diff.match_structure(i, start, j)?;
        let new_end = match span.kind {
            SpanKind::Text if new.string_lit(j).is_some() => j + 1,
            SpanKind::Text => return Err(diff.structural_reason(start, end, j)),
            SpanKind::Attr(_) => value_span_end(new, j, new_len),
        };
        if !old.same_range(start, end, new, j) || end - start != new_end - j {
            match span.kind {
                SpanKind::Text => diff.record_text(span.node, start, j)?,
                SpanKind::Attr(spec) => {
                    diff.record_attr(span.node, spec, (start, end), (j, new_end))?
                }
            }
        }
        (i, j) = (end, new_end);
    }
    if diff.match_structure(i, old_len, j)? != new_len {
        return Err(diff.structural_reason(old_len, old_len, j + old_len - i));
    }
    Ok(diff.finish())
}

struct SpanDiff<'s, 'l> {
    old: Lex<'s>,
    new: Lex<'s>,
    index: &'l RsxIndex,
    patch: TypedRsxPatch,
}

impl SpanDiff<'_, '_> {
    fn finish(self) -> Option<TypedRsxPatch> {
        (!self.patch.is_empty()).then_some(self.patch)
    }

    /// Requires `old[i..i_end]` to equal the new tokens from `j`; returns the new index after them.
    fn match_structure(&self, i: usize, i_end: usize, j: usize) -> Result<usize, String> {
        if self.old.same_range(i, i_end, self.new, j) {
            Ok(j + i_end - i)
        } else {
            Err(self.structural_reason(i, i_end, j))
        }
    }

    /// Diagnostic for a token mismatch, pointing at the first differing token's line in the
    /// edited source.
    fn structural_reason(&self, i: usize, i_end: usize, j: usize) -> String {
        let (old, new) = (self.old, self.new);
        let first_diff = (0..)
            .find(|&k| i + k >= i_end || j + k >= new.len() || !old.same(i + k, new, j + k))
            .map_or(j, |k| j + k);
        format!(
            "structural or logic edit outside patchable rsx! values (line {})",
            self.new_line(first_diff)
        )
    }

    fn new_line(&self, tok: usize) -> u32 {
        let offset = if tok < self.new.len() {
            self.new.start(tok)
        } else {
            self.new.src.len()
        };
        line_col(self.new.src, offset).0
    }

    fn target(&self, node: u32) -> TemplateTarget {
        let node = &self.index.nodes[node as usize];
        let anchor = self.index.templates[node.template as usize] as usize;
        let (line, column) = line_col(self.old.src, self.old.start(anchor));
        TemplateTarget {
            template: TemplateCoord {
                ordinal: node.template,
                line,
                column,
            },
            node_path: self.index.node_path(node),
            test_id: self.index.test_id(self.old, node),
            anchor_tok: node.anchor_tok,
        }
    }

    fn record_attr(
        &mut self,
        node: u32,
        spec: &'static AttrSpec,
        (old_start, old_end): (usize, usize),
        (new_start, new_end): (usize, usize),
    ) -> Result<(), String> {
        let name = spec.name;
        let line = || self.new_line(new_start);
        let old = evaluate(self.old, old_start, old_end).map_err(|e| {
            format!(
                "`{name}` baseline value is not a Tier-1 constant: {} (line {})",
                e.as_str(),
                line()
            )
        })?;
        let new = evaluate(self.new, new_start, new_end).map_err(|e| {
            format!(
                "`{name}` value is not a Tier-1 constant: {} (line {})",
                e.as_str(),
                line()
            )
        })?;
        let leaf_ident = matches!(
            old,
            TypedAttrValue::Enum { .. } | TypedAttrValue::ThemeToken(_)
        );
        if !same_shape(
            self.old,
            (old_start, old_end),
            self.new,
            (new_start, new_end),
            leaf_ident,
        ) {
            return Err(format!(
                "`{name}` value shape changed ({old} -> {new}); Tier-1 only patches literal \
                 values (line {})",
                line()
            ));
        }
        if old == new {
            return Ok(());
        }
        if !spec.accepts_value(&new) {
            return Err(format!("`{name}` does not accept {new} (line {})", line()));
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

    fn record_text(&mut self, node: u32, old_tok: usize, new_tok: usize) -> Result<(), String> {
        let line = || self.new_line(new_tok);
        let (Some((old_raw, old_is_raw)), Some((new_raw, new_is_raw))) =
            (self.old.string_lit(old_tok), self.new.string_lit(new_tok))
        else {
            return Err(self.structural_reason(old_tok, old_tok + 1, new_tok));
        };
        let unescape = |raw: &str, is_raw: bool, side: &str| {
            unescape_string_literal(raw, is_raw).map_err(|e| {
                format!(
                    "invalid {side} string literal: {} (line {})",
                    e.as_str(),
                    line()
                )
            })
        };
        let old = unescape(old_raw, old_is_raw, "baseline")?;
        let new = unescape(new_raw, new_is_raw, "new")?;
        // Semantically identical after raw/escaped normalization (e.g., `"hi"` vs `r#"hi"#`).
        if old == new {
            return Ok(());
        }
        if has_interpolation_braces(&old) || has_interpolation_braces(&new) {
            return Err(format!(
                "interpolated `{{expr}}` text literal changed (line {})",
                line()
            ));
        }
        if old.is_empty() {
            return Err(format!(
                "empty static text literal cannot be matched uniquely (line {})",
                line()
            ));
        }
        let target = self.target(node);
        self.patch.text_changes.push(TextChange {
            target,
            old: unescape_format_braces(old),
            new: unescape_format_braces(new),
        });
        Ok(())
    }
}
