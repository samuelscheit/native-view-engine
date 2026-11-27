//! Structural `rsx! { ... }` scanner indexing every patchable value span (static text children and
//! native-element style attribute values of any token count) with its template coordinates.

use std::ops::Range;

use super::attrs::{AttrSpec, is_test_id_attr, lookup_patchable_attr};
use super::lexer::{
    LexResult, TokenKind, has_interpolation_braces, string_lit_inner, unescape_format_braces,
    unescape_string_literal,
};

/// What a patchable span holds.
#[derive(Debug, Clone, Copy)]
pub(super) enum SpanKind {
    /// Native-element style attribute value (`padding: px(8.0)`), any token count.
    Attr(&'static AttrSpec),
    /// Static text child literal (exactly one `StringLit` token); `text_index` counts text
    /// children in file order.
    Text { text_index: u32 },
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
    /// Ordinal (file order) of the innermost enclosing `rsx!` invocation.
    pub(super) block: u32,
    /// Token index of the element's opening `{`, or of the text literal.
    pub(super) anchor_tok: u32,
    /// Child-index path, as a range into the shared path arena.
    path: Range<u32>,
    /// Token index of the element's (text: the enclosing element's) static `test_id` literal.
    test_id_tok: Option<u32>,
}

/// Every patchable span of one source file plus the coordinates of the nodes owning them.
pub(super) struct RsxIndex {
    /// Patchable spans, sorted by `start` and never overlapping.
    pub(super) spans: Vec<PatchSpan>,
    pub(super) nodes: Vec<RsxNode>,
    /// Token index of each `rsx` macro identifier, by block ordinal.
    pub(super) blocks: Vec<u32>,
    paths: Vec<u32>,
}

impl RsxIndex {
    pub(super) fn build(lex: &LexResult<'_>) -> Self {
        let mut scanner = Scanner {
            lex,
            index: Self {
                spans: Vec::new(),
                nodes: Vec::new(),
                blocks: Vec::new(),
                paths: Vec::new(),
            },
            path: Vec::with_capacity(16),
            path_base: 0,
            block: 0,
            next_text_index: 0,
        };
        scanner.scan_rust_range(0, lex.tokens.len());
        let mut index = scanner.index;
        index.spans.sort_unstable_by_key(|s| s.start);
        index
    }

    pub(super) fn node_path(&self, node: &RsxNode) -> Vec<usize> {
        let range = node.path.start as usize..node.path.end as usize;
        self.paths[range].iter().map(|&i| i as usize).collect()
    }

    /// The node's static `test_id`, when it is a non-empty, interpolation-free literal.
    pub(super) fn test_id(&self, lex: &LexResult<'_>, node: &RsxNode) -> Option<String> {
        let TokenKind::StringLit { raw, is_raw } = lex.tokens[node.test_id_tok? as usize] else {
            return None;
        };
        let value = unescape_string_literal(raw, is_raw).ok()?;
        (!value.is_empty() && !has_interpolation_braces(&value))
            .then(|| unescape_format_braces(value))
    }
}

fn is_rust_control_keyword(ident: &str) -> bool {
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
    )
}

struct Scanner<'l, 'a> {
    lex: &'l LexResult<'a>,
    index: RsxIndex,
    /// Child-index stack; entries from `path_base` on form the current node path.
    path: Vec<u32>,
    path_base: usize,
    block: u32,
    next_text_index: u32,
}

impl Scanner<'_, '_> {
    fn is_rsx_macro(&self, pos: usize, end: usize) -> bool {
        let t = &self.lex.tokens;
        pos + 2 < end
            && t[pos] == TokenKind::Ident("rsx")
            && t[pos + 1] == TokenKind::Punct('!')
            && t[pos + 2] == TokenKind::Punct('{')
    }

    fn push_node(&mut self, anchor_tok: usize, test_id_tok: Option<u32>) -> u32 {
        let start = self.index.paths.len() as u32;
        self.index
            .paths
            .extend_from_slice(&self.path[self.path_base..]);
        self.index.nodes.push(RsxNode {
            block: self.block,
            anchor_tok: anchor_tok as u32,
            path: start..self.index.paths.len() as u32,
            test_id_tok,
        });
        (self.index.nodes.len() - 1) as u32
    }

    /// Scans normal Rust tokens looking for `rsx! { ... }` macro invocations.
    fn scan_rust_range(&mut self, start: usize, end: usize) {
        let mut pos = start;
        while pos + 2 < end {
            if self.is_rsx_macro(pos, end) {
                let open = pos + 2;
                let close = self.lex.matching_close[open];
                if close > open && close < end {
                    self.scan_rsx_block(pos, close, None, true);
                    pos = close + 1;
                    continue;
                }
            }
            pos += 1;
        }
    }

    /// Scans one `rsx! { ... }` body as its own template block. A macro in Rust code starts a
    /// fresh node path; a macro in RSX child position continues the runtime child path.
    fn scan_rsx_block(
        &mut self,
        macro_tok: usize,
        close: usize,
        parent_test_id: Option<u32>,
        fresh_path: bool,
    ) {
        let (saved_block, saved_base) = (self.block, self.path_base);
        self.block = self.index.blocks.len() as u32;
        self.index.blocks.push(macro_tok as u32);
        if fresh_path {
            self.path_base = self.path.len();
        }
        self.scan_rsx_children(macro_tok + 3, close, parent_test_id);
        (self.block, self.path_base) = (saved_block, saved_base);
    }

    fn scan_rsx_children(&mut self, start: usize, end: usize, parent_test_id: Option<u32>) {
        let mut pos = start;
        let mut child_idx = 0u32;
        while pos < end {
            pos = self.scan_one_rsx_child(pos, end, parent_test_id, &mut child_idx);
        }
    }

    /// Scans `if <cond> { <rsx> } (else if <cond> { <rsx> })* (else { <rsx> })?` from `pos`
    /// (just past `if`) and returns the next token index.
    fn scan_if_chain(&mut self, mut pos: usize, end: usize, parent_test_id: Option<u32>) -> usize {
        let lex = self.lex;
        let t = &lex.tokens;
        loop {
            let Some(brace_open) = find_block_brace_at_depth0(self.lex, pos, end) else {
                self.scan_rust_range(pos, end);
                return end;
            };
            self.scan_rust_range(pos, brace_open);
            let brace_close = self.lex.matching_close[brace_open];
            self.scan_rsx_children(brace_open + 1, brace_close, parent_test_id);
            pos = brace_close + 1;
            if pos >= end || t[pos] != TokenKind::Ident("else") {
                return pos;
            }
            pos += 1;
            if pos < end && t[pos] == TokenKind::Ident("if") {
                pos += 1;
                continue;
            }
            if pos < end && t[pos] == TokenKind::Punct('{') {
                let else_close = self.lex.matching_close[pos];
                self.scan_rsx_children(pos + 1, else_close, parent_test_id);
                return else_close + 1;
            }
            return pos;
        }
    }

    /// Scans one RSX child item starting at `pos` and returns the next token index.
    fn scan_one_rsx_child(
        &mut self,
        pos: usize,
        end: usize,
        parent_test_id: Option<u32>,
        child_idx: &mut u32,
    ) -> usize {
        let lex = self.lex;
        let next_is = |c: char| pos + 1 < end && lex.tokens[pos + 1] == TokenKind::Punct(c);
        match lex.tokens[pos] {
            TokenKind::Punct(',' | ';') => pos + 1,

            // Direct nested `rsx! { ... }` invocation in RSX child position.
            TokenKind::Ident("rsx") if self.is_rsx_macro(pos, end) => {
                let close = lex.matching_close[pos + 2];
                self.scan_rsx_block(pos, close, parent_test_id, false);
                close + 1
            }

            TokenKind::Ident("if") => self.scan_if_chain(pos + 1, end, parent_test_id),

            // `for <pat> in <iter> { <rsx> }`
            TokenKind::Ident("for") => {
                let Some(brace_open) = find_block_brace_at_depth0(lex, pos + 1, end) else {
                    self.scan_rust_range(pos + 1, end);
                    return end;
                };
                self.scan_rust_range(pos + 1, brace_open);
                let brace_close = lex.matching_close[brace_open];
                self.scan_rsx_children(brace_open + 1, brace_close, parent_test_id);
                brace_close + 1
            }

            // `match <expr> { ... }` inside RSX: the body is Rust code (still finds nested `rsx!`).
            TokenKind::Ident("match") => match find_block_brace_at_depth0(lex, pos + 1, end) {
                Some(brace_open) => {
                    let brace_close = lex.matching_close[brace_open];
                    self.scan_rust_range(pos + 1, brace_close);
                    brace_close + 1
                }
                None => end,
            },

            // Embedded Rust block `{ ... }` inside RSX (may contain nested `rsx! { ... }`).
            TokenKind::Punct('{') => {
                let close = lex.matching_close[pos];
                self.scan_rust_range(pos + 1, close);
                close + 1
            }

            // RSX element (`view { ... }`) or component (`Card { ... }`, `ui::Card { ... }`).
            TokenKind::Ident(tag) if !is_rust_control_keyword(tag) && next_is('{') => {
                let open = pos + 1;
                let close = lex.matching_close[open];
                let path_qualified = pos >= 2 && lex.tokens[pos - 1] == TokenKind::Punct(':');
                let is_native =
                    !path_qualified && tag.starts_with(|c: char| c.is_ascii_lowercase());
                self.path.push(*child_idx);
                *child_idx += 1;
                self.scan_rsx_element(open, close, is_native);
                self.path.pop();
                close + 1
            }

            // Static RSX text literal child (`"hello"`, `r#"..."#`); not a quoted attribute name.
            TokenKind::StringLit { .. } if !next_is(':') => {
                let text_index = self.next_text_index;
                self.next_text_index += 1;
                self.path.push(*child_idx);
                *child_idx += 1;
                let node = self.push_node(pos, parent_test_id);
                self.path.pop();
                self.index.spans.push(PatchSpan {
                    start: pos as u32,
                    end: pos as u32 + 1,
                    node,
                    kind: SpanKind::Text { text_index },
                });
                pos + 1
            }

            TokenKind::Punct('(' | '[') => {
                let close = lex.matching_close[pos];
                self.scan_rust_range(pos + 1, close);
                close + 1
            }
            // Any other token (shorthand prop, path segment) is skipped safely.
            _ => pos + 1,
        }
    }

    fn scan_rsx_element(&mut self, open: usize, close: usize, is_native: bool) {
        // Pass 1: the element's static `test_id`, so attributes declared before it still target it.
        let test_id_tok = find_element_static_test_id(self.lex, open + 1, close);
        let mut node = None;

        // Pass 2: attributes and child items inside `{ ... }`.
        let mut pos = open + 1;
        let mut child_idx = 0u32;
        while pos < close {
            if let Some(attr) = parse_rsx_attribute(self.lex, pos, close) {
                match lookup_patchable_attr(attr.name).filter(|_| is_native) {
                    Some(spec) => {
                        let node = match node {
                            Some(n) => n,
                            None => *node.insert(self.push_node(open, test_id_tok)),
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

            // Shorthand component prop (`accent,`): identifier followed by `,`.
            if matches!(self.lex.tokens[pos], TokenKind::Ident(id) if !is_rust_control_keyword(id))
                && pos + 1 < close
                && self.lex.tokens[pos + 1] == TokenKind::Punct(',')
            {
                pos += 2;
                continue;
            }

            pos = self.scan_one_rsx_child(pos, close, test_id_tok, &mut child_idx);
        }
    }
}

/// Token index of the first depth-0 `test_id` / `testID` / `"data-testid"` whose value is a
/// single string literal.
fn find_element_static_test_id(lex: &LexResult<'_>, start: usize, end: usize) -> Option<u32> {
    let mut pos = start;
    while pos < end {
        if let Some(attr) = parse_rsx_attribute(lex, pos, end) {
            if is_test_id_attr(attr.name)
                && attr.value.len() == 1
                && matches!(lex.tokens[attr.value.start], TokenKind::StringLit { .. })
            {
                return Some(attr.value.start as u32);
            }
            pos = attr.next;
            continue;
        }
        pos = match lex.tokens[pos] {
            TokenKind::Punct('{' | '(' | '[') => lex.matching_close[pos] + 1,
            _ => pos + 1,
        };
    }
    None
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
fn parse_rsx_attribute<'a>(lex: &LexResult<'a>, pos: usize, end: usize) -> Option<RsxAttr<'a>> {
    if pos + 2 >= end
        || lex.tokens[pos + 1] != TokenKind::Punct(':')
        // Reject `::` Rust paths (e.g., `std::foo`).
        || lex.tokens[pos + 2] == TokenKind::Punct(':')
    {
        return None;
    }
    let name = match lex.tokens[pos] {
        TokenKind::Ident(id) if !is_rust_control_keyword(id) => id,
        TokenKind::StringLit { raw, is_raw } => string_lit_inner(raw, is_raw).unwrap_or(""),
        _ => return None,
    };
    let value_end = value_span_end(lex, pos + 2, end);
    let next = if value_end < end && lex.tokens[value_end] == TokenKind::Punct(',') {
        value_end + 1
    } else {
        value_end
    };
    Some(RsxAttr {
        name,
        value: pos + 2..value_end,
        next,
    })
}

/// End of an attribute value starting at `start`: the first depth-0 `,` or closing delimiter
/// (or `limit`). Shared by the baseline index and the edited source, so both agree on spans.
pub(super) fn value_span_end(lex: &LexResult<'_>, start: usize, limit: usize) -> usize {
    let mut cur = start;
    while cur < limit {
        match lex.tokens[cur] {
            TokenKind::Punct(',' | '}' | ')' | ']') => return cur,
            TokenKind::Punct('{' | '(' | '[') => cur = lex.matching_close[cur] + 1,
            _ => cur += 1,
        }
    }
    limit
}

/// Finds the opening `{` of an `if` / `for` / `match` block at depth 0, skipping balanced `()`/`[]`.
fn find_block_brace_at_depth0(lex: &LexResult<'_>, start: usize, end: usize) -> Option<usize> {
    let mut cur = start;
    while cur < end {
        match lex.tokens[cur] {
            TokenKind::Punct('{') => return Some(cur),
            TokenKind::Punct('(' | '[') => cur = lex.matching_close[cur] + 1,
            _ => cur += 1,
        }
    }
    None
}
