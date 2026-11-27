//! String `box-shadow` parsing for the string-based [`super::StyleBuf`] bridge
//! (`[inset] offset-x offset-y [blur [spread]] color`, comma-separated lists).

use nve_abi::num::parse_f32;
use nve_tables::tables::BoxShadow;
use nve_store::Rgba;

/// Parses a standard CSS `box-shadow` value (`[inset] offset-x offset-y [blur [spread]] color`),
/// supporting multiple comma-separated shadows, `#hex`, `rgb(...)`, `rgba(...)`, and named colors.
pub(crate) fn parse_box_shadow(raw: &str) -> Vec<BoxShadow> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for segment in split_top_level_commas(trimmed) {
        if let Some(shadow) = parse_single_shadow(segment) {
            out.push(shadow);
        }
    }
    out
}

fn split_top_level_commas(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(input[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = input[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn tokenize_shadow(input: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    for (i, ch) in input.char_indices() {
        match ch {
            '(' => {
                start.get_or_insert(i);
                depth += 1;
            }
            ')' => depth = depth.saturating_sub(1),
            c if c.is_ascii_whitespace() && depth == 0 => {
                if let Some(s) = start.take() {
                    tokens.push(&input[s..i]);
                }
            }
            _ => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(s) = start {
        tokens.push(&input[s..]);
    }
    tokens
}

fn parse_single_shadow(segment: &str) -> Option<BoxShadow> {
    let tokens = tokenize_shadow(segment);
    if tokens.len() < 2 {
        return None;
    }
    let mut lengths = Vec::with_capacity(4);
    let mut color = None;
    let mut inset = false;

    for tok in tokens {
        if tok.eq_ignore_ascii_case("inset") {
            inset = true;
            continue;
        }
        if let Some(c) = nve_tables::css::parse_color(tok) {
            color = Some(Rgba(c.to_u32()));
        } else {
            let trimmed = tok.trim().strip_suffix("px").unwrap_or(tok.trim());
            let px = parse_f32(trimmed)?;
            if !px.is_finite() {
                return None;
            }
            lengths.push(px);
        }
    }
    if lengths.len() < 2 || lengths.len() > 4 {
        return None;
    }
    Some(BoxShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_radius: lengths.get(2).copied().unwrap_or(0.0).max(0.0),
        spread_radius: lengths.get(3).copied().unwrap_or(0.0),
        color: color.unwrap_or(Rgba::new(0, 0, 0, 255)),
        inset,
    })
}
