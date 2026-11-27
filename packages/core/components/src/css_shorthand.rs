//! CSS shorthand parsing for inline `style` declarations: 1–4 value `padding` / `margin` box
//! edges, `border` (`<width> <style> <color>` in any order), and `flex`
//! (`none` / `auto` / `<grow> [<shrink>] [<basis>]` / `<basis>`).
//!
//! Every parser is all-or-nothing: one unsupported token leaves the target untouched instead of
//! applying half a declaration.

use nve_abi::num::parse_f32;
use nve_abi::{ColorRgba, EdgeInsets};

use crate::{parse_color, parse_px};

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

/// Parses a 1–4 value box-edge shorthand (`top [right [bottom [left]]]`, CSS clockwise order).
pub(crate) fn parse_edges(value: &str) -> Option<EdgeInsets> {
    let mut vals = [0.0f32; 4];
    let mut n = 0usize;
    for token in css_tokens(value) {
        if n == 4 {
            return None;
        }
        vals[n] = parse_px(token)?;
        n += 1;
    }
    let [a, b, c, d] = vals;
    let (top, right, bottom, left) = match n {
        1 => (a, a, a, a),
        2 => (a, b, a, b),
        3 => (a, b, c, b),
        4 => (a, b, c, d),
        _ => return None,
    };
    Some(EdgeInsets {
        top,
        right,
        bottom,
        left,
    })
}

/// Applies a single-edge longhand (`padding-top`, `marginLeft`, `margin-horizontal`, ...) to
/// `edges`. Returns `false` when `suffix` names no edge (or the value is not a length).
pub(crate) fn apply_edge_longhand(edges: &mut EdgeInsets, suffix: &str, value: &str) -> bool {
    let Some(v) = parse_px(value) else {
        // A recognized edge with an unparsable value is still handled (left untouched).
        return edge_targets(suffix).is_some();
    };
    let Some([top, right, bottom, left]) = edge_targets(suffix) else {
        return false;
    };
    for (on, slot) in [
        (top, &mut edges.top),
        (right, &mut edges.right),
        (bottom, &mut edges.bottom),
        (left, &mut edges.left),
    ] {
        if on {
            *slot = v;
        }
    }
    true
}

/// `[top, right, bottom, left]` edges addressed by a kebab-case or camelCase longhand suffix.
fn edge_targets(suffix: &str) -> Option<[bool; 4]> {
    Some(match suffix {
        "-top" | "Top" => [true, false, false, false],
        "-right" | "Right" => [false, true, false, false],
        "-bottom" | "Bottom" => [false, false, true, false],
        "-left" | "Left" => [false, false, false, true],
        "-horizontal" | "Horizontal" => [false, true, false, true],
        "-vertical" | "Vertical" => [true, false, true, false],
        _ => return None,
    })
}

const BORDER_STYLES: [&str; 10] = [
    "none", "hidden", "solid", "dashed", "dotted", "double", "groove", "ridge", "inset", "outset",
];

/// Parses `border: <width> || <style> || <color>` into `(width, color)` with CSS omitted-value
/// rules: omitted color is `currentcolor` (`current_color`), omitted width is `medium` (3px),
/// and an omitted, `none` or `hidden` style computes the width to 0.
pub(crate) fn parse_border(value: &str, current_color: ColorRgba) -> Option<(f32, ColorRgba)> {
    let (mut width, mut style, mut color) = (None, None, None);
    let mut any = false;
    for token in css_tokens(value) {
        any = true;
        let lower = token.to_ascii_lowercase();
        if width.is_none()
            && let Some(w) = match lower.as_str() {
                "thin" => Some(1.0),
                "medium" => Some(3.0),
                "thick" => Some(5.0),
                _ => parse_px(token).filter(|w| *w >= 0.0),
            }
        {
            width = Some(w);
        } else if style.is_none() && BORDER_STYLES.contains(&lower.as_str()) {
            style = Some(lower);
        } else if color.is_none()
            && let Some(c) = parse_color(token)
        {
            color = Some(c);
        } else {
            return None;
        }
    }
    if !any {
        return None;
    }
    let width = match style.as_deref() {
        None | Some("none" | "hidden") => 0.0,
        Some(_) => width.unwrap_or(3.0),
    };
    Some((width, color.unwrap_or(current_color)))
}

/// Parsed CSS `flex` shorthand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FlexShorthand<'a> {
    pub grow: f32,
    pub shrink: f32,
    /// CSS `flex-basis` value (`auto`, `0%`, `280px`, ...).
    pub basis: &'a str,
}

/// Parses the CSS `flex` shorthand (`[<grow> <shrink>?] || <basis>`, factors adjacent, either
/// side of the basis) per the spec's omitted-value rules: omitted shrink is `1`; omitted basis is
/// `0%` when a grow factor is given; `<basis>` alone means `1 1 <basis>`.
pub(crate) fn parse_flex(value: &str) -> Option<FlexShorthand<'_>> {
    let v = value.trim();
    match v.to_ascii_lowercase().as_str() {
        "none" => {
            return Some(FlexShorthand {
                grow: 0.0,
                shrink: 0.0,
                basis: "auto",
            })
        }
        "auto" => {
            return Some(FlexShorthand {
                grow: 1.0,
                shrink: 1.0,
                basis: "auto",
            })
        }
        "initial" => {
            return Some(FlexShorthand {
                grow: 0.0,
                shrink: 1.0,
                basis: "auto",
            })
        }
        _ => {}
    }
    let mut factors = [0.0f32; 2];
    let mut n_factors = 0usize;
    let mut prev_was_factor = false;
    let mut basis: Option<&str> = None;
    for token in css_tokens(v) {
        // A unitless number is a flex factor (`0` included); grow and shrink must be adjacent.
        if let Some(n) = parse_f32(token) {
            if n < 0.0 || n_factors == 2 || (n_factors == 1 && !prev_was_factor) {
                return None;
            }
            factors[n_factors] = n;
            n_factors += 1;
            prev_was_factor = true;
        } else if basis.is_none() && is_flex_basis(token) {
            basis = Some(token);
            prev_was_factor = false;
        } else {
            return None;
        }
    }
    match (n_factors, basis) {
        (0, None) => None,
        (0, Some(basis)) => Some(FlexShorthand {
            grow: 1.0,
            shrink: 1.0,
            basis,
        }),
        (n, basis) => Some(FlexShorthand {
            grow: factors[0],
            shrink: if n == 2 { factors[1] } else { 1.0 },
            basis: basis.unwrap_or("0%"),
        }),
    }
}

/// `auto`, `content`, a length (`280px`, `0`), or a percentage.
fn is_flex_basis(token: &str) -> bool {
    token.eq_ignore_ascii_case("auto")
        || token.eq_ignore_ascii_case("content")
        || parse_px(token).is_some_and(|v| v >= 0.0)
        || token
            .strip_suffix('%')
            .and_then(parse_f32)
            .is_some_and(|v| v >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(t: f32, r: f32, b: f32, l: f32) -> EdgeInsets {
        EdgeInsets {
            top: t,
            right: r,
            bottom: b,
            left: l,
        }
    }

    #[test]
    fn box_edges_follow_css_one_to_four_value_rules() {
        assert_eq!(parse_edges("12px"), Some(EdgeInsets::all(12.0)));
        assert_eq!(parse_edges("12px 14px"), Some(edges(12.0, 14.0, 12.0, 14.0)));
        assert_eq!(parse_edges("12px 14px 16px"), Some(edges(12.0, 14.0, 16.0, 14.0)));
        assert_eq!(
            parse_edges(" 12px  14px 16px 18 "),
            Some(edges(12.0, 14.0, 16.0, 18.0))
        );
        for bad in ["", "   ", "1px 2px 3px 4px 5px", "0 auto", "1em", "12px x"] {
            assert_eq!(parse_edges(bad), None, "{bad:?}");
        }

        let mut e = EdgeInsets::all(1.0);
        assert!(apply_edge_longhand(&mut e, "-top", "5px"));
        assert!(apply_edge_longhand(&mut e, "Left", "7"));
        assert!(apply_edge_longhand(&mut e, "-horizontal", "9px"));
        assert!(apply_edge_longhand(&mut e, "Vertical", "2px"));
        assert_eq!(e, edges(2.0, 9.0, 2.0, 9.0));
        assert!(apply_edge_longhand(&mut e, "-bottom", "auto"), "handled, untouched");
        assert_eq!(e.bottom, 2.0);
        assert!(!apply_edge_longhand(&mut e, "-start", "3px"));
    }

    #[test]
    fn border_shorthand_accepts_any_token_order() {
        let slate = ColorRgba::rgb(0x1e, 0x29, 0x3b);
        let cur = ColorRgba::rgb(1, 2, 3);
        assert_eq!(parse_border("1px solid #1e293b", cur), Some((1.0, slate)));
        assert_eq!(parse_border("#1e293b solid 2px", cur), Some((2.0, slate)));
        assert_eq!(parse_border("solid 1px", cur), Some((1.0, cur)), "currentcolor");
        assert_eq!(parse_border("solid", cur), Some((3.0, cur)), "medium width");
        assert_eq!(parse_border("thick dashed red", cur), Some((5.0, ColorRgba::rgb(239, 68, 68))));
        assert_eq!(parse_border("none", cur), Some((0.0, cur)));
        assert_eq!(parse_border("4px none #1e293b", cur), Some((0.0, slate)));
        assert_eq!(parse_border("0", cur), Some((0.0, cur)));
        assert_eq!(parse_border("2px #1e293b", cur), Some((0.0, slate)), "omitted style is none");
        for bad in ["", "1px 2px", "solid solid", "1px wavy red", "-1px solid", "1px solid #12"] {
            assert_eq!(parse_border(bad, cur), None, "{bad:?}");
        }
    }

    #[test]
    fn flex_shorthand_follows_css_omitted_value_rules() {
        let f = |g, s, b| Some(FlexShorthand { grow: g, shrink: s, basis: b });
        assert_eq!(parse_flex("none"), f(0.0, 0.0, "auto"));
        assert_eq!(parse_flex("AUTO"), f(1.0, 1.0, "auto"));
        assert_eq!(parse_flex("initial"), f(0.0, 1.0, "auto"));
        assert_eq!(parse_flex("1"), f(1.0, 1.0, "0%"));
        assert_eq!(parse_flex("2 3"), f(2.0, 3.0, "0%"));
        assert_eq!(parse_flex("2 280px"), f(2.0, 1.0, "280px"));
        assert_eq!(parse_flex("1 1 280px"), f(1.0, 1.0, "280px"));
        assert_eq!(parse_flex("1 1 110px"), f(1.0, 1.0, "110px"));
        assert_eq!(parse_flex("0 0 auto"), f(0.0, 0.0, "auto"));
        assert_eq!(parse_flex("1 0 50%"), f(1.0, 0.0, "50%"));
        assert_eq!(parse_flex("100px"), f(1.0, 1.0, "100px"));
        assert_eq!(parse_flex("2.5"), f(2.5, 1.0, "0%"));
        assert_eq!(parse_flex("0"), f(0.0, 1.0, "0%"));
        assert_eq!(parse_flex("280px 2 3"), f(2.0, 3.0, "280px"), "basis first");
        for bad in ["", "1 2 3", "1 2 3px 4", "-1", "1 -2", "1 280px 1", "1px 2px", "x", "1 1 fit"] {
            assert_eq!(parse_flex(bad), None, "{bad:?}");
        }
    }
}
