//! Web DOM & SSR adapter for `nve-motion` ([`MotionWebSystem`]): zero-JS native CSS
//! `transition` compilation with springs expressed as CSS `linear()` easings and automatic
//! `@media (prefers-reduced-motion:reduce)` instant-snap handling.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use nve_abi::num::{write_f32, write_u64};
use nve_backend_web::css::format_content_class;
use nve_backend_web::media_css::{format_media_class, CompiledMediaRule};
use nve_backend_web::{ResolvedNodeStyle, WebTableSystem};
use nve_motion::{EasingCurve, Motion, TransitionProperty, TransitionSpec, Transitions};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};

#[cfg(test)]
mod tests;

/// Segments of a spring's CSS `linear()` easing (25 stops at 3 decimals: smooth overshoot,
/// ~150 bytes).
const SPRING_LINEAR_SEGMENTS: u16 = 24;
const REDUCED_MOTION_COND: &str = "(prefers-reduced-motion:reduce)";
const REDUCED_MOTION_ALL_INSTANT_DECLS: &str =
    "transition-duration:0ms!important;transition-delay:0ms!important;";

/// Canonical CSS property identifier of `prop`.
pub const fn property_css(prop: TransitionProperty) -> &'static str {
    match prop {
        TransitionProperty::All => "all",
        TransitionProperty::BackgroundColor => "background-color",
        TransitionProperty::Color => "color",
        TransitionProperty::BorderColor => "border-color",
        TransitionProperty::Opacity => "opacity",
        TransitionProperty::BorderRadius => "border-radius",
        TransitionProperty::Transform => "transform",
        TransitionProperty::Width => "width",
        TransitionProperty::Height => "height",
        TransitionProperty::Padding => "padding",
        TransitionProperty::Gap => "gap",
    }
}

/// Writes the CSS timing function of `curve`: keywords, `cubic-bezier(...)`, or for springs a
/// `linear(...)` easing sampled from the same closed-form response native backends evaluate.
pub fn write_easing_css(curve: EasingCurve, out: &mut String) {
    match curve.sanitized() {
        EasingCurve::Linear => out.push_str("linear"),
        EasingCurve::Ease => out.push_str("ease"),
        EasingCurve::EaseIn => out.push_str("ease-in"),
        EasingCurve::EaseOut => out.push_str("ease-out"),
        EasingCurve::EaseInOut => out.push_str("ease-in-out"),
        EasingCurve::CubicBezier(x1, y1, x2, y2) => {
            out.push_str("cubic-bezier(");
            for (i, v) in [x1, y1, x2, y2].into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_f32(out, v);
            }
            out.push(')');
        }
        spring @ EasingCurve::Spring(_) => {
            out.push_str("linear(");
            for i in 0..=SPRING_LINEAR_SEGMENTS {
                if i > 0 {
                    out.push(',');
                }
                let t = f32::from(i) / f32::from(SPRING_LINEAR_SEGMENTS);
                write_f32(out, spring.sample(t));
            }
            out.push(')');
        }
    }
}

/// Writes one `transition` list item (e.g. `background-color 150ms ease-out 25ms`).
pub fn write_transition_entry(spec: &TransitionSpec, out: &mut String) {
    out.push_str(property_css(spec.property));
    out.push(' ');
    write_u64(out, u64::from(spec.duration_ms));
    out.push_str("ms ");
    write_easing_css(spec.easing, out);
    if spec.delay_ms > 0 {
        out.push(' ');
        write_u64(out, u64::from(spec.delay_ms));
        out.push_str("ms");
    }
}

/// Content-addressed `transition` rule (`nve-t-<8hex>`) plus the optional shared
/// `@media (prefers-reduced-motion:reduce)` override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTransitionRule {
    pub class_name: String,
    pub decls: String,
    pub reduced_motion_rule: Option<CompiledMediaRule>,
}

/// Writes `name:<v0>ms,<v1>ms...!important;` with `ms_of` per spec.
fn write_ms_list(
    specs: impl Iterator<Item = TransitionSpec>,
    name: &str,
    ms_of: impl Fn(&TransitionSpec) -> u32,
    out: &mut String,
) {
    out.push_str(name);
    out.push(':');
    for (idx, spec) in specs.enumerate() {
        if idx > 0 {
            out.push(',');
        }
        write_u64(out, u64::from(ms_of(&spec)));
        out.push_str("ms");
    }
    out.push_str("!important;");
}

/// Compiles `specs` into a content-addressed `transition` rule and, when any spec snaps under
/// reduced motion, a deduplicated `@media (prefers-reduced-motion:reduce)` override.
pub fn compile_transitions<I>(specs: I) -> Option<CompiledTransitionRule>
where
    I: Iterator<Item = TransitionSpec> + Clone,
{
    let mut decls = String::from("transition:");
    for (idx, spec) in specs.clone().enumerate() {
        if idx > 0 {
            decls.push(',');
        }
        write_transition_entry(&spec, &mut decls);
    }
    if decls.len() == "transition:".len() {
        return None;
    }
    decls.push(';');

    let reduced_motion_rule = specs.clone().any(|s| s.reduced_motion_instant).then(|| {
        let rm_decls = if specs.clone().all(|s| s.reduced_motion_instant) {
            REDUCED_MOTION_ALL_INSTANT_DECLS.to_owned()
        } else {
            let snapped = |s: &TransitionSpec, ms: u32| if s.reduced_motion_instant { 0 } else { ms };
            let mut buf = String::new();
            write_ms_list(specs.clone(), "transition-duration", |s| snapped(s, s.duration_ms), &mut buf);
            write_ms_list(specs.clone(), "transition-delay", |s| snapped(s, s.delay_ms), &mut buf);
            buf
        };
        CompiledMediaRule {
            class_name: format_media_class(REDUCED_MOTION_COND, &rm_decls),
            condition: REDUCED_MOTION_COND.to_owned(),
            decls: rm_decls,
        }
    });

    Some(CompiledTransitionRule {
        class_name: format_content_class("nve-t-", &[decls.as_bytes()]),
        decls,
        reduced_motion_rule,
    })
}

/// `WebTableSystem` plugin for [`Motion`] (`nve.motion`).
#[derive(Debug, Default, Clone, Copy)]
pub struct MotionWebSystem;

impl WebTableSystem for MotionWebSystem {
    fn slot(&self) -> TableSlot {
        Motion::slot()
    }

    fn resolve_style(&self, store: &NodeStore, node: NodeId, out: &mut ResolvedNodeStyle) {
        let Some(row) = store.try_row::<Motion>(node) else {
            return;
        };
        let Some(specs) = store.resource::<Transitions>(row.transitions()) else {
            return;
        };
        if let Some(rule) = compile_transitions(specs.iter()) {
            out.push_class_rule(rule.class_name, rule.decls);
            if let Some(rm) = rule.reduced_motion_rule {
                out.push_media_rule(rm);
            }
        }
    }
}
