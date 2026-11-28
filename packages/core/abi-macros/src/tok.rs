//! Lightweight owned token AST mirroring `proc_macro::TokenTree` so derive parsers/emitters can
//! be unit-tested directly outside a proc-macro runtime.

use proc_macro::{Delimiter, Spacing, TokenStream, TokenTree};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delim {
    Paren,
    Bracket,
    Brace,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok {
    Ident(String),
    Punct(char, bool),
    Literal(String),
    Group(Delim, Vec<Tok>),
}

pub(crate) fn from_stream(stream: TokenStream) -> Vec<Tok> {
    stream
        .into_iter()
        .map(|tt| match tt {
            TokenTree::Ident(id) => Tok::Ident(id.to_string()),
            TokenTree::Punct(p) => Tok::Punct(p.as_char(), p.spacing() == Spacing::Joint),
            TokenTree::Literal(lit) => Tok::Literal(lit.to_string()),
            TokenTree::Group(g) => {
                let d = match g.delimiter() {
                    Delimiter::Parenthesis => Delim::Paren,
                    Delimiter::Bracket => Delim::Bracket,
                    Delimiter::Brace => Delim::Brace,
                    Delimiter::None => Delim::None,
                };
                Tok::Group(d, from_stream(g.stream()))
            }
        })
        .collect()
}

pub(crate) fn is_punct(tokens: &[Tok], pos: usize, ch: char) -> bool {
    matches!(tokens.get(pos), Some(Tok::Punct(c, _)) if *c == ch)
}

pub(crate) fn is_ident(tokens: &[Tok], pos: usize, name: &str) -> bool {
    matches!(tokens.get(pos), Some(Tok::Ident(s)) if s == name)
}

pub(crate) fn expect_ident(tokens: &[Tok], pos: &mut usize, desc: &str) -> Result<String, String> {
    match tokens.get(*pos) {
        Some(Tok::Ident(id)) => {
            *pos += 1;
            Ok(id.clone())
        }
        other => Err(format!("expected {desc}, found {other:?}")),
    }
}

pub(crate) fn skip_vis(tokens: &[Tok], pos: &mut usize) {
    if is_ident(tokens, *pos, "pub") {
        *pos += 1;
        if matches!(tokens.get(*pos), Some(Tok::Group(Delim::Paren, _))) {
            *pos += 1;
        }
    }
}

pub(crate) fn is_colon2(tokens: &[Tok], i: usize) -> bool {
    matches!(tokens.get(i), Some(Tok::Punct(':', true))) && is_punct(tokens, i + 1, ':')
}

pub(crate) fn paren_inner(rest: &[Tok]) -> Option<&[Tok]> {
    match rest.first() {
        Some(Tok::Group(Delim::Paren, inner)) => Some(inner.as_slice()),
        _ => None,
    }
}

fn skip_generic_args(tokens: &[Tok], pos: &mut usize) -> bool {
    if !is_punct(tokens, *pos, '<') {
        return false;
    }
    let mut i = *pos + 1;
    let mut depth = 1usize;
    while i < tokens.len() && depth > 0 {
        match &tokens[i] {
            Tok::Punct('<', _) => depth += 1,
            Tok::Punct('>', _) => depth -= 1,
            Tok::Ident(_) | Tok::Literal(_) | Tok::Group(_, _) => {}
            Tok::Punct(':' | ',' | '\'' | '_' | '&', _) => {}
            Tok::Punct(_, _) => return false,
        }
        i += 1;
    }
    if depth != 0 {
        return false;
    }
    *pos = i;
    true
}

fn is_pure_path(tokens: &[Tok]) -> bool {
    if tokens.is_empty() {
        return false;
    }
    let mut i = 0usize;
    if is_colon2(tokens, 0) {
        i = 2;
    }
    if i >= tokens.len() {
        return false;
    }
    loop {
        if !matches!(tokens.get(i), Some(Tok::Ident(_))) {
            return false;
        }
        i += 1;
        if is_punct(tokens, i, '<') && !skip_generic_args(tokens, &mut i) {
            return false;
        }
        if i == tokens.len() {
            return true;
        }
        if is_colon2(tokens, i) {
            i += 2;
            if is_punct(tokens, i, '<') {
                if !skip_generic_args(tokens, &mut i) {
                    return false;
                }
                if i == tokens.len() {
                    return true;
                }
                if is_colon2(tokens, i) {
                    i += 2;
                } else {
                    return false;
                }
            }
        } else {
            return false;
        }
    }
}

fn render_tokens_impl(tokens: &[Tok], paren_invisible: bool) -> String {
    let mut out = String::new();
    for (idx, t) in tokens.iter().enumerate() {
        if idx > 0 {
            let prev_joint = matches!(tokens.get(idx - 1), Some(Tok::Punct(_, true)));
            if !prev_joint {
                out.push(' ');
            }
        }
        match t {
            Tok::Ident(s) | Tok::Literal(s) => out.push_str(s),
            Tok::Punct(c, _) => out.push(*c),
            Tok::Group(d, inner) => {
                let (l, r) = match d {
                    Delim::Paren => ("(", ")"),
                    Delim::Bracket => ("[", "]"),
                    Delim::Brace => ("{", "}"),
                    Delim::None => {
                        let skip_paren = inner.is_empty()
                            || matches!(inner.as_slice(), [Tok::Ident(_) | Tok::Literal(_)])
                            || (idx >= 2 && is_colon2(tokens, idx - 2))
                            || is_colon2(tokens, idx + 1)
                            || is_pure_path(inner);
                        if paren_invisible && !skip_paren {
                            ("(", ")")
                        } else {
                            ("", "")
                        }
                    }
                };
                out.push_str(l);
                out.push_str(&render_tokens_impl(inner, paren_invisible));
                out.push_str(r);
            }
        }
    }
    out
}

pub(crate) fn render_tokens(tokens: &[Tok]) -> String {
    render_tokens_impl(tokens, false)
}

/// Renders expression tokens while parenthesizing invisible `Delim::None` macro-metavar groups so
/// `macro_rules!` expansions such as `#[default($a * 2)]` preserve operator precedence.
pub(crate) fn render_expr_tokens(tokens: &[Tok]) -> String {
    render_tokens_impl(tokens, true)
}

#[cfg(test)]
pub(crate) fn lex(src: &str) -> Vec<Tok> {
    let bytes = src.as_bytes();
    let mut pos = 0;
    lex_until(bytes, &mut pos, None)
}

#[cfg(test)]
fn lex_until(bytes: &[u8], pos: &mut usize, end: Option<u8>) -> Vec<Tok> {
    let mut out = Vec::new();
    while *pos < bytes.len() {
        let b = bytes[*pos];
        if b.is_ascii_whitespace() {
            *pos += 1;
            continue;
        }
        if end == Some(b) {
            *pos += 1;
            break;
        }
        match b {
            b'(' => {
                *pos += 1;
                out.push(Tok::Group(Delim::Paren, lex_until(bytes, pos, Some(b')'))));
            }
            b'[' => {
                *pos += 1;
                out.push(Tok::Group(Delim::Bracket, lex_until(bytes, pos, Some(b']'))));
            }
            b'{' => {
                *pos += 1;
                out.push(Tok::Group(Delim::Brace, lex_until(bytes, pos, Some(b'}'))));
            }
            b'"' => {
                let start = *pos;
                *pos += 1;
                while *pos < bytes.len() {
                    if bytes[*pos] == b'\\' {
                        *pos += 2;
                    } else if bytes[*pos] == b'"' {
                        *pos += 1;
                        break;
                    } else {
                        *pos += 1;
                    }
                }
                out.push(Tok::Literal(
                    std::str::from_utf8(&bytes[start..*pos]).unwrap().to_owned(),
                ));
            }
            b'0'..=b'9' => {
                let start = *pos;
                while *pos < bytes.len()
                    && (bytes[*pos].is_ascii_alphanumeric()
                        || bytes[*pos] == b'_'
                        || (bytes[*pos] == b'.'
                            && bytes.get(*pos + 1).is_some_and(u8::is_ascii_digit)))
                {
                    *pos += 1;
                }
                out.push(Tok::Literal(
                    std::str::from_utf8(&bytes[start..*pos]).unwrap().to_owned(),
                ));
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = *pos;
                if b == b'b' && bytes.get(*pos + 1) == Some(&b'"') {
                    *pos += 2;
                    while *pos < bytes.len() {
                        if bytes[*pos] == b'\\' {
                            *pos += 2;
                        } else if bytes[*pos] == b'"' {
                            *pos += 1;
                            break;
                        } else {
                            *pos += 1;
                        }
                    }
                    out.push(Tok::Literal(
                        std::str::from_utf8(&bytes[start..*pos]).unwrap().to_owned(),
                    ));
                    continue;
                }
                if b == b'r' && bytes.get(*pos + 1) == Some(&b'#') {
                    *pos += 2;
                }
                while *pos < bytes.len()
                    && (bytes[*pos].is_ascii_alphanumeric() || bytes[*pos] == b'_')
                {
                    *pos += 1;
                }
                out.push(Tok::Ident(
                    std::str::from_utf8(&bytes[start..*pos]).unwrap().to_owned(),
                ));
            }
            _ => {
                let ch = b as char;
                let next_is_punct = bytes.get(*pos + 1).is_some_and(|nb| {
                    nb.is_ascii_punctuation() && !matches!(nb, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'"')
                });
                *pos += 1;
                out.push(Tok::Punct(ch, next_is_punct));
            }
        }
    }
    out
}
