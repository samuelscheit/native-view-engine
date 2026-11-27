//! Structural `rsx! { ... }` scanner indexing every patchable value span (static text children and
//! native-element style attribute values of any token count) with its template coordinates.
//!
//! Templates mirror Dioxus: every `rsx!` invocation and every `if`/`else`/`for` body is its own
//! template; inside its parent it occupies one child slot (a dynamic node). Node paths are child
//! indices relative to the innermost template root.

use std::ops::Range;

use super::attrs::{AttrSpec, is_native_tag, is_test_id_attr, lookup_patchable_attr};
use super::lexer::{Lex, Tok};
use super::strings::{
    has_interpolation_braces, string_lit_inner, unescape_format_braces, unescape_string_literal,
};

/// What a patchable span holds.
#[derive(Debug, Clone, Copy)]
pub(super) enum SpanKind {
    /// Native-element style attribute value (`padding: 8.0`), any token count.
    Attr(&'static AttrSpec),
    /// Static text child literal (exactly one string token).
    Text,
}

/// A patchable baseline token span `start..end`.
#[derive(Debug, Clone, Copy)]
pub(super) struct PatchSpan {
    pub(super) start: u32,
    pub(super) end: u32,
    /// Index into [`RsxIndex::nodes`]: the owning element (attrs) or the text node itself.
    pub(super) node: u32,
    pub(super) kind: SpanKind,
}

/// Template coordinates of an indexed RSX node.
#[derive(Debug, Clone)]
pub(super) struct RsxNode {
    /// Ordinal (file order) of the innermost enclosing template.
    pub(super) template: u32,
    /// Token index of the element's opening `{`, or of the text literal.
    pub(super) anchor_tok: u32,
    /// Child-index path from the template root (range into the shared path arena).
    path: Range<u32>,
    /// Static `test_id` literal token: the element's own (attributes) or the nearest native
    /// ancestor's (text).
    test_id_tok: Option<u32>,
}

/// Every patchable span of one source file plus the coordinates of the nodes owning them.
#[derive(Debug, Default)]
pub(super) struct RsxIndex {
    /// Patchable spans, sorted by `start` and never overlapping.
    pub(super) spans: Vec<PatchSpan>,
    pub(super) nodes: Vec<RsxNode>,
    /// Anchor token of each template (`rsx` identifier or control-flow body `{`), by ordinal.
    pub(super) templates: Vec<u32>,
    paths: Vec<u32>,
}

impl RsxIndex {
    pub(super) fn build(lex: Lex<'_>) -> Self {
        // One patchable span per ~24 tokens and two path entries per node in dense RSX.
        let estimate = lex.len() / 24;
        let mut scanner = Scanner {
            lex,
            index: Self {
                spans: Vec::with_capacity(estimate),
                nodes: Vec::with_capacity(estimate),
                templates: Vec::with_capacity(estimate / 8),
                paths: Vec::with_capacity(estimate * 3),
            },
            path: Vec::with_capacity(16),
            path_base: 0,
            template: 0,
        };
        scanner.scan_rust_range(0, lex.len());
        let index = scanner.index;
        debug_assert!(
            index.spans.windows(2).all(|w| w[0].end <= w[1].start),
            "patch spans must be sorted and disjoint"
        );
        index
    }

    pub(super) fn node_path(&self, node: &RsxNode) -> Vec<usize> {
        let range = node.path.start as usize..node.path.end as usize;
        self.paths[range].iter().map(|&i| i as usize).collect()
    }

    /// The node's static `test_id`, when it is a non-empty, interpolation-free literal.
    pub(super) fn test_id(&self, lex: Lex<'_>, node: &RsxNode) -> Option<String> {
        let (raw, is_raw) = lex.string_lit(node.test_id_tok? as usize)?;
        let value = unescape_string_literal(raw, is_raw).ok()?;
        (!value.is_empty() && !has_interpolation_braces(&value))
            .then(|| unescape_format_braces(value))
    }
}

fn is_rust_keyword(ident: &str) -> bool {
    matches!(
        ident,
        "if" | "else"
            | "for"
            | "match"
            | "while"
            | "loop"
            | "let"
            | "move"
            | "async"
            | "unsafe"
            | "return"
            | "fn"
            | "struct"
            | "enum"
            | "impl"
            | "mod"
            | "use"
            | "in"
            | "const"
            | "break"
    )
}

struct Scanner<'a> {
    lex: Lex<'a>,
    index: RsxIndex,
    /// Child-index stack; entries from `path_base` on form the current node path.
    path: Vec<u32>,
    path_base: usize,
    template: u32,
}

impl Scanner<'_> {
    fn is_rsx_macro(&self, pos: usize, end: usize) -> bool {
        pos + 2 < end
            && self.lex.is_ident(pos, "rsx")
            && self.lex.is_punct(pos + 1, b'!')
            && self.lex.is_punct(pos + 2, b'{')
    }

    fn push_node(&mut self, anchor_tok: usize, test_id_tok: Option<u32>) -> u32 {
        let start = self.index.paths.len() as u32;
        self.index
            .paths
            .extend_from_slice(&self.path[self.path_base..]);
        self.index.nodes.push(RsxNode {
            template: self.template,
            anchor_tok: anchor_tok as u32,
            path: start..self.index.paths.len() as u32,
            test_id_tok,
        });
        (self.index.nodes.len() - 1) as u32
    }

    /// Scans `open+1..close` as the children of a new template anchored at `anchor`.
    fn scan_template(&mut self, anchor: usize, open: usize, owner: Option<u32>) -> usize {
        let close = self.lex.close(open);
        let (saved_template, saved_base) = (self.template, self.path_base);
        self.template = self.index.templates.len() as u32;
        self.index.templates.push(anchor as u32);
        self.path_base = self.path.len();
        self.scan_rsx_children(open + 1, close, owner);
        (self.template, self.path_base) = (saved_template, saved_base);
        close + 1
    }

    /// Scans normal Rust tokens looking for `rsx! { ... }` macro invocations.
    fn scan_rust_range(&mut self, start: usize, end: usize) {
        let mut pos = start;
        while pos + 2 < end {
            if self.is_rsx_macro(pos, end) && self.lex.close(pos + 2) < end {
                pos = self.scan_template(pos, pos + 2, None);
                continue;
            }
            pos += 1;
        }
    }

    fn scan_rsx_children(&mut self, start: usize, end: usize, owner: Option<u32>) {
        let mut pos = start;
        let mut child_idx = 0u32;
        while pos < end {
            pos = self.scan_one_rsx_child(pos, end, owner, &mut child_idx);
        }
    }

    /// Scans `<head> { <rsx> } (else if <head> { <rsx> })* (else { <rsx> })?` from `pos` (just
    /// past `if`); every body is its own template. Returns the next token index.
    fn scan_if_chain(&mut self, mut pos: usize, end: usize, owner: Option<u32>) -> usize {
        loop {
            let Some(body) = find_body_brace(self.lex, pos, end, false) else {
                self.scan_rust_range(pos, end);
                return end;
            };
            self.scan_rust_range(pos, body);
            pos = self.scan_template(body, body, owner);
            if !self.lex.is_ident(pos, "else") || pos >= end {
                return pos;
            }
            pos += 1;
            if self.lex.is_ident(pos, "if") {
                pos += 1;
            } else if self.lex.is_punct(pos, b'{') {
                return self.scan_template(pos, pos, owner);
            } else {
                return pos;
            }
        }
    }

    /// Scans one RSX child item starting at `pos` and returns the next token index.
    fn scan_one_rsx_child(
        &mut self,
        pos: usize,
        end: usize,
        owner: Option<u32>,
        child_idx: &mut u32,
    ) -> usize {
        let lex = self.lex;
        if lex.is_punct(pos, b',') || lex.is_punct(pos, b';') {
            return pos + 1;
        }
        // Dynamic children (nested `rsx!`, control flow, Rust blocks) take one slot each.
        let dynamic_slot = |idx: &mut u32| *idx += 1;

        if self.is_rsx_macro(pos, end) {
            dynamic_slot(child_idx);
            return self.scan_template(pos, pos + 2, owner);
        }
        if lex.is_ident(pos, "if") {
            dynamic_slot(child_idx);
            return self.scan_if_chain(pos + 1, end, owner);
        }
        if lex.is_ident(pos, "for") {
            dynamic_slot(child_idx);
            let Some(body) = find_body_brace(lex, pos + 1, end, true) else {
                self.scan_rust_range(pos + 1, end);
                return end;
            };
            self.scan_rust_range(pos + 1, body);
            return self.scan_template(body, body, owner);
        }
        if lex.is_ident(pos, "match") {
            dynamic_slot(child_idx);
            let Some(arms) = find_body_brace(lex, pos + 1, end, false) else {
                self.scan_rust_range(pos + 1, end);
                return end;
            };
            let close = lex.close(arms);
            self.scan_rust_range(pos + 1, close);
            return close + 1;
        }
        if lex.is_punct(pos, b'{') {
            dynamic_slot(child_idx);
            let close = lex.close(pos);
            self.scan_rust_range(pos + 1, close);
            return close + 1;
        }
        if let Some(tag) = lex.ident(pos)
            && !is_rust_keyword(tag)
            && lex.is_punct(pos + 1, b'{')
            && pos + 1 < end
        {
            // `ui::Card { .. }` is a component even when its last segment is lowercase.
            let path_qualified = pos >= 1 && lex.is_punct(pos - 1, b':');
            let is_native = !path_qualified && is_native_tag(tag);
            self.path.push(*child_idx);
            *child_idx += 1;
            let close = self.scan_rsx_element(pos + 1, is_native, owner);
            self.path.pop();
            return close + 1;
        }
        if matches!(lex.kind(pos), Tok::Str | Tok::RawStr) && !lex.is_punct(pos + 1, b':') {
            self.path.push(*child_idx);
            *child_idx += 1;
            let node = self.push_node(pos, owner);
            self.path.pop();
            self.index.spans.push(PatchSpan {
                start: pos as u32,
                end: pos as u32 + 1,
                node,
                kind: SpanKind::Text,
            });
            return pos + 1;
        }
        if lex.is_punct(pos, b'(') || lex.is_punct(pos, b'[') {
            let close = lex.close(pos);
            self.scan_rust_range(pos + 1, close);
            return close + 1;
        }
        // Any other token (shorthand prop, path segment) is skipped safely.
        pos + 1
    }

    /// Scans an element or component body opened at `open`; returns its closing index.
    fn scan_rsx_element(&mut self, open: usize, is_native: bool, owner: Option<u32>) -> usize {
        let lex = self.lex;
        let close = lex.close(open);
        // Only native elements carry a `DebugId`; a component's `test_id` is just a prop.
        let own_test_id = is_native
            .then(|| find_element_static_test_id(lex, open + 1, close))
            .flatten();
        let child_owner = own_test_id.or(owner);
        let mut node = None;

        let mut pos = open + 1;
        let mut child_idx = 0u32;
        while pos < close {
            if let Some(attr) = parse_rsx_attribute(lex, pos, close) {
                match lookup_patchable_attr(attr.name).filter(|_| is_native) {
                    Some(spec) => {
                        let node = match node {
                            Some(n) => n,
                            None => *node.insert(self.push_node(open, own_test_id)),
                        };
                        self.index.spans.push(PatchSpan {
                            start: attr.value.start as u32,
                            end: attr.value.end as u32,
                            node,
                            kind: SpanKind::Attr(spec),
                        });
                    }
                    None => self.scan_rust_range(attr.value.start, attr.value.end),
                }
                pos = attr.next;
                continue;
            }
            // Shorthand component prop (`accent,`).
            if lex.ident(pos).is_some_and(|id| !is_rust_keyword(id)) && lex.is_punct(pos + 1, b',')
            {
                pos += 2;
                continue;
            }
            pos = self.scan_one_rsx_child(pos, close, child_owner, &mut child_idx);
        }
        close
    }
}

/// Token index of the first depth-0 `test_id` / `testID` / `"data-testid"` whose value is a
/// single string literal.
fn find_element_static_test_id(lex: Lex<'_>, start: usize, end: usize) -> Option<u32> {
    let mut pos = start;
    while pos < end {
        if let Some(attr) = parse_rsx_attribute(lex, pos, end) {
            if is_test_id_attr(attr.name)
                && attr.value.len() == 1
                && lex.string_lit(attr.value.start).is_some()
            {
                return Some(attr.value.start as u32);
            }
            pos = attr.next;
            continue;
        }
        pos = skip_token_or_group(lex, pos);
    }
    None
}

fn skip_token_or_group(lex: Lex<'_>, pos: usize) -> usize {
    if lex.is_punct(pos, b'{') || lex.is_punct(pos, b'(') || lex.is_punct(pos, b'[') {
        lex.close(pos) + 1
    } else {
        pos + 1
    }
}

/// An RSX attribute `name: <value>` (or `"quoted-name": <value>`).
struct RsxAttr<'a> {
    /// Attribute name; empty for a quoted name that needs unescaping (never patchable).
    name: &'a str,
    value: Range<usize>,
    /// Token index after the value and its `,` separator.
    next: usize,
}

/// Parses an RSX attribute at `pos` (bounded by the element's closing `end`).
fn parse_rsx_attribute<'a>(lex: Lex<'a>, pos: usize, end: usize) -> Option<RsxAttr<'a>> {
    // `name :` but not a `::` path.
    if pos + 2 >= end || !lex.is_punct(pos + 1, b':') || lex.is_punct(pos + 2, b':') {
        return None;
    }
    let name = match lex.kind(pos) {
        Tok::Ident if !is_rust_keyword(lex.text(pos)) => lex.text(pos),
        Tok::Str | Tok::RawStr => {
            string_lit_inner(lex.text(pos), lex.kind(pos) == Tok::RawStr).unwrap_or("")
        }
        _ => return None,
    };
    let value_end = value_span_end(lex, pos + 2, end);
    let next = value_end + usize::from(value_end < end && lex.is_punct(value_end, b','));
    Some(RsxAttr {
        name,
        value: pos + 2..value_end,
        next,
    })
}

/// End of an attribute value starting at `start`: the first depth-0 `,` or closing delimiter
/// (or `limit`). Shared by the baseline index and the edited source, so both agree on spans.
pub(super) fn value_span_end(lex: Lex<'_>, start: usize, limit: usize) -> usize {
    let mut cur = start;
    while cur < limit {
        if lex.kind(cur) == Tok::Punct {
            match lex.src.as_bytes()[lex.start(cur)] {
                b',' | b'}' | b')' | b']' => return cur,
                b'{' | b'(' | b'[' => {
                    cur = lex.close(cur) + 1;
                    continue;
                }
                _ => {}
            }
        }
        cur += 1;
    }
    limit
}

/// Whether token `i` leaves the parser expecting an operand, so a following `{` opens a block
/// expression (part of an `if`/`match` head) rather than the body.
fn expects_operand(lex: Lex<'_>, i: usize) -> bool {
    match lex.kind(i) {
        Tok::Punct => !matches!(lex.src.as_bytes()[lex.start(i)], b')' | b']' | b'}' | b'?'),
        Tok::Ident => matches!(
            lex.text(i),
            "if" | "in"
                | "return"
                | "move"
                | "unsafe"
                | "async"
                | "loop"
                | "const"
                | "break"
                | "match"
        ),
        _ => false,
    }
}

/// Whether the token at `k` continues an expression after a `{ .. }` group.
fn continues_expr(lex: Lex<'_>, k: usize, end: usize) -> bool {
    k < end
        && (lex.is_ident(k, "as")
            || (lex.kind(k) == Tok::Punct
                && matches!(
                    lex.src.as_bytes()[lex.start(k)],
                    b'.' | b'?'
                        | b'='
                        | b'!'
                        | b'<'
                        | b'>'
                        | b'+'
                        | b'-'
                        | b'*'
                        | b'/'
                        | b'%'
                        | b'^'
                        | b'&'
                        | b'|'
                )))
}

/// Index of the depth-0 binding `=` of a `let` pattern (not `==`, `=>`, `<=`, `..=`, `+=`, ...).
fn find_let_eq(lex: Lex<'_>, mut cur: usize, end: usize) -> Option<usize> {
    while cur < end {
        if lex.is_punct(cur, b'=')
            && !lex.is_punct(cur + 1, b'=')
            && !lex.is_punct(cur + 1, b'>')
            && !(cur > 0
                && lex.kind(cur - 1) == Tok::Punct
                && b"=!<>+-*/%^&|.".contains(&lex.src.as_bytes()[lex.start(cur - 1)]))
        {
            return Some(cur);
        }
        cur = skip_token_or_group(lex, cur);
    }
    None
}

/// Finds the body `{` of an `if`/`match` head (or, with `is_for`, a `for PAT in EXPR` head)
/// starting at `start`. Struct-pattern braces (`if let S { .. } = s`, `for S { .. } in xs`),
/// block expressions in operand position (`if { a } {`), match arms inside the head and braces
/// followed by an operator are all part of the head.
fn find_body_brace(lex: Lex<'_>, start: usize, end: usize, is_for: bool) -> Option<usize> {
    let mut cur = start;
    if is_for {
        while !lex.is_ident(cur, "in") {
            if cur >= end {
                return None;
            }
            cur = skip_token_or_group(lex, cur);
        }
        cur += 1;
    }
    let mut operand_expected = true;
    let mut pending_match_arms = 0u32;
    while cur < end {
        if lex.is_ident(cur, "let") {
            cur = find_let_eq(lex, cur + 1, end)? + 1;
            operand_expected = true;
            continue;
        }
        if lex.is_punct(cur, b'{') {
            let close = lex.close(cur);
            let is_head = if operand_expected {
                true
            } else if pending_match_arms > 0 {
                pending_match_arms -= 1;
                true
            } else {
                continues_expr(lex, close + 1, end)
            };
            if !is_head {
                return Some(cur);
            }
            cur = close + 1;
            operand_expected = false;
            continue;
        }
        if lex.is_ident(cur, "match") {
            pending_match_arms += 1;
        }
        let is_group = lex.is_punct(cur, b'(') || lex.is_punct(cur, b'[');
        operand_expected = !is_group && expects_operand(lex, cur);
        cur = skip_token_or_group(lex, cur);
    }
    None
}
