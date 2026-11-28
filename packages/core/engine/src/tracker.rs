//! Reusable zero-allocation stateful interaction tracker (`InteractionTracker`) for native backends
//! (`HeadlessBackend`, `SkiaBackend`).
//!
//! Tracks leaf-to-root ancestor chains for `:hover` and `:active`, active pointer capture
//! (`(pointer_id, node)`), `:focus` + [`FocusModality`] (`:focus-visible`), and `:disabled`.
//! All steady-state transitions reuse internal scratch buffers with zero heap allocations.

use nve_abi::hash::HashSet;
use nve_abi::NodeId;
use nve_tables::interaction::{FocusModality, InteractionState};

const MAX_CHAIN_DEPTH: usize = 256;

/// Reusable stateful interaction tracker for native backends (`HeadlessBackend`, `SkiaBackend`).
///
/// Tracks leaf-to-root `hovered_chain`, `active_chain`, `captured` pointer ownership, `focused` +
/// [`FocusModality`] (`FOCUS` vs `FOCUS_VISIBLE`), and `disabled` nodes. Slice-returning methods
/// operate with **0 heap allocations** in steady state by reusing internal scratch buffers.
#[derive(Debug, Clone, Default)]
pub struct InteractionTracker {
    /// Leaf-to-root hovered ancestor chain (`[leaf, parent, ..., root]`).
    hovered_chain: Vec<NodeId>,
    /// Leaf-to-root active (non-disabled) ancestor chain.
    active_chain: Vec<NodeId>,
    /// Active pointer capture `(pointer_id, captured_node)`.
    captured: Option<(u32, NodeId)>,
    focused: Option<NodeId>,
    focus_modality: FocusModality,
    disabled: HashSet<NodeId>,
    /// Top-down (`root -> leaf`) nodes that entered hover on the latest move/down/up transition.
    entered_scratch: Vec<NodeId>,
    /// Bottom-up (`leaf -> root`) nodes that exited hover on the latest move/down/up/leave/cancel.
    exited_scratch: Vec<NodeId>,
    /// Sorted deduplicated dirty `NodeId`s produced by the latest transition.
    prev_dirty_scratch: Vec<NodeId>,
    /// Reusable buffer for assembling the next leaf-to-root ancestor chain.
    next_chain_scratch: Vec<NodeId>,
    /// Reusable buffer snapshotting prior [`InteractionState`] of candidate nodes.
    candidate_scratch: Vec<(NodeId, InteractionState)>,
}

impl InteractionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn hovered(&self) -> Option<NodeId> {
        self.hovered_chain.first().copied()
    }

    #[inline]
    pub fn active(&self) -> Option<NodeId> {
        self.active_chain.first().copied()
    }

    #[inline]
    pub fn hovered_chain(&self) -> &[NodeId] {
        &self.hovered_chain
    }

    #[inline]
    pub fn active_chain(&self) -> &[NodeId] {
        &self.active_chain
    }

    #[inline]
    pub fn captured(&self) -> Option<(u32, NodeId)> {
        self.captured
    }

    #[inline]
    pub fn captured_node(&self) -> Option<NodeId> {
        self.captured.map(|(_, id)| id)
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

    /// Top-down (`root -> leaf`) nodes that entered hover during the latest pointer transition.
    #[inline]
    pub fn entered(&self) -> &[NodeId] {
        &self.entered_scratch
    }

    /// Bottom-up (`leaf -> root`) nodes that exited hover during the latest pointer transition.
    #[inline]
    pub fn exited(&self) -> &[NodeId] {
        &self.exited_scratch
    }

    /// Sorted, deduplicated dirty `NodeId`s from the latest transition.
    #[inline]
    pub fn last_dirty(&self) -> &[NodeId] {
        &self.prev_dirty_scratch
    }

    /// Returns the node that currently governs the window pointer cursor:
    /// prefers `captured`, then `active`, falling back to `hovered`.
    #[inline]
    pub fn active_cursor_node(&self) -> Option<NodeId> {
        self.captured_node()
            .or_else(|| self.active())
            .or_else(|| self.hovered())
    }

    /// Sets active pointer capture on `(pointer_id, node)`.
    #[inline]
    pub fn set_capture(&mut self, pointer_id: u32, node: NodeId) {
        if !self.disabled.contains(&node) {
            self.captured = Some((pointer_id, node));
        }
    }

    /// Releases capture if owned by `pointer_id`, returning the previously captured node.
    #[inline]
    pub fn release_capture(&mut self, pointer_id: u32) -> Option<NodeId> {
        if let Some((pid, node)) = self.captured
            && pid == pointer_id
        {
            self.captured = None;
            return Some(node);
        }
        None
    }

    /// Unconditionally clears active pointer capture, returning `(pointer_id, node)` if set.
    #[inline]
    pub fn clear_capture(&mut self) -> Option<(u32, NodeId)> {
        self.captured.take()
    }

    /// Computes the exact active [`InteractionState`] flags for `id`.
    #[inline]
    pub fn state(&self, id: NodeId) -> InteractionState {
        let mut s = InteractionState::NONE;
        if self.hovered_chain.contains(&id) {
            s |= InteractionState::HOVER;
        }
        if self.focused == Some(id) {
            s |= InteractionState::FOCUS;
            if self.focus_modality == FocusModality::Keyboard {
                s |= InteractionState::FOCUS_VISIBLE;
            }
        }
        if self.active_chain.contains(&id) {
            s |= InteractionState::ACTIVE;
        }
        if self.disabled.contains(&id) {
            s |= InteractionState::DISABLED;
        }
        s
    }

    fn build_chain(&mut self, target: Option<NodeId>, parent_of: impl Fn(NodeId) -> Option<NodeId>) {
        self.next_chain_scratch.clear();
        let Some(mut cur) = target else {
            return;
        };
        for _ in 0..MAX_CHAIN_DEPTH {
            if !self.next_chain_scratch.contains(&cur) {
                self.next_chain_scratch.push(cur);
            }
            match parent_of(cur) {
                Some(p) if p != cur => cur = p,
                _ => break,
            }
        }
    }

    #[inline]
    fn push_candidate_id(&mut self, id: NodeId) {
        if !self.candidate_scratch.iter().any(|(existing, _)| *existing == id) {
            let st = self.state(id);
            self.candidate_scratch.push((id, st));
        }
    }

    fn snapshot_pointer_candidates(&mut self) {
        self.candidate_scratch.clear();
        for i in 0..self.hovered_chain.len() {
            let id = self.hovered_chain[i];
            self.push_candidate_id(id);
        }
        for i in 0..self.active_chain.len() {
            let id = self.active_chain[i];
            self.push_candidate_id(id);
        }
        for i in 0..self.next_chain_scratch.len() {
            let id = self.next_chain_scratch[i];
            self.push_candidate_id(id);
        }
    }

    fn diff_hover_enter_exit(&mut self) {
        self.exited_scratch.clear();
        for &id in &self.hovered_chain {
            if !self.next_chain_scratch.contains(&id) {
                self.exited_scratch.push(id);
            }
        }
        self.entered_scratch.clear();
        for &id in self.next_chain_scratch.iter().rev() {
            if !self.hovered_chain.contains(&id) {
                self.entered_scratch.push(id);
            }
        }
    }

    fn finalize_dirty(&mut self) -> &[NodeId] {
        self.prev_dirty_scratch.clear();
        for i in 0..self.candidate_scratch.len() {
            let (id, prev) = self.candidate_scratch[i];
            if self.state(id) != prev && !self.prev_dirty_scratch.contains(&id) {
                self.prev_dirty_scratch.push(id);
            }
        }
        self.prev_dirty_scratch.sort_unstable();
        &self.prev_dirty_scratch
    }

    /// Updates hover ancestor chain to `target` (zero-alloc steady state).
    ///
    /// Populates [`Self::exited`] (bottom-up) and [`Self::entered`] (top-down). If no pointer
    /// capture is active, cancels `:active` on ancestors no longer hovered (`press-cancel on drag-off`).
    pub fn pointer_move_with(
        &mut self,
        target: Option<NodeId>,
        parent_of: impl Fn(NodeId) -> Option<NodeId>,
    ) -> &[NodeId] {
        self.build_chain(target, parent_of);
        self.snapshot_pointer_candidates();
        self.diff_hover_enter_exit();
        core::mem::swap(&mut self.hovered_chain, &mut self.next_chain_scratch);
        if self.captured.is_none() {
            self.active_chain.retain(|id| self.hovered_chain.contains(id));
        }
        self.finalize_dirty()
    }

    /// Clears hover when the pointer exits the surface/window (zero-alloc steady state).
    ///
    /// Preserves `captured` and `active_chain` while a pointer capture is active so captured drags
    /// survive `CursorLeft`.
    pub fn pointer_leave_slice(&mut self) -> &[NodeId] {
        self.next_chain_scratch.clear();
        self.snapshot_pointer_candidates();
        self.exited_scratch.clear();
        self.exited_scratch.extend_from_slice(&self.hovered_chain);
        self.entered_scratch.clear();
        self.hovered_chain.clear();
        if self.captured.is_none() {
            self.active_chain.clear();
        }
        self.finalize_dirty()
    }

    /// Cancels any active pointer interaction, unconditionally releasing capture, `:active`, and
    /// `:hover` (zero-alloc steady state).
    pub fn pointer_cancel_slice(&mut self) -> &[NodeId] {
        self.next_chain_scratch.clear();
        self.snapshot_pointer_candidates();
        self.exited_scratch.clear();
        self.exited_scratch.extend_from_slice(&self.hovered_chain);
        self.entered_scratch.clear();
        self.captured = None;
        self.hovered_chain.clear();
        self.active_chain.clear();
        self.finalize_dirty()
    }

    /// Begins a pointer press on `target`'s ancestor chain (zero-alloc steady state).
    /// Disabled nodes stay in `hovered_chain` for cursor resolution but never enter `active_chain`.
    pub fn pointer_down_with(
        &mut self,
        target: Option<NodeId>,
        parent_of: impl Fn(NodeId) -> Option<NodeId>,
    ) -> &[NodeId] {
        self.build_chain(target, parent_of);
        self.snapshot_pointer_candidates();
        self.diff_hover_enter_exit();
        core::mem::swap(&mut self.hovered_chain, &mut self.next_chain_scratch);
        self.active_chain.clear();
        for &id in &self.hovered_chain {
            if !self.disabled.contains(&id) {
                self.active_chain.push(id);
            }
        }
        self.finalize_dirty()
    }

    /// Releases any active press and pointer capture, updating hover to `target`'s chain (zero-alloc
    /// steady state). Returns `(clicked_node, dirty)` where `clicked_node` is the deepest common
    /// non-disabled node present in both the press `active_chain` and release target chain.
    pub fn pointer_up_with(
        &mut self,
        target: Option<NodeId>,
        parent_of: impl Fn(NodeId) -> Option<NodeId>,
    ) -> (Option<NodeId>, &[NodeId]) {
        self.build_chain(target, parent_of);
        self.snapshot_pointer_candidates();
        let clicked = self
            .active_chain
            .iter()
            .copied()
            .find(|id| !self.disabled.contains(id) && self.next_chain_scratch.contains(id));
        self.diff_hover_enter_exit();
        self.active_chain.clear();
        self.captured = None;
        core::mem::swap(&mut self.hovered_chain, &mut self.next_chain_scratch);
        let dirty = self.finalize_dirty();
        (clicked, dirty)
    }

    /// Moves focus to `target` under `modality` (zero-alloc steady state).
    /// Attempting to focus a disabled node is ignored without disturbing existing focus.
    pub fn set_focus(&mut self, target: Option<NodeId>, modality: FocusModality) -> &[NodeId] {
        if let Some(id) = target
            && self.disabled.contains(&id)
        {
            self.prev_dirty_scratch.clear();
            return &self.prev_dirty_scratch;
        }
        self.candidate_scratch.clear();
        if let Some(prev) = self.focused {
            self.push_candidate_id(prev);
        }
        if let Some(next) = target {
            self.push_candidate_id(next);
        }
        self.focused = target;
        self.focus_modality = modality;
        self.finalize_dirty()
    }

    /// Marks `id` disabled or enabled (zero-alloc when `disabled` set already has capacity).
    /// Disabling clears `active`, `focused`, and `captured` on `id`.
    pub fn set_disabled_slice(&mut self, id: NodeId, disabled: bool) -> &[NodeId] {
        self.candidate_scratch.clear();
        self.push_candidate_id(id);
        if disabled {
            self.disabled.insert(id);
            self.active_chain.retain(|&n| n != id);
            if self.focused == Some(id) {
                self.focused = None;
            }
            if self.captured.is_some_and(|(_, n)| n == id) {
                self.captured = None;
            }
        } else {
            self.disabled.remove(&id);
        }
        self.finalize_dirty()
    }

    /// Removes `id` from all tracked chains, capture, focus, and disabled state (zero-alloc).
    pub fn remove_node_slice(&mut self, id: NodeId) -> &[NodeId] {
        let prev = self.state(id);
        self.hovered_chain.retain(|&n| n != id);
        self.active_chain.retain(|&n| n != id);
        if self.focused == Some(id) {
            self.focused = None;
        }
        if self.captured.is_some_and(|(_, n)| n == id) {
            self.captured = None;
        }
        self.disabled.remove(&id);
        self.prev_dirty_scratch.clear();
        if prev != InteractionState::NONE {
            self.prev_dirty_scratch.push(id);
        }
        &self.prev_dirty_scratch
    }

    /// Updates pointer hover to `target` (single-node convenience wrapper).
    pub fn pointer_move(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.pointer_move_with(target, |_| None).to_vec()
    }

    /// Clears hover (and non-captured active press) when the pointer exits the surface/window.
    pub fn pointer_leave(&mut self) -> Vec<NodeId> {
        self.pointer_leave_slice().to_vec()
    }

    /// Begins a pointer press on `target` (single-node convenience wrapper).
    pub fn pointer_down(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.pointer_down_with(target, |_| None).to_vec()
    }

    /// Releases any active pointer press and updates hover to `target`, returning `(clicked, dirty)`.
    pub fn pointer_up_clicked(&mut self, target: Option<NodeId>) -> (Option<NodeId>, Vec<NodeId>) {
        let (clicked, dirty) = self.pointer_up_with(target, |_| None);
        (clicked, dirty.to_vec())
    }

    /// Releases any active pointer press and updates hover to `target`.
    pub fn pointer_up(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.pointer_up_with(target, |_| None).1.to_vec()
    }

    /// Begins a touch contact on `target`: clears hover (preventing sticky hover on hybrid devices)
    /// and activates `target` unless disabled.
    pub fn touch_start(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.build_chain(target, |_| None);
        self.snapshot_pointer_candidates();
        self.exited_scratch.clear();
        self.exited_scratch.extend_from_slice(&self.hovered_chain);
        self.entered_scratch.clear();
        self.hovered_chain.clear();
        self.active_chain.clear();
        for &id in &self.next_chain_scratch {
            if !self.disabled.contains(&id) {
                self.active_chain.push(id);
            }
        }
        self.finalize_dirty().to_vec()
    }

    /// Completes a touch contact, leaving hover, active, and capture cleared.
    pub fn touch_end(&mut self) -> Vec<NodeId> {
        self.pointer_cancel_slice().to_vec()
    }

    /// Handles a completed touch tap, ensuring hover/active/capture are cleared.
    pub fn touch_tap(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.build_chain(target, |_| None);
        self.snapshot_pointer_candidates();
        self.exited_scratch.clear();
        self.exited_scratch.extend_from_slice(&self.hovered_chain);
        self.entered_scratch.clear();
        self.captured = None;
        self.hovered_chain.clear();
        self.active_chain.clear();
        self.finalize_dirty().to_vec()
    }

    /// Moves keyboard focus (`FOCUS | FOCUS_VISIBLE`) to `target` (or blurs when `None`).
    pub fn focus_keyboard(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.set_focus(target, FocusModality::Keyboard).to_vec()
    }

    /// Moves pointer focus (`FOCUS` without `FOCUS_VISIBLE`) to `target` (or blurs when `None`).
    pub fn focus_pointer(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.set_focus(target, FocusModality::Pointer).to_vec()
    }

    /// Clears focus from the currently focused node.
    pub fn blur(&mut self) -> Vec<NodeId> {
        self.set_focus(None, self.focus_modality).to_vec()
    }

    /// Marks `id` as disabled or enabled.
    pub fn set_disabled(&mut self, id: NodeId, disabled: bool) -> Vec<NodeId> {
        self.set_disabled_slice(id, disabled).to_vec()
    }

    /// Cleans up all tracked state when `id` is unmounted from the tree.
    pub fn remove_node(&mut self, id: NodeId) -> Vec<NodeId> {
        self.remove_node_slice(id).to_vec()
    }

    /// Resets all tracked interaction state (retaining scratch capacity).
    pub fn clear(&mut self) {
        self.hovered_chain.clear();
        self.active_chain.clear();
        self.captured = None;
        self.focused = None;
        self.focus_modality = FocusModality::Pointer;
        self.disabled.clear();
        self.entered_scratch.clear();
        self.exited_scratch.clear();
        self.prev_dirty_scratch.clear();
        self.next_chain_scratch.clear();
        self.candidate_scratch.clear();
    }
}
