//! Extended layout synchronization (`LayoutExt`), typed 2D transforms (`Transform2D`),
//! deterministic `position: sticky` clamping, and transform/scroll-aware hit-testing for
//! [`crate::HeadlessBackend`].

use std::collections::HashMap;

use nve_abi::{ComponentKind, NodeId, RawPropValue};
use nve_components::layout_ext::{compute_sticky_offset, LayoutDirection, LayoutExt, PositionMode};
use nve_components::transform::Transform2D;
use nve_engine::{LayoutEngine, LayoutRect};

use crate::{HeadlessBackend, HeadlessNode, MAX_TREE_DEPTH};

#[inline]
pub(crate) fn node_clips_overflow(node: &HeadlessNode) -> bool {
    if node.kind == ComponentKind::ScrollView {
        return true;
    }
    if let Some(RawPropValue::String(ov)) = node.raw_props.get("overflow") {
        let lower = ov.trim().to_ascii_lowercase();
        if matches!(lower.as_str(), "hidden" | "clip" | "scroll" | "auto") {
            return true;
        }
    }
    matches!(node.raw_props.get("clip"), Some(RawPropValue::Bool(true)))
}

impl<L: LayoutEngine> HeadlessBackend<L> {
    /// Directly attaches or clears zero-serialization [`LayoutExt`] on `id`, forwarding to the
    /// active [`LayoutEngine`]. Returns `true` when the stored metadata changed.
    pub fn set_node_layout_ext(&mut self, id: NodeId, ext: Option<LayoutExt>) -> bool {
        let next = ext.filter(|e| !e.is_default()).map(Box::new);
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        if node.layout_ext == next {
            return false;
        }
        node.layout_ext = next;
        let ext_ref = node.layout_ext.as_deref();
        self.layout.set_layout_ext(id, ext_ref);
        true
    }

    /// Directly attaches or clears a zero-serialization [`Transform2D`] on `id`.
    /// Returns `true` when the stored transform changed.
    pub fn set_node_transform(&mut self, id: NodeId, transform: Option<Transform2D>) -> bool {
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        if node.transform == transform {
            return false;
        }
        node.transform = transform;
        true
    }

    /// Resolves the effective inherited [`LayoutDirection`] for `node_id`.
    pub fn resolved_direction(&self, node_id: NodeId) -> LayoutDirection {
        let mut cur = Some(node_id);
        let mut depth = 0usize;
        while let Some(id) = cur {
            if depth > MAX_TREE_DEPTH {
                break;
            }
            depth += 1;
            let Some(node) = self.nodes.get(id) else {
                break;
            };
            if let Some(dir) = node.layout_ext.as_deref().and_then(|e| e.direction) {
                return dir;
            }
            cur = node.parent;
        }
        LayoutDirection::Ltr
    }

    /// Resolves the effective [`Transform2D`] for `node_id`, applying any active interaction
    /// [`StylePatch`] override over the base transform.
    #[inline]
    pub fn effective_transform(&self, node_id: NodeId) -> Option<Transform2D> {
        let node = self.nodes.get(node_id)?;
        let patch = self.effective_patch(node_id);
        patch.transform.or(node.transform)
    }

    /// Sets the scroll offset `(x, y)` for a scrollable container in [`HeadlessBackend`].
    pub fn set_scroll_offset(&mut self, scroll_id: NodeId, x: f32, y: f32) {
        let sx = if x.is_finite() { x.max(0.0) } else { 0.0 };
        let sy = if y.is_finite() { y.max(0.0) } else { 0.0 };
        if sx == 0.0 && sy == 0.0 {
            self.scroll_offsets.remove(&scroll_id);
        } else {
            self.scroll_offsets.insert(scroll_id, (sx, sy));
        }
    }

    /// Returns the stored `(scroll_x, scroll_y)` offset for `scroll_id`.
    #[inline]
    pub fn scroll_offset(&self, scroll_id: NodeId) -> (f32, f32) {
        self.scroll_offsets.get(&scroll_id).copied().unwrap_or((0.0, 0.0))
    }

    /// Computes total `(content_width, content_height)` of a scroll container from its layout tree.
    pub fn scroll_content_bounds(&self, scroll_id: NodeId) -> Option<(f32, f32)> {
        let node = self.nodes.get(scroll_id)?;
        let ox = node.computed_rect.x;
        let oy = node.computed_rect.y;
        let mut max_w = node.computed_rect.width.max(0.0);
        let mut max_h = node.computed_rect.height.max(0.0);

        fn walk<L: LayoutEngine>(
            tree: &HeadlessBackend<L>,
            id: NodeId,
            ox: f32,
            oy: f32,
            max_w: &mut f32,
            max_h: &mut f32,
            depth: usize,
        ) {
            if depth > MAX_TREE_DEPTH {
                return;
            }
            let Some(n) = tree.node(id) else {
                return;
            };
            let r = n.computed_rect;
            let right = (r.x + r.width) - ox;
            let bottom = (r.y + r.height) - oy;
            if right.is_finite() && right > *max_w {
                *max_w = right;
            }
            if bottom.is_finite() && bottom > *max_h {
                *max_h = bottom;
            }
            if n.kind != ComponentKind::ScrollView {
                for &c in &n.children {
                    walk(tree, c, ox, oy, max_w, max_h, depth + 1);
                }
            }
        }

        for &child in &node.children {
            walk(self, child, ox, oy, &mut max_w, &mut max_h, 0);
        }
        if !node.children.is_empty() {
            let dir = self.resolved_direction(scroll_id);
            let pad = node
                .layout_ext
                .as_deref()
                .map_or(node.layout.padding, |e| e.padding_inline.resolve_into(node.layout.padding, dir));
            max_w = (max_w + pad.right.max(0.0)).max(node.computed_rect.width);
            max_h = (max_h + pad.bottom.max(0.0)).max(node.computed_rect.height);
        }
        Some((max_w, max_h))
    }

    /// Computes deterministic CSS `position: sticky` clamping offset `(dx, dy)` for `node_id`
    /// against `scroll_offsets` (shared by both [`HeadlessBackend`] and `SkiaBackend`).
    pub fn compute_sticky_delta(
        &self,
        node_id: NodeId,
        scroll_offsets: &HashMap<NodeId, (f32, f32)>,
    ) -> (f32, f32) {
        let Some(node) = self.nodes.get(node_id) else {
            return (0.0, 0.0);
        };
        let Some(ext) = node.layout_ext.as_deref() else {
            return (0.0, 0.0);
        };
        if ext.position != PositionMode::Sticky || ext.insets.is_auto() {
            return (0.0, 0.0);
        }
        let Some(parent_id) = node.parent else {
            return (0.0, 0.0);
        };
        let Some(parent) = self.nodes.get(parent_id) else {
            return (0.0, 0.0);
        };

        // Locate nearest scrollable ancestor (ScrollView or node with an active scroll offset).
        let mut scroll_id = None;
        let mut cursor = Some(parent_id);
        let mut depth = 0usize;
        while let Some(aid) = cursor {
            if depth > MAX_TREE_DEPTH {
                break;
            }
            depth += 1;
            let Some(anc) = self.nodes.get(aid) else {
                break;
            };
            if anc.kind == ComponentKind::ScrollView || scroll_offsets.contains_key(&aid) {
                scroll_id = Some(aid);
                break;
            }
            cursor = anc.parent;
        }
        let Some(sid) = scroll_id else {
            return (0.0, 0.0);
        };
        let Some(scroll_node) = self.nodes.get(sid) else {
            return (0.0, 0.0);
        };
        let scroll_off = scroll_offsets.get(&sid).copied().unwrap_or((0.0, 0.0));

        // Containing block is the parent's content box; when the parent IS the scroll container
        // itself, CSS sticky clamps within the scroll container's full scrollable content area.
        let p_dir = self.resolved_direction(parent_id);
        let p_pad = parent
            .layout_ext
            .as_deref()
            .map_or(parent.layout.padding, |e| {
                e.padding_inline.resolve_into(parent.layout.padding, p_dir)
            });
        let (cb_w, cb_h) = if parent_id == sid {
            self.scroll_content_bounds(sid)
                .unwrap_or((parent.computed_rect.width, parent.computed_rect.height))
        } else {
            (parent.computed_rect.width, parent.computed_rect.height)
        };
        let container_rect = (
            parent.computed_rect.x + p_pad.left,
            parent.computed_rect.y + p_pad.top,
            (cb_w - p_pad.left - p_pad.right).max(0.0),
            (cb_h - p_pad.top - p_pad.bottom).max(0.0),
        );

        let ir = node.computed_rect;
        let sr = scroll_node.computed_rect;
        compute_sticky_offset(
            (ir.x, ir.y, ir.width, ir.height),
            container_rect,
            (sr.x, sr.y, sr.width, sr.height),
            scroll_off,
            ext.insets,
        )
    }

    /// Returns the visual layout rectangle of `node_id` after applying ancestor scroll offsets and
    /// deterministic `position: sticky` clamping offsets.
    pub fn visual_rect_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        let node = self.nodes.get(node_id)?;
        let mut rect = node.computed_rect;
        let (own_sx, own_sy) = self.compute_sticky_delta(node_id, &self.scroll_offsets);
        rect.x += own_sx;
        rect.y += own_sy;

        let mut cur = node.parent;
        let mut depth = 0usize;
        while let Some(pid) = cur {
            if depth > MAX_TREE_DEPTH {
                break;
            }
            depth += 1;
            let Some(pnode) = self.nodes.get(pid) else {
                break;
            };
            if let Some(&(sx, sy)) = self.scroll_offsets.get(&pid) {
                rect.x -= sx;
                rect.y -= sy;
            }
            let (stx, sty) = self.compute_sticky_delta(pid, &self.scroll_offsets);
            rect.x += stx;
            rect.y += sty;
            cur = pnode.parent;
        }
        Some(rect)
    }
}
