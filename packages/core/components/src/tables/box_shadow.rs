//! Parsed CSS `box-shadow` value shared by web CSS emission, Skia rasterization, and `Visual`.

use nve_abi::num::{parse_f32, write_f32};
use nve_store::Rgba;

use crate::paint::write_css_color;

/// One CSS `box-shadow` entry in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: Rgba,
    pub inset: bool,
}

impl BoxShadow {
    /// Formats a single shadow into canonical CSS syntax.
    pub fn write_css_into(&self, out: &mut String) {
        if self.inset {
            out.push_str("inset ");
        }
        write_px(out, self.offset_x);
        out.push(' ');
        write_px(out, self.offset_y);
        if self.blur_radius != 0.0 || self.spread_radius != 0.0 {
            out.push(' ');
            write_px(out, self.blur_radius.max(0.0));
            if self.spread_radius != 0.0 {
                out.push(' ');
                write_px(out, self.spread_radius);
            }
        }
        out.push(' ');
        write_css_color(nve_abi::ColorRgba::from_u32(self.color.0), out);
    }
}

/// Formats a list of box shadows into canonical CSS syntax (`"none"` if empty).
pub fn write_box_shadows_css(out: &mut String, shadows: &[BoxShadow]) {
    if shadows.is_empty() {
        out.push_str("none");
        return;
    }
    for (i, s) in shadows.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        s.write_css_into(out);
    }
}

fn write_px(out: &mut String, v: f32) {
    if !v.is_finite() || v == 0.0 {
        out.push('0');
    } else {
        write_f32(out, v);
        out.push_str("px");
    }
}

/// Parses a standard CSS `box-shadow` value (`[inset] offset-x offset-y [blur [spread]] color`),
/// supporting multiple comma-separated shadows, `#hex`, `rgb(...)`, `rgba(...)`, and named colors.
pub fn parse_box_shadow(raw: &str) -> Vec<BoxShadow> {
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
        if let Some(c) = crate::parse_color(tok) {
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
