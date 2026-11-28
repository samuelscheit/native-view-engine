//! Primitive CSS value parsers, all zero-allocation and range-sealed at ingress: `px` lengths and
//! [`Dimension`]s within [`MAX_STYLE_SCALAR`](nve_abi::num::MAX_STYLE_SCALAR), hex (`#rgb`,
//! `#rgba`, `#rrggbb`, `#rrggbbaa`), strict CSS Color 4 `rgb()` / `rgba()`, named colors, font
//! weights, and ASCII case-insensitive keywords.

use nve_abi::num::{is_valid_style_scalar, parse_f32};
use nve_abi::{ColorRgba, Dimension, FlexAlign, FlexDirection, FlexJustify};
use nve_store::Length;

/// Parses a CSS pixel length (`12px`, `12PX`) or a unitless RN-style number (`12`) within
/// `[-MAX_STYLE_SCALAR, MAX_STYLE_SCALAR]`.
pub fn parse_px(raw: &str) -> Option<f32> {
    let s = raw.trim();
    let num = match s.len().checked_sub(2) {
        Some(at) if s.is_char_boundary(at) && s[at..].eq_ignore_ascii_case("px") => &s[..at],
        _ => s,
    };
    parse_f32(num).filter(|&v| is_valid_style_scalar(v))
}

/// Strict CSS box dimension: `auto`, a non-negative `<f32>%`, or a non-negative `px` length, all
/// within `MAX_STYLE_SCALAR`; `None` on invalid, negative, or out-of-range text.
pub fn try_parse_dim(raw: &str) -> Option<Dimension> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("auto") {
        Some(Dimension::Auto)
    } else if let Some(pct) = s.strip_suffix('%') {
        parse_f32(pct.trim())
            .filter(|&v| is_valid_style_scalar(v) && v >= 0.0)
            .map(Dimension::Percent)
    } else {
        parse_px(s).filter(|&v| v >= 0.0).map(Dimension::Points)
    }
}

/// [`try_parse_dim`], defaulting to `Auto`.
pub fn parse_dim(raw: &str) -> Dimension {
    try_parse_dim(raw).unwrap_or(Dimension::Auto)
}

/// Converts a [`Dimension`] into a word-table [`Length`].
#[inline]
pub const fn dim_to_length(d: Dimension) -> Length {
    match d {
        Dimension::Auto => Length::AUTO,
        Dimension::Points(v) => Length::points(v),
        Dimension::Percent(v) => Length::percent(v),
    }
}

/// Looks `raw` (trimmed, ASCII case-insensitive) up in a keyword table.
#[inline]
pub(crate) fn keyword<T: Copy>(raw: &str, table: &[(&str, T)]) -> Option<T> {
    let v = raw.trim();
    table
        .iter()
        .find_map(|&(name, value)| v.eq_ignore_ascii_case(name).then_some(value))
}

const FLEX_DIRECTIONS: &[(&str, FlexDirection)] = &[
    ("column", FlexDirection::Column),
    ("row", FlexDirection::Row),
    ("column-reverse", FlexDirection::ColumnReverse),
    ("columnReverse", FlexDirection::ColumnReverse),
    ("row-reverse", FlexDirection::RowReverse),
    ("rowReverse", FlexDirection::RowReverse),
];

const FLEX_JUSTIFY: &[(&str, FlexJustify)] = &[
    ("flex-start", FlexJustify::FlexStart),
    ("start", FlexJustify::FlexStart),
    ("flexStart", FlexJustify::FlexStart),
    ("center", FlexJustify::Center),
    ("flex-end", FlexJustify::FlexEnd),
    ("end", FlexJustify::FlexEnd),
    ("flexEnd", FlexJustify::FlexEnd),
    ("space-between", FlexJustify::SpaceBetween),
    ("spaceBetween", FlexJustify::SpaceBetween),
    ("space-around", FlexJustify::SpaceAround),
    ("spaceAround", FlexJustify::SpaceAround),
    ("space-evenly", FlexJustify::SpaceEvenly),
    ("spaceEvenly", FlexJustify::SpaceEvenly),
];

const FLEX_ALIGN: &[(&str, FlexAlign)] = &[
    ("stretch", FlexAlign::Stretch),
    ("flex-start", FlexAlign::FlexStart),
    ("start", FlexAlign::FlexStart),
    ("flexStart", FlexAlign::FlexStart),
    ("center", FlexAlign::Center),
    ("flex-end", FlexAlign::FlexEnd),
    ("end", FlexAlign::FlexEnd),
    ("flexEnd", FlexAlign::FlexEnd),
    ("baseline", FlexAlign::Baseline),
];

/// Parses a CSS/RN `flex-direction` keyword (ASCII case-insensitive, zero heap allocations).
pub fn parse_flex_direction(raw: &str) -> Option<FlexDirection> {
    keyword(raw, FLEX_DIRECTIONS)
}

/// Parses a CSS/RN `justify-content` keyword (ASCII case-insensitive, zero heap allocations).
pub fn parse_flex_justify(raw: &str) -> Option<FlexJustify> {
    keyword(raw, FLEX_JUSTIFY)
}

/// Parses a CSS/RN `align-items` keyword (ASCII case-insensitive, zero heap allocations).
pub fn parse_flex_align(raw: &str) -> Option<FlexAlign> {
    keyword(raw, FLEX_ALIGN)
}

const NAMED_COLORS: &[(&str, ColorRgba)] = &[
    ("white", ColorRgba::WHITE),
    ("black", ColorRgba::BLACK),
    ("transparent", ColorRgba::TRANSPARENT),
    ("red", ColorRgba::rgb(239, 68, 68)),
    ("green", ColorRgba::rgb(34, 197, 94)),
    ("blue", ColorRgba::rgb(59, 130, 246)),
];

/// Parses `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, strict `rgb()` / `rgba()`, or a basic named
/// color (zero heap allocations).
pub fn parse_color(raw: &str) -> Option<ColorRgba> {
    let s = raw.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(args) = strip_ascii_prefix(s, "rgba(").or_else(|| strip_ascii_prefix(s, "rgb(")) {
        return parse_rgb_args(args.strip_suffix(')')?);
    }
    keyword(s, NAMED_COLORS)
}

fn parse_hex(hex: &str) -> Option<ColorRgba> {
    // Rejects non-ASCII (no char-boundary panics) and signs (`#+f+f+f`), which
    // `from_str_radix` would otherwise accept.
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok().map(|n| n * 17);
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        3 => Some(ColorRgba::rgb(nibble(0)?, nibble(1)?, nibble(2)?)),
        4 => Some(ColorRgba::rgba(nibble(0)?, nibble(1)?, nibble(2)?, nibble(3)?)),
        6 => Some(ColorRgba::rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(ColorRgba::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

fn strip_ascii_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// One `rgb()` channel and whether it was a percentage.
fn parse_rgb_channel(token: &str) -> Option<(u8, bool)> {
    let t = token.trim();
    if let Some(pct) = t.strip_suffix('%') {
        let v = parse_f32(pct)?;
        Some((((v / 100.0).clamp(0.0, 1.0) * 255.0).round() as u8, true))
    } else {
        let v = parse_f32(t)?;
        Some((v.clamp(0.0, 255.0).round() as u8, false))
    }
}

fn parse_alpha_channel(token: &str) -> Option<u8> {
    let t = token.trim();
    let alpha = match t.strip_suffix('%') {
        Some(pct) => parse_f32(pct)? / 100.0,
        None => parse_f32(t)?,
    };
    Some((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Parses strict CSS Color 4 `rgb()` / `rgba()` arguments:
/// - Legacy comma grammar: `r, g, b` or `r, g, b, a` (RGB all numeric or all `%`, no `/`).
/// - Modern space grammar: `r g b` or `r g b / a` (numeric or `%` channels, at most one `/`).
fn parse_rgb_args(args: &str) -> Option<ColorRgba> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains(',') {
        if trimmed.contains('/') {
            return None;
        }
        let mut iter = trimmed.split(',');
        let (r, r_pct) = parse_rgb_channel(iter.next()?)?;
        let (g, g_pct) = parse_rgb_channel(iter.next()?)?;
        let (b, b_pct) = parse_rgb_channel(iter.next()?)?;
        if r_pct != g_pct || g_pct != b_pct {
            return None;
        }
        let a = match iter.next() {
            None => 255,
            Some(raw) => parse_alpha_channel(raw)?,
        };
        if iter.next().is_some() {
            return None;
        }
        return Some(ColorRgba::rgba(r, g, b, a));
    }
    let (rgb_part, alpha_part) = match trimmed.split_once('/') {
        Some((_, rhs)) if rhs.contains('/') => return None,
        Some((lhs, rhs)) => (lhs, Some(rhs)),
        None => (trimmed, None),
    };
    let mut rgb_iter = rgb_part.split_ascii_whitespace();
    let (r, _) = parse_rgb_channel(rgb_iter.next()?)?;
    let (g, _) = parse_rgb_channel(rgb_iter.next()?)?;
    let (b, _) = parse_rgb_channel(rgb_iter.next()?)?;
    if rgb_iter.next().is_some() {
        return None;
    }
    let a = match alpha_part {
        None => 255,
        Some(raw) => {
            let mut a_iter = raw.split_ascii_whitespace();
            let tok = a_iter.next()?;
            if a_iter.next().is_some() {
                return None;
            }
            parse_alpha_channel(tok)?
        }
    };
    Some(ColorRgba::rgba(r, g, b, a))
}

const FONT_WEIGHTS: &[(&str, u16)] = &[
    ("thin", 100),
    ("light", 300),
    ("normal", 400),
    ("regular", 400),
    ("medium", 500),
    ("semibold", 600),
    ("bold", 700),
    ("extrabold", 800),
    ("black", 900),
];

/// Parses a CSS font-weight keyword or numeric value (`100..=900`), returning `None` if invalid.
pub fn try_parse_font_weight(raw: &str) -> Option<u16> {
    keyword(raw, FONT_WEIGHTS).or_else(|| raw.trim().parse::<u16>().ok().map(|w| w.clamp(100, 900)))
}

/// Parses a CSS font-weight keyword or numeric value (`100..=900`), defaulting to `400`.
pub fn parse_font_weight(raw: &str) -> u16 {
    try_parse_font_weight(raw).unwrap_or(400)
}
