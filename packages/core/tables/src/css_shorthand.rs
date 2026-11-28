//! CSS shorthand parsing: 1–4 value `padding` / `margin` box edges, `border`
//! (`<width> || <style> || <color>`), and `flex` (`none` / `auto` / `initial` /
//! `<grow> [<shrink>] [<basis>]` / `<basis>`).
//!
//! Every parser is all-or-nothing and allocation-free: one unsupported token rejects the whole
//! declaration instead of applying half of it.

use nve_abi::num::{is_valid_style_scalar, parse_f32};
use nve_store::{Length, Rgba};

use crate::css_values::{parse_color, parse_px};

/// Splits a CSS value on ASCII whitespace outside parentheses (`rgb(1, 2, 3)` stays one token).
fn css_tokens(value: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0u32;
    value
        .split(move |c: char| {
            match c {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                _ => {}
            }
            depth == 0 && c.is_ascii_whitespace()
        })
        .filter(|t| !t.is_empty())
}

/// Parses one box-edge length; negative lengths only where `allow_negative` (margins).
pub(crate) fn parse_edge(token: &str, allow_negative: bool) -> Option<f32> {
    parse_px(token).filter(|v| allow_negative || *v >= 0.0)
}

/// Parses a 1–4 value box-edge shorthand into `[top, right, bottom, left]` (CSS clockwise order).
pub(crate) fn parse_edges(value: &str, allow_negative: bool) -> Option<[f32; 4]> {
    let mut vals = [0.0f32; 4];
    let mut n = 0usize;
    for token in css_tokens(value) {
        if n == 4 {
            return None;
        }
        vals[n] = parse_edge(token, allow_negative)?;
        n += 1;
    }
    let [a, b, c, d] = vals;
    match n {
        1 => Some([a, a, a, a]),
        2 => Some([a, b, a, b]),
        3 => Some([a, b, c, b]),
        4 => Some([a, b, c, d]),
        _ => None,
    }
}

const BORDER_STYLES: [&str; 10] = [
    "none", "hidden", "solid", "dashed", "dotted", "double", "groove", "ridge", "inset", "outset",
];

/// A border color: concrete, or the symbolic `currentColor` (`None`), resolved against the node's
/// effective text color after the whole declaration list applied.
pub(crate) type BorderColor = Option<Rgba>;

/// Parses a `border-color` value: `currentColor` (`Some(None)`) or a concrete color.
pub(crate) fn parse_border_color(token: &str) -> Option<BorderColor> {
    if token.trim().eq_ignore_ascii_case("currentcolor") {
        Some(None)
    } else {
        parse_color(token).map(|c| Some(Rgba(c.to_u32())))
    }
}

/// Parses a border width token (`thin`, `medium`, `thick`, `0`, or non-negative `px`); unitless
/// non-zero widths are rejected, like `flex-basis`.
fn parse_border_width(token: &str) -> Option<f32> {
    if token.eq_ignore_ascii_case("thin") {
        Some(1.0)
    } else if token.eq_ignore_ascii_case("medium") {
        Some(3.0)
    } else if token.eq_ignore_ascii_case("thick") {
        Some(5.0)
    } else if let Some(n) = parse_f32(token) {
        (n == 0.0).then_some(0.0)
    } else {
        parse_px(token).filter(|w| *w >= 0.0)
    }
}

/// Parses a standalone `border-width` with the same grammar as the `border` shorthand.
pub(crate) fn parse_border_width_value(value: &str) -> Option<f32> {
    let t = value.trim();
    if t.is_empty() {
        return None;
    }
    parse_border_width(t)
}

/// Parses `border: <width> || <style> || <color>` into `(width, color)`. An omitted width is
/// `medium` (3px); an omitted, `none`, or `hidden` style computes the width to `0`; an omitted or
/// `currentColor` color is symbolic (`None`).
pub(crate) fn parse_border(value: &str) -> Option<(f32, BorderColor)> {
    let (mut width, mut style_visible, mut color) = (None, None, None);
    let mut any = false;
    for token in css_tokens(value) {
        any = true;
        if width.is_none()
            && let Some(w) = parse_border_width(token)
        {
            width = Some(w);
        } else if style_visible.is_none()
            && BORDER_STYLES.iter().any(|s| token.eq_ignore_ascii_case(s))
        {
            style_visible =
                Some(!token.eq_ignore_ascii_case("none") && !token.eq_ignore_ascii_case("hidden"));
        } else if color.is_none()
            && let Some(c) = parse_border_color(token)
        {
            color = Some(c);
        } else {
            return None;
        }
    }
    if !any {
        return None;
    }
    let width = match style_visible {
        Some(true) => width.unwrap_or(3.0),
        Some(false) | None => 0.0,
    };
    Some((width, color.flatten()))
}

/// Parsed CSS `flex` shorthand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FlexShorthand {
    pub(crate) grow: f32,
    pub(crate) shrink: f32,
    pub(crate) basis: Length,
}

/// Parses the CSS `flex` shorthand (`[<grow> <shrink>?] || <basis>`, factors adjacent, either side
/// of the basis) per the spec's omitted-value rules: omitted shrink is `1`; omitted basis is `0%`
/// when a grow factor is given; `<basis>` alone means `1 1 <basis>`. A third unitless number is a
/// basis only when it is `0` (`flex: 1 1 0`).
pub(crate) fn parse_flex(value: &str) -> Option<FlexShorthand> {
    let v = value.trim();
    let keyword = |grow, shrink| Some(FlexShorthand { grow, shrink, basis: Length::AUTO });
    if v.eq_ignore_ascii_case("none") {
        return keyword(0.0, 0.0);
    } else if v.eq_ignore_ascii_case("auto") {
        return keyword(1.0, 1.0);
    } else if v.eq_ignore_ascii_case("initial") {
        return keyword(0.0, 1.0);
    }
    let mut factors = [0.0f32; 2];
    let mut n_factors = 0usize;
    let mut prev_was_factor = false;
    let mut basis: Option<Length> = None;
    for token in css_tokens(v) {
        // A unitless number is a flex factor (`0` included); grow and shrink must be adjacent.
        if let Some(n) = parse_f32(token) {
            if n_factors == 2 && basis.is_none() && n == 0.0 {
                basis = Some(Length::points(0.0));
                prev_was_factor = false;
                continue;
            }
            if !is_valid_style_scalar(n)
                || n < 0.0
                || n_factors == 2
                || (n_factors == 1 && !prev_was_factor)
            {
                return None;
            }
            factors[n_factors] = n;
            n_factors += 1;
            prev_was_factor = true;
        } else if basis.is_none()
            && let Some(b) = parse_flex_basis(token)
        {
            basis = Some(b);
            prev_was_factor = false;
        } else {
            return None;
        }
    }
    match (n_factors, basis) {
        (0, None) => None,
        (0, Some(basis)) => Some(FlexShorthand { grow: 1.0, shrink: 1.0, basis }),
        (n, basis) => Some(FlexShorthand {
            grow: factors[0],
            shrink: if n == 2 { factors[1] } else { 1.0 },
            basis: basis.unwrap_or(Length::percent(0.0)),
        }),
    }
}

/// Parses a CSS `flex-basis`: `auto`, a non-negative `px` length (unitless only for `0`), or a
/// non-negative percentage. `content` and other units are unsupported (`None`).
pub(crate) fn parse_flex_basis(token: &str) -> Option<Length> {
    let t = token.trim();
    if t.eq_ignore_ascii_case("auto") {
        return Some(Length::AUTO);
    }
    if let Some(pct) = t.strip_suffix('%') {
        return parse_f32(pct)
            .filter(|&v| is_valid_style_scalar(v) && v >= 0.0)
            .map(Length::percent);
    }
    if let Some(n) = parse_f32(t) {
        return (n == 0.0).then_some(Length::points(0.0));
    }
    parse_px(t).filter(|v| *v >= 0.0).map(Length::points)
}
