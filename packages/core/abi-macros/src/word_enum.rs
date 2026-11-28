//! Zero-dependency `#[derive(WordEnum)]` for packed fieldless `#[repr(u8)]` / contiguous `0..N`
//! enums used inside `#[derive(NveTable)]` rows.
//!
//! Emits:
//! - `pub const COUNT: usize`
//! - `pub const BITS: u8` (`ceil(log2(COUNT))`, min `1`)
//! - `pub const ALL: &'static [Self]`
//! - `pub const fn to_word(self) -> u32`
//! - `pub const fn try_from_word(bits: u32) -> Option<Self>` (fallible; returns `None` out of range)
//! - `pub const fn from_word(bits: u32) -> Self` (total; out-of-range bits fall back to the
//!   `#[default]` variant when present, otherwise variant 0)
//! - `pub const fn opt_to_word(v: Option<Self>) -> u32` (`None` -> `0`, `Some(v)` -> `v + 1`)
//! - `impl ::nve_store::WordValue`
//! - `impl ::nve_store::OptWord` (with `OPT_BITS = ceil(log2(COUNT + 1))`, `0 = None`,
//!   `disc + 1 = Some(v)`)
//!
//! Rejects at compile time:
//! - non-enum items, generic enums, `where` clauses, empty enums, `> 256` variants
//! - tuple or struct variants
//! - any variant named `Unset` (model optionality with `Option<T>` instead)
//! - non-contiguous or non-zero-based discriminants

use proc_macro::TokenStream;

use crate::parse::{parse_i128_literal, parse_string_literal};
use crate::tok::{attr_name, expect_ident, from_stream, is_ident, is_punct, skip_vis, Delim, Tok};

#[cfg(test)]
#[path = "word_enum_tests.rs"]
mod tests;

const DEFAULT_CRATE_PATH: &str = "::nve_store";

struct ParsedWordEnum {
    ident: String,
    crate_path: String,
    variants: Vec<String>,
    default_idx: usize,
}

pub(crate) fn derive(input: TokenStream) -> Result<String, String> {
    let tokens = from_stream(input);
    derive_tokens(&tokens)
}

pub(crate) fn derive_tokens(tokens: &[Tok]) -> Result<String, String> {
    let parsed = parse(tokens)?;
    Ok(emit(&parsed))
}

const fn bits_for_count(n: usize) -> u8 {
    if n <= 1 {
        1
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as u8
    }
}

fn parse(tokens: &[Tok]) -> Result<ParsedWordEnum, String> {
    let mut pos = 0usize;
    let mut crate_path: Option<String> = None;
    let mut repr_ok = false;
    let mut bad_repr: Option<String> = None;

    while is_punct(tokens, pos, '#') {
        let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(pos + 1) else {
            return Err("#[derive(WordEnum)] expected `[...]` after `#`".into());
        };
        pos += 2;
        let (name, rest) = attr_name(inner);
        match name {
            "repr" => {
                if let Some(Tok::Group(Delim::Paren, rtoks)) = rest.first() {
                    for t in rtoks {
                        if let Tok::Ident(id) = t {
                            match id.as_str() {
                                "u8" => repr_ok = true,
                                "C" | "transparent" => {}
                                other => bad_repr = Some(other.to_owned()),
                            }
                        }
                    }
                }
            }
            "word_enum" => {
                let Some(Tok::Group(Delim::Paren, args)) = rest.first() else {
                    return Err("expected `#[word_enum(...)]`".into());
                };
                let mut i = 0;
                while i < args.len() {
                    let k = expect_ident(args, &mut i, "word_enum key")?;
                    if !is_punct(args, i, '=') {
                        return Err(format!("expected `=` after #[word_enum] key `{k}`"));
                    }
                    i += 1;
                    let Some(Tok::Literal(lit)) = args.get(i) else {
                        return Err(format!("expected string literal for #[word_enum({k} = ...)]"));
                    };
                    i += 1;
                    let val = parse_string_literal(lit)?;
                    match k.as_str() {
                        "crate_path" => crate_path = Some(val),
                        other => return Err(format!("unknown #[word_enum] key `{other}`")),
                    }
                    if is_punct(args, i, ',') {
                        i += 1;
                    }
                }
            }
            _ => {}
        }
    }

    skip_vis(tokens, &mut pos);
    let kw = expect_ident(tokens, &mut pos, "enum")?;
    if kw != "enum" {
        return Err(format!("#[derive(WordEnum)] only supports enums, found `{kw}`"));
    }
    let ident = expect_ident(tokens, &mut pos, "enum name")?;
    if is_punct(tokens, pos, '<') {
        return Err(format!("#[derive(WordEnum)] does not support generic parameters on `{ident}`"));
    }
    if is_ident(tokens, pos, "where") {
        return Err(format!("#[derive(WordEnum)] does not support `where` clauses on `{ident}`"));
    }
    if let Some(bad) = bad_repr {
        return Err(format!(
            "#[derive(WordEnum)] on `{ident}` requires `#[repr(u8)]`, found `#[repr({bad})]`"
        ));
    }
    if !repr_ok {
        return Err(format!("#[derive(WordEnum)] on `{ident}` requires `#[repr(u8)]`"));
    }

    let Some(Tok::Group(Delim::Brace, body)) = tokens.get(pos) else {
        return Err(format!("#[derive(WordEnum)] requires a braced enum body on `{ident}`"));
    };

    let (variants, default_idx) = parse_variants(body, &ident)?;
    Ok(ParsedWordEnum {
        ident,
        crate_path: crate_path.unwrap_or_else(|| DEFAULT_CRATE_PATH.to_owned()),
        variants,
        default_idx,
    })
}

fn parse_variants(body: &[Tok], enum_name: &str) -> Result<(Vec<String>, usize), String> {
    let mut pos = 0usize;
    let mut variants = Vec::new();
    let mut default_idx: Option<usize> = None;

    while pos < body.len() {
        let mut is_default = false;
        while is_punct(body, pos, '#') {
            let Some(Tok::Group(Delim::Bracket, inner)) = body.get(pos + 1) else {
                return Err(format!("malformed variant attribute on `{enum_name}`"));
            };
            pos += 2;
            if matches!(inner.as_slice(), [Tok::Ident(id)] if id == "default") {
                is_default = true;
            }
        }
        if pos >= body.len() {
            break;
        }

        let vname = expect_ident(body, &mut pos, "variant name")?;
        let stripped = vname.strip_prefix("r#").unwrap_or(&vname);
        if stripped == "Unset" {
            return Err(format!(
                "#[derive(WordEnum)] on `{enum_name}` bans `Unset` sentinel variants; model optionality with `Option<{enum_name}>`"
            ));
        }
        if matches!(
            body.get(pos),
            Some(Tok::Group(Delim::Paren | Delim::Brace, _))
        ) {
            return Err(format!(
                "#[derive(WordEnum)] variant `{enum_name}::{vname}` must be unit-only"
            ));
        }

        let expected = variants.len() as i128;
        if is_punct(body, pos, '=') {
            pos += 1;
            let neg = if is_punct(body, pos, '-') {
                pos += 1;
                true
            } else {
                false
            };
            let Some(Tok::Literal(lit)) = body.get(pos) else {
                return Err(format!(
                    "#[derive(WordEnum)] variant `{enum_name}::{vname}` requires an integer literal discriminant"
                ));
            };
            pos += 1;
            let val = parse_i128_literal(lit, neg)?;
            if val != expected {
                return Err(format!(
                    "#[derive(WordEnum)] variant `{enum_name}::{vname}` has discriminant {val}, expected contiguous {expected} (0..N)"
                ));
            }
            if pos < body.len() && !is_punct(body, pos, ',') {
                return Err(format!(
                    "#[derive(WordEnum)] variant `{enum_name}::{vname}` discriminant must be a plain integer literal"
                ));
            }
        }

        if is_punct(body, pos, ',') {
            pos += 1;
        } else if pos < body.len() {
            return Err(format!(
                "expected `,` after variant `{enum_name}::{vname}`"
            ));
        }

        if is_default {
            if default_idx.is_some() {
                return Err(format!(
                    "#[derive(WordEnum)] on `{enum_name}` has multiple `#[default]` variants"
                ));
            }
            default_idx = Some(variants.len());
        }
        variants.push(vname);
    }

    if variants.is_empty() {
        return Err(format!("#[derive(WordEnum)] enum `{enum_name}` must have at least 1 variant"));
    }
    if variants.len() > 256 {
        return Err(format!(
            "#[derive(WordEnum)] enum `{enum_name}` has {} variants (max 256 for #[repr(u8)])",
            variants.len()
        ));
    }

    Ok((variants, default_idx.unwrap_or(0)))
}

fn emit(e: &ParsedWordEnum) -> String {
    let id = &e.ident;
    let s = &e.crate_path;
    let count = e.variants.len();
    let bits = bits_for_count(count);
    let opt_bits = bits_for_count(count + 1);
    let fallback = &e.variants[e.default_idx];

    let all_items = e
        .variants
        .iter()
        .map(|v| format!("Self::{v}"))
        .collect::<Vec<_>>()
        .join(", ");

    let value_items = e
        .variants
        .iter()
        .enumerate()
        .map(|(i, v)| format!("(\"{v}\", {i}u32), "))
        .collect::<String>();

    let match_arms = e
        .variants
        .iter()
        .enumerate()
        .map(|(i, v)| format!("{i}u32 => Self::{v}, "))
        .collect::<String>();

    let try_match_arms = e
        .variants
        .iter()
        .enumerate()
        .map(|(i, v)| format!("{i}u32 => ::core::option::Option::Some(Self::{v}), "))
        .collect::<String>();

    format!(
        r#"
#[allow(dead_code)]
impl {id} {{
    pub const COUNT: usize = {count};
    pub const BITS: u8 = {bits};
    pub const ALL: &'static [Self] = &[{all_items}];
    /// Named variants mirrored to hosts as `NVE_<TYPE>_<VARIANT>`.
    pub const VALUE_SET: {s}::__private::ValueSetDesc = {s}::__private::ValueSetDesc {{
        name: "{id}",
        flags: false,
        values: &[{value_items}],
    }};

    #[inline(always)]
    pub const fn to_word(self) -> u32 {{
        self as u32
    }}

    #[inline(always)]
    pub const fn try_from_word(__nve_bits: u32) -> ::core::option::Option<Self> {{
        match __nve_bits {{
            {try_match_arms}_ => ::core::option::Option::None,
        }}
    }}

    #[inline(always)]
    pub const fn from_word(__nve_bits: u32) -> Self {{
        match __nve_bits {{
            {match_arms}_ => Self::{fallback},
        }}
    }}

    #[inline(always)]
    pub const fn opt_to_word(__nve_v: ::core::option::Option<Self>) -> u32 {{
        match __nve_v {{
            ::core::option::Option::None => 0,
            ::core::option::Option::Some(__nve_x) => (__nve_x as u32) + 1,
        }}
    }}
}}

impl {s}::WordValue for {id} {{
    const BITS: u8 = {bits};
    const KIND: {s}::__private::FieldKind = {s}::__private::FieldKind::Enum;
    const VALUES: ::core::option::Option<&'static {s}::__private::ValueSetDesc> =
        ::core::option::Option::Some(&Self::VALUE_SET);

    #[inline(always)]
    fn to_bits(self) -> u32 {{
        self as u32
    }}

    #[inline(always)]
    fn from_bits(__nve_bits: u32) -> Self {{
        Self::from_word(__nve_bits)
    }}
}}

impl {s}::OptWord for {id} {{
    const OPT_BITS: u8 = {opt_bits};
    const OPT_KIND: {s}::__private::FieldKind = {s}::__private::FieldKind::Enum;
    const OPT_ENCODING: {s}::__private::OptEncoding = {s}::__private::OptEncoding::PlusOne;
    const VALUES: ::core::option::Option<&'static {s}::__private::ValueSetDesc> =
        ::core::option::Option::Some(&Self::VALUE_SET);

    #[inline(always)]
    fn opt_to_bits(__nve_v: ::core::option::Option<Self>) -> u32 {{
        Self::opt_to_word(__nve_v)
    }}

    #[inline(always)]
    fn opt_from_bits(__nve_bits: u32) -> ::core::option::Option<Self> {{
        Self::try_from_word(__nve_bits.wrapping_sub(1))
    }}
}}
"#
    )
}
