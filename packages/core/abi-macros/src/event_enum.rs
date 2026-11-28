//! Zero-dependency `#[derive(EventEnum)]` for table-scoped `#[repr(u8)]` contiguous `0..N`
//! (`1 <= N <= 32`) event enums paired with `EventSet<E>` and `#[nve_table(events = E)]`.

use proc_macro::TokenStream;

use crate::parse::{parse_i128_literal, parse_string_literal};
use crate::tok::{expect_ident, from_stream, is_colon2, is_ident, is_punct, skip_vis, Delim, Tok};

const DEFAULT_CRATE_PATH: &str = "::nve_abi";

struct ParsedEventEnum {
    ident: String,
    crate_path: String,
    variants: Vec<String>,
}

pub(crate) fn derive(input: TokenStream) -> Result<String, String> {
    let tokens = from_stream(input);
    derive_tokens(&tokens)
}

pub(crate) fn derive_tokens(tokens: &[Tok]) -> Result<String, String> {
    let parsed = parse(tokens)?;
    Ok(emit(&parsed))
}

fn attr_name(inner: &[Tok]) -> (String, &[Tok]) {
    let mut i = 0;
    let mut name = String::new();
    while let Some(Tok::Ident(id)) = inner.get(i) {
        name = id.clone();
        i += 1;
        if is_colon2(inner, i) {
            i += 2;
        } else {
            break;
        }
    }
    (name, &inner[i..])
}

fn parse(tokens: &[Tok]) -> Result<ParsedEventEnum, String> {
    let mut pos = 0usize;
    let mut crate_path: Option<String> = None;
    let mut repr_ok = false;
    let mut bad_repr: Option<String> = None;

    while is_punct(tokens, pos, '#') {
        let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(pos + 1) else {
            return Err("#[derive(EventEnum)] expected `[...]` after `#`".into());
        };
        pos += 2;
        let (name, rest) = attr_name(inner);
        match name.as_str() {
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
            "event_enum" => {
                let Some(Tok::Group(Delim::Paren, args)) = rest.first() else {
                    return Err("expected `#[event_enum(...)]`".into());
                };
                let mut i = 0;
                while i < args.len() {
                    let k = expect_ident(args, &mut i, "event_enum key")?;
                    if !is_punct(args, i, '=') {
                        return Err(format!("expected `=` after #[event_enum] key `{k}`"));
                    }
                    i += 1;
                    let Some(Tok::Literal(lit)) = args.get(i) else {
                        return Err(format!(
                            "expected string literal for #[event_enum({k} = ...)]"
                        ));
                    };
                    i += 1;
                    let val = parse_string_literal(lit)?;
                    match k.as_str() {
                        "crate_path" => crate_path = Some(val),
                        other => return Err(format!("unknown #[event_enum] key `{other}`")),
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
            "#[derive(EventEnum)] only supports enums, found `{kw}`"
        ));
    }
    let ident = expect_ident(tokens, &mut pos, "enum name")?;
    if is_punct(tokens, pos, '<') {
        return Err(format!(
            "#[derive(EventEnum)] does not support generic parameters on `{ident}`"
        ));
    }
    if is_ident(tokens, pos, "where") {
        return Err(format!(
            "#[derive(EventEnum)] does not support `where` clauses on `{ident}`"
        ));
    }
    if let Some(bad) = bad_repr {
        return Err(format!(
            "#[derive(EventEnum)] on `{ident}` requires `#[repr(u8)]`, found `#[repr({bad})]`"
        ));
    }
    if !repr_ok {
        return Err(format!(
            "#[derive(EventEnum)] on `{ident}` requires `#[repr(u8)]`"
        ));
    }

    let Some(Tok::Group(Delim::Brace, body)) = tokens.get(pos) else {
        return Err(format!(
            "#[derive(EventEnum)] requires a braced enum body on `{ident}`"
        ));
    };

    let variants = parse_variants(body, &ident)?;
    Ok(ParsedEventEnum {
        ident,
        crate_path: crate_path.unwrap_or_else(|| DEFAULT_CRATE_PATH.to_owned()),
        variants,
    })
}

fn parse_variants(body: &[Tok], enum_name: &str) -> Result<Vec<String>, String> {
    let mut pos = 0usize;
    let mut variants = Vec::new();

    while pos < body.len() {
        while is_punct(body, pos, '#') {
            if !matches!(body.get(pos + 1), Some(Tok::Group(Delim::Bracket, _))) {
                return Err(format!("malformed variant attribute on `{enum_name}`"));
            }
            pos += 2;
        }
        if pos >= body.len() {
            break;
        }

        let vname = expect_ident(body, &mut pos, "variant name")?;
        let stripped = vname.strip_prefix("r#").unwrap_or(&vname);
        if stripped == "Unset" {
            return Err(format!(
                "#[derive(EventEnum)] on `{enum_name}` bans `Unset` sentinel variants"
            ));
        }
        if matches!(
            body.get(pos),
            Some(Tok::Group(Delim::Paren | Delim::Brace, _))
        ) {
            return Err(format!(
                "#[derive(EventEnum)] variant `{enum_name}::{vname}` must be unit-only"
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
                    "#[derive(EventEnum)] variant `{enum_name}::{vname}` requires an integer literal discriminant"
                ));
            };
            pos += 1;
            let val = parse_i128_literal(lit, neg)?;
            if val != expected {
                return Err(format!(
                    "#[derive(EventEnum)] variant `{enum_name}::{vname}` has discriminant {val}, expected contiguous {expected} (0..N)"
                ));
            }
            if pos < body.len() && !is_punct(body, pos, ',') {
                return Err(format!(
                    "#[derive(EventEnum)] variant `{enum_name}::{vname}` discriminant must be a plain integer literal"
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

        variants.push(vname);
    }

    if variants.is_empty() {
        return Err(format!(
            "#[derive(EventEnum)] enum `{enum_name}` must have at least 1 variant"
        ));
    }
    if variants.len() > 32 {
        return Err(format!(
            "#[derive(EventEnum)] enum `{enum_name}` has {} variants (max 32 for EventSet<E>)",
            variants.len()
        ));
    }

    Ok(variants)
}

fn emit(e: &ParsedEventEnum) -> String {
    let id = &e.ident;
    let cp = &e.crate_path;
    let count = e.variants.len();

    let all_items = e
        .variants
        .iter()
        .map(|v| format!("Self::{v}"))
        .collect::<Vec<_>>()
        .join(", ");

    let set_items = e
        .variants
        .iter()
        .enumerate()
        .map(|(i, v)| format!("(\"{v}\", 1u32 << {i}u32), "))
        .collect::<String>();

    let match_arms = e
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
    pub const ALL: &'static [Self] = &[{all_items}];

    #[inline(always)]
    pub const fn event_id(self) -> {cp}::EventId {{
        {cp}::EventId(self as u32)
    }}

    #[inline(always)]
    pub const fn bit(self) -> u32 {{
        1u32 << (self as u32)
    }}

    #[inline(always)]
    pub const fn set(self) -> {cp}::EventSet<Self> {{
        {cp}::EventSet::from_bit_index(self as u32)
    }}

    #[inline(always)]
    pub const fn from_event_id(__nve_id: {cp}::EventId) -> ::core::option::Option<Self> {{
        match __nve_id.raw() {{
            {match_arms}_ => ::core::option::Option::None,
        }}
    }}
}}

impl {cp}::EventEnum for {id} {{
    const COUNT: usize = {count};
    const ALL: &'static [Self] = Self::ALL;
    const EVENT_SET: {cp}::table::ValueSetDesc = {cp}::table::ValueSetDesc {{
        name: "{id}Set",
        flags: true,
        values: &[{set_items}],
    }};

    #[inline(always)]
    fn event_id(self) -> {cp}::EventId {{
        Self::event_id(self)
    }}

    #[inline(always)]
    fn from_event_id(id: {cp}::EventId) -> ::core::option::Option<Self> {{
        Self::from_event_id(id)
    }}
}}

impl ::core::convert::From<{id}> for {cp}::EventSet<{id}> {{
    #[inline(always)]
    fn from(ev: {id}) -> Self {{
        Self::one(ev)
    }}
}}

impl ::core::ops::BitOr for {id} {{
    type Output = {cp}::EventSet<Self>;

    #[inline(always)]
    fn bitor(self, rhs: Self) -> Self::Output {{
        {cp}::EventSet::one(self).union({cp}::EventSet::one(rhs))
    }}
}}

impl ::core::ops::BitOr<{cp}::EventSet<Self>> for {id} {{
    type Output = {cp}::EventSet<Self>;

    #[inline(always)]
    fn bitor(self, rhs: {cp}::EventSet<Self>) -> Self::Output {{
        {cp}::EventSet::one(self).union(rhs)
    }}
}}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::derive_tokens;
    use crate::tok::lex;

    #[test]
    fn derive_event_enum_emits_trait_and_bitor_and_rejects_invalid_inputs() {
        let out = derive_tokens(&lex(
            "#[repr(u8)] pub enum MyEv { Press = 0, Blur = 1 }",
        ))
        .expect("valid EventEnum");
        assert!(out.contains("impl ::nve_abi::EventEnum for MyEv"));
        assert!(out.contains("pub const COUNT: usize = 2;"));
        assert!(out.contains("0u32 => ::core::option::Option::Some(Self::Press)"));
        assert!(out.contains("impl ::core::ops::BitOr for MyEv"));

        let cases = [
            ("enum NoRepr { A = 0 }", "requires `#[repr(u8)]`"),
            ("#[repr(u16)] enum BadRepr { A = 0 }", "found `#[repr(u16)]`"),
            ("#[repr(u8)] enum Empty {}", "at least 1 variant"),
            ("#[repr(u8)] enum HasUnset { Unset = 0 }", "bans `Unset`"),
            ("#[repr(u8)] enum Gap { A = 0, B = 2 }", "expected contiguous 1"),
            ("#[repr(u8)] enum Tuple { A(u32) }", "must be unit-only"),
            ("#[repr(u8)] enum Gen<T> { A }", "generic parameters"),
        ];
        for (src, expected) in cases {
            let err = derive_tokens(&lex(src)).expect_err(src);
            assert!(err.contains(expected), "for `{src}`, got `{err}`");
        }

        let mut many = String::from("#[repr(u8)] enum TooMany { ");
        for i in 0..33 {
            many.push_str(&format!("V{i} = {i}, "));
        }
        many.push('}');
        let err = derive_tokens(&lex(&many)).expect_err("33 variants");
        assert!(err.contains("max 32"), "got `{err}`");
    }
}
