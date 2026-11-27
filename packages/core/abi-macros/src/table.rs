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
    no_inherit: bool,
    doc: Vec<String>,
}

struct Table {
    vis: String,
    ident: String,
    name: String,
    dense: bool,
    flags: u8,
    inherited: bool,
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

fn strip_outer_parens(mut toks: &[Tok]) -> &[Tok] {
    while let [Tok::Group(Delim::Paren | Delim::None, inner)] = toks {
        toks = inner.as_slice();
    }
    toks
}

fn flatten_none_groups<'a>(toks: &'a [Tok], out: &mut Vec<&'a Tok>) {
    for t in toks {
        if let Tok::Group(Delim::None, inner) = t {
            flatten_none_groups(inner, out);
        } else {
            out.push(t);
        }
    }
}

fn trailing_two_path_idents(mut toks: &[Tok]) -> Option<(&str, &str)> {
    toks = strip_outer_parens(toks);
    let mut negated = false;
    while is_punct(toks, 0, '-') {
        negated = true;
        toks = strip_outer_parens(&toks[1..]);
    }
    let mut flat = Vec::with_capacity(toks.len());
    flatten_none_groups(toks, &mut flat);
    let mut segs: Vec<&str> = Vec::new();
    let mut i = 0;
    if matches!(flat.first(), Some(Tok::Punct(':', _)))
        && matches!(flat.get(1), Some(Tok::Punct(':', _)))
    {
        i = 2;
    }
    while i < flat.len() {
        let Tok::Ident(id) = flat[i] else {
            return None;
        };
        segs.push(id.as_str());
        i += 1;
        if i == flat.len() {
            break;
        }
        if matches!(flat.get(i), Some(Tok::Punct(':', _)))
            && matches!(flat.get(i + 1), Some(Tok::Punct(':', _)))
        {
            i += 2;
        } else {
            return None;
        }
    }
    if segs.len() < 2 {
        return None;
    }
    let ty = segs[segs.len() - 2];
    let item = segs[segs.len() - 1];
    if negated
        && !matches!(
            (ty, item),
            ("f32" | "f64", "NAN" | "INFINITY" | "NEG_INFINITY")
        )
    {
        return None;
    }
    Some((ty, item))
}

fn check_no_magic_sentinel(field_name: &str, toks: &[Tok]) -> Result<(), String> {
    if let Some((ty, item)) = trailing_two_path_idents(toks) {
        let banned = matches!(
            (ty, item),
            ("f32" | "f64", "NAN" | "INFINITY" | "NEG_INFINITY")
                | ("u8" | "u16" | "u32" | "u64", "MAX")
                | ("i8" | "i16" | "i32" | "i64", "MIN")
        );
        if banned {
            return Err(format!(
                "field `{field_name}`: magic sentinel default `{}` is forbidden; use `Option<T>` instead",
                render_tokens(toks)
            ));
        }
    }
    Ok(())
}

fn check_no_split_bool_pairs(fields: &[Field]) -> Result<(), String> {
    for f in fields {
        if f.short_ty != "bool" || f.inner_opt_short.is_some() {
            continue;
        }
        let candidates = [
            f.name.strip_suffix("_enabled"),
            f.name.strip_suffix("_set"),
            f.name.strip_suffix("_present"),
            f.name.strip_suffix("_specified"),
            f.name.strip_prefix("has_"),
        ];
        for base in candidates.into_iter().flatten() {
            if !base.is_empty() && fields.iter().any(|other| other.name == base) {
                return Err(format!(
                    "field `{}`: split-bool sentinel pair with `{base}` is forbidden; use `Option<T>` or `#[nve_table(inherited)]` instead",
                    f.name
                ));
            }
        }
    }
    Ok(())
}

fn check_no_method_collisions(inherited: bool, fields: &[Field]) -> Result<(), String> {
    for f in fields {
        if f.name == "get" {
            return Err("field `get` collides with generated `*Mut::get()` method".into());
        }
        if inherited && f.name == "explicit_mask" {
            return Err(
                "field `explicit_mask` collides with generated `*Ref::explicit_mask()` method on inherited table"
                    .into(),
            );
        }
    }
    if inherited {
        for f in fields {
            let clear_name = format!("clear_{}", f.name);
            if fields.iter().any(|other| other.name == clear_name) {
                return Err(format!(
                    "field `{clear_name}` collides with generated `clear_{}()` method on inherited table",
                    f.name
                ));
            }
            let explicit_name = format!("is_{}_explicit", f.name);
            if fields.iter().any(|other| other.name == explicit_name) {
                return Err(format!(
                    "field `{explicit_name}` collides with generated `is_{}_explicit()` method on inherited table",
                    f.name
                ));
            }
        }
    }
    Ok(())
}

fn parse(tokens: &[Tok]) -> Result<Table, String> {
    let mut pos = 0;
    let (mut name, mut dense, mut flags, mut inherited, mut events_ty) =
        (None, true, 0u8, false, None::<String>);
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
                match k.as_str() {
                    "inherited" => {
                        if is_punct(kv, i + 1, '=') {
                            return Err(
                                "#[nve_table]: `inherited` is a bare flag, not `key = value`"
                                    .into(),
                            );
                        }
                        if inherited {
                            return Err("#[nve_table]: duplicate `inherited` flag".into());
                        }
                        inherited = true;
                        i += 1;
                    }
                    "events" => {
                        if !is_punct(kv, i + 1, '=') {
                            return Err("expected `=` after #[nve_table] key `events`".into());
                        }
                        i += 2;
                        let start = i;
                        let mut depth = 0i32;
                        while i < kv.len() {
                            match &kv[i] {
                                Tok::Punct('<', _) => depth += 1,
                                Tok::Punct('>', _) => depth -= 1,
                                Tok::Punct(',', _) if depth == 0 => break,
                                _ => {}
                            }
                            i += 1;
                        }
                        if i == start {
                            return Err(
                                "#[nve_table]: `events = <EnumType>` requires an event enum type"
                                    .into(),
                            );
                        }
                        events_ty = Some(render_tokens(&kv[start..i]));
                    }
                    key => {
                        if !is_punct(kv, i + 1, '=') {
                            return Err(format!("expected `=` after #[nve_table] key `{key}`"));
                        }
                        let Tok::Literal(v) =
                            kv.get(i + 2).ok_or("expected value in #[nve_table]")?
                        else {
                            return Err(format!(
                                "expected string literal for #[nve_table] key `{key}`"
                            ));
                        };
                        let v = parse_string_literal(v)?;
                        match key {
                            "name" => name = Some(v),
                            "storage" => {
                                dense = match v.as_str() {
                                    "dense" => true,
                                    "sparse" => false,
                                    o => {
                                        return Err(format!(
                                            "storage must be dense|sparse, got `{o}`"
                                        ))
                                    }
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
                    }
                }
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

    if inherited && events_ty.is_some() {
        return Err("#[nve_table]: cannot combine `inherited` and `events`".into());
    }
    if let Some(ev) = events_ty {
        return Err(format!(
            "#[nve_table(events = {ev})]: EventEnum table support lands in C5"
        ));
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
    if inherited {
        // Set `table_flags::INHERITED` (1 << 3) so wire `DeclareTables` carries inherited state.
        flags |= 1 << 3;
        if fields.len() > 32 {
            return Err(format!(
                "#[nve_table(inherited)] table `{ident}` supports at most 32 fields (u32 explicit mask), got {}",
                fields.len()
            ));
        }
    } else if let Some(f) = fields.iter().find(|f| f.no_inherit) {
        return Err(format!(
            "field `{}`: `#[no_inherit]` is only valid on `#[nve_table(inherited)]` tables",
            f.name
        ));
    }
    check_no_split_bool_pairs(&fields)?;
    check_no_method_collisions(inherited, &fields)?;
    Ok(Table {
        vis,
        ident,
        name,
        dense,
        flags,
        inherited,
        fields,
    })
}

fn parse_fields(tokens: &[Tok]) -> Result<Vec<Field>, String> {
    let (mut pos, mut out) = (0, Vec::new());
    while pos < tokens.len() {
        let (mut effects, mut default_toks, mut bits, mut no_inherit, mut doc) =
            (0u8, None, None, false, Vec::new());
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
                    let raw = parse_usize_literal(l)?;
                    if raw == 0 || raw >= 32 {
                        return Err(format!(
                            "#[bits({raw})] is only valid for strictly narrowing integer primitives (`1..natural_bits`)"
                        ));
                    }
                    bits = Some(raw as u32);
                }
                "no_inherit" => {
                    if inner.len() > 1 {
                        return Err("#[no_inherit] takes no arguments".into());
                    }
                    if no_inherit {
                        return Err("duplicate #[no_inherit] attribute".into());
                    }
                    no_inherit = true;
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
            let inner_head = inner.split('<').next().unwrap_or(inner);
            let head_short = inner_head.rsplit("::").next().unwrap_or(inner_head);
            let short = if head_short == "NonZero"
                && let Some(nz_lt) = inner.find('<')
                && inner.ends_with('>')
            {
                let arg = &inner[nz_lt + 1..inner.len() - 1];
                let arg_short = arg.rsplit("::").next().unwrap_or(arg);
                format!("NonZero<{arg_short}>")
            } else {
                head_short.to_owned()
            };
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
        if let Some(ref toks) = default_toks {
            check_no_magic_sentinel(&name, toks)?;
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
            no_inherit,
            doc,
        });
    }
    Ok(out)
}

fn upper(s: &str) -> String {
    s.to_ascii_uppercase()
}
