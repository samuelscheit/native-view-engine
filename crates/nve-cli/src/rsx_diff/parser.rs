//! Structural `rsx! { ... }` scanner indexing patchable text children and style/layout attributes.

use std::collections::HashMap;

use super::lexer::{has_interpolation_braces, unescape_string_literal, LexResult, TokenKind};

/// Maps a patchable Dioxus RSX style/layout attribute name to its canonical NVE/CSS attribute name.
/// Returns `None` for non-style attributes (`test_id`, `key`, event handlers, unknown props) so
/// edits to identity/logic props fall back safely to Tier-2.
fn normalize_patchable_attr_name(name: &str) -> Option<String> {
    match name {
        "style" => Some("style".into()),
        "width" | "height" | "min_width" | "min-width" | "min_height" | "min-height"
        | "max_width" | "max-width" | "max_height" | "max-height" | "flex_direction"
        | "flex-direction" | "justify_content" | "justify-content" | "align_items"
        | "align-items" | "flex" | "flex_grow" | "flex-grow" | "flex_shrink" | "flex-shrink"
        | "gap" | "padding" | "padding_horizontal" | "padding-horizontal" | "padding_vertical"
        | "padding-vertical" | "margin" | "background" | "background_color"
        | "background-color" | "color" | "border_color" | "border-color" | "border_width"
        | "border-width" | "border_radius" | "border-radius" | "opacity" | "font_size"
        | "font-size" | "z_index" | "z-index" => Some(name.replace('_', "-")),
        _ => None,
    }
}

fn is_test_id_attr(name: &str) -> bool {
    matches!(name, "test_id" | "testID" | "data-testid" | "test-id")
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

#[derive(Debug, Clone)]
pub(super) enum RsxLitContext {
    Attr {
        /// Token index of the owning RSX element's opening `{`, uniquely identifying the element.
        element_open_idx: usize,
        attr_name: String,
        element_test_id: Option<String>,
        node_path: Vec<usize>,
    },
    Text {
        text_index: usize,
        parent_test_id: Option<String>,
        node_path: Vec<usize>,
    },
}

pub(super) struct RsxIndex {
    pub(super) literals: HashMap<usize, RsxLitContext>,
    next_text_index: usize,
}

impl RsxIndex {
    pub(super) fn build(lex: &LexResult<'_>) -> Self {
        let mut idx = Self {
            literals: HashMap::new(),
            next_text_index: 0,
        };
        idx.scan_rust_range(lex, 0, lex.tokens.len());
        idx
    }

    /// Scans normal Rust tokens looking for `rsx! { ... }` macro invocations.
    fn scan_rust_range(&mut self, lex: &LexResult<'_>, start: usize, end: usize) {
        let mut pos = start;
        while pos + 2 < end {
            if matches!(lex.tokens[pos], TokenKind::Ident("rsx"))
                && matches!(lex.tokens[pos + 1], TokenKind::Punct('!'))
                && matches!(lex.tokens[pos + 2], TokenKind::Punct('{'))
            {
                let open = pos + 2;
                let close = lex.matching_close[open];
                if close > open && close < end {
                    let mut path = Vec::new();
                    self.scan_rsx_children(lex, open + 1, close, &mut path, None);
                    pos = close + 1;
                    continue;
                }
            }
            pos += 1;
        }
    }

    fn scan_rsx_children(
        &mut self,
        lex: &LexResult<'_>,
        start: usize,
        end: usize,
        parent_path: &mut Vec<usize>,
        parent_test_id: Option<&str>,
    ) {
        let mut pos = start;
        let mut child_idx = 0usize;
        while pos < end {
            pos = self.scan_one_rsx_child(
                lex,
                pos,
                end,
                parent_path,
                parent_test_id,
                &mut child_idx,
            );
        }
    }

    /// Scans one RSX child item starting at `pos` and returns the next token index.
    fn scan_one_rsx_child(
        &mut self,
        lex: &LexResult<'_>,
        pos: usize,
        end: usize,
        parent_path: &mut Vec<usize>,
        parent_test_id: Option<&str>,
        child_idx: &mut usize,
    ) -> usize {
        match lex.tokens[pos] {
            TokenKind::Punct(',') | TokenKind::Punct(';') => pos + 1,

            // Direct nested `rsx! { ... }` invocation in RSX child position.
            TokenKind::Ident("rsx")
                if pos + 2 < end
                    && matches!(lex.tokens[pos + 1], TokenKind::Punct('!'))
                    && matches!(lex.tokens[pos + 2], TokenKind::Punct('{')) =>
            {
                let open = pos + 2;
                let close = lex.matching_close[open];
                self.scan_rsx_children(lex, open + 1, close, parent_path, parent_test_id);
                close + 1
            }

            // `if <cond> { <rsx> } (else if <cond> { <rsx> })* (else { <rsx> })?`
            TokenKind::Ident("if") => {
                let mut cur = pos + 1;
                loop {
                    let Some(brace_open) = find_block_brace_at_depth0(lex, cur, end) else {
                        self.scan_rust_range(lex, cur, end);
                        return end;
                    };
                    self.scan_rust_range(lex, cur, brace_open);
                    let brace_close = lex.matching_close[brace_open];
                    self.scan_rsx_children(
                        lex,
                        brace_open + 1,
                        brace_close,
                        parent_path,
                        parent_test_id,
                    );
                    cur = brace_close + 1;
                    if cur < end && matches!(lex.tokens[cur], TokenKind::Ident("else")) {
                        cur += 1;
                        if cur < end && matches!(lex.tokens[cur], TokenKind::Ident("if")) {
                            cur += 1;
                            continue;
                        } else if cur < end && matches!(lex.tokens[cur], TokenKind::Punct('{')) {
                            let else_close = lex.matching_close[cur];
                            self.scan_rsx_children(
                                lex,
                                cur + 1,
                                else_close,
                                parent_path,
                                parent_test_id,
                            );
                            return else_close + 1;
                        }
                    }
                    return cur;
                }
            }

            // `for <pat> in <iter> { <rsx> }`
            TokenKind::Ident("for") => {
                let Some(brace_open) = find_block_brace_at_depth0(lex, pos + 1, end) else {
                    self.scan_rust_range(lex, pos + 1, end);
                    return end;
                };
                self.scan_rust_range(lex, pos + 1, brace_open);
                let brace_close = lex.matching_close[brace_open];
                self.scan_rsx_children(
                    lex,
                    brace_open + 1,
                    brace_close,
                    parent_path,
                    parent_test_id,
                );
                brace_close + 1
            }

            // `match <expr> { ... }` inside RSX: treat body as Rust code (still finds nested `rsx!`).
            TokenKind::Ident("match") => {
                if let Some(brace_open) = find_block_brace_at_depth0(lex, pos + 1, end) {
                    let brace_close = lex.matching_close[brace_open];
                    self.scan_rust_range(lex, pos + 1, brace_close);
                    brace_close + 1
                } else {
                    end
                }
            }

            // Embedded Rust block `{ ... }` inside RSX (may contain nested `rsx! { ... }`).
            TokenKind::Punct('{') => {
                let close = lex.matching_close[pos];
                self.scan_rust_range(lex, pos + 1, close);
                close + 1
            }

            // RSX element or component: `tag { ... }`
            TokenKind::Ident(tag)
                if !is_rust_control_keyword(tag)
                    && pos + 1 < end
                    && matches!(lex.tokens[pos + 1], TokenKind::Punct('{')) =>
            {
                let open = pos + 1;
                let close = lex.matching_close[open];
                let idx = *child_idx;
                *child_idx += 1;
                parent_path.push(idx);
                self.scan_rsx_element(lex, open, close, parent_path);
                parent_path.pop();
                close + 1
            }

            // Static RSX text literal child (`"hello"`, `r#"..."#`).
            TokenKind::StringLit { .. }
                if !(pos + 1 < end && matches!(lex.tokens[pos + 1], TokenKind::Punct(':'))) =>
            {
                let idx = *child_idx;
                *child_idx += 1;
                let text_index = self.next_text_index;
                self.next_text_index += 1;
                let mut text_path = parent_path.clone();
                text_path.push(idx);
                self.literals.insert(
                    pos,
                    RsxLitContext::Text {
                        text_index,
                        parent_test_id: parent_test_id.map(ToOwned::to_owned),
                        node_path: text_path,
                    },
                );
                pos + 1
            }

            // Skip any other token (e.g. shorthand prop or path segment) safely.
            TokenKind::Punct('(') | TokenKind::Punct('[') => {
                let close = lex.matching_close[pos];
                self.scan_rust_range(lex, pos + 1, close);
                close + 1
            }
            _ => pos + 1,
        }
    }

    fn scan_rsx_element(
        &mut self,
        lex: &LexResult<'_>,
        open: usize,
        close: usize,
        elem_path: &mut Vec<usize>,
    ) {
        // Pass 1: discover static `test_id` / `"data-testid"` / `"testID"` on this element so
        // attributes declared before `test_id` still carry the element's `target_test_id`.
        let elem_test_id = find_element_static_test_id(lex, open + 1, close);

        // Pass 2: walk attributes and child items inside `{ ... }`.
        let mut pos = open + 1;
        let mut child_idx = 0usize;
        while pos < close {
            if let Some((attr_name, val_start, val_end, next_pos)) =
                parse_rsx_attribute(lex, pos, close)
            {
                if val_end == val_start + 1 {
                    if let TokenKind::StringLit { .. } = lex.tokens[val_start]
                        && let Some(css_attr) = normalize_patchable_attr_name(&attr_name)
                    {
                        self.literals.insert(
                            val_start,
                            RsxLitContext::Attr {
                                element_open_idx: open,
                                attr_name: css_attr,
                                element_test_id: elem_test_id.clone(),
                                node_path: elem_path.clone(),
                            },
                        );
                    }
                } else {
                    self.scan_rust_range(lex, val_start, val_end);
                }
                pos = next_pos;
                continue;
            }

            // Shorthand component prop (`accent,`) — identifier followed by `,` (not an element or text).
            if matches!(lex.tokens[pos], TokenKind::Ident(id) if !is_rust_control_keyword(id))
                && pos + 1 < close
                && matches!(lex.tokens[pos + 1], TokenKind::Punct(','))
            {
                pos += 2;
                continue;
            }

            pos = self.scan_one_rsx_child(
                lex,
                pos,
                close,
                elem_path,
                elem_test_id.as_deref(),
                &mut child_idx,
            );
        }
    }
}

/// Scans an element's depth-0 attributes for a static `test_id` / `testID` / `"data-testid"`.
fn find_element_static_test_id(lex: &LexResult<'_>, start: usize, end: usize) -> Option<String> {
    let mut pos = start;
    while pos < end {
        if let Some((attr_name, val_start, val_end, next_pos)) = parse_rsx_attribute(lex, pos, end)
        {
            if is_test_id_attr(&attr_name)
                && val_end == val_start + 1
                && let TokenKind::StringLit { raw, is_raw } = lex.tokens[val_start]
                && let Ok(val) = unescape_string_literal(raw, is_raw)
                && !val.is_empty()
                && !has_interpolation_braces(&val)
            {
                return Some(val);
            }
            pos = next_pos;
            continue;
        }
        match lex.tokens[pos] {
            TokenKind::Punct('{') | TokenKind::Punct('(') | TokenKind::Punct('[') => {
                pos = lex.matching_close[pos] + 1;
            }
            _ => pos += 1,
        }
    }
    None
}

/// Parses an RSX attribute `name: <expr> [,|}]` or `"quoted-name": <expr> [,|}]` at `pos`.
/// Returns `(attr_name, val_start, val_end, next_pos_after_comma)`.
fn parse_rsx_attribute(
    lex: &LexResult<'_>,
    pos: usize,
    end: usize,
) -> Option<(String, usize, usize, usize)> {
    if pos + 2 >= end {
        return None;
    }
    if !matches!(lex.tokens[pos + 1], TokenKind::Punct(':')) {
        return None;
    }
    // Reject `::` Rust paths (e.g., `std::foo`).
    if matches!(lex.tokens[pos + 2], TokenKind::Punct(':')) {
        return None;
    }
    let attr_name = match lex.tokens[pos] {
        TokenKind::Ident(id) if !is_rust_control_keyword(id) => id.to_owned(),
        TokenKind::StringLit { raw, is_raw } => unescape_string_literal(raw, is_raw).ok()?,
        _ => return None,
    };

    let val_start = pos + 2;
    let mut cur = val_start;
    while cur < end {
        match lex.tokens[cur] {
            TokenKind::Punct(',') => {
                return Some((attr_name, val_start, cur, cur + 1));
            }
            TokenKind::Punct('{') | TokenKind::Punct('(') | TokenKind::Punct('[') => {
                cur = lex.matching_close[cur] + 1;
            }
            _ => cur += 1,
        }
    }
    Some((attr_name, val_start, end, end))
}

/// Finds the opening `{` of an `if` / `for` / `match` block at depth 0, skipping balanced `()`/`[]`.
fn find_block_brace_at_depth0(lex: &LexResult<'_>, start: usize, end: usize) -> Option<usize> {
    let mut cur = start;
    while cur < end {
        match lex.tokens[cur] {
            TokenKind::Punct('{') => return Some(cur),
            TokenKind::Punct('(') | TokenKind::Punct('[') => {
                cur = lex.matching_close[cur] + 1;
            }
            _ => cur += 1,
        }
    }
    None
}
