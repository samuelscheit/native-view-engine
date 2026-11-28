//! Zero-serialization typed background fills (`BackgroundFill`, `LinearGradient`,
//! `RadialGradient`, `GradientStop`) with CSS-aligned angle geometry, premultiplied-alpha
//! stop sampling, deterministic hashing, and injection-free CSS gradient formatting.

use std::sync::Arc;

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

/// Clamps stop offsets to `0.0..=1.0` and stably sorts them in ascending offset order in place.
pub fn normalize_stops_in_place(stops: &mut [GradientStop]) {
    for s in stops.iter_mut() {
        s.offset = if s.offset.is_finite() {
            s.offset.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
}

/// Clamps stop offsets to `0.0..=1.0` and stably sorts them in ascending offset order.
pub fn normalize_stops(mut stops: Vec<GradientStop>) -> Vec<GradientStop> {
    normalize_stops_in_place(&mut stops);
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

    #[test]
    fn normalize_stops_in_place_clamps_sanitizes_and_stably_sorts() {
        let red = ColorRgba::rgb(255, 0, 0);
        let green = ColorRgba::rgb(0, 255, 0);
        let blue = ColorRgba::rgb(0, 0, 255);
        let yellow = ColorRgba::rgb(255, 255, 0);
        let mut stops = [
            GradientStop { offset: 1.5, color: blue },
            GradientStop { offset: 0.5, color: red },
            GradientStop { offset: f32::NAN, color: yellow },
            GradientStop { offset: 0.5, color: green },
            GradientStop { offset: -0.25, color: red },
        ];
        let via_vec = normalize_stops(stops.to_vec());
        normalize_stops_in_place(&mut stops);
        assert_eq!(stops.as_slice(), via_vec.as_slice());
        assert_eq!(stops[0], GradientStop { offset: 0.0, color: yellow });
        assert_eq!(stops[1], GradientStop { offset: 0.0, color: red });
        assert_eq!(stops[2], GradientStop { offset: 0.5, color: red });
        assert_eq!(stops[3], GradientStop { offset: 0.5, color: green });
        assert_eq!(stops[4], GradientStop { offset: 1.0, color: blue });
        assert!(stops_are_normalized(&stops));
    }
}
