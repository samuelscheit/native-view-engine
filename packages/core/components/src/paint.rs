//! Zero-serialization typed background fills (`BackgroundFill`, `LinearGradient`,
//! `RadialGradient`, `GradientStop`) with CSS-aligned angle geometry, premultiplied-alpha
//! stop sampling, deterministic hashing, and injection-free CSS gradient formatting.

use std::sync::Arc;

use nve_abi::num::{write_f32, write_u64};
use nve_abi::{ColorRgba, Xxh64Hasher};

/// One color stop along a linear or radial gradient (`offset` in `0.0..=1.0`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: ColorRgba,
}

impl GradientStop {
    /// Creates a gradient stop with `offset` clamped to `0.0..=1.0` (non-finite -> `0.0`).
    #[inline]
    pub fn new(offset: f32, color: ColorRgba) -> Self {
        let clamped = if offset.is_finite() {
            offset.clamp(0.0, 1.0)
        } else {
            0.0
        };
        Self {
            offset: clamped,
            color,
        }
    }
}

/// Typed linear gradient following the CSS `linear-gradient(<deg>, ...)` angle convention:
/// `0°` = bottom → top, `90°` = left → right, `180°` = top → bottom, `270°` = right → left.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    pub angle_deg: f32,
    pub stops: Arc<[GradientStop]>,
}

impl LinearGradient {
    /// Creates a linear gradient with sanitized `angle_deg` and clamped, stably sorted stops.
    pub fn new(angle_deg: f32, stops: impl Into<Vec<GradientStop>>) -> Self {
        let angle = if angle_deg.is_finite() {
            angle_deg
        } else {
            180.0
        };
        let normalized = normalize_stops(stops.into());
        Self {
            angle_deg: angle,
            stops: Arc::from(normalized),
        }
    }

    /// Computes the CSS Images 3/4 gradient line start and end points `(p0, p1)` for box
    /// `(x, y, width, height)`. Shared by software rasterization and Skia GPU shaders so
    /// both render bit-compatible angles.
    #[inline]
    pub fn endpoints(&self, x: f32, y: f32, width: f32, height: f32) -> ((f32, f32), (f32, f32)) {
        let angle = if self.angle_deg.is_finite() {
            self.angle_deg
        } else {
            180.0
        };
        let rad = angle.to_radians();
        let sin_a = rad.sin();
        let cos_a = rad.cos();
        // CSS angle convention: 0° points straight up (0, -1); 90° points right (+1, 0).
        let ux = sin_a;
        let uy = -cos_a;
        let w = width.max(0.0);
        let h = height.max(0.0);
        let half_len = 0.5 * ((w * sin_a).abs() + (h * cos_a).abs());
        let cx = x + w * 0.5;
        let cy = y + h * 0.5;
        (
            (cx - ux * half_len, cy - uy * half_len),
            (cx + ux * half_len, cy + uy * half_len),
        )
    }
}

/// Typed radial gradient with normalized center (`0.0..=1.0` of box dimensions) and `radius`
/// relative to box half-extent (`1.0` reaches the box edges: `rx = radius * w/2`, `ry = radius * h/2`).
#[derive(Debug, Clone, PartialEq)]
pub struct RadialGradient {
    pub center_x: f32,
    pub center_y: f32,
    pub radius: f32,
    pub stops: Arc<[GradientStop]>,
}

impl RadialGradient {
    /// Creates a radial gradient with sanitized normalized center, non-negative `radius`,
    /// and clamped, stably sorted stops.
    pub fn new(
        center_x: f32,
        center_y: f32,
        radius: f32,
        stops: impl Into<Vec<GradientStop>>,
    ) -> Self {
        let cx = if center_x.is_finite() {
            center_x.clamp(0.0, 1.0)
        } else {
            0.5
        };
        let cy = if center_y.is_finite() {
            center_y.clamp(0.0, 1.0)
        } else {
            0.5
        };
        let r = if radius.is_finite() {
            radius.max(0.0)
        } else {
            1.0
        };
        let normalized = normalize_stops(stops.into());
        Self {
            center_x: cx,
            center_y: cy,
            radius: r,
            stops: Arc::from(normalized),
        }
    }

    /// Computes pixel-space `(center_x, center_y, radius_x, radius_y)` for box `(x, y, w, h)`.
    #[inline]
    pub fn geometry(&self, x: f32, y: f32, width: f32, height: f32) -> (f32, f32, f32, f32) {
        let cx_norm = if self.center_x.is_finite() {
            self.center_x
        } else {
            0.5
        };
        let cy_norm = if self.center_y.is_finite() {
            self.center_y
        } else {
            0.5
        };
        let r = if self.radius.is_finite() {
            self.radius.max(0.0)
        } else {
            1.0
        };
        let w = width.max(0.0);
        let h = height.max(0.0);
        (
            x + cx_norm * w,
            y + cy_norm * h,
            r * w * 0.5,
            r * h * 0.5,
        )
    }
}

/// Unified typed background fill: solid color, linear gradient, or radial gradient.
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundFill {
    Solid(ColorRgba),
    Linear(LinearGradient),
    Radial(RadialGradient),
}

impl BackgroundFill {
    /// Convenience constructor for a linear gradient background fill.
    #[inline]
    pub fn linear(angle_deg: f32, stops: impl Into<Vec<GradientStop>>) -> Self {
        Self::Linear(LinearGradient::new(angle_deg, stops))
    }

    /// Convenience constructor for a radial gradient background fill.
    #[inline]
    pub fn radial(
        center_x: f32,
        center_y: f32,
        radius: f32,
        stops: impl Into<Vec<GradientStop>>,
    ) -> Self {
        Self::Radial(RadialGradient::new(center_x, center_y, radius, stops))
    }

    /// Writes the full CSS declaration(s) for this background fill into `out`.
    ///
    /// When `clear_image_on_solid` is `true` (used inside `StylePatch` overrides for `@media`
    /// and `:hover`/`:active`), `Solid` emits `background-image:none;background-color:<c>;` so
    /// it cleanly replaces an inherited/base gradient.
    pub fn write_css_decl_into(&self, clear_image_on_solid: bool, out: &mut String) {
        match self {
            Self::Solid(c) => {
                if clear_image_on_solid {
                    out.push_str("background-image:none;background-color:");
                    write_css_color(*c, out);
                    out.push(';');
                } else if c.a > 0 {
                    out.push_str("background-color:");
                    write_css_color(*c, out);
                    out.push(';');
                }
            }
            Self::Linear(_) | Self::Radial(_) => {
                out.push_str("background-image:");
                write_css_gradient(self, out);
                out.push(';');
            }
        }
    }

    /// Feeds this background fill into a deterministic XXH64 structural style hash.
    pub fn hash_into(&self, h: &mut Xxh64Hasher) {
        match self {
            Self::Solid(c) => h.update(&[1, c.r, c.g, c.b, c.a]),
            Self::Linear(lg) => {
                h.update(&[2]);
                h.update_f32(lg.angle_deg);
                hash_stops(h, &lg.stops);
            }
            Self::Radial(rg) => {
                h.update(&[3]);
                h.update_f32(rg.center_x);
                h.update_f32(rg.center_y);
                h.update_f32(rg.radius);
                hash_stops(h, &rg.stops);
            }
        }
    }
}

/// Clamps stop offsets to `0.0..=1.0` and stably sorts them in ascending offset order.
pub fn normalize_stops(mut stops: Vec<GradientStop>) -> Vec<GradientStop> {
    for s in &mut stops {
        s.offset = if s.offset.is_finite() {
            s.offset.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    stops
}

/// Returns `true` if `stops` are already finite, clamped in `0.0..=1.0`, and non-decreasing.
#[inline]
pub fn stops_are_normalized(stops: &[GradientStop]) -> bool {
    let mut prev = 0.0f32;
    for s in stops {
        if !s.offset.is_finite() || s.offset < 0.0 || s.offset > 1.0 || s.offset < prev {
            return false;
        }
        prev = s.offset;
    }
    true
}

/// Samples a gradient stop list at position `t` (`0.0..=1.0`) with premultiplied-alpha color
/// interpolation, clamping out-of-range offsets, handling unsorted stops, single stops, and
/// identical-offset hard stops without division by zero.
pub fn sample_gradient_stops(stops: &[GradientStop], t: f32) -> ColorRgba {
    if stops.is_empty() {
        return ColorRgba::TRANSPARENT;
    }
    if stops.len() == 1 {
        return stops[0].color;
    }
    if stops_are_normalized(stops) {
        sample_normalized_stops(stops, t)
    } else {
        let sorted = normalize_stops(stops.to_vec());
        sample_normalized_stops(&sorted, t)
    }
}

/// Fast-path sampling over stops already verified to be clamped & sorted ascending.
#[inline]
pub fn sample_normalized_stops(stops: &[GradientStop], t: f32) -> ColorRgba {
    if stops.is_empty() {
        return ColorRgba::TRANSPARENT;
    }
    if stops.len() == 1 {
        return stops[0].color;
    }
    let tc = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if tc <= stops[0].offset {
        return stops[0].color;
    }
    let last = stops.len() - 1;
    if tc >= stops[last].offset {
        return stops[last].color;
    }
    // Find first stop strictly greater than `tc`: `stops[hi - 1].offset <= tc < stops[hi].offset`,
    // guaranteeing `span > 0.0` even when duplicate offsets (hard stops) exist.
    let mut hi = 1usize;
    while hi < stops.len() && stops[hi].offset <= tc {
        hi += 1;
    }
    if hi >= stops.len() {
        return stops[last].color;
    }
    let lo = stops[hi - 1];
    let hi_stop = stops[hi];
    let span = hi_stop.offset - lo.offset;
    if span <= 1e-7 {
        return hi_stop.color;
    }
    let frac = ((tc - lo.offset) / span).clamp(0.0, 1.0);
    lerp_color_premul(lo.color, hi_stop.color, frac)
}

/// Premultiplied-alpha linear interpolation between `a` and `b` at `t` (`0.0..=1.0`).
/// Prevents dark fringes when interpolating an opaque color toward `ColorRgba::TRANSPARENT`.
#[inline]
pub fn lerp_color_premul(a: ColorRgba, b: ColorRgba, t: f32) -> ColorRgba {
    let f = t.clamp(0.0, 1.0);
    if f <= 0.0 {
        return a;
    }
    if f >= 1.0 {
        return b;
    }
    if a.a == 255 && b.a == 255 {
        let lerp_u8 = |u: u8, v: u8| -> u8 {
            (f32::from(u) + (f32::from(v) - f32::from(u)) * f)
                .clamp(0.0, 255.0)
                .round() as u8
        };
        return ColorRgba::rgb(lerp_u8(a.r, b.r), lerp_u8(a.g, b.g), lerp_u8(a.b, b.b));
    }
    let a0 = f32::from(a.a) / 255.0;
    let a1 = f32::from(b.a) / 255.0;
    let alpha = a0 + (a1 - a0) * f;
    if alpha <= 1e-6 {
        return ColorRgba::TRANSPARENT;
    }
    let inv_alpha = 1.0 / alpha;
    let mix = |c0: u8, c1: u8| -> u8 {
        let pm0 = f32::from(c0) * a0;
        let pm1 = f32::from(c1) * a1;
        ((pm0 + (pm1 - pm0) * f) * inv_alpha)
            .clamp(0.0, 255.0)
            .round() as u8
    };
    ColorRgba::rgba(
        mix(a.r, b.r),
        mix(a.g, b.g),
        mix(a.b, b.b),
        (alpha * 255.0).clamp(0.0, 255.0).round() as u8,
    )
}

/// Formats a canonical, injection-free CSS value for [`BackgroundFill`] directly into `out`:
/// - `Linear`: `linear-gradient(<deg>deg,<stop1>,<stop2>,...)`
/// - `Radial`: `radial-gradient(<rx>% <ry>% at <cx>% <cy>%,<stop1>,<stop2>,...)`
/// - `Solid`: compact `#rrggbb` / `transparent` / `rgba(...)`
pub fn write_css_gradient(fill: &BackgroundFill, out: &mut String) {
    match fill {
        BackgroundFill::Solid(c) => write_css_color(*c, out),
        BackgroundFill::Linear(lg) => {
            let angle = if lg.angle_deg.is_finite() {
                lg.angle_deg
            } else {
                180.0
            };
            out.push_str("linear-gradient(");
            write_f32(out, angle);
            out.push_str("deg");
            write_css_stops(&lg.stops, out);
            out.push(')');
        }
        BackgroundFill::Radial(rg) => {
            let cx = if rg.center_x.is_finite() {
                rg.center_x.clamp(0.0, 1.0) * 100.0
            } else {
                50.0
            };
            let cy = if rg.center_y.is_finite() {
                rg.center_y.clamp(0.0, 1.0) * 100.0
            } else {
                50.0
            };
            let r_pct = if rg.radius.is_finite() {
                rg.radius.max(0.0) * 50.0
            } else {
                50.0
            };
            out.push_str("radial-gradient(");
            write_f32(out, r_pct);
            out.push_str("% ");
            write_f32(out, r_pct);
            out.push_str("% at ");
            write_f32(out, cx);
            out.push_str("% ");
            write_f32(out, cy);
            out.push('%');
            write_css_stops(&rg.stops, out);
            out.push(')');
        }
    }
}

fn write_css_stops(stops: &[GradientStop], out: &mut String) {
    if stops.is_empty() {
        out.push_str(",transparent 0%,transparent 100%");
        return;
    }
    if stops.len() == 1 {
        let c = stops[0].color;
        out.push(',');
        write_css_color(c, out);
        out.push_str(" 0%,");
        write_css_color(c, out);
        out.push_str(" 100%");
        return;
    }
    if stops_are_normalized(stops) {
        for s in stops {
            write_single_css_stop(*s, out);
        }
    } else {
        let sorted = normalize_stops(stops.to_vec());
        for s in sorted {
            write_single_css_stop(s, out);
        }
    }
}

#[inline]
fn write_single_css_stop(stop: GradientStop, out: &mut String) {
    out.push(',');
    write_css_color(stop.color, out);
    out.push(' ');
    write_f32(out, stop.offset.clamp(0.0, 1.0) * 100.0);
    out.push('%');
}

/// Formats a compact, injection-free CSS color (`#rrggbb`, `transparent`, or `rgba(r,g,b,a)`).
pub fn write_css_color(c: ColorRgba, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    if c.a == 255 {
        out.push('#');
        for ch in [c.r, c.g, c.b] {
            out.push(HEX[usize::from(ch >> 4)] as char);
            out.push(HEX[usize::from(ch & 15)] as char);
        }
    } else if c.a == 0 {
        out.push_str("transparent");
    } else {
        out.push_str("rgba(");
        for ch in [c.r, c.g, c.b] {
            write_u64(out, u64::from(ch));
            out.push(',');
        }
        write_f32(out, f32::from(c.a) / 255.0);
        out.push(')');
    }
}

fn hash_stops(h: &mut Xxh64Hasher, stops: &[GradientStop]) {
    h.update_u32(stops.len() as u32);
    for s in stops {
        h.update_f32(s.offset);
        h.update(&[s.color.r, s.color.g, s.color.b, s.color.a]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_handles_empty_single_unsorted_hard_stops_and_premul_alpha() {
        assert_eq!(sample_gradient_stops(&[], 0.5), ColorRgba::TRANSPARENT);
        let red = ColorRgba::rgb(255, 0, 0);
        let blue = ColorRgba::rgb(0, 0, 255);
        assert_eq!(sample_gradient_stops(&[GradientStop::new(0.4, red)], 0.0), red);
        assert_eq!(sample_gradient_stops(&[GradientStop::new(0.4, red)], 1.0), red);

        // Unsorted stops normalize cleanly.
        let unsorted = [
            GradientStop { offset: 1.0, color: blue },
            GradientStop { offset: 0.0, color: red },
        ];
        let mid = sample_gradient_stops(&unsorted, 0.5);
        assert!((126..=129).contains(&mid.r) && mid.g == 0 && (126..=129).contains(&mid.b));

        // Identical offsets (hard stop) never produce NaN or panic.
        let hard = [
            GradientStop::new(0.0, red),
            GradientStop::new(0.5, red),
            GradientStop::new(0.5, blue),
            GradientStop::new(1.0, blue),
        ];
        assert_eq!(sample_gradient_stops(&hard, 0.49), red);
        assert_eq!(sample_gradient_stops(&hard, 0.5), blue);
        assert_eq!(sample_gradient_stops(&hard, 0.51), blue);

        // Premultiplied alpha: red -> transparent at 0.5 stays pure red at 50% alpha.
        let fade = [
            GradientStop::new(0.0, red),
            GradientStop::new(1.0, ColorRgba::TRANSPARENT),
        ];
        let half = sample_gradient_stops(&fade, 0.5);
        assert_eq!(half.r, 255);
        assert_eq!(half.g, 0);
        assert_eq!(half.b, 0);
        assert!((127..=128).contains(&half.a));
    }

    #[test]
    fn css_gradient_formatting_is_canonical_and_injection_free() {
        let lin = BackgroundFill::linear(
            90.0,
            [
                GradientStop::new(0.0, ColorRgba::rgb(255, 0, 0)),
                GradientStop::new(1.0, ColorRgba::rgb(0, 0, 255)),
            ],
        );
        let mut css = String::new();
        write_css_gradient(&lin, &mut css);
        assert_eq!(css, "linear-gradient(90deg,#ff0000 0%,#0000ff 100%)");

        let rad = BackgroundFill::radial(
            0.5,
            0.25,
            1.0,
            [
                GradientStop::new(0.0, ColorRgba::WHITE),
                GradientStop::new(1.0, ColorRgba::TRANSPARENT),
            ],
        );
        css.clear();
        write_css_gradient(&rad, &mut css);
        assert_eq!(
            css,
            "radial-gradient(50% 50% at 50% 25%,#ffffff 0%,transparent 100%)"
        );
    }

    #[test]
    fn linear_gradient_endpoints_match_css_angle_convention() {
        let g0 = LinearGradient::new(0.0, []);
        let (p0, p1) = g0.endpoints(0.0, 0.0, 100.0, 200.0);
        assert!((p0.0 - 50.0).abs() < 1e-3 && (p0.1 - 200.0).abs() < 1e-3);
        assert!((p1.0 - 50.0).abs() < 1e-3 && (p1.1 - 0.0).abs() < 1e-3);

        let g90 = LinearGradient::new(90.0, []);
        let (p0, p1) = g90.endpoints(0.0, 0.0, 100.0, 200.0);
        assert!((p0.0 - 0.0).abs() < 1e-3 && (p0.1 - 100.0).abs() < 1e-3);
        assert!((p1.0 - 100.0).abs() < 1e-3 && (p1.1 - 100.0).abs() < 1e-3);

        let g180 = LinearGradient::new(180.0, []);
        let (p0, p1) = g180.endpoints(0.0, 0.0, 100.0, 200.0);
        assert!((p0.0 - 50.0).abs() < 1e-3 && (p0.1 - 0.0).abs() < 1e-3);
        assert!((p1.0 - 50.0).abs() < 1e-3 && (p1.1 - 200.0).abs() < 1e-3);
    }
}
