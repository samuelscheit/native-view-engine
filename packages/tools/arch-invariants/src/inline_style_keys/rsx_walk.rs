//! Token-aware RSX walker scanning native element attribute position for banned `style:` /
//! `"style":` / `r#style:` attributes, recursing through outer macros (`vec!`, `macro_rules!`)
//! without false-positiving on PascalCase component props or Rust struct literals inside closures.

use proc_macro2::{Delimiter, Spacing, TokenTree};
use syn::LitStr;

use crate::{line_at, Violation};

pub(super) fn scan_macro_for_nested_rsx(
    rel: &str,
    raw: &str,
    tokens: &[TokenTree],
    out: &mut Vec<Violation>,
) {
    let mut i = 0;
    while i < tokens.len() {
        if i + 2 < tokens.len()
            && is_ident(&tokens[i], "rsx")
            && is_punct(&tokens[i + 1], '!')
            && let TokenTree::Group(ref g) = tokens[i + 2]
            && g.delimiter() == Delimiter::Brace
        {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            scan_rsx_children(rel, raw, &inner, out);
            i += 3;
            continue;
        }
        if let TokenTree::Group(ref g) = tokens[i] {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            scan_macro_for_nested_rsx(rel, raw, &inner, out);
        }
        i += 1;
    }
}

pub(super) fn scan_rsx_children(
    rel: &str,
    raw: &str,
    tokens: &[TokenTree],
    out: &mut Vec<Violation>,
) {
    let mut pos = 0;
    while pos < tokens.len() {
        pos = scan_one_rsx_child(rel, raw, tokens, pos, out);
    }
}

fn scan_one_rsx_child(
    rel: &str,
    raw: &str,
    tokens: &[TokenTree],
    pos: usize,
    out: &mut Vec<Violation>,
) -> usize {
    let len = tokens.len();
    if is_punct(&tokens[pos], ',') || is_punct(&tokens[pos], ';') {
        return pos + 1;
    }
    if pos + 2 < len
        && is_ident(&tokens[pos], "rsx")
        && is_punct(&tokens[pos + 1], '!')
        && let TokenTree::Group(ref g) = tokens[pos + 2]
        && g.delimiter() == Delimiter::Brace
    {
        let inner: Vec<TokenTree> = g.stream().into_iter().collect();
        scan_rsx_children(rel, raw, &inner, out);
        return pos + 3;
    }
    if is_ident(&tokens[pos], "if") {
        return scan_if_chain(rel, raw, tokens, pos + 1, out);
    }
    if is_ident(&tokens[pos], "for") {
        let Some(body_idx) = find_body_brace(tokens, pos + 1, true) else {
            scan_macro_for_nested_rsx(rel, raw, &tokens[pos + 1..], out);
            return len;
        };
        scan_macro_for_nested_rsx(rel, raw, &tokens[pos + 1..body_idx], out);
        if let TokenTree::Group(ref g) = tokens[body_idx] {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            scan_rsx_children(rel, raw, &inner, out);
        }
        return body_idx + 1;
    }
    if is_ident(&tokens[pos], "match") {
        let Some(arms_idx) = find_body_brace(tokens, pos + 1, false) else {
            scan_macro_for_nested_rsx(rel, raw, &tokens[pos + 1..], out);
            return len;
        };
        scan_macro_for_nested_rsx(rel, raw, &tokens[pos + 1..=arms_idx], out);
        return arms_idx + 1;
    }
    if let TokenTree::Group(ref g) = tokens[pos] {
        let inner: Vec<TokenTree> = g.stream().into_iter().collect();
        scan_macro_for_nested_rsx(rel, raw, &inner, out);
        return pos + 1;
    }
    if let TokenTree::Ident(ref tag_ident) = tokens[pos]
        && pos + 1 < len
        && let TokenTree::Group(ref body) = tokens[pos + 1]
        && body.delimiter() == Delimiter::Brace
    {
        let tag = tag_ident.to_string();
        if !is_rust_keyword(&tag) {
            let path_qualified = pos >= 1 && is_punct(&tokens[pos - 1], ':');
            let is_native = !path_qualified
                && tag.bytes().next().is_some_and(|b| b.is_ascii_lowercase());
            let inner: Vec<TokenTree> = body.stream().into_iter().collect();
            scan_rsx_element_body(rel, raw, &inner, is_native, out);
            return pos + 2;
        }
    }
    pos + 1
}

fn scan_rsx_element_body(
    rel: &str,
    raw: &str,
    tokens: &[TokenTree],
    is_native: bool,
    out: &mut Vec<Violation>,
) {
    let mut pos = 0;
    while pos < tokens.len() {
        if pos + 1 < tokens.len() && is_punct(&tokens[pos], '.') && is_punct(&tokens[pos + 1], '.')
        {
            let val_end = attr_value_end(tokens, pos + 2);
            scan_macro_for_nested_rsx(rel, raw, &tokens[pos + 2..val_end], out);
            pos = val_end + usize::from(val_end < tokens.len() && is_punct(&tokens[val_end], ','));
            continue;
        }
        if let Some((attr_name, line, val_start, val_end, next)) = parse_attr(tokens, pos) {
            if is_native && attr_name == "style" {
                out.push(Violation::new(
                    rel,
                    line,
                    line_at(raw, line),
                    "RSX `style:` attribute is banned (zero runtime CSS parser); use `nve_style: TypedStyle::new()...`",
                ));
            }
            scan_macro_for_nested_rsx(rel, raw, &tokens[val_start..val_end], out);
            pos = next;
            continue;
        }
        if let TokenTree::Ident(ref id) = tokens[pos]
            && !is_rust_keyword(&id.to_string())
            && tokens.get(pos + 1).is_some_and(|t| is_punct(t, ','))
        {
            pos += 2;
            continue;
        }
        pos = scan_one_rsx_child(rel, raw, tokens, pos, out);
    }
}

fn scan_if_chain(
    rel: &str,
    raw: &str,
    tokens: &[TokenTree],
    mut pos: usize,
    out: &mut Vec<Violation>,
) -> usize {
    loop {
        let Some(body_idx) = find_body_brace(tokens, pos, false) else {
            scan_macro_for_nested_rsx(rel, raw, &tokens[pos..], out);
            return tokens.len();
        };
        scan_macro_for_nested_rsx(rel, raw, &tokens[pos..body_idx], out);
        if let TokenTree::Group(ref g) = tokens[body_idx] {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            scan_rsx_children(rel, raw, &inner, out);
        }
        pos = body_idx + 1;
        if pos >= tokens.len() || !is_ident(&tokens[pos], "else") {
            return pos;
        }
        pos += 1;
        if pos < tokens.len() && is_ident(&tokens[pos], "if") {
            pos += 1;
        } else if pos < tokens.len()
            && let TokenTree::Group(ref g) = tokens[pos]
            && g.delimiter() == Delimiter::Brace
        {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            scan_rsx_children(rel, raw, &inner, out);
            return pos + 1;
        } else {
            return pos;
        }
    }
}

fn parse_attr(tokens: &[TokenTree], pos: usize) -> Option<(String, usize, usize, usize, usize)> {
    if pos + 2 >= tokens.len() {
        return None;
    }
    let TokenTree::Punct(ref colon) = tokens[pos + 1] else {
        return None;
    };
    if colon.as_char() != ':' || colon.spacing() != Spacing::Alone || is_punct(&tokens[pos + 2], ':')
    {
        return None;
    }
    let (name, line) = match &tokens[pos] {
        TokenTree::Ident(id) => {
            let raw_name = id.to_string();
            let unraw = raw_name.strip_prefix("r#").unwrap_or(&raw_name);
            if raw_name == unraw && is_rust_keyword(unraw) {
                return None;
            }
            (unraw.to_owned(), id.span().start().line)
        }
        TokenTree::Literal(lit) => {
            let s = lit.to_string();
            let parsed = syn::parse_str::<LitStr>(&s).ok()?;
            (parsed.value(), lit.span().start().line)
        }
        _ => return None,
    };
    let val_end = attr_value_end(tokens, pos + 2);
    let next = val_end + usize::from(val_end < tokens.len() && is_punct(&tokens[val_end], ','));
    Some((name, line, pos + 2, val_end, next))
}

fn attr_value_end(tokens: &[TokenTree], start: usize) -> usize {
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
    matches!(tok, TokenTree::Ident(id) if id == name)
}

fn is_punct(tok: &TokenTree, ch: char) -> bool {
    matches!(tok, TokenTree::Punct(p) if p.as_char() == ch)
}

fn is_brace_group(tok: &TokenTree) -> bool {
    matches!(tok, TokenTree::Group(g) if g.delimiter() == Delimiter::Brace)
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

fn expects_operand(tok: &TokenTree) -> bool {
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

fn continues_expr(tokens: &[TokenTree], k: usize) -> bool {
    let Some(tok) = tokens.get(k) else {
        return false;
    };
    match tok {
        TokenTree::Ident(id) => id == "as",
        TokenTree::Punct(p) => matches!(
            p.as_char(),
            '.' | '?' | '=' | '!' | '<' | '>' | '+' | '-' | '*' | '/' | '%' | '^' | '&' | '|'
        ),
        _ => false,
    }
}

fn find_let_eq(tokens: &[TokenTree], mut cur: usize) -> Option<usize> {
    while cur < tokens.len() {
        if is_punct(&tokens[cur], '=')
            && !tokens
                .get(cur + 1)
                .is_some_and(|t| is_punct(t, '=') || is_punct(t, '>'))
            && !(cur > 0
                && matches!(
                    &tokens[cur - 1],
                    TokenTree::Punct(p) if matches!(
                        p.as_char(),
                        '=' | '!' | '<' | '>' | '+' | '-' | '*' | '/' | '%' | '^' | '&' | '|' | '.'
                    )
                ))
        {
            return Some(cur);
        }
        cur += 1;
    }
    None
}

fn find_body_brace(tokens: &[TokenTree], start: usize, is_for: bool) -> Option<usize> {
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
            cur = find_let_eq(tokens, cur + 1)? + 1;
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
                continues_expr(tokens, cur + 1)
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
        operand_expected = !is_group && expects_operand(&tokens[cur]);
        cur += 1;
    }
    None
}
