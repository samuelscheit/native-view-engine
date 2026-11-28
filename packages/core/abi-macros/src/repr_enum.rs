//! Zero-dependency `#[derive(ReprEnum)]` for unit-only `#[repr(u8|u16|u32|u64|i8|i16|i32|i64)]`
//! enums (contiguous, 1-based, sparse, or signed).
//!
//! Emits pure `::core::*` code (zero external trait/crate dependencies):
//! - `pub const COUNT: usize`
//! - `pub const ALL: [Self; N]`
//! - `pub const fn as_<repr>(self) -> <repr>`
//! - `pub const fn try_from_<repr>(raw: <repr>) -> Option<Self>`
//! - ONLY when explicitly opted in via `#[repr_enum(fallback = Variant)]`:
//!   `pub const fn from_<repr>(raw: <repr>) -> Self` (NEVER implicit from `#[default]`).

use std::fmt::Write as _;

use proc_macro::TokenStream;

use crate::fn_sig::{strip_raw, unwrap_none};
use crate::parse::parse_i128_literal;
use crate::tok::{
    attr_name, check_no_cfg_attr, expect_ident, from_stream, is_ident, is_punct, paren_inner,
    skip_vis, Delim, Tok,
};

#[cfg(test)]
#[path = "repr_enum_tests.rs"]
mod tests;

struct ParsedReprEnum {
    ident: String,
    repr: String,
    variants: Vec<(String, i128)>,
    fallback: Option<String>,
}

pub(crate) fn derive(input: TokenStream) -> Result<String, String> {
    derive_tokens(&from_stream(input))
}

pub(crate) fn derive_tokens(tokens: &[Tok]) -> Result<String, String> {
    let parsed = parse(tokens)?;
    Ok(emit(&parsed))
}

fn repr_bounds(repr: &str) -> Option<(i128, i128)> {
    Some(match repr {
        "u8" => (0, i128::from(u8::MAX)),
        "u16" => (0, i128::from(u16::MAX)),
        "u32" => (0, i128::from(u32::MAX)),
        "u64" => (0, i128::from(u64::MAX)),
        "i8" => (i128::from(i8::MIN), i128::from(i8::MAX)),
        "i16" => (i128::from(i16::MIN), i128::from(i16::MAX)),
        "i32" => (i128::from(i32::MIN), i128::from(i32::MAX)),
        "i64" => (i128::from(i64::MIN), i128::from(i64::MAX)),
        _ => return None,
    })
}

fn parse(tokens: &[Tok]) -> Result<ParsedReprEnum, String> {
    let mut pos = 0usize;
    let mut repr: Option<String> = None;
    let mut bad_repr: Option<String> = None;
    let mut fallback: Option<String> = None;

    while is_punct(tokens, pos, '#') {
        let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(pos + 1) else {
            return Err("#[derive(ReprEnum)] expected `[...]` after `#`".to_owned());
        };
        pos += 2;
        let (name, rest) = attr_name(inner);
        check_no_cfg_attr(
            name,
            rest,
            false,
            &["repr_enum", "repr", "cfg"],
            "#[derive(ReprEnum)]",
        )?;
        match name {
            "repr" => {
                if let Some(args) = paren_inner(rest) {
                    let mut i = 0;
                    while i < args.len() {
                        if let Tok::Ident(id) = &args[i] {
                            i += 1;
                            if repr_bounds(id).is_some() {
                                if let Some(prev) = &repr {
                                    return Err(format!(
                                        "#[derive(ReprEnum)] found conflicting integer reprs `{prev}` and `{id}`"
                                    ));
                                }
                                repr = Some(id.clone());
                            } else {
                                bad_repr = Some(id.clone());
                                if matches!(args.get(i), Some(Tok::Group(_, _))) {
                                    i += 1;
                                }
                            }
                        } else {
                            i += 1;
                        }
                        if is_punct(args, i, ',') {
                            i += 1;
                        }
                    }
                }
            }
            "repr_enum" => {
                let args = paren_inner(rest).ok_or("expected `#[repr_enum(...)]`")?;
                let mut i = 0;
                while i < args.len() {
                    let k = expect_ident(args, &mut i, "repr_enum key")?;
                    if !is_punct(args, i, '=') {
                        return Err(format!("expected `=` after #[repr_enum] key `{k}`"));
                    }
                    i += 1;
                    match k.as_str() {
                        "fallback" => {
                            let v = expect_ident(args, &mut i, "fallback variant name")?;
                            if fallback.replace(v).is_some() {
                                return Err(
                                    "#[derive(ReprEnum)] allows at most one `fallback` variant"
                                        .to_owned(),
                                );
                            }
                        }
                        other => return Err(format!("unknown #[repr_enum] key `{other}`")),
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
        return Err(format!(
            "#[derive(ReprEnum)] only supports enums, found `{kw}`"
        ));
    }
    let ident = expect_ident(tokens, &mut pos, "enum name")?;
    if is_punct(tokens, pos, '<') {
        return Err(format!(
            "#[derive(ReprEnum)] does not support generic parameters on `{ident}`"
        ));
    }
    if is_ident(tokens, pos, "where") {
        return Err(format!(
            "#[derive(ReprEnum)] does not support `where` clauses on `{ident}`"
        ));
    }

    if let Some(bad) = bad_repr {
        return Err(format!(
            "#[derive(ReprEnum)] on `{ident}` requires `#[repr(u8|u16|u32|u64|i8|i16|i32|i64)]`, found `#[repr({bad})]`"
        ));
    }
    let repr = repr.ok_or_else(|| {
        format!(
            "#[derive(ReprEnum)] on `{ident}` requires `#[repr(u8|u16|u32|u64|i8|i16|i32|i64)]`"
        )
    })?;

    let Some(Tok::Group(Delim::Brace, body)) = tokens.get(pos) else {
        return Err(format!(
            "#[derive(ReprEnum)] requires a braced enum body on `{ident}`"
        ));
    };
    let (min_val, max_val) = repr_bounds(&repr).expect("validated repr");
    let variants = parse_variants(body, &ident, &repr, min_val, max_val)?;

    let resolved_fallback = match fallback {
        Some(fb) => {
            let fb_stripped = strip_raw(&fb);
            let matched = variants
                .iter()
                .find(|(v, _)| *v == fb || strip_raw(v) == fb_stripped)
                .map(|(v, _)| v.clone());
            match matched {
                Some(v) => Some(v),
                None => {
                    return Err(format!(
                        "#[derive(ReprEnum)] on `{ident}`: `fallback = {fb}` does not match any variant"
                    ));
                }
            }
        }
        None => None,
    };

    Ok(ParsedReprEnum {
        ident,
        repr,
        variants,
        fallback: resolved_fallback,
    })
}

fn parse_variants(
    body: &[Tok],
    enum_name: &str,
    repr: &str,
    min_val: i128,
    max_val: i128,
) -> Result<Vec<(String, i128)>, String> {
    let mut pos = 0usize;
    let mut variants: Vec<(String, i128)> = Vec::new();
    let mut next_val: Option<i128> = Some(0);

    while pos < body.len() {
        while is_punct(body, pos, '#') {
            let Some(Tok::Group(Delim::Bracket, inner)) = body.get(pos + 1) else {
                return Err(format!("malformed variant attribute on `{enum_name}`"));
            };
            pos += 2;
            let (name, rest) = attr_name(inner);
            check_no_cfg_attr(
                name,
                rest,
                true,
                &["repr_enum", "cfg"],
                "#[derive(ReprEnum)] variant",
            )?;
            if name == "repr_enum" {
                return Err(format!(
                    "#[derive(ReprEnum)] on `{enum_name}` does not support variant-level `#[repr_enum(...)]`"
                ));
            }
        }
        if pos >= body.len() {
            break;
        }

        let vname = expect_ident(body, &mut pos, "variant name")?;
        if matches!(
            body.get(pos),
            Some(Tok::Group(Delim::Paren | Delim::Brace, _))
        ) {
            return Err(format!(
                "#[derive(ReprEnum)] variant `{enum_name}::{vname}` must be unit-only"
            ));
        }

        let val = if is_punct(body, pos, '=') {
            pos += 1;
            let disc_start = pos;
            while pos < body.len() && !is_punct(body, pos, ',') {
                pos += 1;
            }
            let disc_toks = unwrap_none(&body[disc_start..pos]);
            match disc_toks {
                [Tok::Literal(lit)] => parse_i128_literal(lit, false)?,
                [Tok::Punct('-', _), Tok::Literal(lit)] => parse_i128_literal(lit, true)?,
                _ => {
                    return Err(format!(
                        "#[derive(ReprEnum)] variant `{enum_name}::{vname}` requires integer literal discriminant"
                    ));
                }
            }
        } else {
            next_val.ok_or_else(|| {
                format!(
                    "#[derive(ReprEnum)] variant `{enum_name}::{vname}` implicit discriminant overflows `{repr}`"
                )
            })?
        };

        if val < min_val || val > max_val {
            return Err(format!(
                "#[derive(ReprEnum)] variant `{enum_name}::{vname}` discriminant {val} out of range for `#[repr({repr})]` ({min_val}..={max_val})"
            ));
        }
        if let Some((prev, _)) = variants.iter().find(|(_, v)| *v == val) {
            return Err(format!(
                "#[derive(ReprEnum)] enum `{enum_name}` has duplicate discriminant {val} on `{prev}` and `{vname}`"
            ));
        }
        next_val = val.checked_add(1).filter(|&n| n <= max_val);

        if is_punct(body, pos, ',') {
            pos += 1;
        } else if pos < body.len() {
            return Err(format!("expected `,` after variant `{enum_name}::{vname}`"));
        }
        variants.push((vname, val));
    }

    if variants.is_empty() {
        return Err(format!(
            "#[derive(ReprEnum)] enum `{enum_name}` must have at least one variant"
        ));
    }
    Ok(variants)
}

fn emit(e: &ParsedReprEnum) -> String {
    let id = &e.ident;
    let r = &e.repr;
    let count = e.variants.len();
    let mut out = String::with_capacity(512);

    let all_items = e
        .variants
        .iter()
        .map(|(v, _)| format!("Self::{v}"))
        .collect::<Vec<_>>()
        .join(", ");

    let mut try_arms = String::new();
    for (v, disc) in &e.variants {
        let _ = write!(
            try_arms,
            "{disc}_{r} => ::core::option::Option::Some(Self::{v}), "
        );
    }

    let _ = writeln!(
        out,
        "#[allow(dead_code)]\nimpl {id} {{\n    pub const COUNT: usize = {count};\n    pub const ALL: [Self; {count}] = [{all_items}];\n\n    #[inline(always)]\n    pub const fn as_{r}(self) -> {r} {{\n        self as {r}\n    }}\n\n    #[inline]\n    pub const fn try_from_{r}(__nve_raw: {r}) -> ::core::option::Option<Self> {{\n        match __nve_raw {{\n            {try_arms}_ => ::core::option::Option::None,\n        }}\n    }}"
    );

    if let Some(ref fb) = e.fallback {
        let _ = writeln!(
            out,
            "\n    #[inline]\n    pub const fn from_{r}(__nve_raw: {r}) -> Self {{\n        match Self::try_from_{r}(__nve_raw) {{\n            ::core::option::Option::Some(__nve_v) => __nve_v,\n            ::core::option::Option::None => Self::{fb},\n        }}\n    }}"
        );
    }

    let _ = writeln!(out, "}}");
    out
}
