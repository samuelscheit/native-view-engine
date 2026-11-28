//! Standalone motion capability: timing curves, closed-form springs, the [`MotionClock`] track
//! slab, interpolation, and the declarative [`Motion`] (`nve.motion`) table with its
//! [`Transitions`] resource.
//!
//! Pure-spec root crate (zero backend or platform code): `nve-motion-web` compiles transitions
//! to native CSS, `nve-motion-skia` drives them on the Skia frame tick. Core crates and backends
//! never depend on this module.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod clock;
mod easing;
mod interpolate;
mod spring;
mod table;
#[cfg(test)]
mod tests;
mod tick_hook;

pub use clock::{MotionClock, MotionCompletion, MotionKey, MotionTrack, TimedTransition};
pub use easing::EasingCurve;
pub use interpolate::{
    lerp_color_rgba, lerp_f32, lerp_style_property, lerp_style_props, lerp_transform_2d,
};
pub use spring::{
    MotionPreference, SpringSpec, SpringState, DEFAULT_REST_DISPLACEMENT, DEFAULT_REST_VELOCITY,
};
pub use table::{
    resolve_transition_for, write_transitions, Motion, MotionMut, MotionRef, TransitionProperty,
    TransitionSpec, Transitions, TransitionsView,
};
pub use tick_hook::MotionRuntimeTickHook;
