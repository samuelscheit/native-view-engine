//! `ScrollView` content bounds, offset clamping, and virtualized scroll reconciliation for
//! [`crate::SkiaBackend`].

use nve_abi::{ComponentKind, NodeId, ROOT_NODE_ID};
use nve_backend_headless::{HeadlessBackend, HeadlessNode};
use nve_engine::LayoutEngine;

use crate::display_list::parse_px_raw_prop;
use crate::{SkiaBackend, MAX_HIT_TEST_DEPTH};

impl<L: LayoutEngine> SkiaBackend<L> {
    /// Returns the stored `(scroll_x, scroll_y)` offset for a `ScrollView` node.
    pub fn scroll_offset(&self, node_id: NodeId) -> (f32, f32) {
        self.scroll_offsets.get(&node_id).copied().unwrap_or((0.0, 0.0))
    }

    /// Computes `(content_width, content_height)` for a `ScrollView` node (taking into account
    /// both measured descendant extents and optional virtualized `virtual-total-height`).
    pub fn scroll_content_size(&self, node_id: NodeId) -> Option<(f32, f32)> {
        compute_scroll_content_size(&self.layout_tree, node_id)
    }

    /// Sets the scroll offset of `node_id`, clamping it against `[0..=content - viewport]`,
    /// rebuilds the retained [`crate::DisplayList`] if changed, and returns the clamped `(x, y)`.
    pub fn set_scroll_offset(&mut self, node_id: NodeId, x: f32, y: f32) -> (f32, f32) {
        let Some(node) = self.layout_tree.node(node_id) else {
            return (0.0, 0.0);
        };
        let rect = node.computed_rect;
        let (cw, ch) = compute_scroll_content_size(&self.layout_tree, node_id)
            .unwrap_or((rect.width, rect.height));
        let max_x = (cw - rect.width).max(0.0);
        let max_y = (ch - rect.height).max(0.0);
        let clamped_x = if x.is_finite() { x.clamp(0.0, max_x) } else { 0.0 };
        let clamped_y = if y.is_finite() { y.clamp(0.0, max_y) } else { 0.0 };

        let prev = self.scroll_offsets.get(&node_id).copied().unwrap_or((0.0, 0.0));
        self.scroll_offsets.insert(node_id, (clamped_x, clamped_y));
        if (prev.0 - clamped_x).abs() > 1e-4 || (prev.1 - clamped_y).abs() > 1e-4 {
            self.rebuild_display_list();
        }
        (clamped_x, clamped_y)
    }

    /// Applies an incremental scroll delta `(dx, dy)` to `node_id`, clamping against content
    /// bounds and rebuilding the display list when the offset changes.
    pub fn apply_scroll_delta(&mut self, node_id: NodeId, dx: f32, dy: f32) -> (f32, f32) {
        let (cur_x, cur_y) = self.scroll_offset(node_id);
        let nx = cur_x + if dx.is_finite() { dx } else { 0.0 };
        let ny = cur_y + if dy.is_finite() { dy } else { 0.0 };
        self.set_scroll_offset(node_id, nx, ny)
    }

    pub(crate) fn visual_scroll_shift(&self, node: &HeadlessNode) -> (f32, f32) {
        if node.kind != ComponentKind::ScrollView {
            return (0.0, 0.0);
        }
        let (sx, sy) = self.scroll_offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        if let Some(top_spacer) = parse_px_raw_prop(node.raw_props.get("virtual-top-spacer")) {
            // Virtualized ScrollView mounts children starting at `virtual-top-spacer`.
            (sx, (sy - top_spacer).max(0.0))
        } else {
            (sx, sy)
        }
    }

    pub(crate) fn reconcile_scroll_offsets(&mut self) {
        // Prune scroll state for removed nodes.
        self.scroll_offsets
            .retain(|id, _| self.layout_tree.node(*id).is_some());
        self.last_virtual_scroll_y
            .retain(|id, _| self.layout_tree.node(*id).is_some());

        // Sync virtual-scroll-y updates initiated by the app (e.g. Jump 50k buttons) and re-clamp.
        let mut updates: Vec<(NodeId, f32, f32, Option<f32>)> = Vec::new();
        self.collect_scroll_reconcile(ROOT_NODE_ID, &mut updates, 0);
        for (id, x, y, v_scroll) in updates {
            if let Some(vs) = v_scroll {
                self.last_virtual_scroll_y.insert(id, vs);
            }
            if x != 0.0 || y != 0.0 || self.scroll_offsets.contains_key(&id) {
                self.scroll_offsets.insert(id, (x, y));
            }
        }
    }

    fn collect_scroll_reconcile(
        &self,
        id: NodeId,
        out: &mut Vec<(NodeId, f32, f32, Option<f32>)>,
        depth: usize,
    ) {
        if depth > MAX_HIT_TEST_DEPTH {
            return;
        }
        let Some(node) = self.layout_tree.node(id) else {
            return;
        };
        if node.kind == ComponentKind::ScrollView {
            let rect = node.computed_rect;
            let (cw, ch) = compute_scroll_content_size(&self.layout_tree, id)
                .unwrap_or((rect.width, rect.height));
            let max_x = (cw - rect.width).max(0.0);
            let max_y = (ch - rect.height).max(0.0);
            let (mut sx, mut sy) = self.scroll_offsets.get(&id).copied().unwrap_or((0.0, 0.0));
            let v_scroll = parse_px_raw_prop(node.raw_props.get("virtual-scroll-y"));
            if let Some(app_vy) = v_scroll {
                let prev_vy = self.last_virtual_scroll_y.get(&id).copied();
                if prev_vy.is_none_or(|p| (p - app_vy).abs() > 0.5) {
                    sy = app_vy;
                }
            }
            sx = sx.clamp(0.0, max_x);
            sy = sy.clamp(0.0, max_y);
            out.push((id, sx, sy, v_scroll));
        }
        for &child in &node.children {
            self.collect_scroll_reconcile(child, out, depth + 1);
        }
    }
}

pub(crate) fn compute_scroll_content_size<L: LayoutEngine>(
    layout_tree: &HeadlessBackend<L>,
    scroll_id: NodeId,
) -> Option<(f32, f32)> {
    let node = layout_tree.node(scroll_id)?;
    let origin_x = node.computed_rect.x;
    let origin_y = node.computed_rect.y;
    let mut max_w = node.computed_rect.width.max(0.0);
    let mut max_h = node.computed_rect.height.max(0.0);

    fn walk<L: LayoutEngine>(
        tree: &HeadlessBackend<L>,
        id: NodeId,
        origin_x: f32,
        origin_y: f32,
        max_w: &mut f32,
        max_h: &mut f32,
        depth: usize,
    ) {
        if depth > MAX_HIT_TEST_DEPTH {
            return;
        }
        let Some(n) = tree.node(id) else {
            return;
        };
        let r = n.computed_rect;
        let right = (r.x + r.width) - origin_x;
        let bottom = (r.y + r.height) - origin_y;
        if right.is_finite() && right > *max_w {
            *max_w = right;
        }
        if bottom.is_finite() && bottom > *max_h {
            *max_h = bottom;
        }
        // Do not descend through nested ScrollViews for outer content bounds (their own content clips).
        if n.kind != ComponentKind::ScrollView {
            for &child in &n.children {
                walk(tree, child, origin_x, origin_y, max_w, max_h, depth + 1);
            }
        }
    }

    for &child in &node.children {
        walk(
            layout_tree,
            child,
            origin_x,
            origin_y,
            &mut max_w,
            &mut max_h,
            0,
        );
    }

    if !node.children.is_empty() {
        max_w = (max_w + node.layout.padding.right.max(0.0)).max(node.computed_rect.width);
        max_h = (max_h + node.layout.padding.bottom.max(0.0)).max(node.computed_rect.height);
    }

    if let Some(virtual_h) = parse_px_raw_prop(node.raw_props.get("virtual-total-height")) {
        if virtual_h > max_h {
            max_h = virtual_h;
        }
    }
    if let Some(virtual_w) = parse_px_raw_prop(node.raw_props.get("virtual-total-width")) {
        if virtual_w > max_w {
            max_w = virtual_w;
        }
    }

    Some((max_w, max_h))
}
