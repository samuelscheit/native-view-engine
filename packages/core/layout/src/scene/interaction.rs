//! Interactive pseudo-state tracking (`:hover`, `:focus`, `:focus-visible`, `:active`,
//! `:disabled`), pointer capture, and ancestor-inheriting [`CursorKind`] resolution shared across
//! [`LaidOutScene`] consumers.

use nve_engine::InteractionTracker;
use nve_store::{NodeId, NodeStore, RowPatchView};
use nve_tables::interaction::{
    CursorKind, FocusModality, InteractionState, resolve_cursor_from_view,
    resolve_layout_and_style_from_view, resolve_style_from_view, visit_state_rules_in_cascade,
};
use nve_tables::snapshots::{LayoutSnapshot, VisualSnapshot};
use nve_tables::tables::{
    InputKind, Interaction, PointerCaptureMode, StateRules, StateRulesView, TextInput,
    is_node_display_none, paint_of_box,
};

use crate::{MAX_TREE_DEPTH, StoreLayoutEngine};

use super::{LaidOutScene, layout_snapshot_of, visual_snapshot_of};

impl<L: StoreLayoutEngine> LaidOutScene<L> {
    #[inline]
    pub fn interaction_tracker(&self) -> &InteractionTracker {
        &self.interaction
    }

    #[inline]
    pub fn interaction_tracker_mut(&mut self) -> &mut InteractionTracker {
        &mut self.interaction
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
                || i.focusable()
                || i.capture_mode() != PointerCaptureMode::None
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

    fn local_cursor_of(&self, store: &NodeStore, id: NodeId) -> CursorKind {
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

    /// Resolves the effective [`CursorKind`] for `id` by walking `id` -> ancestors leaf-to-root
    /// until the first non-`Auto` cursor is found (converting disabled `Pointer` -> `NotAllowed`).
    pub fn effective_cursor(&self, store: &NodeStore, id: NodeId) -> CursorKind {
        if !store.is_live(id) || is_node_display_none(store, id) {
            return CursorKind::Auto;
        }
        let mut cur = Some(id);
        let mut disabled_seen = false;
        for _ in 0..MAX_TREE_DEPTH {
            let Some(nid) = cur else {
                break;
            };
            if self.interaction.is_disabled(nid)
                || store.try_row::<Interaction>(nid).is_some_and(|i| i.disabled())
            {
                disabled_seen = true;
            }
            let resolved = self.local_cursor_of(store, nid);
            if resolved != CursorKind::Auto {
                return if disabled_seen && resolved == CursorKind::Pointer {
                    CursorKind::NotAllowed
                } else {
                    resolved
                };
            }
            cur = store.parent(nid).filter(|&p| p != nid);
        }
        CursorKind::Auto
    }

    /// Resolves the active cursor: checks captured node's ancestor chain first, then `hovered_chain`
    /// leaf-to-root, then `active_chain`, falling back to `CursorKind::Default`.
    pub fn current_cursor(&self, store: &NodeStore) -> CursorKind {
        if let Some(cap) = self.interaction.captured_node() {
            let c = self.effective_cursor(store, cap);
            return if c == CursorKind::Auto {
                CursorKind::Default
            } else {
                c
            };
        }
        for &id in self.interaction.hovered_chain() {
            let c = self.effective_cursor(store, id);
            if c != CursorKind::Auto {
                return c;
            }
        }
        for &id in self.interaction.active_chain() {
            let c = self.effective_cursor(store, id);
            if c != CursorKind::Auto {
                return c;
            }
        }
        CursorKind::Default
    }

    /// Zero-alloc ancestor-chain pointer move against `store`.
    #[inline]
    pub fn pointer_move_in_store(
        &mut self,
        store: &NodeStore,
        target: Option<NodeId>,
    ) -> &[NodeId] {
        self.interaction
            .pointer_move_with(target, |id| store.parent(id))
    }

    /// Zero-alloc ancestor-chain pointer down against `store`.
    #[inline]
    pub fn pointer_down_in_store(
        &mut self,
        store: &NodeStore,
        target: Option<NodeId>,
    ) -> &[NodeId] {
        self.interaction
            .pointer_down_with(target, |id| store.parent(id))
    }

    /// Zero-alloc ancestor-chain pointer up against `store`, returning `(clicked_node, dirty)`.
    #[inline]
    pub fn pointer_up_in_store(
        &mut self,
        store: &NodeStore,
        target: Option<NodeId>,
    ) -> (Option<NodeId>, &[NodeId]) {
        self.interaction
            .pointer_up_with(target, |id| store.parent(id))
    }

    /// Zero-alloc pointer leave (preserves active capture).
    #[inline]
    pub fn pointer_leave_slice(&mut self) -> &[NodeId] {
        self.interaction.pointer_leave_slice()
    }

    /// Zero-alloc pointer cancel (clears capture, active, and hover).
    #[inline]
    pub fn pointer_cancel_slice(&mut self) -> &[NodeId] {
        self.interaction.pointer_cancel_slice()
    }

    /// Zero-alloc focus update with explicit modality (`Pointer` vs `Keyboard`).
    #[inline]
    pub fn set_focus_slice(
        &mut self,
        node_id: Option<NodeId>,
        modality: FocusModality,
    ) -> &[NodeId] {
        self.interaction.set_focus(node_id, modality)
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
        self.pointer_move_in_store(store, hit).to_vec()
    }

    pub fn dispatch_pointer_leave(&mut self) -> Vec<NodeId> {
        self.interaction.pointer_leave()
    }

    pub fn dispatch_pointer_down(&mut self, store: &NodeStore, x: f32, y: f32) -> Vec<NodeId> {
        let hit = self.hit_test_interaction(store, x, y);
        self.pointer_down_in_store(store, hit).to_vec()
    }

    pub fn dispatch_pointer_up(&mut self, store: &NodeStore, x: f32, y: f32) -> Vec<NodeId> {
        let hit = self.hit_test_interaction(store, x, y);
        self.pointer_up_in_store(store, hit).1.to_vec()
    }

    pub fn dispatch_pointer_cancel(&mut self) -> Vec<NodeId> {
        self.interaction.pointer_cancel_slice().to_vec()
    }

    pub fn dispatch_focus(
        &mut self,
        node_id: Option<NodeId>,
        modality: FocusModality,
    ) -> Vec<NodeId> {
        self.interaction.set_focus(node_id, modality).to_vec()
    }
}
