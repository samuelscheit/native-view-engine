//! Zero-dependency `proc_macro::TokenStream` parser for `#[derive(NveAbi)]`.

use proc_macro::{Delimiter, Spacing, TokenStream, TokenTree};

use crate::{ParsedEnum, ParsedField, ParsedItem, ParsedRecord, ParsedVariant};

const DEFAULT_CRATE_PATH: &str = "::nve_abi::schema";

#[derive(Default)]
struct Attrs {
    doc_override: Option<String>,
    doc_lines: Vec<String>,
    repr_c: bool,
    repr_align: Option<usize>,
    repr_int: Option<String>,
    kvs: Vec<(String, Kv)>,
}

impl Attrs {
    fn doc(self) -> Option<String> {
        self.doc_override.or_else(|| (!self.doc_lines.is_empty()).then(|| self.doc_lines.join("\n")))
    }
}

enum Kv { Str(String), Usize(usize) }
impl Kv {
    fn str(self, k: &str) -> Result<String, String> { match self { Self::Str(s) => Ok(s), _ => Err(format!("expected string for `{k}`")) } }
    fn usize(self, k: &str) -> Result<usize, String> { match self { Self::Usize(u) => Ok(u), _ => Err(format!("expected integer for `{k}`")) } }
}

pub(crate) fn parse_item(input: TokenStream) -> Result<ParsedItem, String> {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let mut pos = 0;
    let mut a = collect_attrs(&tokens, &mut pos, true)?;
    skip_vis(&tokens, &mut pos);

    let kw = expect_ident(&tokens, &mut pos, "struct, union, or enum")?;
    let rust_ident = expect_ident(&tokens, &mut pos, "type name")?;
    if is_punct(&tokens, pos, '<') { return Err(format!("#[derive(NveAbi)] does not support generics on `{rust_ident}`")); }
    let Some(TokenTree::Group(body)) = tokens.get(pos).filter(|t| is_delim(t, Delimiter::Brace)) else {
        return Err(format!("#[derive(NveAbi)] requires a braced body on `{rust_ident}`"));
    };

    let (mut name, mut cp, mut size, mut size_64, mut align, mut c_prefix, mut dart_fb) = (None, None, None, None, None, None, None);
    for (k, v) in core::mem::take(&mut a.kvs) {
        match k.as_str() {
            "name" => name = Some(v.str(&k)?), "doc" => a.doc_override = Some(v.str(&k)?),
            "crate_path" => cp = Some(v.str(&k)?), "c_prefix" => c_prefix = Some(v.str(&k)?),
            "dart_fallback" => dart_fb = Some(v.str(&k)?), "size" => size = Some(v.usize(&k)?),
            "size_64" => size_64 = Some(v.usize(&k)?), "align" => align = Some(v.usize(&k)?),
            other => return Err(format!("unknown #[nve_abi] item key `{other}`")),
        }
    }
    let abi_name = name.unwrap_or_else(|| strip_raw(&rust_ident));
    let crate_path = cp.unwrap_or_else(|| DEFAULT_CRATE_PATH.to_owned());
    let (repr_c, repr_align, repr_int) = (a.repr_c, a.repr_align, a.repr_int.take());
    let doc = a.doc();

    match kw.as_str() {
        "struct" | "union" => {
            if !repr_c { return Err(format!("#[derive(NveAbi)] on `{rust_ident}` requires #[repr(C)]")); }
            if c_prefix.is_some() || dart_fb.is_some() { return Err(format!("`c_prefix`/`dart_fallback` are enum-only (`{rust_ident}`)")); }
            let rec = ParsedRecord {
                rust_ident, abi_name, doc, crate_path, explicit_align: align.or(repr_align),
                expected_size: size, expected_size_64: size_64, fields: parse_fields(body.stream())?,
            };
            Ok(if kw == "union" { ParsedItem::Union(rec) } else { ParsedItem::Struct(rec) })
        }
        "enum" => {
            let repr = repr_int.ok_or_else(|| format!("#[derive(NveAbi)] on `{rust_ident}` requires #[repr(u8|u16|u32|i32)]"))?;
            if size.is_some() || size_64.is_some() || align.is_some() { return Err(format!("`size`/`size_64`/`align` are struct/union-only (`{rust_ident}`)")); }
            Ok(ParsedItem::Enum(ParsedEnum {
                variants: parse_variants(body.stream(), &rust_ident)?,
                rust_ident, abi_name, repr, doc, crate_path, c_prefix, dart_fallback: dart_fb,
            }))
        }
        other => Err(format!("#[derive(NveAbi)] expects struct/union/enum; found `{other}`")),
    }
}

fn collect_attrs(tokens: &[TokenTree], pos: &mut usize, read_repr: bool) -> Result<Attrs, String> {
    let mut out = Attrs::default();
    while is_punct(tokens, *pos, '#') {
        let Some(TokenTree::Group(g)) = tokens.get(*pos + 1).filter(|t| is_delim(t, Delimiter::Bracket)) else {
            return Err("expected `[...]` after `#`".to_owned());
        };
        *pos += 2;
        let inner: Vec<_> = g.stream().into_iter().collect();
        let (mut i, mut name) = (0, String::new());
        while let Some(TokenTree::Ident(id)) = inner.get(i) {
            name = id.to_string(); i += 1;
            if is_colon2(&inner, i) { i += 2; } else { break; }
        }
        let rest = &inner[i..];
        match name.as_str() {
            "doc" if is_punct(rest, 0, '=') => if let Some(TokenTree::Literal(lit)) = rest.get(1) {
                let s = parse_string_literal(&lit.to_string())?;
                out.doc_lines.push(s.strip_prefix(' ').unwrap_or(&s).to_owned());
            },
            "repr" if read_repr => parse_repr(paren_inner(rest, "repr")?, &mut out)?,
            "nve_abi" => out.kvs.extend(parse_kvs(paren_inner(rest, "nve_abi")?)?),
            _ => {}
        }
    }
    Ok(out)
}

fn parse_repr(inner: Vec<TokenTree>, out: &mut Attrs) -> Result<(), String> {
    let mut i = 0;
    while i < inner.len() {
        if let TokenTree::Ident(id) = &inner[i] {
            let s = id.to_string(); i += 1;
            match s.as_str() {
                "C" => out.repr_c = true,
                "u8" | "u16" | "u32" | "i32" => out.repr_int = Some(s),
                "align" => if let Some(TokenTree::Group(g)) = inner.get(i) {
                    i += 1;
                    let args: Vec<_> = g.stream().into_iter().collect();
                    let [TokenTree::Literal(lit)] = args.as_slice() else { return Err("expected integer in `repr(align(...))`".to_owned()); };
                    out.repr_align = Some(parse_usize_literal(&lit.to_string())?);
                },
                _ => if matches!(inner.get(i), Some(TokenTree::Group(_))) { i += 1; },
            }
        } else { i += 1; }
        if is_punct(&inner, i, ',') { i += 1; }
    }
    Ok(())
}

fn parse_fields(body: TokenStream) -> Result<Vec<ParsedField>, String> {
    let tokens: Vec<TokenTree> = body.into_iter().collect();
    let (mut pos, mut out) = (0, Vec::new());
    while pos < tokens.len() {
        let mut a = collect_attrs(&tokens, &mut pos, false)?;
        if pos >= tokens.len() { break; }
        let (mut off, mut off64, mut c_type) = (None, None, None);
        for (k, v) in core::mem::take(&mut a.kvs) {
            match k.as_str() {
                "offset" => off = Some(v.usize(&k)?), "offset64" => off64 = Some(v.usize(&k)?),
                "c_type" => c_type = Some(v.str(&k)?), "doc" => a.doc_override = Some(v.str(&k)?),
                other => return Err(format!("unknown #[nve_abi] field key `{other}`")),
            }
        }
        skip_vis(&tokens, &mut pos);
        let rust_ident = expect_ident(&tokens, &mut pos, "field name")?;
        if !is_punct(&tokens, pos, ':') { return Err(format!("expected `:` after `{rust_ident}`")); }
        pos += 1;
        let (start, mut depth) = (pos, 0usize);
        while pos < tokens.len() {
            if let TokenTree::Punct(p) = &tokens[pos] {
                match p.as_char() { '<' => depth += 1, '>' => depth = depth.saturating_sub(1), ',' if depth == 0 => break, _ => {} }
            }
            pos += 1;
        }
        if pos == start { return Err(format!("missing type for `{rust_ident}`")); }
        let ty = format_type(&tokens[start..pos]);
        if is_punct(&tokens, pos, ',') { pos += 1; }
        out.push(ParsedField { abi_name: strip_raw(&rust_ident), rust_ident, ty, c_type, doc: a.doc(), expected_offset: off, expected_offset_64: off64 });
    }
    Ok(out)
}

fn parse_variants(body: TokenStream, enum_name: &str) -> Result<Vec<ParsedVariant>, String> {
    let tokens: Vec<TokenTree> = body.into_iter().collect();
    let (mut pos, mut out) = (0, Vec::new());
    while pos < tokens.len() {
        let mut a = collect_attrs(&tokens, &mut pos, false)?;
        if pos >= tokens.len() { break; }
        for (k, v) in core::mem::take(&mut a.kvs) {
            if k == "doc" { a.doc_override = Some(v.str(&k)?); } else { return Err(format!("unknown #[nve_abi] variant key `{k}`")); }
        }
        let rust_ident = expect_ident(&tokens, &mut pos, "enum variant")?;
        if matches!(tokens.get(pos), Some(t) if is_delim(t, Delimiter::Parenthesis) || is_delim(t, Delimiter::Brace)) {
            return Err(format!("#[derive(NveAbi)] enum `{enum_name}::{rust_ident}` must be unit-only"));
        }
        if is_punct(&tokens, pos, '=') { pos += 1; while pos < tokens.len() && !is_punct(&tokens, pos, ',') { pos += 1; } }
        if is_punct(&tokens, pos, ',') { pos += 1; }
        out.push(ParsedVariant { abi_name: strip_raw(&rust_ident), rust_ident, doc: a.doc() });
    }
    Ok(out)
}

fn parse_kvs(tokens: Vec<TokenTree>) -> Result<Vec<(String, Kv)>, String> {
    let (mut pos, mut out) = (0, Vec::new());
    while pos < tokens.len() {
        let k = expect_ident(&tokens, &mut pos, "attribute key")?;
        if !is_punct(&tokens, pos, '=') { return Err(format!("expected `=` after `{k}`")); }
        pos += 1;
        let Some(TokenTree::Literal(lit)) = tokens.get(pos) else { return Err(format!("expected literal for `#[nve_abi({k} = ...)]`")); };
        pos += 1;
        let raw = lit.to_string();
        let v = if raw.starts_with('"') || raw.starts_with('r') { Kv::Str(parse_string_literal(&raw)?) } else { Kv::Usize(parse_usize_literal(&raw)?) };
        out.push((k, v));
        if is_punct(&tokens, pos, ',') { pos += 1; }
    }
    Ok(out)
}

fn paren_inner(rest: &[TokenTree], name: &str) -> Result<Vec<TokenTree>, String> {
    match rest.first() { Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => Ok(g.stream().into_iter().collect()), _ => Err(format!("expected `#[{name}(...)]`")) }
}

fn format_type(tokens: &[TokenTree]) -> String {
    let (mut out, mut i) = (String::new(), 0);
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Punct(p) if p.as_char() == '*' => {
                out.push('*'); i += 1;
                if let Some(TokenTree::Ident(kw)) = tokens.get(i) { out.push_str(&kw.to_string()); out.push(' '); i += 1; }
            }
            TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => {
                let inner: Vec<_> = g.stream().into_iter().collect();
                out.push('[');
                if let Some(s) = inner.iter().position(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ';')) {
                    out.push_str(&format_type(&inner[..s])); out.push_str("; "); out.push_str(&format_type(&inner[s + 1..]));
                } else { out.push_str(&format_type(&inner)); }
                out.push(']'); i += 1;
            }
            TokenTree::Ident(id) => {
                let mut last = id.to_string(); i += 1;
                while is_colon2(tokens, i) { i += 2; if let Some(TokenTree::Ident(seg)) = tokens.get(i) { last = seg.to_string(); i += 1; } }
                out.push_str(&last);
            }
            TokenTree::Punct(p) => { out.push(p.as_char()); if p.as_char() == ',' { out.push(' '); } i += 1; }
            other => { out.push_str(&other.to_string()); i += 1; }
        }
    }
    out.trim().to_owned()
}

fn is_delim(t: &TokenTree, d: Delimiter) -> bool { matches!(t, TokenTree::Group(g) if g.delimiter() == d) }
fn is_punct(tokens: &[TokenTree], i: usize, ch: char) -> bool { matches!(tokens.get(i), Some(TokenTree::Punct(p)) if p.as_char() == ch) }
fn is_colon2(tokens: &[TokenTree], i: usize) -> bool { matches!(tokens.get(i), Some(TokenTree::Punct(p)) if p.as_char() == ':' && p.spacing() == Spacing::Joint) && is_punct(tokens, i + 1, ':') }
fn skip_vis(tokens: &[TokenTree], pos: &mut usize) {
    if matches!(tokens.get(*pos), Some(TokenTree::Ident(id)) if id.to_string() == "pub") {
        *pos += 1; if matches!(tokens.get(*pos), Some(t) if is_delim(t, Delimiter::Parenthesis)) { *pos += 1; }
    }
}
fn expect_ident(tokens: &[TokenTree], pos: &mut usize, desc: &str) -> Result<String, String> {
    match tokens.get(*pos) { Some(TokenTree::Ident(id)) => { *pos += 1; Ok(id.to_string()) } other => Err(format!("expected {desc}, found {other:?}")) }
}
fn strip_raw(s: &str) -> String { s.strip_prefix("r#").unwrap_or(s).to_owned() }

pub(crate) fn parse_usize_literal(raw: &str) -> Result<usize, String> {
    let c = raw.replace('_', "");
    let t = c.trim_end_matches("usize").trim_end_matches("u64").trim_end_matches("u32").trim_end_matches("u16").trim_end_matches("u8");
    let (r, d) = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) { (16, h) }
        else if let Some(b) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) { (2, b) }
        else if let Some(o) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) { (8, o) }
        else { (10, t) };
    usize::from_str_radix(d, r).map_err(|_| format!("invalid integer `{raw}`"))
}

pub(crate) fn parse_string_literal(raw: &str) -> Result<String, String> {
    if let Some(rest) = raw.strip_prefix('r') {
        let n = rest.bytes().take_while(|&b| b == b'#').count();
        let body = rest[n..].strip_prefix('"').ok_or_else(|| format!("malformed raw string `{raw}`"))?;
        return body.strip_suffix(&format!("\"{}", "#".repeat(n))).map(str::to_owned).ok_or_else(|| format!("unterminated raw string `{raw}`"));
    }
    let inner = raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')).ok_or_else(|| format!("expected string, found `{raw}`"))?;
    let (mut out, mut chars) = (String::with_capacity(inner.len()), inner.chars());
    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue; }
        match chars.next() {
            Some('n') => out.push('\n'), Some('r') => out.push('\r'), Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'), Some('"') => out.push('"'), Some('\'') => out.push('\''), Some('0') => out.push('\0'),
            Some('x') => {
                let hex = format!("{}{}", chars.next().ok_or("bad \\x")?, chars.next().ok_or("bad \\x")?);
                out.push(u8::from_str_radix(&hex, 16).map_err(|_| format!("bad \\x{hex}"))? as char);
            }
            Some('u') if chars.next() == Some('{') => {
                let hex: String = chars.by_ref().take_while(|&ch| ch != '}').filter(|&ch| ch != '_').collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).map_err(|_| format!("bad \\u{{{hex}}}"))?).ok_or("invalid Unicode")?);
            }
            Some('\n') => while matches!(chars.clone().next(), Some(' ' | '\t' | '\r' | '\n')) { chars.next(); },
            other => return Err(format!("unsupported escape `\\{other:?}`")),
        }
    }
    Ok(out)
}
