//! Interactive pseudo-state tracking (`:hover`, `:focus`, `:focus-visible`, `:active`,
//! `:disabled`) and [`CursorKind`] resolution shared across [`LaidOutScene`] consumers.

use nve_engine::InteractionTracker;
use nve_store::{NodeId, NodeStore, RowPatchView};
use nve_tables::interaction::{
    CursorKind, FocusModality, InteractionState, resolve_cursor_from_view,
    resolve_layout_and_style_from_view, resolve_style_from_view, visit_state_rules_in_cascade,
};
use nve_tables::snapshots::{LayoutSnapshot, VisualSnapshot};
use nve_tables::tables::{
    InputKind, Interaction, StateRules, StateRulesView, TextInput, is_node_display_none,
    paint_of_box,
};

use crate::StoreLayoutEngine;

use super::{LaidOutScene, layout_snapshot_of, visual_snapshot_of};

impl<L: StoreLayoutEngine> LaidOutScene<L> {
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
    pub fn is_interaction_target(&self, store: &NodeStore, id: NodeId) -> bool {
        if !store.is_live(id) || is_node_display_none(store, id) {
            return false;
        }
        let has_inter = store.try_row::<Interaction>(id).is_some_and(|i| {
            !i.listeners().is_empty()
                || i.cursor() != CursorKind::Auto
                || i.disabled()
                || !i.state_rules().is_none()
                || i.tab_index().is_some()
        });
        let has_visible_input = store
            .try_row::<TextInput>(id)
            .is_some_and(|t| t.input_kind() != InputKind::Hidden);
        has_inter || has_visible_input || self.interaction.is_disabled(id)
    }

    /// Hit-tests `(x, y)` in reverse paint order for the deepest interaction-eligible node.
    pub fn hit_test_interaction(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_where(store, x, y, |id| self.is_interaction_target(store, id))
    }

    fn state_rules_of<'a>(&self, store: &'a NodeStore, id: NodeId) -> StateRulesView<'a> {
        store
            .try_row::<Interaction>(id)
            .map(|i| i.state_rules())
            .filter(|res| !res.is_none())
            .and_then(|res| store.resource::<StateRules>(res))
            .unwrap_or(StateRulesView::EMPTY)
    }

    /// Visits every [`RowPatchView`] of `id`'s state rules matching its current
    /// [`InteractionState`] in deterministic cascade order (zero allocations up to 16 rules).
    pub fn visit_effective_state_patches<'a>(
        &self,
        store: &'a NodeStore,
        id: NodeId,
        f: impl FnMut(RowPatchView<'a>),
    ) {
        visit_state_rules_in_cascade(self.state_rules_of(store, id), self.interaction.state(id), f);
    }

    /// Resolves the effective [`VisualSnapshot`] for `id` with active pseudo-state rules applied.
    pub fn effective_visual(&self, store: &NodeStore, id: NodeId) -> VisualSnapshot {
        let base = visual_snapshot_of(store, id);
        let rules = self.state_rules_of(store, id);
        paint_of_box(
            store,
            id,
            resolve_style_from_view(&base, rules, self.interaction.state(id)),
        )
    }

    /// Resolves both [`LayoutSnapshot`] and [`VisualSnapshot`] for `id` with active state rules.
    pub fn effective_layout_and_visual(
        &self,
        store: &NodeStore,
        id: NodeId,
    ) -> (LayoutSnapshot, VisualSnapshot) {
        let base_l = layout_snapshot_of(store, id);
        let base_v = visual_snapshot_of(store, id);
        let rules = self.state_rules_of(store, id);
        let (layout, visual) =
            resolve_layout_and_style_from_view(&base_l, &base_v, rules, self.interaction.state(id));
        (layout, paint_of_box(store, id, visual))
    }

    /// Resolves the effective [`CursorKind`] for `id` under its current [`InteractionState`].
    pub fn effective_cursor(&self, store: &NodeStore, id: NodeId) -> CursorKind {
        if is_node_display_none(store, id) {
            return CursorKind::Auto;
        }
        let base_cursor = store
            .try_row::<Interaction>(id)
            .map(|i| i.cursor())
            .unwrap_or(CursorKind::Auto);
        let has_visible_input = store
            .try_row::<TextInput>(id)
            .is_some_and(|t| t.input_kind() != InputKind::Hidden);
        let base = if base_cursor == CursorKind::Auto && has_visible_input {
            CursorKind::Text
        } else {
            base_cursor
        };
        let rules = self.state_rules_of(store, id);
        resolve_cursor_from_view(base, rules, self.interaction.state(id))
    }

    /// Resolves the active cursor for the currently hovered/active node, or `CursorKind::Default`.
    pub fn current_cursor(&self, store: &NodeStore) -> CursorKind {
        self.interaction
            .active_cursor_node()
            .map(|id| self.effective_cursor(store, id))
            .unwrap_or(CursorKind::Default)
    }

    #[inline]
    pub fn interaction_move(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.interaction.pointer_move(target)
    }

    #[inline]
    pub fn interaction_down(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.interaction.pointer_down(target)
    }

    #[inline]
    pub fn interaction_up(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
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
