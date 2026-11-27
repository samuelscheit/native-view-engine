//! CSS shorthand parsing for inline `style` declarations: 1–4 value `padding` / `margin` box
//! edges, `border` (`<width> <style> <color>` in any order), and `flex`
//! (`none` / `auto` / `<grow> [<shrink>] [<basis>]` / `<basis>`).
//!
//! Every parser is all-or-nothing: one unsupported token leaves the target untouched instead of
//! applying half a declaration.

use nve_abi::num::parse_f32;
use nve_abi::{ColorRgba, Dimension, EdgeInsets};

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

/// Parses one box-edge length; negative lengths only where `allow_negative` (margins).
fn parse_edge(token: &str, allow_negative: bool) -> Option<f32> {
    parse_px(token).filter(|v| allow_negative || *v >= 0.0)
}

/// Parses a 1–4 value box-edge shorthand (`top [right [bottom [left]]]`, CSS clockwise order).
pub(crate) fn parse_edges(value: &str, allow_negative: bool) -> Option<EdgeInsets> {
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
/// `edges`. Returns `false` when `suffix` names no edge; an invalid length (or a negative one
/// unless `allow_negative`) leaves `edges` untouched.
pub(crate) fn apply_edge_longhand(
    edges: &mut EdgeInsets,
    suffix: &str,
    value: &str,
    allow_negative: bool,
) -> bool {
    let Some(v) = parse_edge(value, allow_negative) else {
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

#[derive(Clone, Copy)]
enum BorderColorToken {
    Current,
    Concrete(ColorRgba),
}

/// Parses a CSS border width token (`thin`, `medium`, `thick`, `0`, or non-negative `px`;
/// unitless non-zero widths are rejected for consistency with `flex-basis`).
fn parse_border_width(token: &str, lower: &str) -> Option<f32> {
    match lower {
        "thin" => Some(1.0),
        "medium" => Some(3.0),
        "thick" => Some(5.0),
        _ => {
            if let Some(n) = parse_f32(token) {
                (n == 0.0).then_some(0.0)
            } else {
                parse_px(token).filter(|w| *w >= 0.0)
            }
        }
    }
}

/// Parses `border: <width> || <style> || <color>` into `(width, color)` where `color == None`
/// denotes `currentColor` (either omitted or explicit) so callers can resolve it after the full
/// declaration list has updated `text_color`. Omitted width is `medium` (3px), and an omitted,
/// `none`, or `hidden` style computes width to `0`.
pub(crate) fn parse_border(value: &str) -> Option<(f32, Option<ColorRgba>)> {
    let (mut width, mut style, mut color) = (None, None, None);
    let mut any = false;
    for token in css_tokens(value) {
        any = true;
        let lower = token.to_ascii_lowercase();
        if width.is_none()
            && let Some(w) = parse_border_width(token, &lower)
        {
            width = Some(w);
        } else if style.is_none() && BORDER_STYLES.contains(&lower.as_str()) {
            style = Some(lower);
        } else if color.is_none()
            && let Some(c) = if lower == "currentcolor" {
                Some(BorderColorToken::Current)
            } else {
                parse_color(token).map(BorderColorToken::Concrete)
            }
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
    let resolved_color = match color {
        Some(BorderColorToken::Concrete(c)) => Some(c),
        Some(BorderColorToken::Current) | None => None,
    };
    Some((width, resolved_color))
}

/// Parsed CSS `flex` shorthand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FlexShorthand {
    pub grow: f32,
    pub shrink: f32,
    pub basis: Dimension,
}

impl FlexShorthand {
    const fn new(grow: f32, shrink: f32, basis: Dimension) -> Self {
        Self {
            grow,
            shrink,
            basis,
        }
    }
}

/// Parses the CSS `flex` shorthand (`[<grow> <shrink>?] || <basis>`, factors adjacent, either
/// side of the basis) per the spec's omitted-value rules: omitted shrink is `1`; omitted basis is
/// `0%` when a grow factor is given; `<basis>` alone means `1 1 <basis>`. A third unitless number
/// is a basis only when it is `0` (`flex: 1 1 0`).
pub(crate) fn parse_flex(value: &str) -> Option<FlexShorthand> {
    let v = value.trim();
    match v.to_ascii_lowercase().as_str() {
        "none" => return Some(FlexShorthand::new(0.0, 0.0, Dimension::Auto)),
        "auto" => return Some(FlexShorthand::new(1.0, 1.0, Dimension::Auto)),
        "initial" => return Some(FlexShorthand::new(0.0, 1.0, Dimension::Auto)),
        _ => {}
    }
    let mut factors = [0.0f32; 2];
    let mut n_factors = 0usize;
    let mut prev_was_factor = false;
    let mut basis: Option<Dimension> = None;
    for token in css_tokens(v) {
        // A unitless number is a flex factor (`0` included); grow and shrink must be adjacent.
        if let Some(n) = parse_f32(token) {
            if n_factors == 2 && basis.is_none() && n == 0.0 {
                basis = Some(Dimension::Points(0.0));
                prev_was_factor = false;
                continue;
            }
            if n < 0.0 || n_factors == 2 || (n_factors == 1 && !prev_was_factor) {
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
        (0, Some(basis)) => Some(FlexShorthand::new(1.0, 1.0, basis)),
        (n, basis) => Some(FlexShorthand::new(
            factors[0],
            if n == 2 { factors[1] } else { 1.0 },
            basis.unwrap_or(Dimension::Percent(0.0)),
        )),
    }
}

/// Parses a CSS `flex-basis`: `auto`, a non-negative `px` length (unitless only for `0`), or a
/// non-negative percentage. `content` and other units are unsupported (`None`).
pub(crate) fn parse_flex_basis(token: &str) -> Option<Dimension> {
    let t = token.trim();
    if t.eq_ignore_ascii_case("auto") {
        return Some(Dimension::Auto);
    }
    if let Some(pct) = t.strip_suffix('%') {
        return parse_f32(pct).filter(|v| *v >= 0.0).map(Dimension::Percent);
    }
    if let Some(n) = parse_f32(t) {
        return (n == 0.0).then_some(Dimension::Points(0.0));
    }
    parse_px(t).filter(|v| *v >= 0.0).map(Dimension::Points)
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
        for allow_negative in [false, true] {
            let p = |v| parse_edges(v, allow_negative);
            assert_eq!(p("12px"), Some(EdgeInsets::all(12.0)));
            assert_eq!(p("12PX 14Px"), Some(edges(12.0, 14.0, 12.0, 14.0)));
            assert_eq!(p("12px 14px 16px"), Some(edges(12.0, 14.0, 16.0, 14.0)));
            assert_eq!(p(" 12px  14px 16px 18 "), Some(edges(12.0, 14.0, 16.0, 18.0)));
            for bad in ["", "   ", "1px 2px 3px 4px 5px", "0 auto", "1em", "12px x"] {
                assert_eq!(p(bad), None, "{bad:?}");
            }
        }
        // Negative lengths: valid margins, dropped paddings (all-or-nothing).
        assert_eq!(parse_edges("-4px 8px", true), Some(edges(-4.0, 8.0, -4.0, 8.0)));
        assert_eq!(parse_edges("-4px 8px", false), None);
        assert_eq!(parse_edges("4px -1", false), None);

        let mut e = EdgeInsets::all(1.0);
        assert!(apply_edge_longhand(&mut e, "-top", "5px", false));
        assert!(apply_edge_longhand(&mut e, "Left", "7", false));
        assert!(apply_edge_longhand(&mut e, "-horizontal", "9PX", false));
        assert!(apply_edge_longhand(&mut e, "Vertical", "2px", false));
        assert_eq!(e, edges(2.0, 9.0, 2.0, 9.0));
        assert!(apply_edge_longhand(&mut e, "-bottom", "auto", false), "handled, untouched");
        assert!(apply_edge_longhand(&mut e, "-bottom", "-3px", false), "handled, untouched");
        assert_eq!(e.bottom, 2.0);
        assert!(apply_edge_longhand(&mut e, "-bottom", "-3px", true));
        assert_eq!(e.bottom, -3.0);
        assert!(!apply_edge_longhand(&mut e, "-start", "3px", true));
    }

    #[test]
    fn border_shorthand_accepts_any_token_order() {
        let slate = ColorRgba::rgb(0x1e, 0x29, 0x3b);
        assert_eq!(parse_border("1px solid #1e293b"), Some((1.0, Some(slate))));
        assert_eq!(parse_border("#1e293b solid 2px"), Some((2.0, Some(slate))));
        assert_eq!(parse_border("solid 1px"), Some((1.0, None)), "omitted color is currentColor");
        assert_eq!(parse_border("solid"), Some((3.0, None)), "medium width + currentColor");
        assert_eq!(
            parse_border("thick dashed red"),
            Some((5.0, Some(ColorRgba::rgb(239, 68, 68))))
        );
        assert_eq!(parse_border("none"), Some((0.0, None)));
        assert_eq!(parse_border("4px none #1e293b"), Some((0.0, Some(slate))));
        assert_eq!(parse_border("0"), Some((0.0, None)));
        assert_eq!(parse_border("2px #1e293b"), Some((0.0, Some(slate))), "omitted style is none");
        assert_eq!(parse_border("1px solid currentColor"), Some((1.0, None)));
        assert_eq!(parse_border("CURRENTCOLOR dashed 2PX"), Some((2.0, None)));
        assert_eq!(
            parse_border("1px solid rgba(30, 41, 59, 0.5)"),
            Some((1.0, Some(ColorRgba::rgba(0x1e, 0x29, 0x3b, 128))))
        );
        assert_eq!(parse_border("1px solid rgb(30 41 59)"), Some((1.0, Some(slate))));
        for bad in [
            "",
            "1px 2px",
            "2 solid red",
            "solid solid",
            "1px wavy red",
            "-1px solid",
            "1px solid #12",
            "1px solid currentcolor red",
            "1px solid rgb(1, 2)",
            "1px solid rgb(1, 2, 3, 4, 5)",
            "1px solid rgb(1, 2, 3,)",
        ] {
            assert_eq!(parse_border(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn flex_shorthand_follows_css_omitted_value_rules() {
        use Dimension::{Auto, Percent as Pct, Points as Px};
        let f = |g, s, b| Some(FlexShorthand::new(g, s, b));
        assert_eq!(parse_flex("none"), f(0.0, 0.0, Auto));
        assert_eq!(parse_flex("AUTO"), f(1.0, 1.0, Auto));
        assert_eq!(parse_flex("initial"), f(0.0, 1.0, Auto));
        assert_eq!(parse_flex("1"), f(1.0, 1.0, Pct(0.0)));
        assert_eq!(parse_flex("2 3"), f(2.0, 3.0, Pct(0.0)));
        assert_eq!(parse_flex("2 280px"), f(2.0, 1.0, Px(280.0)));
        assert_eq!(parse_flex("1 1 280px"), f(1.0, 1.0, Px(280.0)));
        assert_eq!(parse_flex("1 1 110PX"), f(1.0, 1.0, Px(110.0)));
        assert_eq!(parse_flex("0 0 auto"), f(0.0, 0.0, Auto));
        assert_eq!(parse_flex("1 0 50%"), f(1.0, 0.0, Pct(50.0)));
        assert_eq!(parse_flex("100px"), f(1.0, 1.0, Px(100.0)));
        assert_eq!(parse_flex("2.5"), f(2.5, 1.0, Pct(0.0)));
        assert_eq!(parse_flex("0"), f(0.0, 1.0, Pct(0.0)));
        assert_eq!(parse_flex("280px 2 3"), f(2.0, 3.0, Px(280.0)), "basis first");
        // A third unitless `0` is the basis.
        assert_eq!(parse_flex("1 1 0"), f(1.0, 1.0, Px(0.0)));
        assert_eq!(parse_flex("1 0 0"), f(1.0, 0.0, Px(0.0)));
        assert_eq!(parse_flex("0 0 0"), f(0.0, 0.0, Px(0.0)));
        assert_eq!(parse_flex("0px 1 1"), f(1.0, 1.0, Px(0.0)));
        for bad in [
            "",
            "1 2 3",
            "1 1 0 0",
            "1 1 0 1px",
            "1 2 3px 4",
            "-1",
            "1 -2",
            "1 280px 1",
            "1px 2px",
            "x",
            "1 1 fit",
            "1 1 content",
            "1 1 -5px",
            "1 1 -5%",
        ] {
            assert_eq!(parse_flex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn flex_basis_accepts_auto_lengths_percentages_and_only_unitless_zero() {
        assert_eq!(parse_flex_basis(" AUTO "), Some(Dimension::Auto));
        assert_eq!(parse_flex_basis("280px"), Some(Dimension::Points(280.0)));
        assert_eq!(parse_flex_basis("12PX"), Some(Dimension::Points(12.0)));
        assert_eq!(parse_flex_basis("0"), Some(Dimension::Points(0.0)));
        assert_eq!(parse_flex_basis("33.5%"), Some(Dimension::Percent(33.5)));
        for bad in ["", "12", "-1px", "-1%", "content", "1em", "calc(1px)", "x%"] {
            assert_eq!(parse_flex_basis(bad), None, "{bad:?}");
        }
    }
}
