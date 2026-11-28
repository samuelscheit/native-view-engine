//! Top-layer floating [`Overlay`] reconciliation and visual-rect queries on [`LaidOutScene`].
//!
//! Resolves viewport-space top-layer origins for every open `nve.overlay` node (anchored via
//! `DebugId::id` through [`crate::overlay::compute_overlay_position`], or viewport-centered for
//! unanchored modals) using reusable zero-alloc dense storage.

use nve_abi::hash::HashMap;
use nve_abi::xxh64;
use nve_engine::LayoutRect;
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable, TableStorage};
use nve_tables::tables::{
    DebugId, Layout as NveLayout, Overlay, OverlayFlags, OverlayLayer, OverlayPlacement,
    Transform as TransformTable, is_node_display_none,
};
use nve_tables::transform::affine_map_aabb;

use crate::overlay::{PositionOptions, Rect, Size, Viewport as OverlayViewport, compute_overlay_position};
use crate::{MAX_TREE_DEPTH, StoreLayoutEngine};

use super::LaidOutScene;

const OVERLAY_NONE: u32 = u32::MAX;

/// Reconciled viewport-space geometry and layer metadata for one open top-layer [`Overlay`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedOverlay {
    pub node: NodeId,
    /// Resolved top-layer origin in logical viewport coordinates.
    pub origin_x: f32,
    pub origin_y: f32,
    /// Effective width after optional `AUTO_SIZE` viewport clamping.
    pub width: f32,
    /// Effective height after optional `AUTO_SIZE` viewport clamping.
    pub height: f32,
    pub placement: OverlayPlacement,
    pub layer: OverlayLayer,
    pub is_modal: bool,
    pub has_backdrop_scrim: bool,
}

/// Reusable zero-allocation top-layer overlay state owned by [`LaidOutScene`].
#[derive(Debug, Clone, Default)]
pub struct OverlayState {
    resolved: Vec<ResolvedOverlay>,
    /// Dense `NodeId::index() -> index into resolved` (`OVERLAY_NONE` when absent).
    node_to_overlay: Vec<u32>,
    walk_scratch: Vec<NodeId>,
    /// Reusable `xxh64(DebugId::id) -> NodeId` index rebuilt in O(D) only when open anchored
    /// overlays exist (0 steady-state allocations).
    anchor_index: HashMap<u64, NodeId>,
}

impl OverlayState {
    #[inline]
    pub fn open_overlays(&self) -> &[ResolvedOverlay] {
        &self.resolved
    }

    #[inline]
    pub fn get(&self, id: NodeId) -> Option<ResolvedOverlay> {
        let &idx = self.node_to_overlay.get(id.index())?;
        if idx == OVERLAY_NONE {
            return None;
        }
        let ro = *self.resolved.get(idx as usize)?;
        (ro.node == id).then_some(ro)
    }

    #[inline]
    pub fn is_open_overlay(&self, id: NodeId) -> bool {
        self.get(id).is_some()
    }

    #[inline]
    pub fn clear(&mut self) {
        for ro in &self.resolved {
            let i = ro.node.index();
            if i < self.node_to_overlay.len() {
                self.node_to_overlay[i] = OVERLAY_NONE;
            }
        }
        self.resolved.clear();
        self.walk_scratch.clear();
        self.anchor_index.clear();
    }

    fn clear_indices(&mut self) {
        for ro in &self.resolved {
            let i = ro.node.index();
            if i < self.node_to_overlay.len() {
                self.node_to_overlay[i] = OVERLAY_NONE;
            }
        }
        self.resolved.clear();
    }

    fn set_index(&mut self, id: NodeId, idx: u32) {
        let i = id.index();
        if i >= self.node_to_overlay.len() {
            self.node_to_overlay.resize(i + 1, OVERLAY_NONE);
        }
        self.node_to_overlay[i] = idx;
    }
}

/// Returns `true` when `id` owns an `Overlay` row whose `OPEN` flag is unset.
#[inline]
pub fn is_overlay_closed(store: &NodeStore, id: NodeId) -> bool {
    store
        .try_row::<Overlay>(id)
        .is_some_and(|ov| !ov.options().contains(OverlayFlags::OPEN))
}

/// Returns `true` when `id` or any ancestor up to root owns a closed [`Overlay`] row.
#[inline]
pub fn is_subtree_closed_overlay(store: &NodeStore, id: NodeId) -> bool {
    let mut cur = Some(id);
    for _ in 0..MAX_TREE_DEPTH {
        let Some(nid) = cur else {
            break;
        };
        if is_overlay_closed(store, nid) {
            return true;
        }
        cur = store.parent(nid).filter(|&p| p != nid);
    }
    false
}

fn find_anchor_node_linear(store: &NodeStore, anchor_id: &str) -> Option<NodeId> {
    let TableStorage {
        owners, row_count, ..
    } = store.table_storage(DebugId::slot())?;
    for &nid in owners.iter().take(row_count) {
        if nid.is_null() || !store.is_live(nid) {
            continue;
        }
        if store.try_row::<DebugId>(nid).is_some_and(|d| d.id() == anchor_id) {
            return Some(nid);
        }
    }
    None
}

fn lookup_anchor_node(
    store: &NodeStore,
    anchor_index: &HashMap<u64, NodeId>,
    anchor_id: &str,
) -> Option<NodeId> {
    let hash = xxh64(anchor_id.as_bytes(), 0);
    if let Some(&nid) = anchor_index.get(&hash) {
        if store.is_live(nid)
            && store.try_row::<DebugId>(nid).is_some_and(|d| d.id() == anchor_id)
        {
            return Some(nid);
        }
        return find_anchor_node_linear(store, anchor_id);
    }
    None
}

impl<L: StoreLayoutEngine> LaidOutScene<L> {
    /// Open top-layer overlays in deterministic document paint order.
    #[inline]
    pub fn open_overlays(&self) -> &[ResolvedOverlay] {
        self.overlays.open_overlays()
    }

    /// Reconciled top-layer geometry for `id` if it is currently an open [`Overlay`].
    #[inline]
    pub fn resolved_overlay(&self, id: NodeId) -> Option<ResolvedOverlay> {
        self.overlays.get(id)
    }

    /// Computes the visual (scroll-shifted, sticky-clamped, `Transform2D`-mapped,
    /// top-layer-adjusted) bounding rectangle of `node_id` in logical viewport coordinates.
    pub fn visual_rect_of(&self, store: &NodeStore, node_id: NodeId) -> Option<LayoutRect> {
        if let Some(ro) = self.overlays.get(node_id) {
            let (mut x0, mut y0) = (ro.origin_x, ro.origin_y);
            let (mut x1, mut y1) = (x0 + ro.width, y0 + ro.height);
            if let Some(tf) = self.effective_transform(store, node_id) {
                let m = tf.to_surface_matrix_2d(ro.origin_x, ro.origin_y, ro.width, ro.height);
                (x0, y0, x1, y1) = affine_map_aabb(&m, x0, y0, x1, y1);
            }
            return Some(LayoutRect {
                x: x0,
                y: y0,
                width: (x1 - x0).max(0.0),
                height: (y1 - y0).max(0.0),
            });
        }
        let base_rect = self.layout.layout_of(node_id)?;
        let (own_sx, own_sy) = self.compute_sticky_delta(store, node_id);
        let mut x0 = base_rect.x + own_sx;
        let mut y0 = base_rect.y + own_sy;
        let mut x1 = x0 + base_rect.width;
        let mut y1 = y0 + base_rect.height;
        if let Some(tf) = self.effective_transform(store, node_id) {
            let m = tf.to_surface_matrix_2d(x0, y0, base_rect.width, base_rect.height);
            (x0, y0, x1, y1) = affine_map_aabb(&m, x0, y0, x1, y1);
        }
        let mut ancestor = store.parent(node_id);
        for _ in 0..MAX_TREE_DEPTH {
            let Some(pid) = ancestor else {
                break;
            };
            let (sx, sy) = self.scroll.visual_shift(store, pid);
            x0 -= sx;
            x1 -= sx;
            y0 -= sy;
            y1 -= sy;
            if let Some(ro) = self.overlays.get(pid) {
                if let Some(base) = self.layout.layout_of(pid) {
                    let dx = ro.origin_x - base.x;
                    let dy = ro.origin_y - base.y;
                    x0 += dx;
                    x1 += dx;
                    y0 += dy;
                    y1 += dy;
                }
                if let Some(tf) = self.effective_transform(store, pid) {
                    let m = tf.to_surface_matrix_2d(ro.origin_x, ro.origin_y, ro.width, ro.height);
                    (x0, y0, x1, y1) = affine_map_aabb(&m, x0, y0, x1, y1);
                }
                break;
            }
            let (stx, sty) = self.compute_sticky_delta(store, pid);
            x0 += stx;
            x1 += stx;
            y0 += sty;
            y1 += sty;
            if let Some(tf) = self.effective_transform(store, pid)
                && let Some(p_rect) = self.layout.layout_of(pid)
            {
                let m = tf.to_surface_matrix_2d(
                    p_rect.x + stx,
                    p_rect.y + sty,
                    p_rect.width,
                    p_rect.height,
                );
                (x0, y0, x1, y1) = affine_map_aabb(&m, x0, y0, x1, y1);
            }
            ancestor = store.parent(pid).filter(|&p| p != pid);
        }
        Some(LayoutRect {
            x: x0,
            y: y0,
            width: (x1 - x0).max(0.0),
            height: (y1 - y0).max(0.0),
        })
    }

    /// Fast-path guard around [`Self::reconcile_overlays`]: skips all work in `O(1)` when no
    /// overlay table or relevant ancestor geometry (`layout`, `scroll`, `transform`, `DebugId`,
    /// tree structure) changed.
    #[inline]
    pub fn reconcile_overlays_if_needed(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        layout_or_scroll_changed: bool,
    ) {
        let has_rows = store
            .table_storage(Overlay::slot())
            .is_some_and(|s| s.row_count > 0);
        if !has_rows {
            if !self.overlays.resolved.is_empty() {
                self.overlays.clear_indices();
            }
            return;
        }
        if !layout_or_scroll_changed
            && changes.removed.is_empty()
            && changes.children.is_empty()
            && changes.table(Overlay::slot()).is_none()
            && changes.table(DebugId::slot()).is_none()
            && changes.table(TransformTable::slot()).is_none()
            && changes.table(NveLayout::slot()).is_none()
        {
            return;
        }
        self.reconcile_overlays(store);
    }

    /// Reconciles viewport-space top-layer coordinates for every open `nve.overlay` node in
    /// deterministic document paint order (O(1) fast-exit when no overlay is open; zero heap
    /// allocations in steady state).
    pub fn reconcile_overlays(&mut self, store: &NodeStore) {
        self.overlays.clear_indices();
        let Some(TableStorage {
            owners, row_count, ..
        }) = store.table_storage(Overlay::slot())
        else {
            return;
        };
        if row_count == 0 || !store.is_live(NodeId::ROOT) {
            return;
        }
        let mut any_open = false;
        let mut any_anchored = false;
        for &oid in owners.iter().take(row_count) {
            if !oid.is_null()
                && store.is_live(oid)
                && let Some(ov) = store.try_row::<Overlay>(oid)
                && ov.options().contains(OverlayFlags::OPEN)
            {
                any_open = true;
                if !ov.anchor_id().is_empty() {
                    any_anchored = true;
                    break;
                }
            }
        }
        if !any_open {
            return;
        }

        let mut anchor_index = core::mem::take(&mut self.overlays.anchor_index);
        anchor_index.clear();
        if any_anchored
            && let Some(TableStorage {
                owners: dbg_owners,
                row_count: dbg_count,
                ..
            }) = store.table_storage(DebugId::slot())
        {
            for &nid in dbg_owners.iter().take(dbg_count) {
                if !nid.is_null()
                    && store.is_live(nid)
                    && let Some(d) = store.try_row::<DebugId>(nid)
                    && !d.id().is_empty()
                {
                    let h = xxh64(d.id().as_bytes(), 0);
                    anchor_index.entry(h).or_insert(nid);
                }
            }
        }

        // Collect open overlay NodeIds in deterministic DFS document order without allocating.
        let mut stack = core::mem::take(&mut self.overlays.walk_scratch);
        stack.clear();
        stack.push(NodeId::ROOT);
        while let Some(id) = stack.pop() {
            if !store.is_live(id) || is_node_display_none(store, id) {
                continue;
            }
            if let Some(ov) = store.try_row::<Overlay>(id) {
                let opts = ov.options();
                if !opts.contains(OverlayFlags::OPEN) {
                    continue;
                }
                let ro = self.compute_resolved_overlay(store, id, ov, &anchor_index);
                let slot_idx = self.overlays.resolved.len() as u32;
                self.overlays.resolved.push(ro);
                self.overlays.set_index(id, slot_idx);
            }
            let children = store.children(id);
            for &cid in children.iter().rev() {
                stack.push(cid);
            }
        }
        self.overlays.walk_scratch = stack;
        self.overlays.anchor_index = anchor_index;
    }

    fn compute_resolved_overlay(
        &self,
        store: &NodeStore,
        id: NodeId,
        ov: nve_tables::tables::OverlayRef<'_>,
        anchor_index: &HashMap<u64, NodeId>,
    ) -> ResolvedOverlay {
        let base_rect = self.layout.layout_of(id).unwrap_or_default();
        let opts = ov.options();
        let layer = ov.layer();
        let is_modal = layer == OverlayLayer::Modal;
        let has_backdrop_scrim = is_modal || opts.contains(OverlayFlags::BACKDROP_SCRIM);
        let pad = ov.viewport_padding_px().max(0.0);
        let vp_w = self.viewport.width.max(0.0);
        let vp_h = self.viewport.height.max(0.0);

        let anchor_id = ov.anchor_id();
        if !anchor_id.is_empty()
            && let Some(aid) = lookup_anchor_node(store, anchor_index, anchor_id)
            && aid != id
            && let Some(ar) = self.visual_rect_of(store, aid)
        {
            let pos_opts = PositionOptions {
                placement: ov.placement(),
                offset: ov.offset_px(),
                viewport_padding: pad,
                flip: opts.contains(OverlayFlags::FLIP),
                shift: opts.contains(OverlayFlags::SHIFT),
                auto_size: opts.contains(OverlayFlags::AUTO_SIZE),
                fallback_placements: Vec::new(),
            };
            let computed = compute_overlay_position(
                Rect::new(ar.x, ar.y, ar.width, ar.height),
                Size::new(base_rect.width, base_rect.height),
                OverlayViewport::new(vp_w, vp_h),
                &pos_opts,
            );
            let width = computed
                .max_width
                .map_or(base_rect.width, |mw| base_rect.width.min(mw));
            let height = computed
                .max_height
                .map_or(base_rect.height, |mh| base_rect.height.min(mh));
            return ResolvedOverlay {
                node: id,
                origin_x: computed.x,
                origin_y: computed.y,
                width,
                height,
                placement: computed.placement,
                layer,
                is_modal,
                has_backdrop_scrim,
            };
        }

        // Unanchored overlay: center modals unless explicit inset overrides are set; clamp to
        // viewport when `SHIFT` is enabled.
        let has_explicit_inset = store.try_row::<NveLayout>(id).is_some_and(|l| {
            !l.inset_top().is_auto()
                || !l.inset_right().is_auto()
                || !l.inset_bottom().is_auto()
                || !l.inset_left().is_auto()
        });
        let (mut ox, mut oy) = if is_modal && !has_explicit_inset {
            (
                ((vp_w - base_rect.width) * 0.5).max(pad),
                ((vp_h - base_rect.height) * 0.5).max(pad),
            )
        } else {
            (base_rect.x, base_rect.y)
        };
        let (w, h) = if opts.contains(OverlayFlags::AUTO_SIZE) {
            (
                base_rect.width.min((vp_w - 2.0 * pad).max(0.0)),
                base_rect.height.min((vp_h - 2.0 * pad).max(0.0)),
            )
        } else {
            (base_rect.width, base_rect.height)
        };
        if opts.contains(OverlayFlags::SHIFT) {
            let max_x = (vp_w - pad - w).max(pad);
            let max_y = (vp_h - pad - h).max(pad);
            ox = ox.clamp(pad, max_x);
            oy = oy.clamp(pad, max_y);
        }
        ResolvedOverlay {
            node: id,
            origin_x: ox,
            origin_y: oy,
            width: w,
            height: h,
            placement: ov.placement(),
            layer,
            is_modal,
            has_backdrop_scrim,
        }
    }
}
