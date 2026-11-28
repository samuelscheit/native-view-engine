//! Reusable O(1) stateful interaction tracker (`InteractionTracker`) for native backends
//! (`HeadlessBackend`, `SkiaBackend`).

use nve_abi::hash::HashSet;
use nve_abi::NodeId;

use nve_tables::interaction::{FocusModality, InteractionState};

/// Reusable O(1) stateful interaction tracker for native backends (`HeadlessBackend`, `SkiaBackend`).
///
/// Tracks `hovered`, `active` (press + drag-off cancel), `focused` + [`FocusModality`] (`FOCUS`
/// vs `FOCUS_VISIBLE`), and `disabled` nodes, returning the exact deduplicated list of dirty
/// [`NodeId`]s whose [`InteractionState`] changed on every transition so backends repaint only
/// affected nodes.
#[derive(Debug, Clone, Default)]
pub struct InteractionTracker {
    hovered: Option<NodeId>,
    active: Option<NodeId>,
    focused: Option<NodeId>,
    focus_modality: FocusModality,
    disabled: HashSet<NodeId>,
}

impl InteractionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn hovered(&self) -> Option<NodeId> {
        self.hovered
    }

    #[inline]
    pub fn active(&self) -> Option<NodeId> {
        self.active
    }

    #[inline]
    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    #[inline]
    pub fn focus_modality(&self) -> FocusModality {
        self.focus_modality
    }

    #[inline]
    pub fn is_disabled(&self, id: NodeId) -> bool {
        self.disabled.contains(&id)
    }

    /// Returns the node that currently governs the window pointer cursor:
    /// prefers the pressed `active` node while held, falling back to `hovered`.
    #[inline]
    pub fn active_cursor_node(&self) -> Option<NodeId> {
        self.active.or(self.hovered)
    }

    /// Computes the exact active [`InteractionState`] flags for `id` in O(1).
    #[inline]
    pub fn state(&self, id: NodeId) -> InteractionState {
        let mut s = InteractionState::NONE;
        if self.hovered == Some(id) {
            s |= InteractionState::HOVER;
        }
        if self.focused == Some(id) {
            s |= InteractionState::FOCUS;
            if self.focus_modality == FocusModality::Keyboard {
                s |= InteractionState::FOCUS_VISIBLE;
            }
        }
        if self.active == Some(id) {
            s |= InteractionState::ACTIVE;
        }
        if self.disabled.contains(&id) {
            s |= InteractionState::DISABLED;
        }
        s
    }

    fn collect_dirty(&self, candidates: &[(NodeId, InteractionState)]) -> Vec<NodeId> {
        let mut dirty = Vec::new();
        for &(id, prev) in candidates {
            if self.state(id) != prev && !dirty.contains(&id) {
                dirty.push(id);
            }
        }
        dirty.sort_unstable();
        dirty
    }

    #[inline]
    fn push_candidate(&self, buf: &mut Vec<(NodeId, InteractionState)>, id: Option<NodeId>) {
        if let Some(nid) = id
            && !buf.iter().any(|(existing, _)| *existing == nid)
        {
            buf.push((nid, self.state(nid)));
        }
    }

    /// Updates pointer hover to `target`. If an `active` press is in flight and the pointer drags
    /// off that node (`target != active`), cancels the active press (`press-cancel on drag-off`).
    pub fn pointer_move(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        let mut candidates = Vec::with_capacity(3);
        self.push_candidate(&mut candidates, self.hovered);
        self.push_candidate(&mut candidates, self.active);
        self.push_candidate(&mut candidates, target);

        self.hovered = target;
        if let Some(act) = self.active
            && target != Some(act)
        {
            self.active = None;
        }

        self.collect_dirty(&candidates)
    }

    /// Clears hover and cancels any in-flight press when the pointer exits the surface/window.
    pub fn pointer_leave(&mut self) -> Vec<NodeId> {
        let mut candidates = Vec::with_capacity(2);
        self.push_candidate(&mut candidates, self.hovered);
        self.push_candidate(&mut candidates, self.active);

        self.hovered = None;
        self.active = None;

        self.collect_dirty(&candidates)
    }

    /// Begins a pointer press on `target`. Disabled nodes cannot become `ACTIVE`, though `hovered`
    /// still tracks them so disabled cursors (`CursorKind::NotAllowed`) resolve accurately.
    pub fn pointer_down(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        let mut candidates = Vec::with_capacity(3);
        self.push_candidate(&mut candidates, self.hovered);
        self.push_candidate(&mut candidates, self.active);
        self.push_candidate(&mut candidates, target);

        self.hovered = target;
        self.active = target.filter(|id| !self.disabled.contains(id));

        self.collect_dirty(&candidates)
    }

    /// Releases any active pointer press and updates hover to `target`, returning `(clicked_node, dirty)`
    /// where `clicked_node` is `Some(id)` iff the pointer went down and up on the same non-disabled node.
    pub fn pointer_up_clicked(&mut self, target: Option<NodeId>) -> (Option<NodeId>, Vec<NodeId>) {
        let mut candidates = Vec::with_capacity(3);
        self.push_candidate(&mut candidates, self.hovered);
        self.push_candidate(&mut candidates, self.active);
        self.push_candidate(&mut candidates, target);

        let clicked = self
            .active
            .filter(|&act| target == Some(act) && !self.disabled.contains(&act));
        self.active = None;
        self.hovered = target;

        (clicked, self.collect_dirty(&candidates))
    }

    /// Releases any active pointer press and updates hover to `target`.
    pub fn pointer_up(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.pointer_up_clicked(target).1
    }

    /// Begins a touch contact on `target`: clears `hovered` (preventing sticky hover on hybrid
    /// touch/mouse devices) and activates `target` unless disabled.
    pub fn touch_start(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        let mut candidates = Vec::with_capacity(3);
        self.push_candidate(&mut candidates, self.hovered);
        self.push_candidate(&mut candidates, self.active);
        self.push_candidate(&mut candidates, target);

        self.hovered = None;
        self.active = target.filter(|id| !self.disabled.contains(id));

        self.collect_dirty(&candidates)
    }

    /// Completes a touch contact, leaving both `hovered` and `active` cleared.
    pub fn touch_end(&mut self) -> Vec<NodeId> {
        self.pointer_leave()
    }

    /// Handles a completed touch tap: guarantees `hovered` and `active` are cleared so hybrid
    /// devices never suffer sticky `:hover` after touch input.
    pub fn touch_tap(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        let mut candidates = Vec::with_capacity(3);
        self.push_candidate(&mut candidates, self.hovered);
        self.push_candidate(&mut candidates, self.active);
        self.push_candidate(&mut candidates, target);

        self.hovered = None;
        self.active = None;

        self.collect_dirty(&candidates)
    }

    /// Moves keyboard focus (`FOCUS | FOCUS_VISIBLE`) to `target` (or blurs when `None`).
    /// Attempting to focus a disabled node is ignored without disturbing existing focus.
    pub fn focus_keyboard(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.set_focus_with_modality(target, FocusModality::Keyboard)
    }

    /// Moves pointer focus (`FOCUS` without `FOCUS_VISIBLE`) to `target` (or blurs when `None`).
    /// Attempting to focus a disabled node is ignored without disturbing existing focus.
    pub fn focus_pointer(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.set_focus_with_modality(target, FocusModality::Pointer)
    }

    /// Clears focus from the currently focused node.
    pub fn blur(&mut self) -> Vec<NodeId> {
        self.set_focus_with_modality(None, self.focus_modality)
    }

    fn set_focus_with_modality(
        &mut self,
        target: Option<NodeId>,
        modality: FocusModality,
    ) -> Vec<NodeId> {
        if let Some(id) = target
            && self.disabled.contains(&id)
        {
            return Vec::new();
        }
        let mut candidates = Vec::with_capacity(2);
        self.push_candidate(&mut candidates, self.focused);
        self.push_candidate(&mut candidates, target);

        self.focused = target;
        self.focus_modality = modality;

        self.collect_dirty(&candidates)
    }

    /// Marks `id` as disabled or enabled. Disabling a node immediately clears `active` and
    /// `focused` if they point to `id`.
    pub fn set_disabled(&mut self, id: NodeId, disabled: bool) -> Vec<NodeId> {
        let prev = self.state(id);
        if disabled {
            self.disabled.insert(id);
            if self.active == Some(id) {
                self.active = None;
            }
            if self.focused == Some(id) {
                self.focused = None;
            }
        } else {
            self.disabled.remove(&id);
        }
        if self.state(id) != prev {
            vec![id]
        } else {
            Vec::new()
        }
    }

    /// Cleans up all tracked state (`hovered`, `active`, `focused`, `disabled`) when `id` is
    /// unmounted from the tree.
    pub fn remove_node(&mut self, id: NodeId) -> Vec<NodeId> {
        let prev = self.state(id);
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        if self.active == Some(id) {
            self.active = None;
        }
        if self.focused == Some(id) {
            self.focused = None;
        }
        self.disabled.remove(&id);
        if prev != InteractionState::NONE {
            vec![id]
        } else {
            Vec::new()
        }
    }

    /// Resets all tracked interaction state.
    pub fn clear(&mut self) {
        self.hovered = None;
        self.active = None;
        self.focused = None;
        self.focus_modality = FocusModality::Pointer;
        self.disabled.clear();
    }
}
