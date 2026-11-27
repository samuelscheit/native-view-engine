//! Pure, allocation-light const evaluator for `rsx!` attribute value token spans.
//!
//! Accepts exactly the constant forms a Tier-1 patch can reproduce without compiling Rust:
//! number / `bool` / static string literals, length and color helpers, `Enum::Variant` keywords and
//! `theme` token paths. Everything else (variables, arbitrary calls, arithmetic, method calls,
//! `{expr}` interpolation) is an [`EvalError`] and the edit falls back to a Tier-2 build.

use std::fmt;

use super::attrs::ValueClasses;
use super::lexer::{
    LexResult, TokenKind, has_interpolation_braces, unescape_format_braces, unescape_string_literal,
};

/// A constant RSX attribute value, evaluated from source tokens.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedAttrValue {
    /// Static, interpolation-free string (format braces `{{`/`}}` already unescaped).
    String(String),
    Float(f32),
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
    /// `Type::Variant` (or a bare imported `Variant`, `ty: None`).
    Enum {
        ty: Option<String>,
        variant: String,
    },
    /// `theme.surface.base`, `Theme::LIGHT`, `theme::ACCENT`: resolved by the running app's theme.
    ThemeToken(String),
}

impl TypedAttrValue {
    /// The attribute value classes this value can satisfy.
    pub fn classes(&self) -> ValueClasses {
        match self {
            Self::String(_) | Self::Bool(_) => ValueClasses::STRING,
            Self::Float(_) | Self::Int(_) => ValueClasses::NUMBER.or(ValueClasses::LENGTH),
            Self::LengthPx(_) | Self::LengthPct(_) | Self::Auto => ValueClasses::LENGTH,
            Self::Rgba { .. } => ValueClasses::COLOR,
            Self::Enum { .. } => ValueClasses::KEYWORD,
            Self::ThemeToken(_) => ValueClasses::LENGTH
                .or(ValueClasses::NUMBER)
                .or(ValueClasses::COLOR),
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
            Self::LengthPx(v) => write!(f, "px({v:?})"),
            Self::LengthPct(v) => write!(f, "pct({v:?})"),
            Self::Auto => f.write_str("Length::Auto"),
            Self::Rgba { r, g, b, a } => write!(f, "rgba({r}, {g}, {b}, {a})"),
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
    /// Variable, field access, `None`, SCREAMING const or any other non-literal expression.
    NotConstant,
    /// Call outside the length/color helper allowlist, or a method call.
    UnsupportedCall,
    /// Tokens after a complete value (`8.0 * 2.0`, `px(8.0).max(..)`).
    TrailingTokens,
    /// Rust has no unary `+`; such a save cannot compile.
    UnaryPlus,
    /// `-` applied to anything but a number literal.
    UnsupportedNegation,
    InvalidNumber,
    /// Float literal that is infinite in `f32` (e.g. `1e39`, `1e400`).
    NonFinite,
    /// Integer literal outside `i64` or its explicit suffix range.
    IntOverflow,
    /// Color channel that is not an integer literal in `0..=255`.
    ChannelRange,
    /// `hex(..)` argument that is not a 6- or 8-digit `0x` literal.
    HexDigits,
    /// Wrong argument count for a helper.
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
            Self::UnsupportedCall => "call outside the px/pct/rgb/rgba/hex helper allowlist",
            Self::TrailingTokens => "trailing tokens after the value (arithmetic or method call)",
            Self::UnaryPlus => "Rust has no unary `+`",
            Self::UnsupportedNegation => "unary `-` on a non-literal",
            Self::InvalidNumber => "invalid number literal",
            Self::NonFinite => "float literal is not finite in f32",
            Self::IntOverflow => "integer literal out of range",
            Self::ChannelRange => "color channel must be an integer literal in 0..=255",
            Self::HexDigits => "hex color must be a 6- or 8-digit `0x` literal",
            Self::Arity => "wrong helper argument count",
            Self::InvalidString => "invalid string literal",
            Self::Interpolated => "interpolated `{expr}` string",
        }
    }
}

/// Evaluates the attribute value tokens `lex.tokens[start..end]` into a [`TypedAttrValue`].
pub(super) fn evaluate(
    lex: &LexResult<'_>,
    start: usize,
    end: usize,
) -> Result<TypedAttrValue, EvalError> {
    let Some(toks) = lex.tokens.get(start..end) else {
        return Err(EvalError::Empty);
    };
    match toks {
        [] => Err(EvalError::Empty),
        [TokenKind::Punct('-'), TokenKind::OtherLit(lit)] => number(lit, true),
        [TokenKind::Punct('-'), ..] => Err(EvalError::UnsupportedNegation),
        [TokenKind::Punct('+'), ..] => Err(EvalError::UnaryPlus),
        [TokenKind::OtherLit(lit)] => number(lit, false),
        [TokenKind::OtherLit(_), ..] | [TokenKind::StringLit { .. }, _, ..] => {
            Err(EvalError::TrailingTokens)
        }
        [TokenKind::StringLit { raw, is_raw }] => string(raw, *is_raw),
        [TokenKind::Ident("true")] => Ok(TypedAttrValue::Bool(true)),
        [TokenKind::Ident("false")] => Ok(TypedAttrValue::Bool(false)),
        [TokenKind::Ident(_), ..] => path_value(lex, start, end),
        _ => Err(EvalError::NotConstant),
    }
}

fn string(raw: &str, is_raw: bool) -> Result<TypedAttrValue, EvalError> {
    let value = unescape_string_literal(raw, is_raw).map_err(|_| EvalError::InvalidString)?;
    if has_interpolation_braces(&value) {
        return Err(EvalError::Interpolated);
    }
    Ok(TypedAttrValue::String(unescape_format_braces(value)))
}

/// Maximum `a::b::c::d` path segments a constant may use.
const MAX_SEGMENTS: usize = 4;
/// Maximum helper arguments (`rgba(r, g, b, a)`).
const MAX_ARGS: usize = 4;

fn path_value(lex: &LexResult<'_>, start: usize, end: usize) -> Result<TypedAttrValue, EvalError> {
    let toks = &lex.tokens;
    let mut segs = [""; MAX_SEGMENTS];
    let mut n = 0usize;
    let mut pos = start;
    loop {
        let TokenKind::Ident(id) = toks[pos] else {
            return Err(EvalError::NotConstant);
        };
        if n == MAX_SEGMENTS {
            return Err(EvalError::NotConstant);
        }
        segs[n] = id;
        n += 1;
        pos += 1;
        let is_path_sep = pos + 2 < end
            && matches!(toks[pos], TokenKind::Punct(':'))
            && matches!(toks[pos + 1], TokenKind::Punct(':'));
        if !is_path_sep {
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
    match toks[pos] {
        TokenKind::Punct('(') => {
            let close = lex.matching_close[pos];
            if close + 1 != end {
                return Err(EvalError::TrailingTokens);
            }
            let mut args = [(0usize, 0usize); MAX_ARGS];
            let argc = split_args(lex, pos + 1, close, &mut args)?;
            call(lex, path, &args[..argc])
        }
        TokenKind::Punct('.') => Err(EvalError::NotConstant),
        _ => Err(EvalError::TrailingTokens),
    }
}

/// `theme.a.b`, `Theme::LIGHT`, `theme::ACCENT` (+ optional `.field` chain); no calls or indexing.
fn theme_token(
    lex: &LexResult<'_>,
    path: &[&str],
    mut pos: usize,
    end: usize,
) -> Result<TypedAttrValue, EvalError> {
    let mut token = path.join("::");
    while pos < end {
        let next = if pos + 1 < end {
            lex.tokens.get(pos + 1)
        } else {
            None
        };
        match (&lex.tokens[pos], next) {
            (TokenKind::Punct('.'), Some(TokenKind::Ident(field))) => {
                token.push('.');
                token.push_str(field);
                pos += 2;
            }
            (TokenKind::Punct('(' | '['), _) => return Err(EvalError::UnsupportedCall),
            _ => return Err(EvalError::TrailingTokens),
        }
    }
    // A bare `theme` is the whole theme struct, not a token.
    if path.len() == 1 && !token.contains('.') {
        return Err(EvalError::NotConstant);
    }
    Ok(TypedAttrValue::ThemeToken(token))
}

fn is_pascal_case(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && (s.len() == 1 || s.bytes().any(|b| b.is_ascii_lowercase()))
        && s != "Self"
        && s != "None"
}

fn path_constant(path: &[&str]) -> Result<TypedAttrValue, EvalError> {
    match path {
        [.., "Length" | "Dimension", "Auto"] => Ok(TypedAttrValue::Auto),
        [.., "Rgba" | "ColorRgba", konst] => match *konst {
            "WHITE" => Ok(rgba(255, 255, 255, 255)),
            "BLACK" => Ok(rgba(0, 0, 0, 255)),
            "TRANSPARENT" => Ok(rgba(0, 0, 0, 0)),
            _ => Err(EvalError::NotConstant),
        },
        [variant] if is_pascal_case(variant) => Ok(TypedAttrValue::Enum {
            ty: None,
            variant: (*variant).to_owned(),
        }),
        [.., ty, variant] if is_pascal_case(ty) && is_pascal_case(variant) => {
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

/// Splits `start..end` at depth-0 commas (trailing comma allowed) into `out`; returns the count.
fn split_args(
    lex: &LexResult<'_>,
    start: usize,
    end: usize,
    out: &mut [(usize, usize); MAX_ARGS],
) -> Result<usize, EvalError> {
    let mut argc = 0usize;
    let mut arg_start = start;
    let mut cur = start;
    loop {
        let at_end = cur >= end;
        if at_end || lex.tokens[cur] == TokenKind::Punct(',') {
            if cur > arg_start {
                *out.get_mut(argc).ok_or(EvalError::Arity)? = (arg_start, cur);
                argc += 1;
            } else if !at_end || (argc == 0 && start != end) {
                // Empty argument (`px(,)`, `rgb(1,,2)`); only one trailing comma is allowed.
                return Err(EvalError::Arity);
            }
            if at_end {
                return Ok(argc);
            }
            cur += 1;
            arg_start = cur;
            continue;
        }
        cur = match lex.tokens[cur] {
            TokenKind::Punct('(' | '[' | '{') => lex.matching_close[cur] + 1,
            _ => cur + 1,
        };
    }
}

fn call(
    lex: &LexResult<'_>,
    path: &[&str],
    args: &[(usize, usize)],
) -> Result<TypedAttrValue, EvalError> {
    let (ty, func) = match path {
        [func] => (None, *func),
        [ty, func] => (Some(*ty), *func),
        _ => return Err(EvalError::UnsupportedCall),
    };
    match (ty, func) {
        (None, "px")
        | (Some("Length"), "Px" | "px" | "points")
        | (Some("Dimension"), "Points" | "Px") => {
            Ok(TypedAttrValue::LengthPx(length_arg(lex, args)?))
        }
        (None, "pct" | "percent")
        | (Some("Length"), "Percent" | "percent" | "pct")
        | (Some("Dimension"), "Percent") => Ok(TypedAttrValue::LengthPct(length_arg(lex, args)?)),
        (None, "rgb") | (Some("Rgba" | "ColorRgba"), "rgb") => {
            let [r, g, b] = channels::<3>(lex, args)?;
            Ok(rgba(r, g, b, 255))
        }
        (None, "rgba") | (Some("Rgba" | "ColorRgba"), "rgba" | "new") => {
            let [r, g, b, a] = channels::<4>(lex, args)?;
            Ok(rgba(r, g, b, a))
        }
        (None, "hex") | (Some("Rgba" | "ColorRgba"), "hex" | "from_hex") => hex_color(lex, args),
        _ => Err(EvalError::UnsupportedCall),
    }
}

fn length_arg(lex: &LexResult<'_>, args: &[(usize, usize)]) -> Result<f32, EvalError> {
    let [(s, e)] = args else {
        return Err(EvalError::Arity);
    };
    match evaluate(lex, *s, *e)? {
        TypedAttrValue::Float(v) => Ok(v),
        TypedAttrValue::Int(v) => Ok(v as f32),
        _ => Err(EvalError::InvalidNumber),
    }
}

fn channels<const N: usize>(
    lex: &LexResult<'_>,
    args: &[(usize, usize)],
) -> Result<[u8; N], EvalError> {
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

fn hex_color(lex: &LexResult<'_>, args: &[(usize, usize)]) -> Result<TypedAttrValue, EvalError> {
    let [(s, e)] = args else {
        return Err(EvalError::Arity);
    };
    let [TokenKind::OtherLit(lit)] = &lex.tokens[*s..*e] else {
        return Err(EvalError::HexDigits);
    };
    let NumLit::Int {
        magnitude,
        hex_digits: Some(digits),
        ..
    } = parse_number_lit(lit)?
    else {
        return Err(EvalError::HexDigits);
    };
    let [b0, b1, b2, b3] = u32::try_from(magnitude)
        .map_err(|_| EvalError::HexDigits)?
        .to_be_bytes();
    match digits {
        6 => Ok(rgba(b1, b2, b3, 255)),
        8 => Ok(rgba(b0, b1, b2, b3)),
        _ => Err(EvalError::HexDigits),
    }
}

/// A parsed, sign-less number literal.
enum NumLit<'a> {
    Int {
        magnitude: u128,
        /// Digit count (underscores excluded) of a `0x` literal.
        hex_digits: Option<u32>,
        suffix: &'a str,
    },
    Float(f64),
}

const INT_SUFFIXES: [&str; 12] = [
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
];
/// Longest literal body accepted after removing `_` separators.
const MAX_LITERAL_DIGITS: usize = 64;

fn parse_number_lit(lit: &str) -> Result<NumLit<'_>, EvalError> {
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
    let digits = &body[..body.len() - suffix.len()];

    let mut buf = [0u8; MAX_LITERAL_DIGITS];
    let mut len = 0usize;
    for &b in digits.as_bytes().iter().filter(|&&b| b != b'_') {
        *buf.get_mut(len).ok_or(EvalError::InvalidNumber)? = b;
        len += 1;
    }
    let clean = std::str::from_utf8(&buf[..len]).map_err(|_| EvalError::InvalidNumber)?;
    if clean.is_empty() {
        return Err(EvalError::InvalidNumber);
    }

    let is_float = float_suffix || (radix == 10 && clean.contains(['.', 'e', 'E']));
    if is_float {
        if !suffix.is_empty() && !float_suffix {
            return Err(EvalError::InvalidNumber);
        }
        let value: f64 = clean.parse().map_err(|_| EvalError::InvalidNumber)?;
        return Ok(NumLit::Float(value));
    }
    if !clean.bytes().all(|b| (b as char).is_digit(radix)) {
        return Err(EvalError::InvalidNumber);
    }
    let magnitude = u128::from_str_radix(clean, radix).map_err(|_| EvalError::IntOverflow)?;
    Ok(NumLit::Int {
        magnitude,
        hex_digits: (radix == 16).then_some(len as u32),
        suffix,
    })
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
        NumLit::Float(v) => {
            let signed = if negative { -v } else { v };
            let v = signed as f32;
            if v.is_finite() {
                Ok(TypedAttrValue::Float(v))
            } else {
                Err(EvalError::NonFinite)
            }
        }
        NumLit::Int {
            magnitude, suffix, ..
        } => {
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
