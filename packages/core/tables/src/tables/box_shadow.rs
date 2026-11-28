//! Typed `box-shadow` value shared by the `BoxShadows` resource, web CSS emission, and Skia
//! rasterization (formatting lives in `nve-backend-web`).

use nve_store::Rgba;

/// One drop (or inset) shadow in logical points.
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
    /// Outer shadow offset by `(offset_x, offset_y)` with `blur_radius` (`>= 0`); non-finite
    /// inputs become `0` so the value always passes `BoxShadows` validation.
    #[inline]
    pub const fn new(offset_x: f32, offset_y: f32, blur_radius: f32, color: Rgba) -> Self {
        Self {
            offset_x: finite_or_zero(offset_x),
            offset_y: finite_or_zero(offset_y),
            blur_radius: finite_or_zero(blur_radius).max(0.0),
            spread_radius: 0.0,
            color,
            inset: false,
        }
    }

    #[inline]
    pub const fn spread(mut self, spread_radius: f32) -> Self {
        self.spread_radius = finite_or_zero(spread_radius);
        self
    }

    #[inline]
    pub const fn inset(mut self) -> Self {
        self.inset = true;
        self
    }
}

#[inline]
const fn finite_or_zero(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}
