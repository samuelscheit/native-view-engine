//! Interactive pseudo-state tracking (`:hover`, `:focus`, `:focus-visible`, `:active`,
//! `:disabled`) and [`CursorKind`] resolution integrated directly into [`crate::HeadlessBackend`],
//! reading `Interaction`, `Events`, and `StateRules` tables directly from `&NodeStore`.

use nve_abi::NodeId;
use nve_components::interaction::{
    resolve_cursor, resolve_layout_and_style, resolve_patch, resolve_style, CursorKind,
    FocusModality, InteractionState, InteractionTracker, StateStyleRule,
};
use nve_components::style_patch::{LayoutSnapshot, StylePatch, VisualSnapshot};
use nve_components::tables::{Interaction, StateRules, TextInput, Visual};
use nve_engine::StoreLayoutEngine;
use nve_store::{NodeId as StoreNodeId, NodeStore};

use crate::{
    is_display_none, layout_snapshot_of, node_clips_overflow, visual_snapshot_of, HeadlessBackend,
    MAX_TREE_DEPTH,
};

impl<L: StoreLayoutEngine> HeadlessBackend<L> {
    #[inline]
    pub fn interaction_tracker(&self) -> &InteractionTracker {
        &self.interaction
    }

    #[inline]
    pub fn interaction_state(&self, node_id: NodeId) -> InteractionState {
        self.interaction.state(node_id)
    }

    /// Marks `id` disabled or enabled in the [`InteractionTracker`], returning dirty `NodeId`s.
    pub fn set_node_disabled(&mut self, id: NodeId, disabled: bool) -> Vec<NodeId> {
        self.interaction.set_disabled(id, disabled)
    }

    /// Returns `true` when `id` participates in pointer hover/active/cursor hit-testing.
    #[inline]
    pub fn is_interaction_target(&self, store: &NodeStore, id: StoreNodeId) -> bool {
        if !store.is_live(id) {
            return false;
        }
        let has_inter = store.try_row::<Interaction>(id).is_some_and(|i| {
            i.event_mask() != 0
                || i.cursor() != CursorKind::Auto
                || i.disabled()
                || !i.state_rules().is_none()
                || i.tab_index_set()
        });
        has_inter || store.has::<TextInput>(id) || self.interaction.is_disabled(id.raw())
    }

    /// Hit-tests `(x, y)` in reverse paint order for the deepest interaction-eligible node.
    pub fn hit_test_interaction(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_interaction_rec(store, StoreNodeId::ROOT, x, y, (0.0, 0.0), false, 0)
    }

    #[allow(clippy::too_many_arguments)]
    fn hit_test_interaction_rec(
        &self,
        store: &NodeStore,
        id: StoreNodeId,
        x: f32,
        y: f32,
        offset: (f32, f32),
        inherited_hidden: bool,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > MAX_TREE_DEPTH || !store.is_live(id) || is_display_none(store, id) {
            return None;
        }
        let effective_hidden = store
            .try_row::<Visual>(id)
            .map_or(inherited_hidden, |v| v.visibility().resolve(inherited_hidden));
        let rect = self.layout.layout_of(id)?;
        let (stx, sty) = self.compute_sticky_delta(store, id);
        let node_offset = (offset.0 + stx, offset.1 + sty);
        let r_x = rect.x + node_offset.0;
        let r_y = rect.y + node_offset.1;
        let (px, py) = if let Some(tf) = self.effective_transform(store, id) {
            tf.inverse_map_surface_point(x, y, r_x, r_y, rect.width, rect.height)?
        } else {
            (x, y)
        };
        let inside = rect.width > 0.0
            && rect.height > 0.0
            && px >= r_x
            && px <= r_x + rect.width
            && py >= r_y
            && py <= r_y + rect.height;
        if !inside && id != StoreNodeId::ROOT && node_clips_overflow(store, id) {
            return None;
        }
        let (sx, sy) = self.scroll_offset(id.raw());
        let child_offset = (node_offset.0 - sx, node_offset.1 - sy);
        for &cid in store.children(id).iter().rev() {
            if let Some(hit) = self.hit_test_interaction_rec(
                store,
                cid,
                px,
                py,
                child_offset,
                effective_hidden,
                depth + 1,
            ) {
                return Some(hit);
            }
        }
        if inside && !effective_hidden && self.is_interaction_target(store, id) {
            Some(id.raw())
        } else {
            None
        }
    }

    fn state_rules_of<'a>(&self, store: &'a NodeStore, id: StoreNodeId) -> &'a [StateStyleRule] {
        store
            .try_row::<Interaction>(id)
            .map(|i| i.state_rules())
            .filter(|res| !res.is_none())
            .and_then(|res| store.resource::<StateRules>(res))
            .map(|r| &*r.0)
            .unwrap_or(&[])
    }

    /// Resolves the effective merged [`StylePatch`] for `id` under its current [`InteractionState`].
    pub fn effective_patch(&self, store: &NodeStore, id: StoreNodeId) -> StylePatch {
        let rules = self.state_rules_of(store, id);
        resolve_patch(rules, self.interaction.state(id.raw()))
    }

    /// Resolves the effective [`VisualSnapshot`] for `id` with active pseudo-state rules applied.
    pub fn effective_visual(&self, store: &NodeStore, id: StoreNodeId) -> VisualSnapshot {
        let base = visual_snapshot_of(store, id);
        let rules = self.state_rules_of(store, id);
        resolve_style(&base, rules, self.interaction.state(id.raw()))
    }

    /// Resolves both [`LayoutSnapshot`] and [`VisualSnapshot`] for `id` with active state rules.
    pub fn effective_layout_and_visual(
        &self,
        store: &NodeStore,
        id: StoreNodeId,
    ) -> (LayoutSnapshot, VisualSnapshot) {
        let base_l = layout_snapshot_of(store, id);
        let base_v = visual_snapshot_of(store, id);
        let rules = self.state_rules_of(store, id);
        resolve_layout_and_style(&base_l, &base_v, rules, self.interaction.state(id.raw()))
    }

    /// Resolves the effective [`CursorKind`] for `id` under its current [`InteractionState`].
    pub fn effective_cursor(&self, store: &NodeStore, id: StoreNodeId) -> CursorKind {
        let base_cursor = store
            .try_row::<Interaction>(id)
            .map(|i| i.cursor())
            .unwrap_or(CursorKind::Auto);
        let base = if base_cursor == CursorKind::Auto && store.has::<TextInput>(id) {
            CursorKind::Text
        } else {
            base_cursor
        };
        let rules = self.state_rules_of(store, id);
        resolve_cursor(base, rules, self.interaction.state(id.raw()))
    }

    #[inline]
    pub fn interaction_tracker_mut_move(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.interaction.pointer_move(target)
    }

    #[inline]
    pub fn interaction_tracker_mut_down(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.interaction.pointer_down(target)
    }

    #[inline]
    pub fn interaction_tracker_mut_up(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.interaction.pointer_up(target)
    }

    pub fn dispatch_pointer_move(&mut self, store: &NodeStore, x: f32, y: f32) -> Vec<NodeId> {
        let hit = self.hit_test_interaction(store, x, y);
        self.interaction.pointer_move(hit)
    }

    pub fn dispatch_pointer_leave(&mut self) -> Vec<NodeId> {
        self.interaction.pointer_leave()
    }

    pub fn dispatch_pointer_down(&mut self, store: &NodeStore, x: f32, y: f32) -> Vec<NodeId> {
        let hit = self.hit_test_interaction(store, x, y);
        self.interaction.pointer_down(hit)
    }

    pub fn dispatch_pointer_up(&mut self, store: &NodeStore, x: f32, y: f32) -> Vec<NodeId> {
        let hit = self.hit_test_interaction(store, x, y);
        self.interaction.pointer_up(hit)
    }

    pub fn dispatch_pointer_cancel(&mut self) -> Vec<NodeId> {
        self.interaction.touch_end()
    }

    pub fn dispatch_focus(
        &mut self,
        node_id: Option<NodeId>,
        modality: FocusModality,
    ) -> Vec<NodeId> {
        match (node_id, modality) {
            (None, _) => self.interaction.blur(),
            (Some(id), FocusModality::Keyboard) => self.interaction.focus_keyboard(Some(id)),
            (Some(id), FocusModality::Pointer) => self.interaction.focus_pointer(Some(id)),
        }
    }
}
