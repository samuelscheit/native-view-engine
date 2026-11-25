//! Zero-serialization typed 2D affine transform (`Transform2D`) with pure-Rust matrix
//! evaluation, inverse-matrix hit-testing, smooth interpolation, and canonical CSS emission.
//!
//! Matrix representation `[a, b, c, d, tx, ty]` follows the standard 2D affine convention
//! (matching CSS `matrix(a, b, c, d, tx, ty)` and Skia's `Matrix::new_all(a, c, tx, b, d, ty, 0, 0, 1)`):
//! - `x' = a * x + c * y + tx`
//! - `y' = b * x + d * y + ty`
//!
//! Transform composition order matches CSS `transform: translate(tx, ty) scale(sx, sy) rotate(deg)`
//! around `(origin_x * width, origin_y * height)`:
//! `M = T(origin) * T(tx, ty) * S(sx, sy) * R(rotate_deg) * T(-origin)`.

use nve_abi::num::write_f32;

const EPS: f32 = 1e-6;
const DET_EPS: f32 = 1e-8;

#[inline]
fn sanitize(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

/// Strictly typed 2D transform (`translate` / `scale` / `rotate` + normalized `origin`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform2D {
    pub translate_x: f32,
    pub translate_y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotate_deg: f32,
    pub origin_x: f32,
    pub origin_y: f32,
}

impl Default for Transform2D {
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform2D {
    /// Identity transform (`translate(0, 0) scale(1, 1) rotate(0deg)` centered at `0.5, 0.5`).
    pub const IDENTITY: Self = Self {
        translate_x: 0.0,
        translate_y: 0.0,
        scale_x: 1.0,
        scale_y: 1.0,
        rotate_deg: 0.0,
        origin_x: 0.5,
        origin_y: 0.5,
    };

    #[inline]
    pub fn translate(x: f32, y: f32) -> Self {
        Self::IDENTITY.with_translate(x, y)
    }

    #[inline]
    pub fn scale(s: f32) -> Self {
        Self::IDENTITY.with_scale(s)
    }

    #[inline]
    pub fn scale_xy(sx: f32, sy: f32) -> Self {
        Self::IDENTITY.with_scale_xy(sx, sy)
    }

    #[inline]
    pub fn rotate(deg: f32) -> Self {
        Self::IDENTITY.with_rotate(deg)
    }

    #[inline]
    pub fn with_translate(mut self, x: f32, y: f32) -> Self {
        self.translate_x = sanitize(x, 0.0);
        self.translate_y = sanitize(y, 0.0);
        self
    }

    #[inline]
    pub fn with_scale(mut self, s: f32) -> Self {
        let v = sanitize(s, 1.0);
        self.scale_x = v;
        self.scale_y = v;
        self
    }

    #[inline]
    pub fn with_scale_xy(mut self, sx: f32, sy: f32) -> Self {
        self.scale_x = sanitize(sx, 1.0);
        self.scale_y = sanitize(sy, 1.0);
        self
    }

    #[inline]
    pub fn with_rotate(mut self, deg: f32) -> Self {
        self.rotate_deg = sanitize(deg, 0.0);
        self
    }

    #[inline]
    pub fn with_origin(mut self, ox: f32, oy: f32) -> Self {
        self.origin_x = sanitize(ox, 0.5).clamp(0.0, 1.0);
        self.origin_y = sanitize(oy, 0.5).clamp(0.0, 1.0);
        self
    }

    /// Returns `true` when this transform produces the identity mapping.
    #[inline]
    pub fn is_identity(&self) -> bool {
        sanitize(self.translate_x, 0.0).abs() <= EPS
            && sanitize(self.translate_y, 0.0).abs() <= EPS
            && (sanitize(self.scale_x, 1.0) - 1.0).abs() <= EPS
            && (sanitize(self.scale_y, 1.0) - 1.0).abs() <= EPS
            && sanitize(self.rotate_deg, 0.0).abs() <= EPS
    }

    /// Returns `true` when `origin_x` or `origin_y` differs from the CSS default `50% 50%`.
    #[inline]
    pub fn has_custom_origin(&self) -> bool {
        (sanitize(self.origin_x, 0.5) - 0.5).abs() > 1e-5
            || (sanitize(self.origin_y, 0.5) - 0.5).abs() > 1e-5
    }

    /// Computes the 2D affine matrix `[a, b, c, d, tx, ty]` in element-local coordinates
    /// `(0..width, 0..height)`.
    #[inline]
    pub fn to_matrix_2d(&self, width: f32, height: f32) -> [f32; 6] {
        self.to_surface_matrix_2d(0.0, 0.0, width, height)
    }

    /// Computes the 2D affine matrix `[a, b, c, d, tx, ty]` in surface/viewport coordinates
    /// for a box whose top-left corner is `(rect_x, rect_y)` with size `(width, height)`.
    pub fn to_surface_matrix_2d(
        &self,
        rect_x: f32,
        rect_y: f32,
        width: f32,
        height: f32,
    ) -> [f32; 6] {
        let tx = sanitize(self.translate_x, 0.0);
        let ty = sanitize(self.translate_y, 0.0);
        let sx = sanitize(self.scale_x, 1.0);
        let sy = sanitize(self.scale_y, 1.0);
        let deg = sanitize(self.rotate_deg, 0.0);
        let ox = sanitize(self.origin_x, 0.5).clamp(0.0, 1.0);
        let oy = sanitize(self.origin_y, 0.5).clamp(0.0, 1.0);
        let w = sanitize(width, 0.0).max(0.0);
        let h = sanitize(height, 0.0).max(0.0);
        let rx = sanitize(rect_x, 0.0);
        let ry = sanitize(rect_y, 0.0);

        let (sin_r, cos_r) = if deg.abs() <= EPS {
            (0.0_f32, 1.0_f32)
        } else {
            deg.to_radians().sin_cos()
        };

        let a = sx * cos_r;
        let b = sy * sin_r;
        let c = -sx * sin_r;
        let d = sy * cos_r;

        let ox_px = rx + ox * w;
        let oy_px = ry + oy * h;
        let e = ox_px + tx - (a * ox_px + c * oy_px);
        let f = oy_px + ty - (b * ox_px + d * oy_px);
        [a, b, c, d, e, f]
    }

    /// Maps a point `(x, y)` in element-local coordinates `(0..width, 0..height)` forward.
    #[inline]
    pub fn map_point(&self, x: f32, y: f32, width: f32, height: f32) -> (f32, f32) {
        let m = self.to_matrix_2d(width, height);
        affine_map_point(&m, x, y)
    }

    /// Maps a transformed point `(x, y)` back into element-local coordinates `(0..width, 0..height)`.
    /// Returns `None` if the transform matrix is singular (e.g. `scale_x == 0` or `scale_y == 0`).
    #[inline]
    pub fn inverse_map_point(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<(f32, f32)> {
        let m = self.to_matrix_2d(width, height);
        affine_inverse_map_point(&m, x, y)
    }

    /// Maps a surface-space point `(px, py)` forward through this box's surface transform.
    #[inline]
    pub fn map_surface_point(
        &self,
        px: f32,
        py: f32,
        rect_x: f32,
        rect_y: f32,
        width: f32,
        height: f32,
    ) -> (f32, f32) {
        let m = self.to_surface_matrix_2d(rect_x, rect_y, width, height);
        affine_map_point(&m, px, py)
    }

    /// Maps a surface-space pointer coordinate `(px, py)` back into untransformed surface space
    /// for hit-testing against the element's layout rect `(rect_x, rect_y, width, height)`.
    #[inline]
    pub fn inverse_map_surface_point(
        &self,
        px: f32,
        py: f32,
        rect_x: f32,
        rect_y: f32,
        width: f32,
        height: f32,
    ) -> Option<(f32, f32)> {
        let m = self.to_surface_matrix_2d(rect_x, rect_y, width, height);
        affine_inverse_map_point(&m, px, py)
    }

    /// Appends canonical, injection-free CSS `transform:` (and optional `transform-origin:`)
    /// declarations into `out`.
    ///
    /// When `emit_identity_as_none` is `true` (used by sparse `StylePatch` overrides), an identity
    /// transform emits `transform:none;` so it cleanly resets a base transform.
    pub fn write_css_decls_into(&self, emit_identity_as_none: bool, out: &mut String) {
        let tx = sanitize(self.translate_x, 0.0);
        let ty = sanitize(self.translate_y, 0.0);
        let sx = sanitize(self.scale_x, 1.0);
        let sy = sanitize(self.scale_y, 1.0);
        let deg = sanitize(self.rotate_deg, 0.0);

        let has_t = tx.abs() > EPS || ty.abs() > EPS;
        let has_s = (sx - 1.0).abs() > EPS || (sy - 1.0).abs() > EPS;
        let has_r = deg.abs() > EPS;

        if !has_t && !has_s && !has_r {
            if emit_identity_as_none {
                out.push_str("transform:none;");
            }
        } else {
            out.push_str("transform:");
            let mut first = true;
            if has_t {
                out.push_str("translate(");
                write_css_px(tx, out);
                out.push(',');
                write_css_px(ty, out);
                out.push(')');
                first = false;
            }
            if has_s {
                if !first {
                    out.push(' ');
                }
                out.push_str("scale(");
                write_f32(out, sx);
                if (sx - sy).abs() > EPS {
                    out.push(',');
                    write_f32(out, sy);
                }
                out.push(')');
                first = false;
            }
            if has_r {
                if !first {
                    out.push(' ');
                }
                out.push_str("rotate(");
                write_f32(out, deg);
                out.push_str("deg)");
            }
            out.push(';');
        }

        if self.has_custom_origin() {
            let ox = sanitize(self.origin_x, 0.5).clamp(0.0, 1.0) * 100.0;
            let oy = sanitize(self.origin_y, 0.5).clamp(0.0, 1.0) * 100.0;
            out.push_str("transform-origin:");
            write_f32(out, ox);
            out.push_str("% ");
            write_f32(out, oy);
            out.push_str("%;");
        }
    }
}

#[inline]
fn write_css_px(v: f32, out: &mut String) {
    let start = out.len();
    write_f32(out, v);
    if &out[start..] != "0" {
        out.push_str("px");
    }
}

/// Applies 2D affine matrix `m = [a, b, c, d, tx, ty]` to `(x, y)`.
#[inline]
pub fn affine_map_point(m: &[f32; 6], x: f32, y: f32) -> (f32, f32) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// Applies the inverse of 2D affine matrix `m = [a, b, c, d, tx, ty]` to `(x, y)`.
#[inline]
pub fn affine_inverse_map_point(m: &[f32; 6], x: f32, y: f32) -> Option<(f32, f32)> {
    let det = m[0] * m[3] - m[1] * m[2];
    if !det.is_finite() || det.abs() < DET_EPS {
        return None;
    }
    let dx = x - m[4];
    let dy = y - m[5];
    let inv_x = (m[3] * dx - m[2] * dy) / det;
    let inv_y = (-m[1] * dx + m[0] * dy) / det;
    (inv_x.is_finite() && inv_y.is_finite()).then_some((inv_x, inv_y))
}

/// Multiplies two 2D affine matrices (`outer * inner`), so `map(concat(o, i), p) == map(o, map(i, p))`.
#[inline]
pub fn affine_concat(outer: &[f32; 6], inner: &[f32; 6]) -> [f32; 6] {
    [
        outer[0] * inner[0] + outer[2] * inner[1],
        outer[1] * inner[0] + outer[3] * inner[1],
        outer[0] * inner[2] + outer[2] * inner[3],
        outer[1] * inner[2] + outer[3] * inner[3],
        outer[0] * inner[4] + outer[2] * inner[5] + outer[4],
        outer[1] * inner[4] + outer[3] * inner[5] + outer[5],
    ]
}

/// Linearly interpolates every component of [`Transform2D`] at progress `t`.
#[inline]
pub fn lerp_transform_2d(from: &Transform2D, to: &Transform2D, t: f32) -> Transform2D {
    if !t.is_finite() || t <= 0.0 {
        return *from;
    }
    if (t - 1.0).abs() <= f32::EPSILON {
        return *to;
    }
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    Transform2D {
        translate_x: lerp(from.translate_x, to.translate_x),
        translate_y: lerp(from.translate_y, to.translate_y),
        scale_x: lerp(from.scale_x, to.scale_x),
        scale_y: lerp(from.scale_y, to.scale_y),
        rotate_deg: lerp(from.rotate_deg, to.rotate_deg),
        origin_x: lerp(from.origin_x, to.origin_x).clamp(0.0, 1.0),
        origin_y: lerp(from.origin_y, to.origin_y).clamp(0.0, 1.0),
    }
}
