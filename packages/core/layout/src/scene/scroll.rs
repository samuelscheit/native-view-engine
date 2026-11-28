//! Per-surface scroll offsets, content bounds, clamping, and exact sequence-driven controlled
//! scroll reconciliation (`Scroll::requested_scroll_seq`).

use std::collections::HashMap;

use nve_engine::LayoutRect;
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable};
use nve_tables::tables::Scroll;

use crate::StoreLayoutEngine;

use super::LaidOutScene;

/// Per-surface scroll offset map plus dense `requested_scroll_seq` deduplication (0-alloc steady state).
#[derive(Debug, Default, Clone)]
pub struct ScrollOffsets {
    offsets: HashMap<NodeId, (f32, f32)>,
    last_requested_seq: Vec<u32>,
    reconcile_scratch: Vec<ScrollReconcileEntry>,
    changed_scratch: Vec<NodeId>,
}

impl ScrollOffsets {
    #[inline]
    pub fn get(&self, id: NodeId) -> (f32, f32) {
        self.offsets.get(&id).copied().unwrap_or((0.0, 0.0))
    }

    /// Every tracked `(node, (x, y))` offset, in unspecified order.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, (f32, f32))> + '_ {
        self.offsets.iter().map(|(&id, &xy)| (id, xy))
    }

    #[inline]
    pub fn contains(&self, id: NodeId) -> bool {
        self.offsets.contains_key(&id)
    }

    #[inline]
    pub fn last_requested_seq(&self, id: NodeId) -> u32 {
        self.last_requested_seq.get(id.index()).copied().unwrap_or(0)
    }

    #[inline]
    fn set_last_requested_seq(&mut self, id: NodeId, seq: u32) {
        let idx = id.index();
        if seq == 0 {
            if idx < self.last_requested_seq.len() {
                self.last_requested_seq[idx] = 0;
            }
            return;
        }
        if idx >= self.last_requested_seq.len() {
            self.last_requested_seq.resize(idx + 1, 0);
        }
        self.last_requested_seq[idx] = seq;
    }

    /// Nodes whose effective scroll offset changed during the most recent reconciliation pass.
    #[inline]
    pub fn last_changed(&self) -> &[NodeId] {
        &self.changed_scratch
    }

    #[inline]
    pub fn set_raw(&mut self, id: NodeId, x: f32, y: f32) {
        self.offsets.insert(id, (x, y));
    }

    #[inline]
    pub fn remove(&mut self, id: NodeId) {
        self.offsets.remove(&id);
        let idx = id.index();
        if idx < self.last_requested_seq.len() {
            self.last_requested_seq[idx] = 0;
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.offsets.clear();
        self.last_requested_seq.fill(0);
        self.reconcile_scratch.clear();
        self.changed_scratch.clear();
    }

    /// Visual child shift for `id`, subtracting any virtualized `Scroll::virtual_top_spacer`.
    #[inline]
    pub fn visual_shift(&self, store: &NodeStore, id: NodeId) -> (f32, f32) {
        let (sx, sy) = self.get(id);
        Self::visual_shift_with(store, id, sx, sy)
    }

    /// [`Self::visual_shift`] for an offset `(sx, sy)` already read (e.g. from [`Self::iter`]).
    #[inline]
    pub fn visual_shift_with(store: &NodeStore, id: NodeId, sx: f32, sy: f32) -> (f32, f32) {
        match store
            .try_row::<Scroll>(id)
            .and_then(|scroll| scroll.virtual_top_spacer())
            .filter(|&v| v.is_finite() && v > 0.0)
        {
            Some(top_spacer) => (sx, (sy - top_spacer).max(0.0)),
            None => (sx, sy),
        }
    }
}

/// Computes `(content_width, content_height)` for a scroll container node.
pub fn compute_scroll_content_size(
    store: &NodeStore,
    layout_of: impl Fn(NodeId) -> Option<LayoutRect>,
    node_id: NodeId,
) -> Option<(f32, f32)> {
    let rect = layout_of(node_id)?;
    let mut max_w = rect.width;
    let mut max_h = rect.height;

    if let Some(scroll) = store.try_row::<Scroll>(node_id) {
        if let Some(vw) = scroll.virtual_total_width().filter(|v| v.is_finite()) {
            max_w = max_w.max(vw);
        }
        if let Some(vh) = scroll.virtual_total_height().filter(|v| v.is_finite()) {
            max_h = max_h.max(vh);
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
    id: NodeId,
    x: f32,
    y: f32,
    seq: u32,
    changed: bool,
}

impl<L: StoreLayoutEngine> LaidOutScene<L> {
    #[inline]
    pub fn scroll_offsets(&self) -> &ScrollOffsets {
        &self.scroll
    }

    #[inline]
    pub fn last_reconciled_scroll_nodes(&self) -> &[NodeId] {
        self.scroll.last_changed()
    }

    #[inline]
    pub fn set_scroll_offset_raw(&mut self, id: NodeId, dx: f32, dy: f32) {
        self.scroll.set_raw(id, dx, dy);
    }

    #[inline]
    pub fn scroll_offset(&self, id: NodeId) -> (f32, f32) {
        self.scroll.get(id)
    }

    #[inline]
    pub fn visual_scroll_shift(&self, store: &NodeStore, id: NodeId) -> (f32, f32) {
        self.scroll.visual_shift(store, id)
    }

    pub fn scroll_content_size(
        &self,
        store: &NodeStore,
        node_id: NodeId,
    ) -> Option<(f32, f32)> {
        compute_scroll_content_size(store, |id| self.layout.layout_of(id), node_id)
    }

    /// Sets `node_id`'s scroll offset clamped to `[0..=content - viewport]`.
    /// Returns `(clamped_x, clamped_y, changed)`.
    pub fn set_scroll_offset_clamped(
        &mut self,
        store: &NodeStore,
        node_id: NodeId,
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

        let prev = self.scroll.get(node_id);
        self.scroll.set_raw(node_id, clamped_x, clamped_y);
        let changed = (prev.0 - clamped_x).abs() > 1e-4 || (prev.1 - clamped_y).abs() > 1e-4;
        (clamped_x, clamped_y, changed)
    }

    /// Applies an incremental scroll delta `(dx, dy)` clamped against content bounds.
    /// Returns `(clamped_x, clamped_y, changed)`.
    pub fn apply_scroll_delta(
        &mut self,
        store: &NodeStore,
        node_id: NodeId,
        dx: f32,
        dy: f32,
    ) -> (f32, f32, bool) {
        let (cur_x, cur_y) = self.scroll.get(node_id);
        let nx = cur_x + if dx.is_finite() { dx } else { 0.0 };
        let ny = cur_y + if dy.is_finite() { dy } else { 0.0 };
        self.set_scroll_offset_clamped(store, node_id, nx, ny)
    }

    /// Fast-path guard around [`Self::reconcile_scroll_offsets`]: skips all work in `O(1)` when
    /// layout did not run, no `Scroll` table row changed, and no nodes were removed.
    /// Returns the slice of nodes whose scroll offset changed.
    #[inline]
    pub fn reconcile_scroll_offsets_if_needed(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        laid_out: bool,
    ) -> &[NodeId] {
        self.scroll.changed_scratch.clear();
        if let Some(tc) = changes.table(Scroll::slot()) {
            for &id in &tc.detached {
                self.scroll.remove(id);
            }
        }
        if !laid_out
            && changes.removed.is_empty()
            && changes.table(Scroll::slot()).is_none()
        {
            return &self.scroll.changed_scratch;
        }
        self.reconcile_scroll_offsets(store)
    }

    /// Prunes removed/detached scroll nodes, honors exact `Scroll::requested_scroll_seq`
    /// transitions (`seq != 0 && seq != prev_seq`), and re-clamps existing offsets against newly
    /// laid-out content bounds in `O(scroll_containers)` time with zero steady-state allocations.
    /// Returns the slice of nodes whose effective scroll offset changed.
    pub fn reconcile_scroll_offsets(&mut self, store: &NodeStore) -> &[NodeId] {
        self.scroll.changed_scratch.clear();
        if !self.scroll.offsets.is_empty() {
            let last_seq = &mut self.scroll.last_requested_seq;
            self.scroll.offsets.retain(|&id, _| {
                let keep = store.is_live(id) && store.has::<Scroll>(id);
                if !keep && id.index() < last_seq.len() {
                    last_seq[id.index()] = 0;
                }
                keep
            });
        }
        // Clear any non-zero seq whose node died or had its `Scroll` row detached.
        for (idx, slot) in self.scroll.last_requested_seq.iter_mut().enumerate() {
            if *slot != 0 && store.id_at(idx).is_none_or(|id| !store.has::<Scroll>(id)) {
                *slot = 0;
            }
        }

        let Some(nve_store::TableStorage {
            owners, row_count, ..
        }) = store.table_storage(Scroll::slot())
        else {
            return &self.scroll.changed_scratch;
        };
        if row_count == 0 {
            return &self.scroll.changed_scratch;
        }

        let mut updates = std::mem::take(&mut self.scroll.reconcile_scratch);
        updates.clear();
        if !owners.is_empty() {
            for &id in &owners[..row_count.min(owners.len())] {
                self.collect_scroll_node(store, id, &mut updates);
            }
        } else {
            for idx in 1..store.capacity() {
                if let Some(id) = store.id_at(idx) {
                    self.collect_scroll_node(store, id, &mut updates);
                }
            }
        }
        for entry in updates.drain(..) {
            self.scroll.set_last_requested_seq(entry.id, entry.seq);
            if entry.x != 0.0 || entry.y != 0.0 || self.scroll.contains(entry.id) {
                self.scroll.set_raw(entry.id, entry.x, entry.y);
            }
            if entry.changed {
                self.scroll.changed_scratch.push(entry.id);
            }
        }
        self.scroll.reconcile_scratch = updates;
        &self.scroll.changed_scratch
    }

    fn collect_scroll_node(
        &self,
        store: &NodeStore,
        id: NodeId,
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

        let (prev_x, prev_y) = self.scroll.get(id);
        let (mut sx, mut sy) = (prev_x, prev_y);
        let seq = scroll.requested_scroll_seq();
        let prev_seq = self.scroll.last_requested_seq(id);

        if seq != 0 && seq != prev_seq {
            if let Some(req_x) = scroll.requested_scroll_x().filter(|v| v.is_finite()) {
                sx = req_x;
            }
            if let Some(req_y) = scroll.requested_scroll_y().filter(|v| v.is_finite()) {
                sy = req_y;
            }
        }

        let nx = sx.clamp(0.0, max_x);
        let ny = sy.clamp(0.0, max_y);
        let changed = (nx - prev_x).abs() > 1e-4 || (ny - prev_y).abs() > 1e-4;
        out.push(ScrollReconcileEntry {
            id,
            x: nx,
            y: ny,
            seq,
            changed,
        });
    }
}
