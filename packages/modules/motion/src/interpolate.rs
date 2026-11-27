//! Safe scalar, premultiplied-color, [`VisualSnapshot`], and [`Transform2D`] interpolation.

use nve_abi::ColorRgba;
use nve_tables::snapshots::VisualSnapshot;
use nve_tables::transform::Transform2D;

use crate::table::TransitionProperty;

/// Linearly interpolates between `from` and `to` at `t` (safe for non-finite inputs).
#[inline]
pub fn lerp_f32(from: f32, to: f32, t: f32) -> f32 {
    if !t.is_finite() || t <= 0.0 {
        return from;
    }
    if (t - 1.0).abs() <= f32::EPSILON {
        return to;
    }
    from + (to - from) * t
}

/// Premultiplied-alpha [`ColorRgba`] interpolation avoiding dark fringes when fading to or from
/// transparent colors, with exact endpoints and safe clamping under spring overshoot.
pub fn lerp_color_rgba(from: ColorRgba, to: ColorRgba, t: f32) -> ColorRgba {
    if from == to || !t.is_finite() || t <= 0.0 {
        return from;
    }
    if t >= 1.0 {
        return to;
    }
    let a0 = f32::from(from.a) / 255.0;
    let a1 = f32::from(to.a) / 255.0;
    let a = (a0 + (a1 - a0) * t).clamp(0.0, 1.0);
    if a <= 1e-6 {
        return ColorRgba::TRANSPARENT;
    }
    let pm = |c0: u8, c1: u8| {
        let v = f32::from(c0) * a0 + (f32::from(c1) * a1 - f32::from(c0) * a0) * t;
        (v / a).round().clamp(0.0, 255.0) as u8
    };
    ColorRgba::rgba(
        pm(from.r, to.r),
        pm(from.g, to.g),
        pm(from.b, to.b),
        (a * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

/// Interpolates all continuous visual fields of [`VisualSnapshot`] at progress `t`.
pub fn lerp_style_props(from: &VisualSnapshot, to: &VisualSnapshot, t: f32) -> VisualSnapshot {
    if !t.is_finite() || t <= 0.0 {
        return *from;
    }
    if t >= 1.0 {
        return *to;
    }
    VisualSnapshot {
        background: lerp_color_rgba(from.background, to.background, t),
        text_color: lerp_color_rgba(from.text_color, to.text_color, t),
        border_color: lerp_color_rgba(from.border_color, to.border_color, t),
        border_width: lerp_f32(from.border_width, to.border_width, t).max(0.0),
        border_radius: lerp_f32(from.border_radius, to.border_radius, t).max(0.0),
        opacity: lerp_f32(from.opacity, to.opacity, t).clamp(0.0, 1.0),
        font_size: lerp_f32(from.font_size, to.font_size, t).max(1.0),
        z_index: if t >= 0.5 { to.z_index } else { from.z_index },
    }
}

/// Interpolates one visual [`TransitionProperty`] from `from` toward `to` into `out`.
pub fn lerp_style_property(
    from: &VisualSnapshot,
    to: &VisualSnapshot,
    prop: TransitionProperty,
    t: f32,
    out: &mut VisualSnapshot,
) {
    match prop {
        TransitionProperty::All => *out = lerp_style_props(from, to, t),
        TransitionProperty::BackgroundColor => {
            out.background = lerp_color_rgba(from.background, to.background, t);
        }
        TransitionProperty::Color => {
            out.text_color = lerp_color_rgba(from.text_color, to.text_color, t);
        }
        TransitionProperty::BorderColor => {
            out.border_color = lerp_color_rgba(from.border_color, to.border_color, t);
        }
        TransitionProperty::BorderRadius => {
            out.border_radius = lerp_f32(from.border_radius, to.border_radius, t).max(0.0);
        }
        TransitionProperty::Opacity => {
            out.opacity = lerp_f32(from.opacity, to.opacity, t).clamp(0.0, 1.0);
        }
        TransitionProperty::Transform
        | TransitionProperty::Width
        | TransitionProperty::Height
        | TransitionProperty::Padding
        | TransitionProperty::Gap => {}
    }
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
    Transform2D {
        translate_x: lerp_f32(from.translate_x, to.translate_x, t),
        translate_y: lerp_f32(from.translate_y, to.translate_y, t),
        scale_x: lerp_f32(from.scale_x, to.scale_x, t),
        scale_y: lerp_f32(from.scale_y, to.scale_y, t),
        rotate_deg: lerp_f32(from.rotate_deg, to.rotate_deg, t),
        origin_x: lerp_f32(from.origin_x, to.origin_x, t).clamp(0.0, 1.0),
        origin_y: lerp_f32(from.origin_y, to.origin_y, t).clamp(0.0, 1.0),
    }
}
