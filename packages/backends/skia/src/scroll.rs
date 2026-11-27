//! `Scroll` table content bounds, offset clamping, and virtualized scroll reconciliation for
//! [`crate::SkiaBackend`].

use nve_abi::NodeId;
use nve_backend_headless::HeadlessBackend;
use nve_components::tables::Scroll;
use nve_layout::StoreLayoutEngine;
use nve_store::{NodeId as StoreNodeId, NodeStore, NveTable};

use crate::{SkiaBackend, MAX_HIT_TEST_DEPTH};

struct ScrollReconcileEntry {
    raw: NodeId,
    x: f32,
    y: f32,
    v_scroll_x: Option<f32>,
    v_scroll_y: Option<f32>,
}

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    pub fn scroll_offset(&self, node_id: NodeId) -> (f32, f32) {
        self.scroll_offsets.get(&node_id).copied().unwrap_or((0.0, 0.0))
    }

    pub fn scroll_content_size(&self, store: &NodeStore, node_id: StoreNodeId) -> Option<(f32, f32)> {
        compute_scroll_content_size(store, &self.layout_tree, node_id)
    }

    /// Sets the scroll offset of `node_id`, clamping against `[0..=content - viewport]`,
    /// rebuilds the retained [`crate::DisplayList`] if changed, and returns the clamped `(x, y)`.
    pub fn set_scroll_offset(
        &mut self,
        store: &NodeStore,
        node_id: StoreNodeId,
        x: f32,
        y: f32,
    ) -> (f32, f32) {
        let Some(rect) = self.layout_tree.layout_of(node_id.raw()) else {
            return (0.0, 0.0);
        };
        let (cw, ch) = compute_scroll_content_size(store, &self.layout_tree, node_id)
            .unwrap_or((rect.width, rect.height));
        let max_x = (cw - rect.width).max(0.0);
        let max_y = (ch - rect.height).max(0.0);
        let clamped_x = if x.is_finite() { x.clamp(0.0, max_x) } else { 0.0 };
        let clamped_y = if y.is_finite() { y.clamp(0.0, max_y) } else { 0.0 };

        let raw = node_id.raw();
        let prev = self.scroll_offsets.get(&raw).copied().unwrap_or((0.0, 0.0));
        self.scroll_offsets.insert(raw, (clamped_x, clamped_y));
        self.layout_tree.set_scroll_offset(raw, clamped_x, clamped_y);
        if (prev.0 - clamped_x).abs() > 1e-4 || (prev.1 - clamped_y).abs() > 1e-4 {
            self.rebuild_display_list(store);
        }
        (clamped_x, clamped_y)
    }

    pub fn apply_scroll_delta(
        &mut self,
        store: &NodeStore,
        node_id: StoreNodeId,
        dx: f32,
        dy: f32,
    ) -> (f32, f32) {
        let (cur_x, cur_y) = self.scroll_offset(node_id.raw());
        let nx = cur_x + if dx.is_finite() { dx } else { 0.0 };
        let ny = cur_y + if dy.is_finite() { dy } else { 0.0 };
        self.set_scroll_offset(store, node_id, nx, ny)
    }

    pub(crate) fn visual_scroll_shift(&self, store: &NodeStore, id: StoreNodeId) -> (f32, f32) {
        crate::display_list::scroll_shift_for(store, &self.scroll_offsets, id)
    }

    pub(crate) fn reconcile_scroll_offsets(&mut self, store: &NodeStore) {
        self.scroll_offsets
            .retain(|&raw, _| store.is_live(StoreNodeId(raw)));
        self.last_virtual_scroll_x
            .retain(|&raw, _| store.is_live(StoreNodeId(raw)));
        self.last_virtual_scroll_y
            .retain(|&raw, _| store.is_live(StoreNodeId(raw)));

        if store.table_desc(Scroll::slot()).is_none() {
            return;
        }

        let mut updates: Vec<ScrollReconcileEntry> = Vec::new();
        self.collect_scroll_reconcile(store, StoreNodeId::ROOT, &mut updates, 0);
        for entry in updates {
            if let Some(vx) = entry.v_scroll_x {
                self.last_virtual_scroll_x.insert(entry.raw, vx);
            }
            if let Some(vy) = entry.v_scroll_y {
                self.last_virtual_scroll_y.insert(entry.raw, vy);
            }
            if entry.x != 0.0 || entry.y != 0.0 || self.scroll_offsets.contains_key(&entry.raw) {
                self.scroll_offsets.insert(entry.raw, (entry.x, entry.y));
                self.layout_tree
                    .set_scroll_offset(entry.raw, entry.x, entry.y);
            }
        }
    }

    fn collect_scroll_reconcile(
        &self,
        store: &NodeStore,
        id: StoreNodeId,
        out: &mut Vec<ScrollReconcileEntry>,
        depth: usize,
    ) {
        if depth > MAX_HIT_TEST_DEPTH || !store.is_live(id) {
            return;
        }
        if let Some(scroll) = store.try_row::<Scroll>(id)
            && let Some(rect) = self.layout_tree.layout_of(id.raw())
        {
            let (cw, ch) = compute_scroll_content_size(store, &self.layout_tree, id)
                .unwrap_or((rect.width, rect.height));
            let max_x = (cw - rect.width).max(0.0);
            let max_y = (ch - rect.height).max(0.0);
            let raw = id.raw();
            let (mut sx, mut sy) = self.scroll_offsets.get(&raw).copied().unwrap_or((0.0, 0.0));
            let req_y = scroll.requested_scroll_y();
            let mut recorded_req_y = None;
            if req_y.is_finite() {
                let prev_req = self.last_virtual_scroll_y.get(&raw).copied();
                if prev_req.is_none_or(|p| (p - req_y).abs() > 1e-4) {
                    sy = req_y;
                    recorded_req_y = Some(req_y);
                }
            }
            let req_x = scroll.requested_scroll_x();
            let mut recorded_req_x = None;
            if req_x.is_finite() {
                let prev_req = self.last_virtual_scroll_x.get(&raw).copied();
                if prev_req.is_none_or(|p| (p - req_x).abs() > 1e-4) {
                    sx = req_x;
                    recorded_req_x = Some(req_x);
                }
            }
            out.push(ScrollReconcileEntry {
                raw,
                x: sx.clamp(0.0, max_x),
                y: sy.clamp(0.0, max_y),
                v_scroll_x: recorded_req_x,
                v_scroll_y: recorded_req_y,
            });
        }
        for &cid in store.children(id) {
            self.collect_scroll_reconcile(store, cid, out, depth + 1);
        }
    }
}

/// Computes `(content_width, content_height)` for a `Scroll` node.
pub(crate) fn compute_scroll_content_size<L: StoreLayoutEngine>(
    store: &NodeStore,
    layout_tree: &HeadlessBackend<L>,
    node_id: StoreNodeId,
) -> Option<(f32, f32)> {
    let rect = layout_tree.layout_of(node_id.raw())?;
    let mut max_w = rect.width;
    let mut max_h = rect.height;

    if let Some(scroll) = store.try_row::<Scroll>(node_id) {
        let vw = scroll.virtual_total_width();
        if vw.is_finite() && vw > max_w {
            max_w = vw;
        }
        let vh = scroll.virtual_total_height();
        if vh.is_finite() && vh > max_h {
            max_h = vh;
        }
    }

    for &cid in store.children(node_id) {
        if let Some(cr) = layout_tree.layout_of(cid.raw()) {
            max_w = max_w.max((cr.x - rect.x) + cr.width);
            max_h = max_h.max((cr.y - rect.y) + cr.height);
        }
    }
    Some((max_w.max(0.0), max_h.max(0.0)))
}
