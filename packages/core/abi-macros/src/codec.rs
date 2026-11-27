//! Zero-dependency `#[derive(NveCodec)]` implementing `nve_codec::{NveEncode, NveDecode}`
//! for Tier-2 little-endian binary structs and enums (unit-only and data-carrying).

use proc_macro::TokenStream;

use crate::emit::quote_str;
use crate::parse::parse_i128_literal;
use crate::parse::parse_string_literal;
pub(crate) use crate::tok::{from_stream, Delim, Tok};
use crate::tok::{expect_ident, is_ident, is_punct, paren_inner, skip_vis};

#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;

const DEFAULT_CRATE_PATH: &str = "::nve_codec";

enum ItemKind {
    NamedStruct(Vec<(String, String)>),
    TupleStruct(Vec<String>),
    UnitStruct,
    Enum { repr: String, variants: Vec<Variant> },
}

enum VariantFields {
    Unit,
    Tuple(Vec<String>),
    Named(Vec<(String, String)>),
}

struct Variant {
    ident: String,
    tag: i128,
    has_attr_tag: bool,
    fields: VariantFields,
}

struct ParsedCodec {
    ident: String,
    crate_path: String,
    kind: ItemKind,
}

pub(crate) fn derive(input: TokenStream) -> Result<String, String> {
    let tokens = from_stream(input);
    derive_tokens(&tokens)
}

pub(crate) fn derive_tokens(tokens: &[Tok]) -> Result<String, String> {
    let parsed = parse(tokens)?;
    Ok(emit(&parsed))
}

fn format_toks(tokens: &[Tok]) -> String {
    let mut out = String::new();
    for (idx, tok) in tokens.iter().enumerate() {
        match tok {
            Tok::Ident(s) => {
                if idx > 0 && !out.ends_with([':', '\'', '<', '(', '[']) && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(s);
            }
            Tok::Literal(s) => {
                if idx > 0 && !out.ends_with([' ', '(', '[', '<']) {
                    out.push(' ');
                }
                out.push_str(s);
            }
            Tok::Punct(ch, joint) => {
                out.push(*ch);
                if !joint && matches!(ch, ',' | ';') {
                    out.push(' ');
                }
            }
            Tok::Group(d, inner) => {
                let (open, close) = match d {
                    Delim::Paren => ("(", ")"),
                    Delim::Bracket => ("[", "]"),
                    Delim::Brace => ("{", "}"),
                    Delim::None => ("", ""),
                };
                out.push_str(open);
                out.push_str(&format_toks(inner));
                out.push_str(close);
            }
        }
    }
    out.trim().to_owned()
}

fn parse_item_attrs(tokens: &[Tok], pos: &mut usize) -> Result<(String, Option<String>), String> {
    let mut crate_path = DEFAULT_CRATE_PATH.to_owned();
    let mut repr = None;
    while is_punct(tokens, *pos, '#') {
        let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(*pos + 1) else {
            return Err("expected `[...]` after `#`".into());
        };
        *pos += 2;
        let Some(Tok::Ident(name)) = inner.first() else {
            continue;
        };
        match name.as_str() {
            "repr" => {
                if let Some(args) = paren_inner(&inner[1..]) {
                    for tt in args {
                        if let Tok::Ident(s) = tt {
                            match s.as_str() {
                                "u8" | "u16" | "u32" | "i32" => repr = Some(s.clone()),
                                "i8" | "i16" | "u64" | "i64" | "u128" | "i128" | "isize"
                                | "usize" => {
                                    return Err(format!(
                                        "#[derive(NveCodec)] does not support `#[repr({s})]` (supported integer reprs: u8, u16, u32, i32)"
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            "nve_codec" => {
                let kv = paren_inner(&inner[1..]).ok_or("expected `#[nve_codec(...)]`")?;
                let mut i = 0;
                while i < kv.len() {
                    let k = expect_ident(kv, &mut i, "attribute key")?;
                    if !is_punct(kv, i, '=') {
                        return Err(format!("expected `=` after `{k}` in #[nve_codec]"));
                    }
                    i += 1;
                    let Some(Tok::Literal(lit)) = kv.get(i) else {
                        return Err(format!("expected literal for #[nve_codec({k} = ...)]"));
                    };
                    i += 1;
                    match k.as_str() {
                        "crate_path" => crate_path = parse_string_literal(lit)?,
                        other => return Err(format!("unknown #[nve_codec] item key `{other}`")),
                    }
                    if is_punct(kv, i, ',') {
                        i += 1;
                    }
                }
            }
            _ => {}
        }
    }
    Ok((crate_path, repr))
}

fn parse_variant_tag_attr(tokens: &[Tok], pos: &mut usize) -> Result<Option<i128>, String> {
    let mut tag = None;
    while is_punct(tokens, *pos, '#') {
        let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(*pos + 1) else {
            return Err("expected `[...]` after `#`".into());
        };
        *pos += 2;
        if matches!(inner.first(), Some(Tok::Ident(id)) if id == "nve_codec") {
            let kv = paren_inner(&inner[1..]).ok_or("expected `#[nve_codec(...)]`")?;
            let mut i = 0;
            while i < kv.len() {
                let k = expect_ident(kv, &mut i, "variant attribute key")?;
                if !is_punct(kv, i, '=') {
                    return Err(format!("expected `=` after `{k}` in #[nve_codec]"));
                }
                i += 1;
                let neg = if is_punct(kv, i, '-') {
                    i += 1;
                    true
                } else {
                    false
                };
                let Some(Tok::Literal(lit)) = kv.get(i) else {
                    return Err(format!("expected integer literal for #[nve_codec({k} = ...)]"));
                };
                i += 1;
                match k.as_str() {
                    "tag" => {
                        tag = Some(parse_i128_literal(lit, neg)?);
                    }
                    other => return Err(format!("unknown #[nve_codec] variant key `{other}`")),
                }
                if is_punct(kv, i, ',') {
                    i += 1;
                }
            }
        }
    }
    Ok(tag)
}

fn check_field_attrs(tokens: &[Tok], pos: &mut usize) -> Result<(), String> {
    while is_punct(tokens, *pos, '#') {
        let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(*pos + 1) else {
            return Err("expected `[...]` after `#` on field".into());
        };
        *pos += 2;
        if matches!(inner.first(), Some(Tok::Ident(id)) if id == "nve_codec") {
            return Err(
                "#[derive(NveCodec)] does not support field-level `#[nve_codec(...)]` attributes"
                    .into(),
            );
        }
    }
    Ok(())
}

fn parse_type_until_comma(tokens: &[Tok], pos: &mut usize) -> String {
    let start = *pos;
    let mut depth = 0i32;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Tok::Punct('<', _) => depth += 1,
            Tok::Punct('>', _) => depth -= 1,
            Tok::Punct(',', _) if depth == 0 => break,
            _ => {}
        }
        *pos += 1;
    }
    format_toks(&tokens[start..*pos])
}

fn parse_named_fields(tokens: &[Tok]) -> Result<Vec<(String, String)>, String> {
    let (mut pos, mut out) = (0, Vec::new());
    while pos < tokens.len() {
        check_field_attrs(tokens, &mut pos)?;
        if pos >= tokens.len() { break; }
        skip_vis(tokens, &mut pos);
        let name = expect_ident(tokens, &mut pos, "field name")?;
        if !is_punct(tokens, pos, ':') { return Err(format!("expected `:` after `{name}`")); }
        pos += 1;
        let ty = parse_type_until_comma(tokens, &mut pos);
        if ty.is_empty() { return Err(format!("missing type for field `{name}`")); }
        if is_punct(tokens, pos, ',') { pos += 1; }
        out.push((name, ty));
    }
    Ok(out)
}

fn parse_tuple_fields(tokens: &[Tok]) -> Result<Vec<String>, String> {
    let (mut pos, mut out) = (0, Vec::new());
    while pos < tokens.len() {
        check_field_attrs(tokens, &mut pos)?;
        if pos >= tokens.len() { break; }
        skip_vis(tokens, &mut pos);
        let ty = parse_type_until_comma(tokens, &mut pos);
        if ty.is_empty() { return Err("missing tuple field type".into()); }
        if is_punct(tokens, pos, ',') { pos += 1; }
        out.push(ty);
    }
    Ok(out)
}

fn parse_variants(tokens: &[Tok], enum_name: &str) -> Result<Vec<Variant>, String> {
    let (mut pos, mut out, mut next_tag) = (0, Vec::new(), Some(0i128));
    while pos < tokens.len() {
        let attr_tag = parse_variant_tag_attr(tokens, &mut pos)?;
        if pos >= tokens.len() {
            break;
        }
        let ident = expect_ident(tokens, &mut pos, "enum variant")?;
        let fields = match tokens.get(pos) {
            Some(Tok::Group(Delim::Paren, inner)) => {
                pos += 1;
                VariantFields::Tuple(parse_tuple_fields(inner)?)
            }
            Some(Tok::Group(Delim::Brace, inner)) => {
                pos += 1;
                VariantFields::Named(parse_named_fields(inner)?)
            }
            _ => VariantFields::Unit,
        };
        let disc_tag = if is_punct(tokens, pos, '=') {
            pos += 1;
            let neg = if is_punct(tokens, pos, '-') {
                pos += 1;
                true
            } else {
                false
            };
            let Some(Tok::Literal(lit)) = tokens.get(pos) else {
                return Err(format!(
                    "#[derive(NveCodec)] enum `{enum_name}::{ident}` requires integer literal discriminant"
                ));
            };
            pos += 1;
            Some(parse_i128_literal(lit, neg)?)
        } else {
            None
        };
        if attr_tag.is_some() && disc_tag.is_some() {
            return Err(format!(
                "#[derive(NveCodec)] enum `{enum_name}::{ident}` cannot specify both `#[nve_codec(tag = ...)]` and `= discriminant`"
            ));
        }
        let tag = match attr_tag.or(disc_tag) {
            Some(t) => t,
            None => next_tag.ok_or_else(|| {
                format!("#[derive(NveCodec)] enum `{enum_name}::{ident}` implicit tag overflowed")
            })?,
        };
        next_tag = tag.checked_add(1);
        if is_punct(tokens, pos, ',') {
            pos += 1;
        }
        out.push(Variant {
            ident,
            tag,
            has_attr_tag: attr_tag.is_some(),
            fields,
        });
    }
    if out.is_empty() {
        return Err(format!(
            "#[derive(NveCodec)] enum `{enum_name}` must have at least one variant"
        ));
    }
    for i in 0..out.len() {
        for j in (i + 1)..out.len() {
            if out[i].tag == out[j].tag {
                return Err(format!(
                    "#[derive(NveCodec)] enum `{enum_name}` has duplicate tag {} on `{}` and `{}`",
                    out[i].tag, out[i].ident, out[j].ident
                ));
            }
        }
    }
    Ok(out)
}

fn parse(tokens: &[Tok]) -> Result<ParsedCodec, String> {
    let mut pos = 0;
    let (crate_path, repr) = parse_item_attrs(tokens, &mut pos)?;
    skip_vis(tokens, &mut pos);
    let kw = expect_ident(tokens, &mut pos, "struct or enum")?;
    let ident = expect_ident(tokens, &mut pos, "type name")?;
    if is_punct(tokens, pos, '<') {
        return Err(format!("#[derive(NveCodec)] does not support generics on `{ident}`"));
    }
    if is_ident(tokens, pos, "where") {
        return Err(format!(
            "#[derive(NveCodec)] does not support `where` clauses on `{ident}`"
        ));
    }
    let kind = match kw.as_str() {
        "struct" => match tokens.get(pos) {
            Some(Tok::Group(Delim::Brace, inner)) => {
                pos += 1;
                if pos != tokens.len() {
                    return Err(format!(
                        "#[derive(NveCodec)] unexpected trailing tokens after struct `{ident}`"
                    ));
                }
                ItemKind::NamedStruct(parse_named_fields(inner)?)
            }
            Some(Tok::Group(Delim::Paren, inner)) => {
                pos += 1;
                if is_ident(tokens, pos, "where") {
                    return Err(format!(
                        "#[derive(NveCodec)] does not support `where` clauses on `{ident}`"
                    ));
                }
                if !is_punct(tokens, pos, ';') || pos + 1 != tokens.len() {
                    return Err(format!(
                        "#[derive(NveCodec)] expected `;` after tuple struct `{ident}`"
                    ));
                }
                ItemKind::TupleStruct(parse_tuple_fields(inner)?)
            }
            _ if is_punct(tokens, pos, ';') && pos + 1 == tokens.len() => ItemKind::UnitStruct,
            other => {
                return Err(format!(
                    "#[derive(NveCodec)] expected struct body or `;` on `{ident}`, found {other:?}"
                ));
            }
        },
        "enum" => {
            let Some(Tok::Group(Delim::Brace, inner)) = tokens.get(pos) else {
                return Err(format!("#[derive(NveCodec)] requires braced enum `{ident}`"));
            };
            let variants = parse_variants(inner, &ident)?;
            let repr = repr.unwrap_or_else(|| "u8".to_owned());
            let unit_only = variants.iter().all(|v| matches!(v.fields, VariantFields::Unit));
            if unit_only && variants.iter().any(|v| v.has_attr_tag) {
                return Err(format!(
                    "#[derive(NveCodec)] unit-only enum `{ident}` forbids `#[nve_codec(tag = ...)]`; use Rust discriminants (`Variant = N`) instead"
                ));
            }
            if !unit_only && repr != "u8" {
                return Err(format!(
                    "#[derive(NveCodec)] data-carrying enum `{ident}` uses u8 variant tags"
                ));
            }
            for v in &variants {
                let ok = match repr.as_str() {
                    "u8" => (0..=255).contains(&v.tag),
                    "u16" => (0..=65535).contains(&v.tag),
                    "u32" => (0..=i128::from(u32::MAX)).contains(&v.tag),
                    "i32" => (i128::from(i32::MIN)..=i128::from(i32::MAX)).contains(&v.tag),
                    _ => false,
                };
                if !ok {
                    return Err(format!(
                        "#[derive(NveCodec)] variant `{ident}::{}` tag {} out of range for `{repr}`",
                        v.ident, v.tag
                    ));
                }
            }
            ItemKind::Enum { repr, variants }
        }
        other => return Err(format!("#[derive(NveCodec)] expects struct or enum, found `{other}`")),
    };
    Ok(ParsedCodec {
        ident,
        crate_path,
        kind,
    })
}

fn repr_read_write(repr: &str) -> (&'static str, &'static str, usize) {
    match repr {
        "u8" => ("read_u8", "write_u8", 1),
        "u16" => ("read_u16", "write_u16", 2),
        "u32" => ("read_u32", "write_u32", 4),
        "i32" => ("read_i32", "write_i32", 4),
        _ => ("read_u8", "write_u8", 1),
    }
}

fn emit(p: &ParsedCodec) -> String {
    let id = &p.ident;
    let c = &p.crate_path;
    let mut extra = String::new();
    let (encode_body, decode_body, min_bytes) = match &p.kind {
        ItemKind::UnitStruct => (
            "let _ = __nve_w; ::core::result::Result::Ok(())".to_owned(),
            "let _ = __nve_r; ::core::result::Result::Ok(Self)".to_owned(),
            "0usize".to_owned(),
        ),
        ItemKind::NamedStruct(fields) => {
            let mut enc = String::new();
            let mut dec = String::new();
            let mut min = String::from("0usize");
            for (name, ty) in fields {
                enc.push_str(&format!("<{ty} as {c}::NveEncode>::encode(&self.{name}, __nve_w)?;"));
                dec.push_str(&format!("{name}: <{ty} as {c}::NveDecode>::decode(__nve_r)?,"));
                min.push_str(&format!(" + <{ty} as {c}::NveDecode>::MIN_ENCODED_BYTES"));
            }
            if fields.is_empty() {
                enc.push_str("let _ = __nve_w;");
                (
                    format!("{enc} ::core::result::Result::Ok(())"),
                    "let _ = __nve_r; ::core::result::Result::Ok(Self {})".to_owned(),
                    min,
                )
            } else {
                enc.push_str("::core::result::Result::Ok(())");
                (enc, format!("::core::result::Result::Ok(Self {{ {dec} }})"), min)
            }
        }
        ItemKind::TupleStruct(tys) => {
            let mut enc = String::new();
            let mut dec = String::new();
            let mut min = String::from("0usize");
            for (i, ty) in tys.iter().enumerate() {
                enc.push_str(&format!("<{ty} as {c}::NveEncode>::encode(&self.{i}, __nve_w)?;"));
                dec.push_str(&format!("<{ty} as {c}::NveDecode>::decode(__nve_r)?,"));
                min.push_str(&format!(" + <{ty} as {c}::NveDecode>::MIN_ENCODED_BYTES"));
            }
            if tys.is_empty() {
                enc.push_str("let _ = __nve_w;");
                (
                    format!("{enc} ::core::result::Result::Ok(())"),
                    "let _ = __nve_r; ::core::result::Result::Ok(Self())".to_owned(),
                    min,
                )
            } else {
                enc.push_str("::core::result::Result::Ok(())");
                (enc, format!("::core::result::Result::Ok(Self({dec}))"), min)
            }
        }
        ItemKind::Enum { repr, variants } => {
            let (read_fn, write_fn, tag_size) = repr_read_write(repr);
            let unit_only = variants.iter().all(|v| matches!(v.fields, VariantFields::Unit));
            let ty_str = quote_str(id);
            if unit_only {
                let mut to_arms = String::new();
                let mut from_arms = String::new();
                for v in variants {
                    let vn = &v.ident;
                    let t = v.tag;
                    to_arms.push_str(&format!("Self::{vn} => {t}_{repr},"));
                    from_arms.push_str(&format!(
                        "{t}_{repr} => ::core::option::Option::Some(Self::{vn}),"
                    ));
                }
                extra.push_str(&format!(
                    "impl {id} {{ \
                        #[inline] pub const fn to_repr(&self) -> {repr} {{ match self {{ {to_arms} }} }} \
                        #[inline] pub const fn from_repr(__nve_raw: {repr}) -> ::core::option::Option<Self> {{ match __nve_raw {{ {from_arms} _ => ::core::option::Option::None }} }} \
                    }}"
                ));
                let enc = format!("__nve_w.{write_fn}(self.to_repr()); ::core::result::Result::Ok(())");
                let dec = format!(
                    "let __nve_raw = __nve_r.{read_fn}()?; \
                     Self::from_repr(__nve_raw).ok_or({c}::CodecError::BadEnum {{ ty: {ty_str}, raw: ::core::primitive::i64::from(__nve_raw) }})"
                );
                (enc, dec, format!("{tag_size}usize"))
            } else {
                let mut enc_arms = String::new();
                let mut dec_arms = String::new();
                for v in variants {
                    let vn = &v.ident;
                    let t = v.tag;
                    match &v.fields {
                        VariantFields::Unit => {
                            enc_arms.push_str(&format!("Self::{vn} => {{ __nve_w.write_u8({t}_u8); }}"));
                            dec_arms.push_str(&format!("{t}_u8 => ::core::result::Result::Ok(Self::{vn}),"));
                        }
                        VariantFields::Tuple(tys) => {
                            let bindings: Vec<String> = (0..tys.len()).map(|i| format!("__f{i}")).collect();
                            let bind_list = bindings.join(", ");
                            let mut body = format!("__nve_w.write_u8({t}_u8);");
                            let mut dfields = String::new();
                            for (i, ty) in tys.iter().enumerate() {
                                body.push_str(&format!("<{ty} as {c}::NveEncode>::encode(__f{i}, __nve_w)?;"));
                                dfields.push_str(&format!("<{ty} as {c}::NveDecode>::decode(__nve_r)?,"));
                            }
                            enc_arms.push_str(&format!("Self::{vn}({bind_list}) => {{ {body} }}"));
                            dec_arms.push_str(&format!("{t}_u8 => ::core::result::Result::Ok(Self::{vn}({dfields})),"));
                        }
                        VariantFields::Named(fields) => {
                            let mut pats = Vec::with_capacity(fields.len());
                            let mut body = format!("__nve_w.write_u8({t}_u8);");
                            let mut dfields = String::new();
                            for (fname, ty) in fields {
                                let bare = fname.strip_prefix("r#").unwrap_or(fname);
                                let bind = format!("__f_{bare}");
                                pats.push(format!("{fname}: {bind}"));
                                body.push_str(&format!("<{ty} as {c}::NveEncode>::encode({bind}, __nve_w)?;"));
                                dfields.push_str(&format!("{fname}: <{ty} as {c}::NveDecode>::decode(__nve_r)?,"));
                            }
                            let bind_list = pats.join(", ");
                            enc_arms.push_str(&format!("Self::{vn} {{ {bind_list} }} => {{ {body} }}"));
                            dec_arms.push_str(&format!("{t}_u8 => ::core::result::Result::Ok(Self::{vn} {{ {dfields} }}),"));
                        }
                    }
                }
                let enc = format!("match self {{ {enc_arms} }} ::core::result::Result::Ok(())");
                let dec = format!(
                    "match __nve_r.read_u8()? {{ {dec_arms} __nve_other => ::core::result::Result::Err({c}::CodecError::BadEnum {{ ty: {ty_str}, raw: ::core::primitive::i64::from(__nve_other) }}) }}"
                );
                (enc, dec, "1usize".to_owned())
            }
        }
    };

    format!(
        "{extra} \
         impl {c}::NveEncode for {id} {{ \
             #[inline] \
             fn encode(&self, __nve_w: &mut {c}::ByteWriter) -> ::core::result::Result<(), {c}::CodecError> {{ {encode_body} }} \
         }} \
         impl {c}::NveDecode for {id} {{ \
             const MIN_ENCODED_BYTES: usize = {min_bytes}; \
             #[inline] \
             fn decode(__nve_r: &mut {c}::ByteReader<'_>) -> ::core::result::Result<Self, {c}::CodecError> {{ {decode_body} }} \
         }}"
    )
}
