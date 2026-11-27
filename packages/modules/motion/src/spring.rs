//! Closed-form, unconditionally stable damped harmonic oscillator spring physics
//! (`SpringSpec`, `SpringState`) and [`MotionPreference`] reduced-motion policy.
//!
//! All three physical damping regimes (underdamped, critically damped, overdamped) are solved
//! analytically in `f64` so arbitrary frame deltas (`1ms`, `8.3ms`, `16.6ms`, or a `250ms` GC
//! hitch) never diverge, explode, or drift, while mid-flight [`SpringState::retarget`] preserves
//! exact live velocity.

use std::f64::consts::TAU;

use nve_components::media::MediaEnv;

/// Default positional threshold below which a spring is considered settled.
pub const DEFAULT_REST_DISPLACEMENT: f32 = 1e-3;
/// Default velocity threshold (units/sec) below which a spring is considered settled.
pub const DEFAULT_REST_VELOCITY: f32 = 1e-3;

const MIN_MASS: f64 = 1e-4;
const MIN_STIFFNESS: f64 = 1e-4;
const CRITICAL_EPS: f64 = 1e-6;

/// System/user motion preference driving reduced-motion snapping across springs and transitions.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MotionPreference {
    #[default]
    Full = 0,
    Reduce = 1,
}

impl MotionPreference {
    /// Converts a boolean `prefers-reduced-motion` flag into [`MotionPreference`].
    #[inline]
    pub const fn from_reduced_motion(reduced: bool) -> Self {
        if reduced {
            Self::Reduce
        } else {
            Self::Full
        }
    }

    /// Derives [`MotionPreference`] directly from a live [`MediaEnv`] snapshot.
    #[inline]
    pub const fn from_media_env(env: &MediaEnv) -> Self {
        Self::from_reduced_motion(env.reduced_motion)
    }

    /// Returns `true` when reduced motion is active.
    #[inline]
    pub const fn is_reduced(self) -> bool {
        matches!(self, Self::Reduce)
    }
}

/// Physical spring parameters for an interruptible damped harmonic oscillator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringSpec {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
    pub rest_displacement: f32,
    pub rest_velocity: f32,
}

impl Default for SpringSpec {
    #[inline]
    fn default() -> Self {
        Self::snappy()
    }
}

impl SpringSpec {
    /// Creates a sanitized [`SpringSpec`] with default settle thresholds.
    #[inline]
    pub fn new(stiffness: f32, damping: f32, mass: f32) -> Self {
        Self {
            stiffness: sanitize_positive(stiffness, 400.0),
            damping: sanitize_non_negative(damping, 30.0),
            mass: sanitize_positive(mass, 1.0),
            rest_displacement: DEFAULT_REST_DISPLACEMENT,
            rest_velocity: DEFAULT_REST_VELOCITY,
        }
    }

    /// Configures custom position and velocity settle thresholds.
    #[inline]
    pub fn with_rest_thresholds(mut self, displacement: f32, velocity: f32) -> Self {
        self.rest_displacement = sanitize_positive(displacement, DEFAULT_REST_DISPLACEMENT);
        self.rest_velocity = sanitize_positive(velocity, DEFAULT_REST_VELOCITY);
        self
    }

    /// Crisp, responsive UI spring (slight underdamping, `zeta ~ 0.75`).
    #[inline]
    pub const fn snappy() -> Self {
        Self {
            stiffness: 400.0,
            damping: 30.0,
            mass: 1.0,
            rest_displacement: DEFAULT_REST_DISPLACEMENT,
            rest_velocity: DEFAULT_REST_VELOCITY,
        }
    }

    /// Smooth, low-overshoot spring for larger layout or sheet transitions (`zeta ~ 0.91`).
    #[inline]
    pub const fn gentle() -> Self {
        Self {
            stiffness: 120.0,
            damping: 20.0,
            mass: 1.0,
            rest_displacement: DEFAULT_REST_DISPLACEMENT,
            rest_velocity: DEFAULT_REST_VELOCITY,
        }
    }

    /// Expressive, bouncy spring with pronounced overshoot (`zeta ~ 0.39`).
    #[inline]
    pub const fn bouncy() -> Self {
        Self {
            stiffness: 320.0,
            damping: 14.0,
            mass: 1.0,
            rest_displacement: DEFAULT_REST_DISPLACEMENT,
            rest_velocity: DEFAULT_REST_VELOCITY,
        }
    }

    /// Fast, nearly critically damped spring with virtually zero overshoot (`zeta ~ 0.98`).
    #[inline]
    pub const fn stiff() -> Self {
        Self {
            stiffness: 600.0,
            damping: 48.0,
            mass: 1.0,
            rest_displacement: DEFAULT_REST_DISPLACEMENT,
            rest_velocity: DEFAULT_REST_VELOCITY,
        }
    }

    /// Apple SwiftUI-style intuitive spring parameterization:
    ///
    /// - `duration_s`: perceptual settle duration in seconds (clamped to `>= 0.01`).
    /// - `bounce`: bounciness in `[-1.0, 1.0]` (`0.0` = critically damped, `> 0.0` = underdamped
    ///   with overshoot, `< 0.0` = overdamped). Clamped internally to `[-0.95, 0.99]` so damping
    ///   stays strictly positive and finite.
    pub fn from_duration_bounce(duration_s: f32, bounce: f32) -> Self {
        let duration = if duration_s.is_finite() && duration_s > 0.0 {
            f64::from(duration_s).max(0.01)
        } else {
            0.35
        };
        let b = if bounce.is_finite() {
            f64::from(bounce).clamp(-0.95, 0.99)
        } else {
            0.0
        };
        let omega0 = TAU / duration;
        let stiffness = omega0 * omega0;
        let damping = if b >= 0.0 {
            2.0 * omega0 * (1.0 - b)
        } else {
            2.0 * omega0 / (1.0 + b)
        };
        Self {
            stiffness: stiffness as f32,
            damping: damping as f32,
            mass: 1.0,
            rest_displacement: DEFAULT_REST_DISPLACEMENT,
            rest_velocity: DEFAULT_REST_VELOCITY,
        }
    }

    /// Natural angular frequency `omega_0 = sqrt(stiffness / mass)` in rad/s.
    #[inline]
    pub fn angular_frequency(&self) -> f32 {
        let k = f64::from(self.stiffness).max(MIN_STIFFNESS);
        let m = f64::from(self.mass).max(MIN_MASS);
        (k / m).sqrt() as f32
    }

    /// Time in seconds for a step response to stay within `0.1 %` of its target: the slowest
    /// decay envelope (`zeta * omega_0` underdamped, the slow root overdamped, `(1 + x) e^-x`
    /// critically damped) reaching `1e-3`, clamped to `[0.01, 10]`.
    pub fn settle_duration_s(&self) -> f32 {
        const LN_1E3: f64 = 6.907_755_278_982_137;
        const CRITICAL_X: f64 = 9.233_413; // (1 + x) e^-x = 1e-3
        let omega = f64::from(self.angular_frequency());
        let zeta = f64::from(self.damping_ratio());
        let secs = if (zeta - 1.0).abs() <= 1e-3 {
            CRITICAL_X / omega
        } else if zeta < 1.0 {
            LN_1E3 / (zeta * omega).max(1e-6)
        } else {
            LN_1E3 / (omega * (zeta - (zeta * zeta - 1.0).sqrt())).max(1e-6)
        };
        secs.clamp(0.01, 10.0) as f32
    }

    /// Dimensionless damping ratio `zeta = damping / (2 * sqrt(stiffness * mass))`.
    /// `< 1` is underdamped, `== 1` is critically damped, `> 1` is overdamped.
    #[inline]
    pub fn damping_ratio(&self) -> f32 {
        let k = f64::from(self.stiffness).max(MIN_STIFFNESS);
        let m = f64::from(self.mass).max(MIN_MASS);
        let c = f64::from(self.damping).max(0.0);
        (c / (2.0 * (k * m).sqrt())) as f32
    }
}

/// Live scalar spring state (`value`, `velocity`, `target`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpringState {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
}

impl SpringState {
    /// Creates an already-settled spring at `value` (`velocity = 0.0`, `target = value`).
    #[inline]
    pub const fn settled(value: f32) -> Self {
        Self {
            value,
            velocity: 0.0,
            target: value,
        }
    }

    /// Creates a spring starting at `value` with zero initial velocity moving toward `target`.
    #[inline]
    pub const fn new(value: f32, target: f32) -> Self {
        Self {
            value,
            velocity: 0.0,
            target,
        }
    }

    /// Creates a spring with explicit initial `value`, `velocity`, and `target`.
    #[inline]
    pub const fn with_velocity(value: f32, velocity: f32, target: f32) -> Self {
        Self {
            value,
            velocity,
            target,
        }
    }

    /// Retargets this spring mid-flight to `new_target` while preserving exact `value` and
    /// `velocity` (zero jump, full physical momentum conservation).
    #[inline]
    pub fn retarget(&mut self, new_target: f32) {
        if new_target.is_finite() {
            self.target = new_target;
        }
    }

    /// Snaps immediately to `target` with zero velocity.
    #[inline]
    pub fn snap_to_target(&mut self) {
        if !self.target.is_finite() {
            self.target = 0.0;
        }
        self.value = self.target;
        self.velocity = 0.0;
    }

    /// Returns `true` when displacement from `target` and velocity are both within `spec`'s
    /// rest thresholds, allowing hosts/display-links to drop to 0% idle CPU/GPU.
    #[inline]
    pub fn is_settled(&self, spec: &SpringSpec) -> bool {
        if !self.value.is_finite() || !self.velocity.is_finite() || !self.target.is_finite() {
            return true;
        }
        let rest_d = sanitize_positive(spec.rest_displacement, DEFAULT_REST_DISPLACEMENT);
        let rest_v = sanitize_positive(spec.rest_velocity, DEFAULT_REST_VELOCITY);
        (self.value - self.target).abs() <= rest_d && self.velocity.abs() <= rest_v
    }

    /// Advances the spring analytically by `dt_s` seconds under `spec` and `preference`.
    ///
    /// - Under [`MotionPreference::Reduce`], snaps directly to `target` (`velocity = 0.0`) and
    ///   returns `true` so completion logic always fires without stalling.
    /// - Under [`MotionPreference::Full`], evaluates the exact closed-form damped harmonic
    ///   oscillator (underdamped, critically damped, or overdamped) in `f64`, snapping cleanly to
    ///   `(target, 0.0)` once settled.
    ///
    /// Returns `true` when the spring is settled after this step.
    pub fn step(&mut self, spec: &SpringSpec, dt_s: f32, preference: MotionPreference) -> bool {
        if !self.target.is_finite() {
            self.target = 0.0;
        }
        if preference.is_reduced() || !self.value.is_finite() || !self.velocity.is_finite() {
            self.snap_to_target();
            return true;
        }
        if self.is_settled(spec) {
            self.snap_to_target();
            return true;
        }
        if !dt_s.is_finite() || dt_s <= 0.0 {
            return false;
        }

        let t = f64::from(dt_s);
        let m = if spec.mass.is_finite() {
            f64::from(spec.mass).max(MIN_MASS)
        } else {
            1.0
        };
        let k = if spec.stiffness.is_finite() {
            f64::from(spec.stiffness).max(MIN_STIFFNESS)
        } else {
            400.0
        };
        let c = if spec.damping.is_finite() {
            f64::from(spec.damping).max(0.0)
        } else {
            30.0
        };

        let target = f64::from(self.target);
        let x0 = f64::from(self.value) - target;
        let v0 = f64::from(self.velocity);

        let omega0 = (k / m).sqrt();
        let zeta = c / (2.0 * (k * m).sqrt());

        let (x, v) = if zeta < 1.0 - CRITICAL_EPS {
            // Underdamped: decaying sinusoid.
            let alpha = zeta * omega0;
            let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
            let decay = (-alpha * t).exp();
            let a = x0;
            let b = (v0 + alpha * x0) / omega_d;
            let (sin_wt, cos_wt) = (omega_d * t).sin_cos();
            let x = decay * (a * cos_wt + b * sin_wt);
            let v = decay
                * ((b * omega_d - a * alpha) * cos_wt - (a * omega_d + b * alpha) * sin_wt);
            (x, v)
        } else if (zeta - 1.0).abs() <= CRITICAL_EPS {
            // Critically damped: fastest monotonic settle with zero oscillation.
            let decay = (-omega0 * t).exp();
            let a = x0;
            let b = v0 + omega0 * x0;
            let x = decay * (a + b * t);
            let v = decay * (b - omega0 * (a + b * t));
            (x, v)
        } else {
            // Overdamped: sum of two strictly negative real exponentials.
            // Rationalize r1 to avoid catastrophic cancellation at high damping ratios.
            let sqrt_term = (zeta * zeta - 1.0).sqrt();
            let gamma = omega0 * sqrt_term;
            let r1 = -omega0 / (zeta + sqrt_term);
            let r2 = -omega0 * (zeta + sqrt_term);
            let inv_2gamma = 0.5 / gamma;
            let c1 = (v0 - r2 * x0) * inv_2gamma;
            let c2 = (r1 * x0 - v0) * inv_2gamma;
            let e1 = (r1 * t).exp();
            let e2 = (r2 * t).exp();
            let x = c1 * e1 + c2 * e2;
            let v = c1 * r1 * e1 + c2 * r2 * e2;
            (x, v)
        };

        let prev_val = self.value;
        let next_val = (target + x) as f32;
        let next_vel = v as f32;
        if !next_val.is_finite() || !next_vel.is_finite() {
            self.snap_to_target();
            return true;
        }
        self.value = next_val;
        self.velocity = next_vel;

        // Guard against f32 ULP limit cycles at high target magnitudes + tiny `dt` (e.g. 1000Hz)
        // where `v * dt` rounds below half an ULP while the analytic spring is decaying toward
        // `target` within `max(rest_displacement, 64 * eps * |target|)`.
        let rest_d = sanitize_positive(spec.rest_displacement, DEFAULT_REST_DISPLACEMENT);
        let ulp_tol = rest_d.max(self.target.abs() * f32::EPSILON * 64.0);
        let ulp_stalled = next_val == prev_val
            && x.abs() <= x0.abs()
            && (next_val - self.target).abs() <= ulp_tol
            && (f64::from(next_vel).abs() / omega0) <= f64::from(ulp_tol);

        if self.is_settled(spec) || ulp_stalled {
            self.snap_to_target();
            true
        } else {
            false
        }
    }
}

#[inline]
fn sanitize_positive(v: f32, fallback: f32) -> f32 {
    if v.is_finite() && v > 0.0 {
        v
    } else {
        fallback
    }
}

#[inline]
fn sanitize_non_negative(v: f32, fallback: f32) -> f32 {
    if v.is_finite() && v >= 0.0 {
        v
    } else {
        fallback
    }
}
