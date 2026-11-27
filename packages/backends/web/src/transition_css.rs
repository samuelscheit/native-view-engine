//! Zero-serialization, zero-JS native CSS `transition:` compilation for [`TransitionSpec`] slices
//! with automatic `@media (prefers-reduced-motion:reduce)` instant-snap handling.

use nve_abi::num::write_u64;
use nve_components::motion::TransitionSpec;
use nve_components::tables::TransitionsView;

use crate::css::format_content_class;
use crate::media_css::{format_media_class, CompiledMediaRule};

const REDUCED_MOTION_COND: &str = "(prefers-reduced-motion:reduce)";
const REDUCED_MOTION_ALL_INSTANT_DECLS: &str =
    "transition-duration:0ms!important;transition-delay:0ms!important;";

/// Compiled, content-addressed CSS `transition:` rule (`nve-t-<8hex>`) plus optional shared
/// `@media (prefers-reduced-motion:reduce)` override rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTransitionRule {
    pub class_name: String,
    pub decls: String,
    pub reduced_motion_rule: Option<CompiledMediaRule>,
}

/// Formats a deterministic content-addressed class token `nve-t-<8hex>` for a CSS `transition:`
/// declaration string.
pub fn format_transition_class(decls: &str) -> String {
    format_content_class("nve-t-", &[decls.as_bytes()])
}

/// Zero-copy counterpart to [`compile_node_transitions`] reading directly from [`TransitionsView`].
pub fn compile_node_transitions_view(specs: TransitionsView<'_>) -> Option<CompiledTransitionRule> {
    if specs.is_empty() {
        return None;
    }
    let mut decls = String::with_capacity(specs.len().saturating_mul(32) + 12);
    decls.push_str("transition:");
    for (idx, spec) in specs.iter().enumerate() {
        if idx > 0 {
            decls.push(',');
        }
        spec.write_css_entry_into(&mut decls);
    }
    decls.push(';');

    let any_rm = specs.iter().any(|s| s.reduced_motion_instant);
    let reduced_motion_rule = if any_rm {
        let all_rm = specs.iter().all(|s| s.reduced_motion_instant);
        let rm_decls = if all_rm {
            REDUCED_MOTION_ALL_INSTANT_DECLS.to_owned()
        } else {
            let mut buf = String::with_capacity(specs.len().saturating_mul(16) + 56);
            buf.push_str("transition-duration:");
            for (idx, spec) in specs.iter().enumerate() {
                if idx > 0 {
                    buf.push(',');
                }
                let dur = if spec.reduced_motion_instant { 0 } else { spec.duration_ms };
                write_u64(&mut buf, u64::from(dur));
                buf.push_str("ms");
            }
            buf.push_str("!important;transition-delay:");
            for (idx, spec) in specs.iter().enumerate() {
                if idx > 0 {
                    buf.push(',');
                }
                let delay = if spec.reduced_motion_instant { 0 } else { spec.delay_ms };
                write_u64(&mut buf, u64::from(delay));
                buf.push_str("ms");
            }
            buf.push_str("!important;");
            buf
        };
        let rm_class = format_media_class(REDUCED_MOTION_COND, &rm_decls);
        Some(CompiledMediaRule {
            class_name: rm_class,
            condition: REDUCED_MOTION_COND.to_owned(),
            decls: rm_decls,
        })
    } else {
        None
    };

    let class_name = format_transition_class(&decls);
    Some(CompiledTransitionRule {
        class_name,
        decls,
        reduced_motion_rule,
    })
}

/// Compiles a node's [`TransitionSpec`] slice into a content-addressed base `transition:` rule
/// (`nve-t-<8hex>`) and, whenever any spec enables `reduced_motion_instant`, a deduplicated
/// `@media (prefers-reduced-motion:reduce)` override rule.
pub fn compile_node_transitions(specs: &[TransitionSpec]) -> Option<CompiledTransitionRule> {
    if specs.is_empty() {
        return None;
    }

    let mut decls = String::with_capacity(specs.len().saturating_mul(32) + 12);
    TransitionSpec::write_css_list_into(specs, &mut decls);
    if decls.is_empty() {
        return None;
    }

    let any_rm = specs.iter().any(|s| s.reduced_motion_instant);
    let reduced_motion_rule = if any_rm {
        let all_rm = specs.iter().all(|s| s.reduced_motion_instant);
        let rm_decls = if all_rm {
            REDUCED_MOTION_ALL_INSTANT_DECLS.to_owned()
        } else {
            let mut buf = String::with_capacity(specs.len().saturating_mul(16) + 56);
            buf.push_str("transition-duration:");
            for (idx, spec) in specs.iter().enumerate() {
                if idx > 0 {
                    buf.push(',');
                }
                let dur = if spec.reduced_motion_instant {
                    0
                } else {
                    spec.duration_ms
                };
                write_u64(&mut buf, u64::from(dur));
                buf.push_str("ms");
            }
            buf.push_str("!important;transition-delay:");
            for (idx, spec) in specs.iter().enumerate() {
                if idx > 0 {
                    buf.push(',');
                }
                let delay = if spec.reduced_motion_instant {
                    0
                } else {
                    spec.delay_ms
                };
                write_u64(&mut buf, u64::from(delay));
                buf.push_str("ms");
            }
            buf.push_str("!important;");
            buf
        };
        let rm_class = format_media_class(REDUCED_MOTION_COND, &rm_decls);
        Some(CompiledMediaRule {
            class_name: rm_class,
            condition: REDUCED_MOTION_COND.to_owned(),
            decls: rm_decls,
        })
    } else {
        None
    };

    let class_name = format_transition_class(&decls);
    Some(CompiledTransitionRule {
        class_name,
        decls,
        reduced_motion_rule,
    })
}

