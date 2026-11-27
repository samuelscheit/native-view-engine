//! `#[derive(NveTable)]`: word-table schema → zero-cost typed rows for `nve-store`.
//!
//! Uses compile-time `nve_abi::table::pack_table` first-fit bin-packing over effect-matched
//! words, supporting narrow scalars (`WordValue`), narrow `Option<T: OptWord>`, wide
//! `Option<f32|u32|i32|Rgba>`, 64-bit scalars, `Str`, and `Res<R>`.

use proc_macro::TokenStream;

use crate::parse::{parse_string_literal, parse_usize_literal};
use crate::tok::{
    expect_ident, from_stream, is_ident, is_punct, paren_inner, render_tokens, skip_vis, Delim, Tok,
};

#[path = "table_emit.rs"]
mod table_emit;

#[cfg(test)]
#[path = "table_tests.rs"]
mod tests;

const S: &str = "::nve_store";

enum Class {
    Scalar,
    Str,
    Res(String),
}

struct Field {
    name: String,
    ty: String,
    short_ty: String,
    /// Full Rust path inside `Option<...>` (e.g. `crate::tables::Direction`), if optional.
    inner_opt_full: Option<String>,
    /// Last path segment inside `Option<...>` (e.g. `Direction`, `f32`, `u8`), if optional.
    inner_opt_short: Option<String>,
    class: Class,
    effects: u8,
    default_toks: Option<Vec<Tok>>,
    bits: Option<u32>,
    doc: Vec<String>,
}

struct Table {
    vis: String,
    ident: String,
    name: String,
    dense: bool,
    flags: u8,
    fields: Vec<Field>,
}

pub(crate) fn derive(input: TokenStream) -> Result<String, String> {
    let tokens = from_stream(input);
    derive_tokens(&tokens)
}

pub(crate) fn derive_tokens(tokens: &[Tok]) -> Result<String, String> {
    let t = parse(tokens)?;
    Ok(table_emit::emit(&t))
}

fn effect_bit(name: &str) -> Result<u8, String> {
    Ok(match name {
        "layout" => 1,
        "measure" => 2,
        "paint" => 4,
        "composite" => 8,
        "semantics" => 16,
        "realize" => 32,
        "events" => 64,
        other => {
            return Err(format!(
                "unknown effect `{other}` (layout|measure|paint|composite|semantics|realize|events)"
            ))
        }
    })
}

fn parse_effects_list(tokens: &[Tok]) -> Result<u8, String> {
    if tokens.is_empty() {
        return Err("#[effect(...)] requires at least one effect name".into());
    }
    let mut i = 0usize;
    let mut mask = 0u8;
    while i < tokens.len() {
        let name = expect_ident(tokens, &mut i, "effect name in #[effect(...)]")?;
        mask |= effect_bit(&name)?;
        if i < tokens.len() {
            if !is_punct(tokens, i, ',') {
                return Err(format!(
                    "expected `,` between effect names in #[effect(...)], found `{:?}`",
                    tokens[i]
                ));
            }
            i += 1;
        }
    }
    Ok(mask)
}

fn natural_int_bits(short_ty: &str, full_ty: &str) -> Option<u32> {
    if full_ty.contains('<') {
        return None;
    }
    match short_ty {
        "u8" | "i8" => Some(8),
        "u16" | "i16" => Some(16),
        "u32" | "i32" => Some(32),
        _ => None,
    }
}

fn parse(tokens: &[Tok]) -> Result<Table, String> {
    let mut pos = 0;
    let (mut name, mut dense, mut flags) = (None, true, 0u8);
    while is_punct(tokens, pos, '#') {
        if let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(pos + 1)
            && matches!(inner.first(), Some(Tok::Ident(i)) if i == "nve_table")
        {
            let kv = paren_inner(&inner[1..]).ok_or("expected `#[nve_table(...)]`")?;
            let mut i = 0;
            while i < kv.len() {
                let Tok::Ident(k) = &kv[i] else {
                    return Err(format!(
                        "expected key identifier in #[nve_table], found `{:?}`",
                        kv[i]
                    ));
                };
                if !is_punct(kv, i + 1, '=') {
                    return Err(format!("expected `=` after #[nve_table] key `{k}`"));
                }
                let Tok::Literal(v) = kv.get(i + 2).ok_or("expected value in #[nve_table]")? else {
                    return Err(format!(
                        "expected string literal for #[nve_table] key `{k}`"
                    ));
                };
                let v = parse_string_literal(v)?;
                match k.as_str() {
                    "name" => name = Some(v),
                    "storage" => {
                        dense = match v.as_str() {
                            "dense" => true,
                            "sparse" => false,
                            o => return Err(format!("storage must be dense|sparse, got `{o}`")),
                        }
                    }
                    "flags" => {
                        for f in v.split('|').map(str::trim).filter(|f| !f.is_empty()) {
                            flags |= match f {
                                "host" => 1,
                                "publish" => 2,
                                "layout" => 4,
                                o => {
                                    return Err(format!(
                                        "unknown table flag `{o}` (host|publish|layout)"
                                    ))
                                }
                            };
                        }
                    }
                    o => return Err(format!("unknown #[nve_table] key `{o}`")),
                }
                i += 3;
                if i < kv.len() {
                    if !is_punct(kv, i, ',') {
                        return Err(format!(
                            "expected `,` in #[nve_table], found `{:?}`",
                            kv[i]
                        ));
                    }
                    i += 1;
                }
            }
        }
        pos += 2;
    }

    let vis_start = pos;
    skip_vis(tokens, &mut pos);
    let vis = render_tokens(&tokens[vis_start..pos]);
    let kw = expect_ident(tokens, &mut pos, "struct")?;
    if kw != "struct" {
        return Err("#[derive(NveTable)] only supports structs".into());
    }
    let ident = expect_ident(tokens, &mut pos, "struct name")?;
    if is_punct(tokens, pos, '<') || is_ident(tokens, pos, "where") {
        return Err(format!(
            "#[derive(NveTable)] does not support generics on `{ident}`"
        ));
    }
    let Some(Tok::Group(Delim::Brace, body)) = tokens.get(pos) else {
        return Err(format!(
            "#[derive(NveTable)] requires named fields on `{ident}`"
        ));
    };
    let name = name.ok_or_else(|| format!("`{ident}` needs #[nve_table(name = \"...\")]"))?;
    let fields = parse_fields(body)?;
    if fields.is_empty() {
        return Err(format!("table `{ident}` must declare at least 1 field"));
    }
    Ok(Table {
        vis,
        ident,
        name,
        dense,
        flags,
        fields,
    })
}

fn parse_fields(tokens: &[Tok]) -> Result<Vec<Field>, String> {
    let (mut pos, mut out) = (0, Vec::new());
    while pos < tokens.len() {
        let (mut effects, mut default_toks, mut bits, mut doc) = (0u8, None, None, Vec::new());
        while is_punct(tokens, pos, '#') {
            let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(pos + 1) else {
                return Err("bad attribute".into());
            };
            let attr = match inner.first() {
                Some(Tok::Ident(i)) => i.as_str(),
                _ => "",
            };
            match attr {
                "effect" => {
                    let g = paren_inner(&inner[1..]).ok_or("expected `#[effect(...)]`")?;
                    effects |= parse_effects_list(g)?;
                }
                "default" => {
                    let g = paren_inner(&inner[1..]).ok_or("expected `#[default(expr)]`")?;
                    if g.is_empty() {
                        return Err("#[default(...)] requires a constant expression".into());
                    }
                    default_toks = Some(g.to_vec());
                }
                "bits" => {
                    let g = paren_inner(&inner[1..]).ok_or("expected `#[bits(n)]`")?;
                    let [Tok::Literal(l)] = g else {
                        return Err("expected integer in #[bits]".into());
                    };
                    let n = parse_usize_literal(l)? as u32;
                    if n == 0 || n >= 32 {
                        return Err(format!(
                            "#[bits({n})] is only valid for strictly narrowing integer primitives (`1..natural_bits`)"
                        ));
                    }
                    bits = Some(n);
                }
                "doc" => {
                    if let Some(Tok::Literal(l)) = inner.get(2) {
                        doc.push(parse_string_literal(l)?);
                    }
                }
                _ => {}
            }
            pos += 2;
        }
        if pos >= tokens.len() {
            break;
        }
        skip_vis(tokens, &mut pos);
        let name = expect_ident(tokens, &mut pos, "field name")?;
        if !is_punct(tokens, pos, ':') {
            return Err(format!("expected `:` after `{name}`"));
        }
        pos += 1;
        let start = pos;
        let mut depth = 0i32;
        while pos < tokens.len() {
            match &tokens[pos] {
                Tok::Punct('<', _) => depth += 1,
                Tok::Punct('>', _) => depth -= 1,
                Tok::Punct(',', _) if depth == 0 => break,
                _ => {}
            }
            pos += 1;
        }
        let ty = render_tokens(&tokens[start..pos]);
        if is_punct(tokens, pos, ',') {
            pos += 1;
        }
        let compact: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
        let outer_head = compact.split('<').next().unwrap_or(&compact);
        let outer_last = outer_head.rsplit("::").next().unwrap_or(outer_head).to_owned();
        let (inner_opt_full, inner_opt_short) = if outer_last == "Option"
            && let Some(lt) = compact.find('<')
            && compact.ends_with('>')
        {
            let inner = &compact[lt + 1..compact.len() - 1];
            let short = inner.rsplit("::").next().unwrap_or(inner).to_owned();
            (Some(inner.to_owned()), Some(short))
        } else {
            (None, None)
        };
        let class = if outer_last == "Str" && !compact.contains('<') {
            Class::Str
        } else if outer_last == "Res"
            && let Some(lt) = compact.find('<')
            && compact.ends_with('>')
        {
            let inner = compact[lt + 1..compact.len() - 1].to_owned();
            if inner.is_empty() {
                return Err(format!("`{name}`: Res<T> requires a concrete resource type"));
            }
            Class::Res(inner)
        } else {
            Class::Scalar
        };
        if effects == 0 {
            return Err(format!("field `{name}` needs #[effect(...)]"));
        }
        if !matches!(class, Class::Scalar) && (default_toks.is_some() || bits.is_some()) {
            return Err(format!(
                "`{name}`: Str/Res fields take no #[default]/#[bits]"
            ));
        }
        if let Some(n) = bits {
            match natural_int_bits(&outer_last, &compact) {
                Some(natural) if n < natural => {}
                Some(natural) => {
                    return Err(format!(
                        "`{name}`: #[bits({n})] is redundant or wider than `{outer_last}` ({natural} bits); remove #[bits] or narrow strictly (`1..{natural}`)"
                    ));
                }
                None => {
                    return Err(format!(
                        "`{name}`: #[bits({n})] is only permitted for narrowing integer primitives (`u8|i8|u16|i16|u32|i32`), not `{ty}`"
                    ));
                }
            }
        }
        out.push(Field {
            name,
            ty,
            short_ty: outer_last,
            inner_opt_full,
            inner_opt_short,
            class,
            effects,
            default_toks,
            bits,
            doc,
        });
    }
    Ok(out)
}

fn upper(s: &str) -> String {
    s.to_ascii_uppercase()
}
