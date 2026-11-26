//! CSS `box-shadow` parser and analytical Gaussian-falloff rounded-rect shadow rasterizer.
//!
//! Uses an exact SDF + `erfc` Gaussian edge falloff with a solid interior-span fast path so soft
//! rounded shadows render in microseconds without 2D convolution passes or scratch allocations.

use nve_abi::ColorRgba;
use nve_engine::LayoutRect;

use super::clip::{rounded_rect_sdf, ClipStack};
use super::{fill_rounded_rect, PixelBuffer};

/// Parsed CSS outer `box-shadow` specification in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: ColorRgba,
}

/// Parses a standard CSS `box-shadow` value (`offset-x offset-y [blur [spread]] color`),
/// supporting multiple comma-separated shadows, `#hex` (3/4/6/8 digits), `rgb(...)`, `rgba(...)`,
/// and named colors. `inset` shadows and `none` are ignored cleanly.
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
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            ')' => {
                depth = depth.saturating_sub(1);
            }
            c if c.is_ascii_whitespace() && depth == 0 => {
                if let Some(s) = start.take() {
                    tokens.push(&input[s..i]);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
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

    let mut lengths: Vec<f32> = Vec::with_capacity(4);
    let mut color: Option<ColorRgba> = None;

    for tok in tokens {
        if tok.eq_ignore_ascii_case("inset") {
            // Outer drop shadows only; skip inset shadows cleanly.
            return None;
        }
        if let Some(c) = parse_css_color(tok) {
            color = Some(c);
        } else {
            let px = parse_css_length(tok)?;
            lengths.push(px);
        }
    }

    if lengths.len() < 2 || lengths.len() > 4 {
        return None;
    }

    let offset_x = lengths[0];
    let offset_y = lengths[1];
    let blur_radius = lengths.get(2).copied().unwrap_or(0.0).max(0.0);
    let spread_radius = lengths.get(3).copied().unwrap_or(0.0);
    let color = color.unwrap_or_else(|| ColorRgba::rgba(0, 0, 0, 96));

    if color.a == 0 {
        return None;
    }
    Some(BoxShadow {
        offset_x,
        offset_y,
        blur_radius,
        spread_radius,
        color,
    })
}

fn parse_css_length(tok: &str) -> Option<f32> {
    let s = tok.trim();
    let num = s.strip_suffix("px").unwrap_or(s);
    let v = num.parse::<f32>().ok()?;
    v.is_finite().then_some(v)
}

pub(crate) fn parse_css_color(raw: &str) -> Option<ColorRgba> {
    let s = raw.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return match hex.len() {
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
        };
    }

    let rgb_inner = if s.len() >= 6 && s.as_bytes()[..5].eq_ignore_ascii_case(b"rgba(") {
        s[5..].strip_suffix(')')
    } else if s.len() >= 5 && s.as_bytes()[..4].eq_ignore_ascii_case(b"rgb(") {
        s[4..].strip_suffix(')')
    } else {
        None
    };
    if let Some(inner) = rgb_inner {
        let mut comps = [""; 4];
        let mut count = 0usize;
        for tok in inner.split(|c: char| c.is_ascii_whitespace() || c == ',' || c == '/') {
            if tok.is_empty() {
                continue;
            }
            if count == 4 {
                return None;
            }
            comps[count] = tok;
            count += 1;
        }
        if count != 3 && count != 4 {
            return None;
        }
        let r = parse_u8_channel(comps[0])?;
        let g = parse_u8_channel(comps[1])?;
        let b = parse_u8_channel(comps[2])?;
        let a = if count == 4 {
            parse_alpha_channel(comps[3])?
        } else {
            255
        };
        return Some(ColorRgba::rgba(r, g, b, a));
    }

    if s.eq_ignore_ascii_case("black") {
        Some(ColorRgba::BLACK)
    } else if s.eq_ignore_ascii_case("white") {
        Some(ColorRgba::WHITE)
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

fn parse_u8_channel(tok: &str) -> Option<u8> {
    let v = tok.parse::<f32>().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some(v.round().clamp(0.0, 255.0) as u8)
}

fn parse_alpha_channel(tok: &str) -> Option<u8> {
    if let Some(pct) = tok.strip_suffix('%') {
        let v = pct.parse::<f32>().ok()?;
        if !v.is_finite() {
            return None;
        }
        return Some((v * 2.55).round().clamp(0.0, 255.0) as u8);
    }
    let v = tok.parse::<f32>().ok()?;
    if !v.is_finite() {
        return None;
    }
    let scaled = if v <= 1.0 { v * 255.0 } else { v };
    Some(scaled.round().clamp(0.0, 255.0) as u8)
}

/// Fast Abramowitz & Stegun 7.1.26 approximation of `0.5 * erfc(x)` accurate to `< 1e-6`.
/// Maps signed distance normalized by `sigma * sqrt(2)` into smooth Gaussian coverage `0.0..=1.0`.
#[inline]
fn gaussian_edge_coverage(x: f32) -> f32 {
    if x <= -3.0 {
        return 1.0;
    }
    if x >= 3.0 {
        return 0.0;
    }
    let ax = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * ax);
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_72
                + t * (1.421_413_8 + t * (-1.453_152_1 + t * 1.061_405_4))));
    let erfc_abs = poly * (-ax * ax).exp();
    if x >= 0.0 {
        0.5 * erfc_abs
    } else {
        1.0 - 0.5 * erfc_abs
    }
}

/// Rasterizes a soft rounded box shadow in physical pixel coordinates.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_box_shadow(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    rect: LayoutRect,
    radius: f32,
    offset_x: f32,
    offset_y: f32,
    blur_radius: f32,
    spread_radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if color.a == 0 || opacity <= 0.0 || clip.is_empty() {
        return;
    }
    let shadow_w = rect.width + spread_radius * 2.0;
    let shadow_h = rect.height + spread_radius * 2.0;
    if shadow_w <= 0.0 || shadow_h <= 0.0 {
        return;
    }
    let shadow_rect = LayoutRect {
        x: rect.x + offset_x - spread_radius,
        y: rect.y + offset_y - spread_radius,
        width: shadow_w,
        height: shadow_h,
    };
    let base_r = if radius > 0.01 {
        (radius + spread_radius).max(0.0)
    } else {
        0.0
    };

    if blur_radius <= 0.5 {
        fill_rounded_rect(buf, clip, shadow_rect, base_r, color, opacity);
        return;
    }

    // Standard CSS blur-radius ~= 2 * sigma; 3*sigma (~1.5*blur) covers >99.7% of Gaussian energy.
    let sigma = (blur_radius * 0.5).max(0.5);
    let inv_sigma_sqrt2 = 1.0 / (sigma * std::f32::consts::SQRT_2);
    let pad = (sigma * 3.0).ceil();
    let pad_i = pad as i32;

    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (shadow_rect.x.floor() as i32 - pad_i).clamp(cx0, cx1);
    let max_x = ((shadow_rect.x + shadow_rect.width).ceil() as i32 + pad_i).clamp(cx0, cx1);
    let min_y = (shadow_rect.y.floor() as i32 - pad_i).clamp(cy0, cy1);
    let max_y = ((shadow_rect.y + shadow_rect.height).ceil() as i32 + pad_i).clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let hx = shadow_rect.width * 0.5;
    let hy = shadow_rect.height * 0.5;
    let cx = shadow_rect.x + hx;
    let cy = shadow_rect.y + hy;
    let r = (base_r + sigma * 0.35).clamp(0.0, hx.min(hy));
    let rounded_clip = clip.has_rounded();

    // Guaranteed coverage == 1.0 interior core where sdf <= -3*sigma (bypasses exp & sqrt).
    let core_inset = r + pad;
    let core_x0 = ((shadow_rect.x + core_inset).ceil() as i32).max(min_x);
    let core_x1 = ((shadow_rect.x + shadow_rect.width - core_inset).floor() as i32).min(max_x);
    let core_y0 = ((shadow_rect.y + pad).ceil() as i32).max(min_y);
    let core_y1 = ((shadow_rect.y + shadow_rect.height - pad).floor() as i32).min(max_y);
    let full_band_y0 = ((shadow_rect.y + core_inset).ceil() as i32).max(min_y);
    let full_band_y1 = ((shadow_rect.y + shadow_rect.height - core_inset).floor() as i32).min(max_y);
    let band_x0 = ((shadow_rect.x + pad).ceil() as i32).max(min_x);
    let band_x1 = ((shadow_rect.x + shadow_rect.width - pad).floor() as i32).min(max_x);

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let (rect_l, rect_r) = if py >= full_band_y0 && py < full_band_y1 && band_x0 < band_x1 {
            (band_x0, band_x1)
        } else if py >= core_y0 && py < core_y1 && core_x0 < core_x1 {
            (core_x0, core_x1)
        } else {
            (max_x, max_x)
        };
        let (span_l, span_r) = if rounded_clip {
            clip.interior_span_for_row(py, rect_l, rect_r)
        } else {
            (rect_l, rect_r)
        };

        let left_end = span_l.min(max_x);
        for px in min_x..left_end {
            let fx = px as f32 + 0.5;
            let clip_cov = if rounded_clip {
                clip.rounded_coverage(fx, fy)
            } else {
                1.0
            };
            if clip_cov <= 0.0 {
                continue;
            }
            let sdf = rounded_rect_sdf(fx, fy, cx, cy, hx, hy, r);
            let cov = gaussian_edge_coverage(sdf * inv_sigma_sqrt2) * clip_cov;
            if cov > 0.002 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }

        if span_l < span_r {
            buf.blend_span(py, span_l, span_r, color, opacity);
        }

        let right_start = if span_l < span_r { span_r } else { left_end };
        for px in right_start..max_x {
            let fx = px as f32 + 0.5;
            let clip_cov = if rounded_clip {
                clip.rounded_coverage(fx, fy)
            } else {
                1.0
            };
            if clip_cov <= 0.0 {
                continue;
            }
            let sdf = rounded_rect_sdf(fx, fy, cx, cy, hx, hy, r);
            let cov = gaussian_edge_coverage(sdf * inv_sigma_sqrt2) * clip_cov;
            if cov > 0.002 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}
