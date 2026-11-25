//! Interactive pseudo-state tracking (`:hover`, `:focus`, `:focus-visible`, `:active`,
//! `:disabled`) and [`CursorKind`] resolution integrated directly into [`crate::HeadlessBackend`].

use nve_abi::hash::HashMap;
use nve_abi::{ComponentKind, EventMask, LayoutProps, NodeId, StyleProps, ROOT_NODE_ID};
use nve_components::interaction::{
    resolve_cursor, resolve_layout_and_style, resolve_patch, resolve_style, CursorKind,
    InteractionState, InteractionTracker, StateStyleRule,
};
use nve_components::style_patch::StylePatch;
use nve_engine::LayoutEngine;

use crate::{HeadlessBackend, HeadlessNode, MAX_TREE_DEPTH};

impl<L: LayoutEngine> HeadlessBackend<L> {
    /// Returns a read-only reference to the backend's [`InteractionTracker`].
    #[inline]
    pub fn interaction_tracker(&self) -> &InteractionTracker {
        &self.interaction
    }

    /// Returns the active [`InteractionState`] bitflags for `node_id`.
    #[inline]
    pub fn interaction_state(&self, node_id: NodeId) -> InteractionState {
        self.interaction.state(node_id)
    }

    /// Directly attaches or clears a zero-serialization base [`CursorKind`] on `id`.
    /// Returns `true` when the stored cursor changed.
    pub fn set_node_cursor(&mut self, id: NodeId, cursor: Option<CursorKind>) -> bool {
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        if node.cursor != cursor {
            node.cursor = cursor;
            true
        } else {
            false
        }
    }

    /// Directly replaces the zero-serialization [`StateStyleRule`] list on `id`.
    /// Returns `true` when the stored rules changed.
    pub fn set_node_state_rules(&mut self, id: NodeId, rules: Vec<StateStyleRule>) -> bool {
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        if node.state_rules != rules {
            node.state_rules = rules;
            true
        } else {
            false
        }
    }

    /// Bulk-synchronizes typed [`CursorKind`] values and [`StateStyleRule`] slices from any
    /// producer (e.g. `DioxusSurface`), returning sorted deduplicated `NodeId`s whose metadata
    /// changed.
    pub fn sync_interaction_from<'a>(
        &mut self,
        cursors: impl IntoIterator<Item = (NodeId, CursorKind)>,
        state_rules: impl IntoIterator<Item = (NodeId, &'a [StateStyleRule])>,
    ) -> Vec<NodeId> {
        let cursor_map: HashMap<NodeId, CursorKind> = cursors.into_iter().collect();
        let rules_map: HashMap<NodeId, &'a [StateStyleRule]> = state_rules.into_iter().collect();

        let ids: Vec<NodeId> = self.nodes.values().map(|n| n.id).collect();
        let mut dirty = Vec::new();
        for id in ids {
            let Some(node) = self.nodes.get_mut(id) else {
                continue;
            };
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
        dirty.sort_unstable();
        dirty
    }

    /// Marks `id` disabled or enabled in the [`InteractionTracker`], returning dirty `NodeId`s.
    pub fn set_node_disabled(&mut self, id: NodeId, disabled: bool) -> Vec<NodeId> {
        if self.nodes.get(id).is_none() {
            return Vec::new();
        }
        self.interaction.set_disabled(id, disabled)
    }

    /// Returns `true` when `node` participates in pointer hover/active/cursor hit-testing.
    #[inline]
    pub fn is_interaction_target(&self, node: &HeadlessNode) -> bool {
        node.event_mask != 0
            || !node.state_rules.is_empty()
            || node.cursor.is_some()
            || matches!(node.kind, ComponentKind::Pressable | ComponentKind::TextInput)
            || self.interaction.is_disabled(node.id)
    }

    /// Hit-tests `(x, y)` in reverse paint order for the deepest interaction-eligible node
    /// (event subscribers, `Pressable`/`TextInput`, disabled nodes, or any node declaring
    /// `state_rules` or `cursor`).
    pub fn hit_test_interaction(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_interaction_rec(ROOT_NODE_ID, x, y, 0)
    }

    fn hit_test_interaction_rec(
        &self,
        id: NodeId,
        x: f32,
        y: f32,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > MAX_TREE_DEPTH {
            return None;
        }
        let node = self.nodes.get(id)?;
        if node.style.opacity <= 0.001 {
            return None;
        }
        let r = node.computed_rect;
        let inside = r.width > 0.0
            && r.height > 0.0
            && x >= r.x
            && x <= r.x + r.width
            && y >= r.y
            && y <= r.y + r.height;
        if !inside && id != ROOT_NODE_ID {
            return None;
        }
        for &child in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_interaction_rec(child, x, y, depth + 1) {
                return Some(hit);
            }
        }
        if inside && self.is_interaction_target(node) {
            Some(id)
        } else {
            None
        }
    }

    /// Moves the pointer to `(x, y)`, updating hover state and cancelling any active press dragged
    /// off-target. Returns exact dirty `NodeId`s.
    pub fn pointer_move(&mut self, x: f32, y: f32) -> Vec<NodeId> {
        let target = self.hit_test_interaction(x, y);
        self.interaction.pointer_move(target)
    }

    /// Updates pointer hover directly for a pre-hit-tested node (used by scroll/clip-aware
    /// backends like `SkiaBackend`).
    pub fn pointer_move_node(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        let resolved = target.filter(|&id| self.nodes.get(id).is_some());
        self.interaction.pointer_move(resolved)
    }

    /// Clears hover and cancels any in-flight active press when the pointer exits the surface.
    pub fn pointer_leave(&mut self) -> Vec<NodeId> {
        self.interaction.pointer_leave()
    }

    /// Presses the pointer at `(x, y)`, activating the hit node unless disabled.
    pub fn pointer_down(&mut self, x: f32, y: f32) -> Vec<NodeId> {
        let target = self.hit_test_interaction(x, y);
        self.interaction.pointer_down(target)
    }

    /// Presses a pre-hit-tested node unless disabled.
    pub fn pointer_down_node(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        let resolved = target.filter(|&id| self.nodes.get(id).is_some());
        self.interaction.pointer_down(resolved)
    }

    /// Releases the pointer at `(x, y)`, clearing `ACTIVE` and updating `HOVER`.
    pub fn pointer_up(&mut self, x: f32, y: f32) -> Vec<NodeId> {
        let target = self.hit_test_interaction(x, y);
        self.interaction.pointer_up(target)
    }

    /// Releases a pre-hit-tested pointer press.
    pub fn pointer_up_node(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        let resolved = target.filter(|&id| self.nodes.get(id).is_some());
        self.interaction.pointer_up(resolved)
    }

    /// Handles a touch tap at `(x, y)`: clears `hovered` and `active` so hybrid devices never
    /// suffer sticky hover.
    pub fn touch_tap(&mut self, x: f32, y: f32) -> Vec<NodeId> {
        let target = self.hit_test_interaction(x, y);
        self.interaction.touch_tap(target)
    }

    /// Handles a touch tap on a pre-hit-tested node.
    pub fn touch_tap_node(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        let resolved = target.filter(|&id| self.nodes.get(id).is_some());
        self.interaction.touch_tap(resolved)
    }

    /// Moves keyboard focus (`FOCUS | FOCUS_VISIBLE`) to `target` (or blurs when `None`).
    pub fn focus_keyboard(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        if let Some(id) = target
            && self.nodes.get(id).is_none()
        {
            return Vec::new();
        }
        self.interaction.focus_keyboard(target)
    }

    /// Moves pointer focus (`FOCUS` without `FOCUS_VISIBLE`) to `target` (or blurs when `None`).
    pub fn focus_pointer(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        if let Some(id) = target
            && self.nodes.get(id).is_none()
        {
            return Vec::new();
        }
        self.interaction.focus_pointer(target)
    }

    /// Blurs the currently focused node.
    pub fn blur(&mut self) -> Vec<NodeId> {
        self.interaction.blur()
    }

    /// Resolves the merged active [`StylePatch`] for `node_id` given its current [`InteractionState`].
    #[inline]
    pub fn effective_patch(&self, node_id: NodeId) -> StylePatch {
        let Some(node) = self.nodes.get(node_id) else {
            return StylePatch::new();
        };
        resolve_patch(&node.state_rules, self.interaction.state(node_id))
    }

    /// Resolves the effective [`StyleProps`] for `node_id` after applying all matching
    /// [`StateStyleRule`]s in deterministic cascade order.
    #[inline]
    pub fn effective_style(&self, node_id: NodeId) -> Option<StyleProps> {
        let node = self.nodes.get(node_id)?;
        Some(resolve_style(
            &node.style,
            &node.state_rules,
            self.interaction.state(node_id),
        ))
    }

    /// Resolves both [`LayoutProps`] and [`StyleProps`] for `node_id` with active state patches.
    #[inline]
    pub fn effective_layout_and_style(&self, node_id: NodeId) -> Option<(LayoutProps, StyleProps)> {
        let node = self.nodes.get(node_id)?;
        Some(resolve_layout_and_style(
            &node.layout,
            &node.style,
            &node.state_rules,
            self.interaction.state(node_id),
        ))
    }

    /// Computes the base [`CursorKind`] for `node` before state-rule overrides (matching Web's
    /// `.nve-b` / `.nve-p` `cursor:pointer` and `<input>` `cursor:text` defaults).
    #[inline]
    pub fn default_cursor_for_node(node: &HeadlessNode) -> CursorKind {
        if let Some(explicit) = node.cursor {
            explicit
        } else if node.kind == ComponentKind::TextInput {
            CursorKind::Text
        } else if node.kind == ComponentKind::Pressable || (node.event_mask & EventMask::PRESS) != 0
        {
            CursorKind::Pointer
        } else {
            CursorKind::Auto
        }
    }

    /// Resolves the effective [`CursorKind`] for `node_id` in its current [`InteractionState`].
    #[inline]
    pub fn effective_cursor(&self, node_id: NodeId) -> CursorKind {
        let Some(node) = self.nodes.get(node_id) else {
            return CursorKind::Auto;
        };
        let base = Self::default_cursor_for_node(node);
        resolve_cursor(base, &node.state_rules, self.interaction.state(node_id))
    }

    /// Returns the active window [`CursorKind`] derived from the currently pressed or hovered node.
    #[inline]
    pub fn active_cursor(&self) -> CursorKind {
        self.interaction
            .active_cursor_node()
            .map_or(CursorKind::Auto, |id| self.effective_cursor(id))
    }
}
