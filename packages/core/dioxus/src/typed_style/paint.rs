//! Typed zero-serialization background gradient builders on [`TypedStyle`].

use nve_tables::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};

use super::TypedStyle;

impl TypedStyle {
    /// Sets a typed [`BackgroundFill`] (`Solid`, `Linear`, or `Radial`) with zero CSS string
    /// serialization.
    #[inline]
    pub fn background_fill(mut self, fill: BackgroundFill) -> Self {
        self.background_fill = Some(fill);
        self
    }

    /// Sets a typed CSS-aligned [`LinearGradient`] (`0°` = bottom→top, `90°` = left→right,
    /// `180°` = top→bottom).
    #[inline]
    pub fn linear_gradient(
        mut self,
        angle_deg: f32,
        stops: impl Into<Vec<GradientStop>>,
    ) -> Self {
        self.background_fill = Some(BackgroundFill::Linear(LinearGradient::new(
            angle_deg, stops,
        )));
        self
    }

    /// Sets a typed [`RadialGradient`] with normalized center (`0.0..=1.0`) and `radius`
    /// relative to box half-extent.
    #[inline]
    pub fn radial_gradient(
        mut self,
        center_x: f32,
        center_y: f32,
        radius: f32,
        stops: impl Into<Vec<GradientStop>>,
    ) -> Self {
        self.background_fill = Some(BackgroundFill::Radial(RadialGradient::new(
            center_x, center_y, radius, stops,
        )));
        self
    }
}
