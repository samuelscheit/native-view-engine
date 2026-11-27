//! Timing curves ([`EasingCurve`]): CSS keyword / `cubic-bezier` curves solved exactly, and
//! physical springs normalized over their settle duration.

use crate::spring::{MotionPreference, SpringSpec, SpringState};

/// Timing function mapping normalized progress `t in [0, 1]` to eased progress.
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
    /// Damped spring step response from `0` to `1`, time-scaled so `t = 1` is the spring's
    /// [`SpringSpec::settle_duration_s`] (overshoot is preserved; `t >= 1` is exactly `1`).
    Spring(SpringSpec),
}

impl EasingCurve {
    /// Evaluates the curve at normalized progress `t` (`<= 0` → `0`, `>= 1` → `1`).
    pub fn sample(&self, t: f32) -> f32 {
        if !t.is_finite() || t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        match self.sanitized() {
            Self::Linear => t,
            Self::Ease => solve_cubic_bezier(0.25, 0.1, 0.25, 1.0, t),
            Self::EaseIn => solve_cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
            Self::EaseOut => solve_cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
            Self::EaseInOut => solve_cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
            Self::CubicBezier(x1, y1, x2, y2) => solve_cubic_bezier(x1, y1, x2, y2, t),
            Self::Spring(spec) => {
                let mut state = SpringState::new(0.0, 1.0);
                state.step(&spec, t * spec.settle_duration_s(), MotionPreference::Full);
                state.value
            }
        }
    }

    /// Canonical form: `cubic-bezier` x control points clamped to `[0, 1]` and non-finite
    /// coordinates zeroed (the exact curve every backend evaluates or emits).
    pub fn sanitized(self) -> Self {
        match self {
            Self::CubicBezier(x1, y1, x2, y2) => {
                Self::CubicBezier(unit(x1), finite(y1), unit(x2), finite(y2))
            }
            other => other,
        }
    }
}

#[inline]
fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

#[inline]
fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// Evaluates a CSS cubic Bezier `(0,0) -> (x1,y1) -> (x2,y2) -> (1,1)` at `x = t` using
/// bracketed Newton-Raphson with guaranteed bisection fallback (`< 1e-6` accuracy).
fn solve_cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let (x1, y1, x2, y2, t) = (
        f64::from(x1),
        f64::from(y1),
        f64::from(x2),
        f64::from(y2),
        f64::from(t),
    );
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
            return sample_y(u) as f32;
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
    sample_y(u) as f32
}
