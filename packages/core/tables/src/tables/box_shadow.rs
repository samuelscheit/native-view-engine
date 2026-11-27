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
    /// Outer shadow offset by `(offset_x, offset_y)` with `blur_radius` (clamped `>= 0`).
    #[inline]
    pub const fn new(offset_x: f32, offset_y: f32, blur_radius: f32, color: Rgba) -> Self {
        Self {
            offset_x,
            offset_y,
            blur_radius: if blur_radius > 0.0 { blur_radius } else { 0.0 },
            spread_radius: 0.0,
            color,
            inset: false,
        }
    }

    #[inline]
    pub const fn spread(mut self, spread_radius: f32) -> Self {
        self.spread_radius = spread_radius;
        self
    }

    #[inline]
    pub const fn inset(mut self) -> Self {
        self.inset = true;
        self
    }
}
