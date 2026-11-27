//! Per-surface scroll offsets, content bounds, clamping, and virtualized scroll reconciliation.

use std::collections::HashMap;

use nve_abi::NodeId as AbiNodeId;
use nve_components::tables::Scroll;
use nve_engine::LayoutRect;
use nve_store::{ChangeSet, NodeId as StoreNodeId, NodeStore, NveTable};

use crate::StoreLayoutEngine;

use super::LaidOutScene;

/// Per-surface scroll offset map plus programmatic `requested_scroll_{x,y}` deduplication.
#[derive(Debug, Default, Clone)]
pub struct ScrollOffsets {
    offsets: HashMap<AbiNodeId, (f32, f32)>,
    last_requested_x: HashMap<AbiNodeId, f32>,
    last_requested_y: HashMap<AbiNodeId, f32>,
    reconcile_scratch: Vec<ScrollReconcileEntry>,
}

impl ScrollOffsets {
    #[inline]
    pub fn get(&self, id: AbiNodeId) -> (f32, f32) {
        self.offsets.get(&id).copied().unwrap_or((0.0, 0.0))
    }

    #[inline]
    pub fn contains(&self, id: AbiNodeId) -> bool {
        self.offsets.contains_key(&id)
    }

    #[inline]
    pub fn set_raw(&mut self, id: AbiNodeId, x: f32, y: f32) {
        self.offsets.insert(id, (x, y));
    }

    #[inline]
    pub fn remove(&mut self, id: AbiNodeId) {
        self.offsets.remove(&id);
        self.last_requested_x.remove(&id);
        self.last_requested_y.remove(&id);
    }

    /// Every tracked `(container, (x, y))` offset, in unspecified order.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (AbiNodeId, (f32, f32))> + '_ {
        self.offsets.iter().map(|(&id, &xy)| (id, xy))
    }

    #[inline]
    pub fn clear(&mut self) {
        self.offsets.clear();
        self.last_requested_x.clear();
        self.last_requested_y.clear();
    }

    /// Visual child shift for `id`, subtracting any virtualized `Scroll::virtual_top_spacer`.
    #[inline]
    pub fn visual_shift(&self, store: &NodeStore, id: StoreNodeId) -> (f32, f32) {
        let (sx, sy) = self.get(id.raw());
        if let Some(scroll) = store.try_row::<Scroll>(id) {
            let top_spacer = scroll.virtual_top_spacer();
            if top_spacer.is_finite() && top_spacer > 0.0 {
                return (sx, (sy - top_spacer).max(0.0));
            }
        }
        (sx, sy)
    }
}

/// Computes `(content_width, content_height)` for a scroll container node.
pub fn compute_scroll_content_size(
    store: &NodeStore,
    layout_of: impl Fn(StoreNodeId) -> Option<LayoutRect>,
    node_id: StoreNodeId,
) -> Option<(f32, f32)> {
    let rect = layout_of(node_id)?;
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
        if let Some(cr) = layout_of(cid) {
            max_w = max_w.max((cr.x - rect.x) + cr.width);
            max_h = max_h.max((cr.y - rect.y) + cr.height);
        }
    }
    Some((max_w.max(0.0), max_h.max(0.0)))
}

#[derive(Debug, Clone, Copy)]
struct ScrollReconcileEntry {
    raw: AbiNodeId,
    x: f32,
    y: f32,
    req_x: Option<f32>,
    req_y: Option<f32>,
}

impl<L: StoreLayoutEngine> LaidOutScene<L> {
    #[inline]
    pub fn scroll_offsets(&self) -> &ScrollOffsets {
        &self.scroll
    }

    #[inline]
    pub fn set_scroll_offset_raw(&mut self, id: AbiNodeId, dx: f32, dy: f32) {
        self.scroll.set_raw(id, dx, dy);
    }

    #[inline]
    pub fn scroll_offset(&self, id: AbiNodeId) -> (f32, f32) {
        self.scroll.get(id)
    }

    #[inline]
    pub fn visual_scroll_shift(&self, store: &NodeStore, id: StoreNodeId) -> (f32, f32) {
        self.scroll.visual_shift(store, id)
    }

    pub fn scroll_content_size(
        &self,
        store: &NodeStore,
        node_id: StoreNodeId,
    ) -> Option<(f32, f32)> {
        compute_scroll_content_size(store, |id| self.layout.layout_of(id), node_id)
    }

    /// Sets `node_id`'s scroll offset clamped to `[0..=content - viewport]`.
    /// Returns `(clamped_x, clamped_y, changed)`.
    pub fn set_scroll_offset_clamped(
        &mut self,
        store: &NodeStore,
        node_id: StoreNodeId,
        x: f32,
        y: f32,
    ) -> (f32, f32, bool) {
        let Some(rect) = self.layout.layout_of(node_id) else {
            return (0.0, 0.0, false);
        };
        let (cw, ch) = self
            .scroll_content_size(store, node_id)
            .unwrap_or((rect.width, rect.height));
        let max_x = (cw - rect.width).max(0.0);
        let max_y = (ch - rect.height).max(0.0);
        let clamped_x = if x.is_finite() { x.clamp(0.0, max_x) } else { 0.0 };
        let clamped_y = if y.is_finite() { y.clamp(0.0, max_y) } else { 0.0 };

        let raw = node_id.raw();
        let prev = self.scroll.get(raw);
        self.scroll.set_raw(raw, clamped_x, clamped_y);
        let changed = (prev.0 - clamped_x).abs() > 1e-4 || (prev.1 - clamped_y).abs() > 1e-4;
        (clamped_x, clamped_y, changed)
    }

    /// Applies an incremental scroll delta `(dx, dy)` clamped against content bounds.
    /// Returns `(clamped_x, clamped_y, changed)`.
    pub fn apply_scroll_delta(
        &mut self,
        store: &NodeStore,
        node_id: StoreNodeId,
        dx: f32,
        dy: f32,
    ) -> (f32, f32, bool) {
        let (cur_x, cur_y) = self.scroll.get(node_id.raw());
        let nx = cur_x + if dx.is_finite() { dx } else { 0.0 };
        let ny = cur_y + if dy.is_finite() { dy } else { 0.0 };
        self.set_scroll_offset_clamped(store, node_id, nx, ny)
    }

    /// Fast-path guard around [`Self::reconcile_scroll_offsets`]: skips all work in `O(1)` when
    /// layout did not run, no `Scroll` table row changed, and no nodes were removed.
    #[inline]
    pub fn reconcile_scroll_offsets_if_needed(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        laid_out: bool,
    ) {
        if !laid_out
            && changes.removed.is_empty()
            && changes.table(Scroll::slot()).is_none()
        {
            return;
        }
        self.reconcile_scroll_offsets(store);
    }

    /// Prunes removed scroll nodes, honors new `Scroll::requested_scroll_{x,y}`, and re-clamps
    /// existing offsets against newly laid-out content bounds in `O(scroll_containers)` time with
    /// zero steady-state heap allocations.
    pub fn reconcile_scroll_offsets(&mut self, store: &NodeStore) {
        if !self.scroll.offsets.is_empty() {
            self.scroll
                .offsets
                .retain(|&raw, _| store.is_live(StoreNodeId(raw)));
        }
        if !self.scroll.last_requested_x.is_empty() {
            self.scroll
                .last_requested_x
                .retain(|&raw, _| store.is_live(StoreNodeId(raw)));
        }
        if !self.scroll.last_requested_y.is_empty() {
            self.scroll
                .last_requested_y
                .retain(|&raw, _| store.is_live(StoreNodeId(raw)));
        }

        let Some((_rows, owners, row_count)) = store.table_storage(Scroll::slot()) else {
            return;
        };
        if row_count == 0 {
            return;
        }

        let mut updates = std::mem::take(&mut self.scroll.reconcile_scratch);
        updates.clear();
        if !owners.is_empty() {
            for &raw in &owners[..row_count.min(owners.len())] {
                self.collect_scroll_node(store, StoreNodeId(raw), &mut updates);
            }
        } else {
            for idx in 1..store.capacity() {
                if let Some(id) = store.id_at(idx) {
                    self.collect_scroll_node(store, id, &mut updates);
                }
            }
        }
        for entry in updates.drain(..) {
            if let Some(vx) = entry.req_x {
                self.scroll.last_requested_x.insert(entry.raw, vx);
            }
            if let Some(vy) = entry.req_y {
                self.scroll.last_requested_y.insert(entry.raw, vy);
            }
            if entry.x != 0.0 || entry.y != 0.0 || self.scroll.contains(entry.raw) {
                self.scroll.set_raw(entry.raw, entry.x, entry.y);
            }
        }
        self.scroll.reconcile_scratch = updates;
    }

    fn collect_scroll_node(
        &self,
        store: &NodeStore,
        id: StoreNodeId,
        out: &mut Vec<ScrollReconcileEntry>,
    ) {
        if !store.is_live(id) {
            return;
        }
        let Some(scroll) = store.try_row::<Scroll>(id) else {
            return;
        };
        let Some(rect) = self.layout.layout_of(id) else {
            return;
        };
        let (cw, ch) = self
            .scroll_content_size(store, id)
            .unwrap_or((rect.width, rect.height));
        let max_x = (cw - rect.width).max(0.0);
        let max_y = (ch - rect.height).max(0.0);
        let raw = id.raw();
        let (mut sx, mut sy) = self.scroll.get(raw);
        let req_y = scroll.requested_scroll_y();
        let mut recorded_req_y = None;
        if req_y.is_finite() {
            let prev_req = self.scroll.last_requested_y.get(&raw).copied();
            if prev_req.is_none_or(|p| (p - req_y).abs() > 1e-4) {
                sy = req_y;
                recorded_req_y = Some(req_y);
            }
        }
        let req_x = scroll.requested_scroll_x();
        let mut recorded_req_x = None;
        if req_x.is_finite() {
            let prev_req = self.scroll.last_requested_x.get(&raw).copied();
            if prev_req.is_none_or(|p| (p - req_x).abs() > 1e-4) {
                sx = req_x;
                recorded_req_x = Some(req_x);
            }
        }
        out.push(ScrollReconcileEntry {
            raw,
            x: sx.clamp(0.0, max_x),
            y: sy.clamp(0.0, max_y),
            req_x: recorded_req_x,
            req_y: recorded_req_y,
        });
    }
}
