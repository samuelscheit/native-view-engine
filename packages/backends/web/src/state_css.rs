//! Zero-serialization, zero-JS native CSS pseudo-class compilation for interactive
//! [`StateStyleRule`]s (`:hover`, `:focus`, `:focus-visible`, `:active`, `:disabled` /
//! `[aria-disabled="true"]`, plus compound selectors) and base [`CursorKind`] synchronization.
//!
//! Hover rules are wrapped in `@media (hover:hover)` by default so touch taps on mobile/hybrid
//! devices never suffer sticky `:hover` states.

use core::fmt::Write as _;

use nve_abi::hash::HashMap;
use nve_abi::NodeId;
use nve_components::interaction::{CursorKind, InteractionState, StateStyleRule};

use crate::media_css::write_style_patch_decls;
use crate::WebBackend;

const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

/// One compiled, content-addressed CSS interactive pseudo-state rule
/// (`nve-s-<8hex>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledStateRule {
    pub class_name: String,
    pub state: InteractionState,
    pub decls: String,
    pub guard_hover_media: bool,
}

impl CompiledStateRule {
    #[inline]
    fn write_non_disabled_pseudos(non_disabled: InteractionState, out: &mut String) {
        if non_disabled.contains(InteractionState::HOVER) {
            out.push_str(":hover");
        }
        if non_disabled.contains(InteractionState::FOCUS) {
            out.push_str(":focus");
        }
        if non_disabled.contains(InteractionState::FOCUS_VISIBLE) {
            out.push_str(":focus-visible");
        }
        if non_disabled.contains(InteractionState::ACTIVE) {
            out.push_str(":active");
        }
    }

    /// Writes the pseudo-class selector list (e.g. `.nve-s-01234567:hover:focus-visible` or
    /// `.nve-s-01234567:disabled,.nve-s-01234567[aria-disabled="true"]`) into `out`.
    pub fn write_selector_into(&self, out: &mut String) {
        let non_disabled = self.state.difference(InteractionState::DISABLED);
        if self.state.contains(InteractionState::DISABLED) {
            out.push('.');
            out.push_str(&self.class_name);
            Self::write_non_disabled_pseudos(non_disabled, out);
            out.push_str(":disabled,.");
            out.push_str(&self.class_name);
            Self::write_non_disabled_pseudos(non_disabled, out);
            out.push_str("[aria-disabled=\"true\"]");
        } else {
            out.push('.');
            out.push_str(&self.class_name);
            Self::write_non_disabled_pseudos(non_disabled, out);
        }
    }

    /// Writes the raw unwrapped CSS rule (`<selector>{<decls>}`) into `out`.
    pub fn write_unwrapped_css_rule_into(&self, out: &mut String) {
        self.write_selector_into(out);
        out.push('{');
        out.push_str(&self.decls);
        out.push('}');
    }

    /// Appends the complete CSS rule into `out`, wrapping hover rules in
    /// `@media (hover:hover){...}` when [`Self::guard_hover_media`] is set.
    pub fn write_css_rule_into(&self, out: &mut String) {
        let wrap_hover = self.guard_hover_media && self.state.contains(InteractionState::HOVER);
        if wrap_hover {
            out.push_str("@media (hover:hover){");
        }
        self.write_unwrapped_css_rule_into(out);
        if wrap_hover {
            out.push('}');
        }
    }

    /// Formats the complete CSS rule string.
    pub fn to_css_rule(&self) -> String {
        let mut out = String::with_capacity(self.class_name.len() * 2 + self.decls.len() + 48);
        self.write_css_rule_into(&mut out);
        out
    }

    /// Formats the unwrapped CSS rule string (without `@media (hover:hover)`).
    pub fn to_unwrapped_css_rule(&self) -> String {
        let mut out = String::with_capacity(self.class_name.len() * 2 + self.decls.len() + 32);
        self.write_unwrapped_css_rule_into(&mut out);
        out
    }
}

#[inline]
fn fnv1a_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

/// Returns the static base-layer `(class_name, css_decl)` pair for a non-`Auto` [`CursorKind`],
/// or `None` for [`CursorKind::Auto`]. Emitting cursors as dedicated zero-alloc base classes
/// avoids multiplying structural `.s<hash>` rules and guarantees zero duplicate CSS rules when
/// `sync_interaction_from` runs after `submit_frame` during hydration.
#[inline]
pub const fn format_cursor_rule(cursor: CursorKind) -> Option<(&'static str, &'static str)> {
    match cursor {
        CursorKind::Auto => None,
        CursorKind::Default => Some(("nve-c-default", "cursor:default;")),
        CursorKind::Pointer => Some(("nve-c-pointer", "cursor:pointer;")),
        CursorKind::Text => Some(("nve-c-text", "cursor:text;")),
        CursorKind::Grab => Some(("nve-c-grab", "cursor:grab;")),
        CursorKind::Grabbing => Some(("nve-c-grabbing", "cursor:grabbing;")),
        CursorKind::NotAllowed => Some(("nve-c-not-allowed", "cursor:not-allowed;")),
        CursorKind::ColResize => Some(("nve-c-col-resize", "cursor:col-resize;")),
        CursorKind::RowResize => Some(("nve-c-row-resize", "cursor:row-resize;")),
        CursorKind::Move => Some(("nve-c-move", "cursor:move;")),
        CursorKind::Crosshair => Some(("nve-c-crosshair", "cursor:crosshair;")),
        CursorKind::Progress => Some(("nve-c-progress", "cursor:progress;")),
        CursorKind::Wait => Some(("nve-c-wait", "cursor:wait;")),
    }
}

/// Formats a deterministic content-addressed class token `nve-s-<8hex>` for `(state, decls)`.
pub fn format_state_class(state: InteractionState, decls: &str) -> String {
    let mut h = FNV1A_OFFSET_BASIS;
    h = fnv1a_bytes(h, &[state.bits(), 0]);
    h = fnv1a_bytes(h, decls.as_bytes());
    let h32 = (h ^ (h >> 32)) as u32;
    let mut out = String::with_capacity(14);
    let _ = write!(&mut out, "nve-s-{h32:08x}");
    out
}

/// Compiles a node's [`StateStyleRule`] slice into content-addressed [`CompiledStateRule`]s in
/// deterministic cascade order (`Hover < Focus < FocusVisible < Active < Disabled`, preserving
/// declaration order for ties) with `@media (hover:hover)` sticky-hover guarding enabled.
pub fn compile_node_state_rules(rules: &[StateStyleRule], out: &mut Vec<CompiledStateRule>) {
    compile_node_state_rules_with_hover_guard(rules, true, out);
}

/// Configurable variant of [`compile_node_state_rules`] allowing callers/tests to control
/// `@media (hover:hover)` wrapping explicitly.
pub fn compile_node_state_rules_with_hover_guard(
    rules: &[StateStyleRule],
    guard_hover_media: bool,
    out: &mut Vec<CompiledStateRule>,
) {
    if rules.is_empty() {
        return;
    }

    let mut ordered: Vec<usize> = (0..rules.len())
        .filter(|&i| !rules[i].state.is_empty() && !rules[i].patch.is_empty())
        .collect();
    ordered.sort_by_key(|&i| rules[i].state.cascade_order());

    for idx in ordered {
        let rule = &rules[idx];
        let mut decls = String::new();
        write_style_patch_decls(&rule.patch, &mut decls);
        if decls.is_empty() {
            continue;
        }
        let class_name = format_state_class(rule.state, &decls);
        if !out.iter().any(|existing| existing.class_name == class_name) {
            out.push(CompiledStateRule {
                class_name,
                state: rule.state,
                decls,
                guard_hover_media,
            });
        }
    }
}

impl WebBackend {
    /// Directly attaches or clears a zero-serialization base [`CursorKind`] on `id`.
    pub fn set_node_cursor(&mut self, id: NodeId, cursor: Option<CursorKind>) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        if node.cursor != cursor {
            node.cursor = cursor;
            self.flush_or_queue_responsive_dirty(vec![id]);
        }
    }

    /// Directly replaces the zero-serialization [`StateStyleRule`] list on `id`.
    pub fn set_node_state_rules(&mut self, id: NodeId, rules: Vec<StateStyleRule>) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        if node.state_rules != rules {
            node.state_rules = rules;
            self.flush_or_queue_responsive_dirty(vec![id]);
        }
    }

    /// Bulk-synchronizes typed [`CursorKind`] values and [`StateStyleRule`] slices from any
    /// producer (e.g. `DioxusSurface`), clearing stale interaction metadata from nodes no longer
    /// present and flushing updated classes & pseudo-state CSS rules in a single batched pass.
    pub fn sync_interaction_from<'a>(
        &mut self,
        cursors: impl IntoIterator<Item = (NodeId, CursorKind)>,
        state_rules: impl IntoIterator<Item = (NodeId, &'a [StateStyleRule])>,
    ) {
        let cursor_map: HashMap<NodeId, CursorKind> = cursors.into_iter().collect();
        let rules_map: HashMap<NodeId, &'a [StateStyleRule]> = state_rules.into_iter().collect();

        let mut dirty: Vec<NodeId> = Vec::new();
        for (&id, node) in &mut self.nodes {
            let mut changed = false;
            let next_cursor = cursor_map.get(&id).copied();
            if node.cursor != next_cursor {
                node.cursor = next_cursor;
                changed = true;
            }
            let next_rules = rules_map.get(&id).copied().unwrap_or(&[]);
            if node.state_rules.as_slice() != next_rules {
                node.state_rules.clear();
                node.state_rules.extend_from_slice(next_rules);
                changed = true;
            }
            if changed {
                dirty.push(id);
            }
        }

        if !dirty.is_empty() {
            dirty.sort_unstable();
            dirty.dedup();
            self.flush_or_queue_responsive_dirty(dirty);
        }
    }
}
