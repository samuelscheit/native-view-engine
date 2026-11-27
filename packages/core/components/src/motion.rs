//! Declarative style transitions ([`TransitionProperty`], [`TransitionSpec`], [`EasingCurve`]),
//! premultiplied [`ColorRgba`] & [`StyleProps`] interpolation, and re-exports of the zero-idle-CPU
//! [`MotionClock`] / [`MotionGroup`] track manager.

use nve_abi::num::{write_f32, write_u64};
use nve_abi::ColorRgba;

use crate::snapshots::VisualSnapshot;

pub use crate::motion_clock::{
    MotionClock, MotionCompletion, MotionGroup, MotionKey, MotionTrack, TimedTransition,
};
pub use crate::spring::{
    MotionPreference, SpringSpec, SpringState, DEFAULT_REST_DISPLACEMENT, DEFAULT_REST_VELOCITY,
};

/// Standard CSS easing curve with exact Newton-Raphson + bisection evaluation and canonical
/// injection-free CSS formatting.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum EasingCurve {
    Linear,
    #[default]
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Standard CSS `cubic-bezier(x1, y1, x2, y2)` with `x1, x2` clamped to `[0.0, 1.0]`.
    CubicBezier(f32, f32, f32, f32),
}

impl EasingCurve {
    /// Evaluates the easing curve at normalized progress `t in [0.0, 1.0]`.
    pub fn sample(&self, t: f32) -> f32 {
        if !t.is_finite() || t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        match *self {
            Self::Linear => t,
            Self::Ease => solve_cubic_bezier(0.25, 0.1, 0.25, 1.0, f64::from(t)) as f32,
            Self::EaseIn => solve_cubic_bezier(0.42, 0.0, 1.0, 1.0, f64::from(t)) as f32,
            Self::EaseOut => solve_cubic_bezier(0.0, 0.0, 0.58, 1.0, f64::from(t)) as f32,
            Self::EaseInOut => solve_cubic_bezier(0.42, 0.0, 0.58, 1.0, f64::from(t)) as f32,
            Self::CubicBezier(x1, y1, x2, y2) => {
                let cx1 = sanitize_unit(x1);
                let cy1 = sanitize_finite(y1);
                let cx2 = sanitize_unit(x2);
                let cy2 = sanitize_finite(y2);
                solve_cubic_bezier(cx1, cy1, cx2, cy2, f64::from(t)) as f32
            }
        }
    }

    /// Writes the canonical CSS timing-function keyword or `cubic-bezier(...)` into `out`.
    pub fn write_css_into(&self, out: &mut String) {
        match *self {
            Self::Linear => out.push_str("linear"),
            Self::Ease => out.push_str("ease"),
            Self::EaseIn => out.push_str("ease-in"),
            Self::EaseOut => out.push_str("ease-out"),
            Self::EaseInOut => out.push_str("ease-in-out"),
            Self::CubicBezier(x1, y1, x2, y2) => {
                out.push_str("cubic-bezier(");
                write_f32(out, sanitize_unit(x1) as f32);
                out.push(',');
                write_f32(out, sanitize_finite(y1) as f32);
                out.push(',');
                write_f32(out, sanitize_unit(x2) as f32);
                out.push(',');
                write_f32(out, sanitize_finite(y2) as f32);
                out.push(')');
            }
        }
    }

    /// Formats this easing curve as a canonical CSS timing-function string.
    pub fn to_css(&self) -> String {
        let mut out = String::with_capacity(28);
        self.write_css_into(&mut out);
        out
    }
}

/// Strictly typed animatable CSS/style property (`#[repr(u8)]`, zero arbitrary strings).
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TransitionProperty {
    #[default]
    All = 0,
    BackgroundColor = 1,
    Color = 2,
    BorderColor = 3,
    Opacity = 4,
    BorderRadius = 5,
    Transform = 6,
    Width = 7,
    Height = 8,
    Padding = 9,
    Gap = 10,
}

impl TransitionProperty {
    /// Returns the canonical CSS property identifier for this variant.
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::BackgroundColor => "background-color",
            Self::Color => "color",
            Self::BorderColor => "border-color",
            Self::Opacity => "opacity",
            Self::BorderRadius => "border-radius",
            Self::Transform => "transform",
            Self::Width => "width",
            Self::Height => "height",
            Self::Padding => "padding",
            Self::Gap => "gap",
        }
    }

    /// Returns `true` when `self` governs `target` (`All` matches every property).
    #[inline]
    pub const fn matches(self, target: Self) -> bool {
        matches!(self, Self::All) || (self as u8) == (target as u8)
    }
}

/// Declarative, zero-serialization style transition specification compiling to native CSS
/// `transition:` on Web and driving [`MotionClock`] / Skia interpolation natively.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransitionSpec {
    pub property: TransitionProperty,
    pub duration_ms: u32,
    pub delay_ms: u32,
    pub easing: EasingCurve,
    pub reduced_motion_instant: bool,
}

impl Default for TransitionSpec {
    #[inline]
    fn default() -> Self {
        Self::all(180)
    }
}

impl TransitionSpec {
    /// Creates a transition for `property` lasting `duration_ms` with `EaseOut` and
    /// `reduced_motion_instant = true`.
    #[inline]
    pub const fn new(property: TransitionProperty, duration_ms: u32) -> Self {
        Self {
            property,
            duration_ms,
            delay_ms: 0,
            easing: EasingCurve::EaseOut,
            reduced_motion_instant: true,
        }
    }

    /// Shorthand for `TransitionSpec::new(TransitionProperty::All, duration_ms)`.
    #[inline]
    pub const fn all(duration_ms: u32) -> Self {
        Self::new(TransitionProperty::All, duration_ms)
    }

    /// Sets the start delay in milliseconds.
    #[inline]
    pub const fn delay_ms(mut self, delay_ms: u32) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    /// Sets the timing [`EasingCurve`].
    #[inline]
    pub const fn easing(mut self, easing: EasingCurve) -> Self {
        self.easing = easing;
        self
    }

    /// Controls whether `prefers-reduced-motion: reduce` / [`MotionPreference::Reduce`] snaps
    /// this transition instantaneously (`true` by default).
    #[inline]
    pub const fn reduced_motion_instant(mut self, instant: bool) -> Self {
        self.reduced_motion_instant = instant;
        self
    }

    #[inline]
    pub fn duration_s(&self) -> f32 {
        self.duration_ms as f32 / 1000.0
    }

    #[inline]
    pub fn delay_s(&self) -> f32 {
        self.delay_ms as f32 / 1000.0
    }

    /// Writes one compact CSS `transition` item (e.g. `background-color 150ms ease-out`).
    pub fn write_css_entry_into(&self, out: &mut String) {
        out.push_str(self.property.as_css());
        out.push(' ');
        write_u64(out, u64::from(self.duration_ms));
        out.push_str("ms ");
        self.easing.write_css_into(out);
        if self.delay_ms > 0 {
            out.push(' ');
            write_u64(out, u64::from(self.delay_ms));
            out.push_str("ms");
        }
    }

    /// Writes a full compact `transition:...;` CSS declaration for `specs` (no-op if empty).
    pub fn write_css_list_into(specs: &[Self], out: &mut String) {
        if specs.is_empty() {
            return;
        }
        out.push_str("transition:");
        for (idx, spec) in specs.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            spec.write_css_entry_into(out);
        }
        out.push(';');
    }
}

/// Resolves the winning [`TransitionSpec`] for `prop` using CSS last-match-wins order.
pub fn resolve_transition_for(
    specs: &[TransitionSpec],
    prop: TransitionProperty,
) -> Option<TransitionSpec> {
    specs.iter().rev().find(|s| s.property.matches(prop)).copied()
}

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
    let r_pm = f32::from(from.r) * a0 + (f32::from(to.r) * a1 - f32::from(from.r) * a0) * t;
    let g_pm = f32::from(from.g) * a0 + (f32::from(to.g) * a1 - f32::from(from.g) * a0) * t;
    let b_pm = f32::from(from.b) * a0 + (f32::from(to.b) * a1 - f32::from(from.b) * a0) * t;
    ColorRgba::rgba(
        (r_pm / a).round().clamp(0.0, 255.0) as u8,
        (g_pm / a).round().clamp(0.0, 255.0) as u8,
        (b_pm / a).round().clamp(0.0, 255.0) as u8,
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

#[inline]
fn sanitize_unit(v: f32) -> f64 {
    if v.is_finite() {
        f64::from(v).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[inline]
fn sanitize_finite(v: f32) -> f64 {
    if v.is_finite() {
        f64::from(v)
    } else {
        0.0
    }
}

/// Evaluates a CSS cubic Bezier `(0,0) -> (x1,y1) -> (x2,y2) -> (1,1)` at `x = t` using
/// bracketed Newton-Raphson with guaranteed bisection fallback (`< 1e-6` accuracy).
fn solve_cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;

    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;

    let sample_x = |u: f64| ((ax * u + bx) * u + cx) * u;
    let sample_dx = |u: f64| (3.0 * ax * u + 2.0 * bx) * u + cx;
    let sample_y = |u: f64| ((ay * u + by) * u + cy) * u;

    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;
    let mut u = t;

    for _ in 0..8 {
        let err = sample_x(u) - t;
        if err.abs() < 1e-7 {
            return sample_y(u);
        }
        if err < 0.0 {
            lo = u;
        } else {
            hi = u;
        }
        let dx = sample_dx(u);
        if dx.abs() > 1e-6 {
            let candidate = u - err / dx;
            if candidate > lo && candidate < hi {
                u = candidate;
                continue;
            }
        }
        u = 0.5 * (lo + hi);
    }

    for _ in 0..24 {
        let err = sample_x(u) - t;
        if err.abs() < 1e-7 || (hi - lo) < 1e-7 {
            break;
        }
        if err < 0.0 {
            lo = u;
        } else {
            hi = u;
        }
        u = 0.5 * (lo + hi);
    }
    sample_y(u)
}
