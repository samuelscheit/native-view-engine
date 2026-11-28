//! Top-layer floating [`Overlay`] reconciliation and visual-rect queries on [`LaidOutScene`].
//!
//! Resolves viewport-space top-layer origins for every open `nve.overlay` node (anchored via
//! `DebugId::id` through [`crate::overlay::compute_overlay_position`], or viewport-centered for
//! unanchored modals) using reusable zero-alloc dense storage.

use nve_engine::LayoutRect;
use nve_store::{NodeId, NodeStore, NveTable, TableStorage};
use nve_tables::tables::{
    DebugId, Layout as NveLayout, Overlay, OverlayFlags, OverlayLayer, OverlayPlacement,
    is_node_display_none,
};

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

fn find_anchor_node_by_id(store: &NodeStore, anchor_id: &str) -> Option<NodeId> {
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

    /// Computes the visual (scroll-shifted, sticky-clamped, top-layer-adjusted) rectangle of
    /// `node_id` in logical viewport coordinates.
    pub fn visual_rect_of(&self, store: &NodeStore, node_id: NodeId) -> Option<LayoutRect> {
        if let Some(ro) = self.overlays.get(node_id) {
            return Some(LayoutRect {
                x: ro.origin_x,
                y: ro.origin_y,
                width: ro.width,
                height: ro.height,
            });
        }
        let mut rect = self.layout.layout_of(node_id)?;
        let (own_sx, own_sy) = self.compute_sticky_delta(store, node_id);
        rect.x += own_sx;
        rect.y += own_sy;
        let mut ancestor = store.parent(node_id);
        for _ in 0..MAX_TREE_DEPTH {
            let Some(pid) = ancestor else {
                break;
            };
            let (sx, sy) = self.scroll.visual_shift(store, pid);
            rect.x -= sx;
            rect.y -= sy;
            if let Some(ro) = self.overlays.get(pid) {
                if let Some(base) = self.layout.layout_of(pid) {
                    rect.x += ro.origin_x - base.x;
                    rect.y += ro.origin_y - base.y;
                }
                break;
            }
            let (stx, sty) = self.compute_sticky_delta(store, pid);
            rect.x += stx;
            rect.y += sty;
            ancestor = store.parent(pid);
        }
        Some(rect)
    }

    /// Reconciles viewport-space top-layer coordinates for every open `nve.overlay` node in
    /// deterministic document paint order (zero heap allocations in steady state).
    pub fn reconcile_overlays(&mut self, store: &NodeStore) {
        self.overlays.clear_indices();
        let has_any = store
            .table_storage(Overlay::slot())
            .is_some_and(|s| s.row_count > 0);
        if !has_any || !store.is_live(NodeId::ROOT) {
            return;
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
                let ro = self.compute_resolved_overlay(store, id, ov);
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
    }

    fn compute_resolved_overlay(
        &self,
        store: &NodeStore,
        id: NodeId,
        ov: nve_tables::tables::OverlayRef<'_>,
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
            && let Some(aid) = find_anchor_node_by_id(store, anchor_id)
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
        let has_explicit_inset = store
            .try_row::<NveLayout>(id)
            .is_some_and(|l| !l.inset_top().is_auto() || !l.inset_left().is_auto());
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
