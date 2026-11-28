//! Zero-dependency proc-macro wrapper around `dioxus_core_macro::rsx!` that stamps debug-only
//! template anchors (`"__nve_tpl": "<rel_file>#<ordinal>@<path>"`) onto native RSX elements for
//! collision-free Tier-1 binary `RowPatch` hot reload without requiring `test_id`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

extern crate proc_macro;

mod lexer;
mod scanner;

use proc_macro::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

use scanner::{is_native_tag, is_rust_keyword, templates_for_file, FileTemplates, LineCol};

/// Attribute name injected in debug builds and stripped by `NveMutationWriter::instantiate_template_node`.
const NVE_TEMPLATE_ATTR: &str = "__nve_tpl";

/// Maximum prefix path depth stored inline in `TemplateRootEntry`.
const MAX_ANCHOR_PATH_LEN: usize = 8;

/// NVE's `rsx!` proc macro.
///
/// In debug builds (`#[cfg(debug_assertions)]` at the call site), injects static `"__nve_tpl"`
/// coordinates on native element roots, component-wrapped native elements, and native siblings
/// following dynamic children so Tier-1 hot reload patches any element (including `for`-loop
/// instances and un-tagged nodes) directly via binary `RowPatch`. In release builds
/// (`#[cfg(not(debug_assertions))]`), forwards the untouched token stream directly to
/// `dioxus_core_macro::rsx!`.
#[proc_macro]
pub fn rsx(input: TokenStream) -> TokenStream {
    let call_span = Span::call_site();
    let tagged = tag_if_resolvable(input.clone(), call_span);
    match tagged {
        Some(debug_tokens) => emit_cfg_gated(debug_tokens, input, call_span),
        None => emit_forward(input, call_span),
    }
}

fn tag_if_resolvable(input: TokenStream, call_span: Span) -> Option<TokenStream> {
    if input.is_empty() {
        return None;
    }
    let local_path = call_span.local_file()?;
    let raw_file = call_span.file();
    let file = templates_for_file(&local_path, &raw_file)?;
    if file.rel_file.contains(['{', '}']) {
        return None;
    }
    let ordinal = file.rsx_ordinal(span_start(call_span), span_end(call_span))?;
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let mut path = Vec::with_capacity(8);
    Some(rewrite_rsx_children(
        &tokens,
        file,
        ordinal,
        &mut path,
        false,
        call_span,
    ))
}

fn span_start(span: Span) -> LineCol {
    let s = span.start();
    (s.line() as u32, s.column() as u32)
}

fn span_end(span: Span) -> LineCol {
    let e = span.end();
    (e.line() as u32, e.column() as u32)
}

fn emit_forward(body: TokenStream, span: Span) -> TokenStream {
    let mut out = Vec::with_capacity(6);
    push_macro_call(&mut out, body, span);
    out.into_iter().collect()
}

fn emit_cfg_gated(debug_body: TokenStream, release_body: TokenStream, span: Span) -> TokenStream {
    let mut inner = Vec::with_capacity(16);
    push_cfg_attr(&mut inner, false, span);
    push_macro_call(&mut inner, debug_body, span);
    push_cfg_attr(&mut inner, true, span);
    push_macro_call(&mut inner, release_body, span);
    let mut block = Group::new(Delimiter::Brace, inner.into_iter().collect());
    block.set_span(span);
    TokenStream::from(TokenTree::Group(block))
}

fn push_cfg_attr(out: &mut Vec<TokenTree>, negated: bool, span: Span) {
    out.push(TokenTree::Punct(spanned_punct('#', Spacing::Alone, span)));
    let cond = if negated {
        let mut inner = Group::new(
            Delimiter::Parenthesis,
            TokenStream::from(TokenTree::Ident(Ident::new("debug_assertions", span))),
        );
        inner.set_span(span);
        let mut v = Vec::with_capacity(2);
        v.push(TokenTree::Ident(Ident::new("not", span)));
        v.push(TokenTree::Group(inner));
        v.into_iter().collect()
    } else {
        TokenStream::from(TokenTree::Ident(Ident::new("debug_assertions", span)))
    };
    let mut cfg_args = Group::new(Delimiter::Parenthesis, cond);
    cfg_args.set_span(span);
    let mut attr_inner = Vec::with_capacity(2);
    attr_inner.push(TokenTree::Ident(Ident::new("cfg", span)));
    attr_inner.push(TokenTree::Group(cfg_args));
    let mut bracket = Group::new(Delimiter::Bracket, attr_inner.into_iter().collect());
    bracket.set_span(span);
    out.push(TokenTree::Group(bracket));
}

fn push_macro_call(out: &mut Vec<TokenTree>, body: TokenStream, span: Span) {
    out.push(TokenTree::Punct(spanned_punct(':', Spacing::Joint, span)));
    out.push(TokenTree::Punct(spanned_punct(':', Spacing::Alone, span)));
    out.push(TokenTree::Ident(Ident::new("nve_dioxus", span)));
    out.push(TokenTree::Punct(spanned_punct(':', Spacing::Joint, span)));
    out.push(TokenTree::Punct(spanned_punct(':', Spacing::Alone, span)));
    out.push(TokenTree::Ident(Ident::new("__dioxus_rsx", span)));
    out.push(TokenTree::Punct(spanned_punct('!', Spacing::Alone, span)));
    let mut grp = Group::new(Delimiter::Brace, body);
    grp.set_span(span);
    out.push(TokenTree::Group(grp));
}

fn spanned_punct(ch: char, spacing: Spacing, span: Span) -> Punct {
    let mut p = Punct::new(ch, spacing);
    p.set_span(span);
    p
}

// ============================================================================
// TokenTree RSX rewriter
// ============================================================================

fn rewrite_rsx_children(
    tokens: &[TokenTree],
    file: &FileTemplates,
    ordinal: u32,
    path: &mut Vec<u16>,
    parent_is_native: bool,
    call_span: Span,
) -> TokenStream {
    let mut out = Vec::with_capacity(tokens.len());
    let mut pos = 0usize;
    let mut child_idx = 0u16;
    let mut seen_dynamic = false;
    while pos < tokens.len() {
        pos = rewrite_one_child(
            tokens,
            pos,
            file,
            ordinal,
            path,
            parent_is_native,
            &mut child_idx,
            &mut seen_dynamic,
            call_span,
            &mut out,
        );
    }
    out.into_iter().collect()
}

#[allow(clippy::too_many_arguments)]
fn rewrite_one_child(
    tokens: &[TokenTree],
    pos: usize,
    file: &FileTemplates,
    ordinal: u32,
    path: &mut Vec<u16>,
    parent_is_native: bool,
    child_idx: &mut u16,
    seen_dynamic: &mut bool,
    call_span: Span,
    out: &mut Vec<TokenTree>,
) -> usize {
    let len = tokens.len();
    if is_punct(&tokens[pos], ',') || is_punct(&tokens[pos], ';') {
        out.push(tokens[pos].clone());
        return pos + 1;
    }
    // Nested `rsx! { ... }` — emit untouched; rustc expands it separately.
    if pos + 2 < len
        && is_ident(&tokens[pos], "rsx")
        && is_punct(&tokens[pos + 1], '!')
        && is_brace_group(&tokens[pos + 2])
    {
        *child_idx = child_idx.saturating_add(1);
        *seen_dynamic = true;
        out.extend_from_slice(&tokens[pos..pos + 3]);
        return pos + 3;
    }
    if is_ident(&tokens[pos], "if") {
        *child_idx = child_idx.saturating_add(1);
        *seen_dynamic = true;
        return rewrite_if_chain(tokens, pos, file, call_span, out);
    }
    if is_ident(&tokens[pos], "for") {
        *child_idx = child_idx.saturating_add(1);
        *seen_dynamic = true;
        return rewrite_for_loop(tokens, pos, file, call_span, out);
    }
    if is_ident(&tokens[pos], "match") {
        *child_idx = child_idx.saturating_add(1);
        *seen_dynamic = true;
        if let Some(arms) = find_body_brace_tt(tokens, pos + 1, false) {
            out.extend_from_slice(&tokens[pos..=arms]);
            return arms + 1;
        }
        out.extend_from_slice(&tokens[pos..]);
        return len;
    }
    if is_brace_group(&tokens[pos]) {
        *child_idx = child_idx.saturating_add(1);
        *seen_dynamic = true;
        out.push(tokens[pos].clone());
        return pos + 1;
    }
    if let TokenTree::Ident(ref tag_ident) = tokens[pos]
        && pos + 1 < len
        && let TokenTree::Group(ref body_group) = tokens[pos + 1]
        && body_group.delimiter() == Delimiter::Brace
    {
        let tag = tag_ident.to_string();
        if !is_rust_keyword(&tag) {
            let path_qualified = pos >= 1 && is_punct(&tokens[pos - 1], ':');
            let is_native = !path_qualified && is_native_tag(&tag);
            let my_idx = *child_idx;
            *child_idx = child_idx.saturating_add(1);
            if !is_native {
                *seen_dynamic = true;
            }
            path.push(my_idx);
            let tag_value = (is_native
                && (!parent_is_native || *seen_dynamic)
                && path.len() <= MAX_ANCHOR_PATH_LEN)
                .then(|| format_anchor_tag(&file.rel_file, ordinal, path));
            let body_tokens: Vec<TokenTree> = body_group.stream().into_iter().collect();
            let rewritten_body = rewrite_element_body(
                &body_tokens,
                file,
                ordinal,
                path,
                is_native,
                tag_value.as_deref(),
                call_span,
            );
            path.pop();
            let mut new_group = Group::new(Delimiter::Brace, rewritten_body);
            new_group.set_span(body_group.span());
            out.push(tokens[pos].clone());
            out.push(TokenTree::Group(new_group));
            return pos + 2;
        }
    }
    if is_string_lit(&tokens[pos]) && !tokens.get(pos + 1).is_some_and(|t| is_punct(t, ':')) {
        *child_idx = child_idx.saturating_add(1);
        out.push(tokens[pos].clone());
        return pos + 1;
    }
    out.push(tokens[pos].clone());
    pos + 1
}

fn rewrite_element_body(
    tokens: &[TokenTree],
    file: &FileTemplates,
    ordinal: u32,
    path: &mut Vec<u16>,
    is_native: bool,
    tag_value: Option<&str>,
    call_span: Span,
) -> TokenStream {
    let mut out = Vec::with_capacity(tokens.len() + 4);
    let mut pos = 0usize;
    let mut child_idx = 0u16;
    let mut seen_dynamic = false;
    let mut inserted_tag = tag_value.is_none();

    while pos < tokens.len() {
        // Attribute spread: `..expr,` — keep untouched before children.
        if pos + 1 < tokens.len() && is_punct(&tokens[pos], '.') && is_punct(&tokens[pos + 1], '.')
        {
            if let Some(tag) = tag_value
                && !inserted_tag
            {
                push_template_attr(&mut out, tag, call_span);
                inserted_tag = true;
            }
            let end = attr_value_end_tt(tokens, pos + 2);
            let next = end + usize::from(end < tokens.len() && is_punct(&tokens[end], ','));
            out.extend_from_slice(&tokens[pos..next]);
            pos = next;
            continue;
        }
        if let Some((attr_is_key, next)) = parse_attr_tt(tokens, pos) {
            if attr_is_key {
                // Keep `key:` first when written first, then insert `"__nve_tpl"` immediately after.
                out.extend_from_slice(&tokens[pos..next]);
                if let Some(tag) = tag_value
                    && !inserted_tag
                {
                    if next > pos && !is_punct(&tokens[next - 1], ',') {
                        out.push(TokenTree::Punct(spanned_punct(
                            ',',
                            Spacing::Alone,
                            call_span,
                        )));
                    }
                    push_template_attr(&mut out, tag, call_span);
                    inserted_tag = true;
                }
            } else {
                if let Some(tag) = tag_value
                    && !inserted_tag
                {
                    push_template_attr(&mut out, tag, call_span);
                    inserted_tag = true;
                }
                out.extend_from_slice(&tokens[pos..next]);
            }
            pos = next;
            continue;
        }
        // Shorthand prop (`accent,`).
        if let TokenTree::Ident(ref id) = tokens[pos]
            && !is_rust_keyword(&id.to_string())
            && tokens.get(pos + 1).is_some_and(|t| is_punct(t, ','))
        {
            if let Some(tag) = tag_value
                && !inserted_tag
            {
                push_template_attr(&mut out, tag, call_span);
                inserted_tag = true;
            }
            out.extend_from_slice(&tokens[pos..pos + 2]);
            pos += 2;
            continue;
        }
        if let Some(tag) = tag_value
            && !inserted_tag
        {
            push_template_attr(&mut out, tag, call_span);
            inserted_tag = true;
        }
        pos = rewrite_one_child(
            tokens,
            pos,
            file,
            ordinal,
            path,
            is_native,
            &mut child_idx,
            &mut seen_dynamic,
            call_span,
            &mut out,
        );
    }

    if let Some(tag) = tag_value
        && !inserted_tag
    {
        push_template_attr(&mut out, tag, call_span);
    }
    out.into_iter().collect()
}

fn rewrite_if_chain(
    tokens: &[TokenTree],
    start: usize,
    file: &FileTemplates,
    call_span: Span,
    out: &mut Vec<TokenTree>,
) -> usize {
    out.push(tokens[start].clone()); // `if`
    let mut pos = start + 1;
    loop {
        let Some(body_idx) = find_body_brace_tt(tokens, pos, false) else {
            out.extend_from_slice(&tokens[pos..]);
            return tokens.len();
        };
        out.extend_from_slice(&tokens[pos..body_idx]);
        push_rewritten_body_group(&tokens[body_idx], file, call_span, out);
        pos = body_idx + 1;
        if pos >= tokens.len() || !is_ident(&tokens[pos], "else") {
            return pos;
        }
        out.push(tokens[pos].clone()); // `else`
        pos += 1;
        if pos < tokens.len() && is_ident(&tokens[pos], "if") {
            out.push(tokens[pos].clone()); // `if`
            pos += 1;
        } else if pos < tokens.len() && is_brace_group(&tokens[pos]) {
            push_rewritten_body_group(&tokens[pos], file, call_span, out);
            return pos + 1;
        } else {
            return pos;
        }
    }
}

fn rewrite_for_loop(
    tokens: &[TokenTree],
    start: usize,
    file: &FileTemplates,
    call_span: Span,
    out: &mut Vec<TokenTree>,
) -> usize {
    out.push(tokens[start].clone()); // `for`
    let Some(body_idx) = find_body_brace_tt(tokens, start + 1, true) else {
        out.extend_from_slice(&tokens[start + 1..]);
        return tokens.len();
    };
    out.extend_from_slice(&tokens[start + 1..body_idx]);
    push_rewritten_body_group(&tokens[body_idx], file, call_span, out);
    body_idx + 1
}

fn push_rewritten_body_group(
    tok: &TokenTree,
    file: &FileTemplates,
    call_span: Span,
    out: &mut Vec<TokenTree>,
) {
    let TokenTree::Group(group) = tok else {
        out.push(tok.clone());
        return;
    };
    let open = span_start(group.span_open());
    let end = span_end(group.span_close());
    let Some(body_ord) = file.body_ordinal(open, end) else {
        out.push(tok.clone());
        return;
    };
    let inner: Vec<TokenTree> = group.stream().into_iter().collect();
    let mut sub_path = Vec::with_capacity(8);
    let rewritten = rewrite_rsx_children(&inner, file, body_ord, &mut sub_path, false, call_span);
    let mut new_group = Group::new(Delimiter::Brace, rewritten);
    new_group.set_span(group.span());
    out.push(TokenTree::Group(new_group));
}

fn format_anchor_tag(rel_file: &str, ordinal: u32, path: &[u16]) -> String {
    use core::fmt::Write as _;
    let mut s = String::with_capacity(rel_file.len() + 16);
    s.push_str(rel_file);
    let _ = write!(s, "#{ordinal}@");
    for (i, &step) in path.iter().enumerate() {
        if i > 0 {
            s.push('/');
        }
        let _ = write!(s, "{step}");
    }
    s
}

fn push_template_attr(out: &mut Vec<TokenTree>, tag_value: &str, span: Span) {
    let mut key = Literal::string(NVE_TEMPLATE_ATTR);
    key.set_span(span);
    let mut val = Literal::string(tag_value);
    val.set_span(span);
    out.push(TokenTree::Literal(key));
    out.push(TokenTree::Punct(spanned_punct(':', Spacing::Alone, span)));
    out.push(TokenTree::Literal(val));
    out.push(TokenTree::Punct(spanned_punct(',', Spacing::Alone, span)));
}

fn parse_attr_tt(tokens: &[TokenTree], pos: usize) -> Option<(bool, usize)> {
    if pos + 2 >= tokens.len()
        || !is_punct(&tokens[pos + 1], ':')
        || is_punct(&tokens[pos + 2], ':')
    {
        return None;
    }
    let is_key = match &tokens[pos] {
        TokenTree::Ident(id) => {
            let s = id.to_string();
            if is_rust_keyword(&s) {
                return None;
            }
            s == "key"
        }
        tok if is_string_lit(tok) => false,
        _ => return None,
    };
    let val_end = attr_value_end_tt(tokens, pos + 2);
    let next = val_end + usize::from(val_end < tokens.len() && is_punct(&tokens[val_end], ','));
    Some((is_key, next))
}

fn attr_value_end_tt(tokens: &[TokenTree], start: usize) -> usize {
    let mut cur = start;
    while cur < tokens.len() {
        if is_punct(&tokens[cur], ',') {
            return cur;
        }
        cur += 1;
    }
    tokens.len()
}

fn is_ident(tok: &TokenTree, name: &str) -> bool {
    matches!(tok, TokenTree::Ident(id) if id.to_string() == name)
}

fn is_punct(tok: &TokenTree, ch: char) -> bool {
    matches!(tok, TokenTree::Punct(p) if p.as_char() == ch)
}

fn is_brace_group(tok: &TokenTree) -> bool {
    matches!(tok, TokenTree::Group(g) if g.delimiter() == Delimiter::Brace)
}

fn is_string_lit(tok: &TokenTree) -> bool {
    let TokenTree::Literal(lit) = tok else {
        return false;
    };
    let s = lit.to_string();
    s.starts_with('"') || s.starts_with("r\"") || s.starts_with("r#")
}

fn expects_operand_tt(tok: &TokenTree) -> bool {
    match tok {
        TokenTree::Punct(p) => p.as_char() != '?',
        TokenTree::Ident(id) => matches!(
            id.to_string().as_str(),
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

fn continues_expr_tt(tokens: &[TokenTree], k: usize) -> bool {
    let Some(tok) = tokens.get(k) else {
        return false;
    };
    match tok {
        TokenTree::Ident(id) => id.to_string() == "as",
        TokenTree::Punct(p) => matches!(
            p.as_char(),
            '.' | '?' | '=' | '!' | '<' | '>' | '+' | '-' | '*' | '/' | '%' | '^' | '&' | '|'
        ),
        _ => false,
    }
}

fn find_let_eq_tt(tokens: &[TokenTree], mut cur: usize) -> Option<usize> {
    while cur < tokens.len() {
        if is_punct(&tokens[cur], '=')
            && !tokens.get(cur + 1).is_some_and(|t| is_punct(t, '=') || is_punct(t, '>'))
            && !(cur > 0
                && matches!(
                    &tokens[cur - 1],
                    TokenTree::Punct(p)
                        if matches!(p.as_char(), '=' | '!' | '<' | '>' | '+' | '-' | '*' | '/' | '%' | '^' | '&' | '|' | '.')
                ))
        {
            return Some(cur);
        }
        cur += 1;
    }
    None
}

fn find_body_brace_tt(tokens: &[TokenTree], start: usize, is_for: bool) -> Option<usize> {
    let mut cur = start;
    if is_for {
        while !tokens.get(cur).is_some_and(|t| is_ident(t, "in")) {
            if cur >= tokens.len() {
                return None;
            }
            cur += 1;
        }
        cur += 1;
    }
    let mut operand_expected = true;
    let mut pending_match_arms = 0u32;
    while cur < tokens.len() {
        if is_ident(&tokens[cur], "let") {
            cur = find_let_eq_tt(tokens, cur + 1)? + 1;
            operand_expected = true;
            continue;
        }
        if is_brace_group(&tokens[cur]) {
            let is_head = if operand_expected {
                true
            } else if pending_match_arms > 0 {
                pending_match_arms -= 1;
                true
            } else {
                continues_expr_tt(tokens, cur + 1)
            };
            if !is_head {
                return Some(cur);
            }
            cur += 1;
            operand_expected = false;
            continue;
        }
        if is_ident(&tokens[cur], "match") {
            pending_match_arms += 1;
        }
        let is_group = matches!(
            &tokens[cur],
            TokenTree::Group(g) if matches!(g.delimiter(), Delimiter::Parenthesis | Delimiter::Bracket)
        );
        operand_expected = !is_group && expects_operand_tt(&tokens[cur]);
        cur += 1;
    }
    None
}
