//! Primitive CSS value parsers: `px` lengths, `Dimension`, hex (`#rgb`, `#rgba`, `#rrggbb`,
//! `#rrggbbaa`), strict CSS Color 4 `rgb()`/`rgba()`, and named colors.

use nve_abi::num::parse_f32;
use nve_abi::{ColorRgba, Dimension, FlexAlign, FlexDirection, FlexJustify};

/// Parses a CSS pixel length (`12px`, `12PX`) or a unitless RN-style number (`12`).
pub fn parse_px(raw: &str) -> Option<f32> {
    let s = raw.trim();
    let num = match s.len().checked_sub(2) {
        Some(at) if s.is_char_boundary(at) && s[at..].eq_ignore_ascii_case("px") => &s[..at],
        _ => s,
    };
    parse_f32(num)
}

/// Strict CSS dimension parser: `auto`, `<f32>%`, or `<px>` length; returns `None` on invalid tokens.
pub(crate) fn parse_dim_checked(raw: &str) -> Option<Dimension> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if s.eq_ignore_ascii_case("auto") {
        Some(Dimension::Auto)
    } else if let Some(pct) = s.strip_suffix('%') {
        parse_f32(pct.trim()).map(Dimension::Percent)
    } else {
        parse_px(s).map(Dimension::Points)
    }
}

pub fn parse_dim(raw: &str) -> Dimension {
    parse_dim_checked(raw).unwrap_or(Dimension::Auto)
}

pub fn parse_color(raw: &str) -> Option<ColorRgba> {
    let s = raw.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                Some(ColorRgba::rgb(r, g, b))
            }
            4 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                let a = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
                Some(ColorRgba::rgba(r, g, b, a))
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(ColorRgba::rgb(r, g, b))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(ColorRgba::rgba(r, g, b, a))
            }
            _ => None,
        }
    } else if let Some(args) =
        strip_ascii_prefix(s, "rgba(").or_else(|| strip_ascii_prefix(s, "rgb("))
    {
        parse_rgb_args(args.strip_suffix(')')?)
    } else if s.eq_ignore_ascii_case("white") {
        Some(ColorRgba::WHITE)
    } else if s.eq_ignore_ascii_case("black") {
        Some(ColorRgba::BLACK)
    } else if s.eq_ignore_ascii_case("transparent") {
        Some(ColorRgba::TRANSPARENT)
    } else if s.eq_ignore_ascii_case("red") {
        Some(ColorRgba::rgb(239, 68, 68))
    } else if s.eq_ignore_ascii_case("green") {
        Some(ColorRgba::rgb(34, 197, 94))
    } else if s.eq_ignore_ascii_case("blue") {
        Some(ColorRgba::rgb(59, 130, 246))
    } else {
        None
    }
}

/// Parses a CSS/RN `flex-direction` keyword (ASCII case-insensitive, zero heap allocations).
pub fn parse_flex_direction(raw: &str) -> Option<FlexDirection> {
    let v = raw.trim();
    if v.eq_ignore_ascii_case("column") {
        Some(FlexDirection::Column)
    } else if v.eq_ignore_ascii_case("row") {
        Some(FlexDirection::Row)
    } else if v.eq_ignore_ascii_case("column-reverse") || v.eq_ignore_ascii_case("columnReverse") {
        Some(FlexDirection::ColumnReverse)
    } else if v.eq_ignore_ascii_case("row-reverse") || v.eq_ignore_ascii_case("rowReverse") {
        Some(FlexDirection::RowReverse)
    } else {
        None
    }
}

/// Parses a CSS/RN `justify-content` keyword (ASCII case-insensitive, zero heap allocations).
pub fn parse_flex_justify(raw: &str) -> Option<FlexJustify> {
    let v = raw.trim();
    if v.eq_ignore_ascii_case("flex-start")
        || v.eq_ignore_ascii_case("start")
        || v.eq_ignore_ascii_case("flexStart")
    {
        Some(FlexJustify::FlexStart)
    } else if v.eq_ignore_ascii_case("center") {
        Some(FlexJustify::Center)
    } else if v.eq_ignore_ascii_case("flex-end")
        || v.eq_ignore_ascii_case("end")
        || v.eq_ignore_ascii_case("flexEnd")
    {
        Some(FlexJustify::FlexEnd)
    } else if v.eq_ignore_ascii_case("space-between") || v.eq_ignore_ascii_case("spaceBetween") {
        Some(FlexJustify::SpaceBetween)
    } else if v.eq_ignore_ascii_case("space-around") || v.eq_ignore_ascii_case("spaceAround") {
        Some(FlexJustify::SpaceAround)
    } else if v.eq_ignore_ascii_case("space-evenly") || v.eq_ignore_ascii_case("spaceEvenly") {
        Some(FlexJustify::SpaceEvenly)
    } else {
        None
    }
}

/// Parses a CSS/RN `align-items` keyword (ASCII case-insensitive, zero heap allocations).
pub fn parse_flex_align(raw: &str) -> Option<FlexAlign> {
    let v = raw.trim();
    if v.eq_ignore_ascii_case("stretch") {
        Some(FlexAlign::Stretch)
    } else if v.eq_ignore_ascii_case("flex-start")
        || v.eq_ignore_ascii_case("start")
        || v.eq_ignore_ascii_case("flexStart")
    {
        Some(FlexAlign::FlexStart)
    } else if v.eq_ignore_ascii_case("center") {
        Some(FlexAlign::Center)
    } else if v.eq_ignore_ascii_case("flex-end")
        || v.eq_ignore_ascii_case("end")
        || v.eq_ignore_ascii_case("flexEnd")
    {
        Some(FlexAlign::FlexEnd)
    } else if v.eq_ignore_ascii_case("baseline") {
        Some(FlexAlign::Baseline)
    } else {
        None
    }
}

fn strip_ascii_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

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
/// - Legacy comma grammar: `r, g, b` or `r, g, b, a` (all RGB channels numeric or all `%`, no `/`).
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
        Some((lhs, rhs)) => {
            if rhs.contains('/') {
                return None;
            }
            (lhs, Some(rhs))
        }
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
