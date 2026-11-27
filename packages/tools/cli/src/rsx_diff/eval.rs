//! Pure const evaluator for `rsx!` attribute value token spans, plus the shape check that limits
//! Tier-1 edits to literal leaves.
//!
//! Accepted forms are literals and real NVE constructors/constants only (compile-checked below):
//! number / `bool` / static string literals, `Length::{points, percent, AUTO, ZERO}`,
//! `Dimension::{Points, Percent, Auto}`, `Rgba::{rgb, new}`, `ColorRgba::{rgb, rgba}`, their
//! `WHITE`/`BLACK`/`TRANSPARENT` constants, `Type::Variant` keywords and `theme` token paths.
//! Everything else (variables, other calls, arithmetic, method calls, indexing, `Option`,
//! `{expr}` interpolation) is an [`EvalError`] and the edit falls back to a Tier-2 build.

use std::fmt;

use super::attrs::ValueClasses;
use super::lexer::{Lex, Tok};
use super::strings::{has_interpolation_braces, unescape_format_braces, unescape_string_literal};

// Every accepted constructor and constant exists with the evaluated signature.
const _: () = {
    let _: nve_store::Length = nve_store::Length::points(1.0);
    let _: nve_store::Length = nve_store::Length::percent(1.0);
    let _: [nve_store::Length; 2] = [nve_store::Length::AUTO, nve_store::Length::ZERO];
    let _: [nve_abi::Dimension; 3] = [
        nve_abi::Dimension::Points(1.0),
        nve_abi::Dimension::Percent(1.0),
        nve_abi::Dimension::Auto,
    ];
    let _: [nve_store::Rgba; 5] = [
        nve_store::Rgba::rgb(1, 2, 3),
        nve_store::Rgba::new(1, 2, 3, 4),
        nve_store::Rgba::WHITE,
        nve_store::Rgba::BLACK,
        nve_store::Rgba::TRANSPARENT,
    ];
    let _: [nve_abi::ColorRgba; 5] = [
        nve_abi::ColorRgba::rgb(1, 2, 3),
        nve_abi::ColorRgba::rgba(1, 2, 3, 4),
        nve_abi::ColorRgba::WHITE,
        nve_abi::ColorRgba::BLACK,
        nve_abi::ColorRgba::TRANSPARENT,
    ];
};

/// A constant RSX attribute value, evaluated from source tokens.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedAttrValue {
    /// Static, interpolation-free string (format braces `{{`/`}}` already unescaped).
    String(String),
    /// Float literal (RSX float attributes are `f64`).
    Float(f64),
    Int(i64),
    Bool(bool),
    LengthPx(f32),
    LengthPct(f32),
    Auto,
    Rgba {
        r: u8,
        g: u8,
        b: u8,
        a: u8,
    },
    /// `Type::Variant`, or a bare imported `Variant` (`ty: None`).
    Enum {
        ty: Option<String>,
        variant: String,
    },
    /// `theme.surface.base`, `Theme::LIGHT`, `theme::ACCENT`: resolved by the running app's theme.
    ThemeToken(String),
}

impl TypedAttrValue {
    /// The single value class of this value.
    pub fn class(&self) -> ValueClasses {
        match self {
            Self::String(_) => ValueClasses::STRING,
            Self::Bool(_) => ValueClasses::BOOL,
            Self::Int(_) => ValueClasses::INT,
            Self::Float(_) => ValueClasses::FLOAT,
            Self::LengthPx(_) => ValueClasses::PX,
            Self::LengthPct(_) => ValueClasses::PCT,
            Self::Auto => ValueClasses::AUTO,
            Self::Rgba { .. } => ValueClasses::COLOR,
            Self::Enum { .. } => ValueClasses::KEYWORD,
            Self::ThemeToken(_) => ValueClasses::THEME,
        }
    }
}

impl fmt::Display for TypedAttrValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(s) => write!(f, "{s:?}"),
            Self::Float(v) => write!(f, "{v:?}"),
            Self::Int(v) => write!(f, "{v}"),
            Self::Bool(v) => write!(f, "{v}"),
            Self::LengthPx(v) => write!(f, "Length::points({v:?})"),
            Self::LengthPct(v) => write!(f, "Length::percent({v:?})"),
            Self::Auto => f.write_str("Length::AUTO"),
            Self::Rgba { r, g, b, a } => write!(f, "Rgba::new({r}, {g}, {b}, {a})"),
            Self::Enum {
                ty: Some(ty),
                variant,
            } => write!(f, "{ty}::{variant}"),
            Self::Enum { ty: None, variant } => f.write_str(variant),
            Self::ThemeToken(path) => f.write_str(path),
        }
    }
}

/// Why a value span is not a Tier-1 patchable constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalError {
    Empty,
    /// Variable, field access, `Option`, SCREAMING const or any other non-literal expression.
    NotConstant,
    /// Call outside the constructor allowlist, or a method call.
    UnsupportedCall,
    /// Tokens after a complete value (`8.0 * 2.0`, `Length::points(8.0).max(..)`).
    TrailingTokens,
    /// Rust has no unary `+`; such a save cannot compile.
    UnaryPlus,
    /// `-` applied to anything but a number literal.
    UnsupportedNegation,
    InvalidNumber,
    /// Float literal that is infinite in `f64` or as the `f32` the store keeps (`1e39`).
    NonFinite,
    /// Integer literal outside `i64` or its explicit suffix range.
    IntOverflow,
    /// Color channel that is not an integer literal in `0..=255`.
    ChannelRange,
    /// Length argument that is not a float literal (`Length::points` takes `f32`).
    LengthArgument,
    /// Wrong argument count for a constructor.
    Arity,
    InvalidString,
    /// String literal containing `{expr}` interpolation.
    Interpolated,
}

impl EvalError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty value",
            Self::NotConstant => {
                "not a constant (variable, field access or non-literal expression)"
            }
            Self::UnsupportedCall => {
                "call outside the Length/Dimension/Rgba/ColorRgba constructors"
            }
            Self::TrailingTokens => "trailing tokens after the value (arithmetic or method call)",
            Self::UnaryPlus => "Rust has no unary `+`",
            Self::UnsupportedNegation => "unary `-` on a non-literal",
            Self::InvalidNumber => "invalid number literal",
            Self::NonFinite => "float literal is not finite in f32",
            Self::IntOverflow => "integer literal out of range",
            Self::ChannelRange => "color channel must be an integer literal in 0..=255",
            Self::LengthArgument => "length argument must be a float literal",
            Self::Arity => "wrong constructor argument count",
            Self::InvalidString => "invalid string literal",
            Self::Interpolated => "interpolated `{expr}` string",
        }
    }
}

/// Evaluates the attribute value tokens `lex[start..end]` into a [`TypedAttrValue`].
pub(super) fn evaluate(
    lex: Lex<'_>,
    start: usize,
    end: usize,
) -> Result<TypedAttrValue, EvalError> {
    if start >= end || end > lex.len() {
        return Err(EvalError::Empty);
    }
    let single = end - start == 1;
    match lex.kind(start) {
        Tok::Punct if lex.is_punct(start, b'-') => {
            if end - start == 2 && lex.kind(start + 1) == Tok::Number {
                number(lex.text(start + 1), true)
            } else {
                Err(EvalError::UnsupportedNegation)
            }
        }
        Tok::Punct if lex.is_punct(start, b'+') => Err(EvalError::UnaryPlus),
        Tok::Number | Tok::Str | Tok::RawStr if !single => Err(EvalError::TrailingTokens),
        Tok::Number => number(lex.text(start), false),
        Tok::Str | Tok::RawStr => string(lex.text(start), lex.kind(start) == Tok::RawStr),
        Tok::Ident if single && lex.text(start) == "true" => Ok(TypedAttrValue::Bool(true)),
        Tok::Ident if single && lex.text(start) == "false" => Ok(TypedAttrValue::Bool(false)),
        Tok::Ident => path_value(lex, start, end),
        Tok::Punct | Tok::OtherLit => Err(EvalError::NotConstant),
    }
}

fn string(raw: &str, is_raw: bool) -> Result<TypedAttrValue, EvalError> {
    let value = unescape_string_literal(raw, is_raw).map_err(|_| EvalError::InvalidString)?;
    if has_interpolation_braces(&value) {
        return Err(EvalError::Interpolated);
    }
    Ok(TypedAttrValue::String(unescape_format_braces(value)))
}

/// Maximum `a::b::c::d::e` path segments a constant may use.
const MAX_SEGMENTS: usize = 5;
/// Maximum constructor arguments (`Rgba::new(r, g, b, a)`).
const MAX_ARGS: usize = 4;

fn path_value(lex: Lex<'_>, start: usize, end: usize) -> Result<TypedAttrValue, EvalError> {
    let mut segs = [""; MAX_SEGMENTS];
    let mut n = 0usize;
    let mut pos = start;
    loop {
        let Some(id) = lex.ident(pos) else {
            return Err(EvalError::NotConstant);
        };
        *segs.get_mut(n).ok_or(EvalError::NotConstant)? = id;
        n += 1;
        pos += 1;
        if !(pos + 2 < end && lex.is_punct(pos, b':') && lex.is_punct(pos + 1, b':')) {
            break;
        }
        pos += 2;
    }
    let path = &segs[..n];

    if matches!(path[0], "theme" | "Theme") {
        return theme_token(lex, path, pos, end);
    }
    if pos == end {
        return path_constant(path);
    }
    if lex.is_punct(pos, b'(') {
        let close = lex.close(pos);
        if close + 1 != end {
            return Err(EvalError::TrailingTokens);
        }
        let mut args = [(0usize, 0usize); MAX_ARGS];
        let argc = split_args(lex, pos + 1, close, &mut args)?;
        return call(lex, path, &args[..argc]);
    }
    if lex.is_punct(pos, b'.') {
        return Err(EvalError::NotConstant);
    }
    Err(EvalError::TrailingTokens)
}

/// `theme.a.b`, `Theme::LIGHT`, `theme::ACCENT` (+ optional `.field` chain); no calls or indexing.
fn theme_token(
    lex: Lex<'_>,
    path: &[&str],
    mut pos: usize,
    end: usize,
) -> Result<TypedAttrValue, EvalError> {
    let mut token = path.join("::");
    while pos < end {
        if lex.is_punct(pos, b'(') || lex.is_punct(pos, b'[') {
            return Err(EvalError::UnsupportedCall);
        }
        match lex
            .ident(pos + 1)
            .filter(|_| pos + 1 < end && lex.is_punct(pos, b'.'))
        {
            Some(field) => {
                token.push('.');
                token.push_str(field);
                pos += 2;
            }
            None => return Err(EvalError::TrailingTokens),
        }
    }
    // A bare `theme` is the whole theme struct, not a token.
    if path.len() == 1 && !token.contains('.') {
        return Err(EvalError::NotConstant);
    }
    Ok(TypedAttrValue::ThemeToken(token))
}

fn is_pascal_case(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_uppercase()) && s.bytes().any(|b| b.is_ascii_lowercase())
}

fn path_constant(path: &[&str]) -> Result<TypedAttrValue, EvalError> {
    match path {
        ["None" | "Some" | "Self" | "NaN"] | [.., "Option" | "Self", _] => {
            Err(EvalError::NotConstant)
        }
        [.., "Length", "AUTO"] | [.., "Dimension", "Auto"] => Ok(TypedAttrValue::Auto),
        [.., "Length", "ZERO"] => Ok(TypedAttrValue::LengthPx(0.0)),
        [.., "Rgba" | "ColorRgba", konst] => match *konst {
            "WHITE" => Ok(rgba(255, 255, 255, 255)),
            "BLACK" => Ok(rgba(0, 0, 0, 255)),
            "TRANSPARENT" => Ok(rgba(0, 0, 0, 0)),
            _ => Err(EvalError::NotConstant),
        },
        // Structs, not enums: any other path is an unknown constant.
        [.., "Length", _] => Err(EvalError::NotConstant),
        // A bare variant needs a lowercase letter: `X`/`N` and SCREAMING consts are not variants.
        [variant] if is_pascal_case(variant) => Ok(TypedAttrValue::Enum {
            ty: None,
            variant: (*variant).to_owned(),
        }),
        [.., ty, variant]
            if is_pascal_case(ty)
                && variant.starts_with(|c: char| c.is_ascii_uppercase())
                && (variant.len() == 1 || is_pascal_case(variant)) =>
        {
            Ok(TypedAttrValue::Enum {
                ty: Some((*ty).to_owned()),
                variant: (*variant).to_owned(),
            })
        }
        _ => Err(EvalError::NotConstant),
    }
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> TypedAttrValue {
    TypedAttrValue::Rgba { r, g, b, a }
}

/// Splits `start..end` at depth-0 commas (one trailing comma allowed) into `out`; returns the count.
fn split_args(
    lex: Lex<'_>,
    start: usize,
    end: usize,
    out: &mut [(usize, usize); MAX_ARGS],
) -> Result<usize, EvalError> {
    let (mut argc, mut arg_start, mut cur) = (0usize, start, start);
    loop {
        let at_end = cur >= end;
        if at_end || lex.is_punct(cur, b',') {
            if cur > arg_start {
                *out.get_mut(argc).ok_or(EvalError::Arity)? = (arg_start, cur);
                argc += 1;
            } else if !at_end || (argc == 0 && start != end) {
                return Err(EvalError::Arity);
            }
            if at_end {
                return Ok(argc);
            }
            cur += 1;
            arg_start = cur;
            continue;
        }
        let opens = lex.is_punct(cur, b'(') || lex.is_punct(cur, b'[') || lex.is_punct(cur, b'{');
        cur = if opens { lex.close(cur) + 1 } else { cur + 1 };
    }
}

fn call(lex: Lex<'_>, path: &[&str], args: &[(usize, usize)]) -> Result<TypedAttrValue, EvalError> {
    let [.., ty, func] = path else {
        return Err(EvalError::UnsupportedCall);
    };
    match (*ty, *func) {
        ("Length", "points") | ("Dimension", "Points") => {
            Ok(TypedAttrValue::LengthPx(length_arg(lex, args)?))
        }
        ("Length", "percent") | ("Dimension", "Percent") => {
            Ok(TypedAttrValue::LengthPct(length_arg(lex, args)?))
        }
        ("Rgba" | "ColorRgba", "rgb") => {
            let [r, g, b] = channels::<3>(lex, args)?;
            Ok(rgba(r, g, b, 255))
        }
        ("Rgba", "new") | ("ColorRgba", "rgba") => {
            let [r, g, b, a] = channels::<4>(lex, args)?;
            Ok(rgba(r, g, b, a))
        }
        _ => Err(EvalError::UnsupportedCall),
    }
}

fn length_arg(lex: Lex<'_>, args: &[(usize, usize)]) -> Result<f32, EvalError> {
    let [(s, e)] = args else {
        return Err(EvalError::Arity);
    };
    match evaluate(lex, *s, *e)? {
        TypedAttrValue::Float(v) => Ok(v as f32),
        _ => Err(EvalError::LengthArgument),
    }
}

fn channels<const N: usize>(lex: Lex<'_>, args: &[(usize, usize)]) -> Result<[u8; N], EvalError> {
    if args.len() != N {
        return Err(EvalError::Arity);
    }
    let mut out = [0u8; N];
    for (slot, &(s, e)) in out.iter_mut().zip(args) {
        *slot = match evaluate(lex, s, e) {
            Ok(TypedAttrValue::Int(v)) => u8::try_from(v).map_err(|_| EvalError::ChannelRange)?,
            Ok(_) | Err(EvalError::IntOverflow) => return Err(EvalError::ChannelRange),
            Err(err) => return Err(err),
        };
    }
    Ok(out)
}

/// A parsed, sign-less number literal with its type suffix (`""` when absent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum NumLit<'a> {
    Int { magnitude: u128, suffix: &'a str },
    Float { value: f64, suffix: &'a str },
}

const INT_SUFFIXES: [&str; 12] = [
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
];
/// Longest literal body accepted after removing `_` separators.
const MAX_LITERAL_DIGITS: usize = 64;

pub(super) fn parse_number_lit(lit: &str) -> Result<NumLit<'_>, EvalError> {
    if !lit.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return Err(EvalError::NotConstant);
    }
    let (radix, body) = match lit.get(..2) {
        Some("0x") => (16, &lit[2..]),
        Some("0o") => (8, &lit[2..]),
        Some("0b") => (2, &lit[2..]),
        _ => (10, lit),
    };
    let float_suffix = radix == 10 && (body.ends_with("f32") || body.ends_with("f64"));
    let suffix = if float_suffix {
        &body[body.len() - 3..]
    } else {
        INT_SUFFIXES
            .iter()
            .find(|s| body.ends_with(*s))
            .map_or("", |s| &body[body.len() - s.len()..])
    };
    let mut buf = [0u8; MAX_LITERAL_DIGITS];
    let mut len = 0usize;
    for &b in body.as_bytes()[..body.len() - suffix.len()]
        .iter()
        .filter(|&&b| b != b'_')
    {
        *buf.get_mut(len).ok_or(EvalError::InvalidNumber)? = b;
        len += 1;
    }
    let clean = std::str::from_utf8(&buf[..len]).map_err(|_| EvalError::InvalidNumber)?;
    if clean.is_empty() {
        return Err(EvalError::InvalidNumber);
    }
    if float_suffix || (radix == 10 && clean.contains(['.', 'e', 'E'])) {
        if !suffix.is_empty() && !float_suffix {
            return Err(EvalError::InvalidNumber);
        }
        let value: f64 = clean.parse().map_err(|_| EvalError::InvalidNumber)?;
        return Ok(NumLit::Float { value, suffix });
    }
    if !clean.bytes().all(|b| (b as char).is_digit(radix)) {
        return Err(EvalError::InvalidNumber);
    }
    let magnitude = u128::from_str_radix(clean, radix).map_err(|_| EvalError::IntOverflow)?;
    Ok(NumLit::Int { magnitude, suffix })
}

/// Inclusive value range of an explicit integer suffix (bounded by `i64` storage).
fn suffix_range(suffix: &str) -> (i128, i128) {
    let max = i64::MAX as i128;
    match suffix {
        "i8" => (i8::MIN.into(), i8::MAX.into()),
        "i16" => (i16::MIN.into(), i16::MAX.into()),
        "i32" => (i32::MIN.into(), i32::MAX.into()),
        "u8" => (0, u8::MAX.into()),
        "u16" => (0, u16::MAX.into()),
        "u32" => (0, u32::MAX.into()),
        "u64" | "u128" | "usize" => (0, max),
        _ => (i64::MIN.into(), max),
    }
}

fn number(lit: &str, negative: bool) -> Result<TypedAttrValue, EvalError> {
    match parse_number_lit(lit)? {
        NumLit::Float { value, .. } => {
            let signed = if negative { -value } else { value };
            if signed.is_finite() && (signed as f32).is_finite() {
                Ok(TypedAttrValue::Float(signed))
            } else {
                Err(EvalError::NonFinite)
            }
        }
        NumLit::Int { magnitude, suffix } => {
            let magnitude = i128::try_from(magnitude).map_err(|_| EvalError::IntOverflow)?;
            let value = if negative { -magnitude } else { magnitude };
            let (min, max) = suffix_range(suffix);
            if value < min || value > max {
                return Err(EvalError::IntOverflow);
            }
            i64::try_from(value)
                .map(TypedAttrValue::Int)
                .map_err(|_| EvalError::IntOverflow)
        }
    }
}

fn is_bool(ident: &str) -> bool {
    matches!(ident, "true" | "false")
}

/// Literal-leaf shape of a number token: float vs. integer and the exact suffix.
fn number_shape(text: &str) -> Option<(bool, &str)> {
    match parse_number_lit(text).ok()? {
        NumLit::Int { suffix, .. } => Some((false, suffix)),
        NumLit::Float { suffix, .. } => Some((true, suffix)),
    }
}

/// Whether two value spans have the same constructor shape: identical tokens except literal
/// leaves (numbers of the same int/float kind and suffix, with optional unary `-`; strings; and,
/// when `leaf_ident`, the final identifier: an enum variant or theme field).
pub(super) fn same_shape(
    old: Lex<'_>,
    (mut i, old_end): (usize, usize),
    new: Lex<'_>,
    (mut j, new_end): (usize, usize),
    leaf_ident: bool,
) -> bool {
    let skip_sign = |lex: Lex<'_>, k: usize, end: usize| {
        if k + 1 < end && lex.is_punct(k, b'-') && lex.kind(k + 1) == Tok::Number {
            k + 1
        } else {
            k
        }
    };
    loop {
        i = skip_sign(old, i, old_end);
        j = skip_sign(new, j, new_end);
        match (i < old_end, j < new_end) {
            (false, false) => return true,
            (true, true) => {}
            _ => return false,
        }
        let same = match (old.kind(i), new.kind(j)) {
            (Tok::Number, Tok::Number) => {
                let shape = number_shape(old.text(i));
                shape.is_some() && shape == number_shape(new.text(j))
            }
            (Tok::Str | Tok::RawStr, Tok::Str | Tok::RawStr) => true,
            (Tok::Ident, Tok::Ident)
                if is_bool(old.text(i)) && is_bool(new.text(j))
                    || (leaf_ident && i + 1 == old_end && j + 1 == new_end) =>
            {
                true
            }
            _ => old.same(i, new, j),
        };
        if !same {
            return false;
        }
        i += 1;
        j += 1;
    }
}
