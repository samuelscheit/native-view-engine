//! `Scroll` table content bounds, O(1) clamped offset updates, `scroll_into_view`, and
//! virtualized scroll reconciliation for [`crate::SkiaBackend`], backed directly by
//! [`nve_layout::LaidOutScene`].

use nve_abi::NodeId;
use nve_backend_headless::HeadlessBackend;
use nve_layout::StoreLayoutEngine;
use nve_store::{ChangeSet, NodeStore};
use nve_tables::tables::Scroll;

use crate::display_list::is_subtree_display_none_effective;
use crate::{SkiaBackend, MAX_HIT_TEST_DEPTH};

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    #[inline]
    pub fn scroll_offset(&self, node_id: NodeId) -> (f32, f32) {
        self.layout_tree.scene().scroll_offset(node_id)
    }

    pub fn scroll_content_size(&self, store: &NodeStore, node_id: NodeId) -> Option<(f32, f32)> {
        if let Some((_, _, cw, ch)) = self.scene.cached_scroll_bounds(node_id) {
            return Some((cw, ch));
        }
        self.layout_tree.scene().scroll_content_size(store, node_id)
    }

    /// Sets the scroll offset of `node_id`, clamping against `[0..=content - viewport]` in O(1)
    /// when `CompositorScene` has cached scroll bounds, and mutating the retained scroll property
    /// node in O(1) (0 allocs) without re-emitting paint ops.
    pub fn set_scroll_offset(
        &mut self,
        store: &NodeStore,
        node_id: NodeId,
        x: f32,
        y: f32,
    ) -> (f32, f32) {
        let (clamped_x, clamped_y, changed) =
            if let Some((vw, vh, cw, ch)) = self.scene.cached_scroll_bounds(node_id) {
                let max_x = (cw - vw).max(0.0);
                let max_y = (ch - vh).max(0.0);
                let cx = if x.is_finite() { x.clamp(0.0, max_x) } else { 0.0 };
                let cy = if y.is_finite() { y.clamp(0.0, max_y) } else { 0.0 };
                let prev = self.layout_tree.scene().scroll_offset(node_id);
                self.layout_tree.scene_mut().set_scroll_offset_raw(node_id, cx, cy);
                let diff = (prev.0 - cx).abs() > 1e-4 || (prev.1 - cy).abs() > 1e-4;
                (cx, cy, diff)
            } else {
                self.layout_tree
                    .scene_mut()
                    .set_scroll_offset_clamped(store, node_id, x, y)
            };
        if changed {
            self.update_scene_scroll(store, node_id, clamped_x, clamped_y);
        }
        (clamped_x, clamped_y)
    }

    pub fn apply_scroll_delta(
        &mut self,
        store: &NodeStore,
        node_id: NodeId,
        dx: f32,
        dy: f32,
    ) -> (f32, f32) {
        let (cur_x, cur_y) = self.scroll_offset(node_id);
        let nx = cur_x + if dx.is_finite() { dx } else { 0.0 };
        let ny = cur_y + if dy.is_finite() { dy } else { 0.0 };
        self.set_scroll_offset(store, node_id, nx, ny)
    }

    /// Scrolls every ancestor `Scroll` container minimally so `node_id` becomes visible within its
    /// viewport (accounting for `virtual_top_spacer` and rejecting `display: none` subtrees).
    /// Returns `true` if any scroll offset changed.
    pub fn scroll_into_view(&mut self, store: &NodeStore, node_id: NodeId) -> bool {
        self.scroll_into_view_collecting(store, node_id, None)
    }

    /// Like [`Self::scroll_into_view`], optionally pushing every scrolled ancestor container
    /// [`NodeId`] into `scrolled_containers` so hosts can emit `ScrollEvent::Scroll` and update
    /// AccessKit in O(1).
    pub fn scroll_into_view_collecting(
        &mut self,
        store: &NodeStore,
        node_id: NodeId,
        mut scrolled_containers: Option<&mut Vec<NodeId>>,
    ) -> bool {
        if !store.is_live(node_id)
            || is_subtree_display_none_effective(store, &self.layout_tree, node_id)
        {
            return false;
        }
        let Some(target_rect) = self.layout_tree.layout_of(node_id) else {
            return false;
        };
        let mut changed = false;
        let mut cur_x0 = target_rect.x;
        let mut cur_y0 = target_rect.y;
        let mut cur_x1 = target_rect.x + target_rect.width;
        let mut cur_y1 = target_rect.y + target_rect.height;

        let mut ancestor = store.parent(node_id);
        let mut depth = 0usize;
        while let Some(pid) = ancestor {
            if depth > MAX_HIT_TEST_DEPTH {
                break;
            }
            depth += 1;
            if let Some(scroll) = store.try_row::<Scroll>(pid)
                && let Some(view) = self.layout_tree.layout_of(pid)
            {
                let spacer = scroll
                    .virtual_top_spacer()
                    .filter(|&v| v.is_finite() && v > 0.0)
                    .unwrap_or(0.0);
                let (sx, sy) = self.scroll_offset(pid);
                let (shift_x, shift_y) = self.visual_scroll_shift(store, pid);
                let vis_x0 = cur_x0 - shift_x;
                let vis_x1 = cur_x1 - shift_x;
                let vis_y0 = cur_y0 - shift_y;
                let vis_y1 = cur_y1 - shift_y;

                let dx = if vis_x0 < view.x {
                    vis_x0 - view.x
                } else if vis_x1 > view.x + view.width {
                    (vis_x1 - (view.x + view.width)).min(vis_x0 - view.x)
                } else {
                    0.0
                };
                let dy = if vis_y0 < view.y {
                    vis_y0 - view.y
                } else if vis_y1 > view.y + view.height {
                    (vis_y1 - (view.y + view.height)).min(vis_y0 - view.y)
                } else {
                    0.0
                };

                if dx.abs() > 1e-3 || dy.abs() > 1e-3 {
                    let target_x = (sx + dx).max(0.0);
                    let target_y = if dy.abs() > 1e-3 {
                        let desired_shift_y = (shift_y + dy).max(0.0);
                        if spacer > 0.0 {
                            spacer + desired_shift_y
                        } else {
                            desired_shift_y
                        }
                    } else {
                        sy
                    };
                    let next = self.set_scroll_offset(store, pid, target_x, target_y);
                    if (sx - next.0).abs() > 1e-4 || (sy - next.1).abs() > 1e-4 {
                        changed = true;
                        if let Some(out) = scrolled_containers.as_deref_mut() {
                            out.push(pid);
                        }
                    }
                }
                let (new_shift_x, new_shift_y) = self.visual_scroll_shift(store, pid);
                cur_x0 -= new_shift_x;
                cur_x1 -= new_shift_x;
                cur_y0 -= new_shift_y;
                cur_y1 -= new_shift_y;
            }
            ancestor = store.parent(pid);
        }
        changed
    }

    #[inline]
    pub fn visual_scroll_shift(&self, store: &NodeStore, id: NodeId) -> (f32, f32) {
        self.layout_tree.scene().visual_scroll_shift(store, id)
    }

    #[inline]
    pub(crate) fn reconcile_scroll_offsets(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        needs_layout: bool,
    ) {
        self.layout_tree
            .scene_mut()
            .reconcile_scroll_offsets_if_needed(store, changes, needs_layout);
    }
}

/// Computes `(content_width, content_height)` for a `Scroll` node.
pub(crate) fn compute_scroll_content_size<L: StoreLayoutEngine>(
    store: &NodeStore,
    layout_tree: &HeadlessBackend<L>,
    node_id: NodeId,
) -> Option<(f32, f32)> {
    layout_tree.scene().scroll_content_size(store, node_id)
}
