//! Zero-serialization, zero-JS native CSS `transition:` compilation for [`TransitionSpec`] slices
//! with automatic `@media (prefers-reduced-motion:reduce)` instant-snap handling.

use core::fmt::Write as _;

use nve_abi::num::write_u64;
use nve_abi::NodeId;
use nve_components::motion::TransitionSpec;

use crate::media_css::{format_media_class, CompiledMediaRule};
use crate::WebBackend;

const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;
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

#[inline]
fn fnv1a_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

/// Formats a deterministic content-addressed class token `nve-t-<8hex>` for a CSS `transition:`
/// declaration string.
pub fn format_transition_class(decls: &str) -> String {
    let h = fnv1a_bytes(FNV1A_OFFSET_BASIS, decls.as_bytes());
    let h32 = (h ^ (h >> 32)) as u32;
    let mut out = String::with_capacity(14);
    let _ = write!(&mut out, "nve-t-{h32:08x}");
    out
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

impl WebBackend {
    /// Directly replaces the zero-serialization [`TransitionSpec`] list on `id`.
    pub fn set_node_transitions(&mut self, id: NodeId, transitions: Vec<TransitionSpec>) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        if node.transitions != transitions {
            node.transitions = transitions;
            self.flush_or_queue_responsive_dirty(vec![id]);
        }
    }
}
