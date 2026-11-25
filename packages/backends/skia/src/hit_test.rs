//! Scroll-aware, clip-aware, and z-index-ordered hit testing for [`crate::SkiaBackend`].
//!
//! Eliminates per-node `children.clone()` heap allocations via a stack buffer for z-ordered
//! containers up to 64 children (with an automatic `Vec` fallback for larger z-ordered siblings).

use nve_abi::{ComponentKind, EventMask, NodeId, ROOT_NODE_ID};
use nve_backend_headless::HeadlessNode;
use nve_engine::{LayoutEngine, LayoutRect};

use crate::display_list::{intersect_rects, offset_rect, point_in_rect, should_clip_children};
use crate::{SkiaBackend, MAX_HIT_TEST_DEPTH};

const INLINE_Z_SORT_CAPACITY: usize = 64;

#[inline]
fn is_focusable_node(node: &HeadlessNode) -> bool {
    node.kind == ComponentKind::TextInput
        || (node.event_mask & (EventMask::FOCUS | EventMask::TEXT_CHANGE)) != 0
}

#[derive(Clone, Copy)]
enum HitTargetFilter {
    Interactive,
    Focusable,
    Interaction,
}

impl HitTargetFilter {
    #[inline]
    fn matches<L: LayoutEngine>(self, layout_tree: &nve_backend_headless::HeadlessBackend<L>, node: &HeadlessNode) -> bool {
        match self {
            Self::Interactive => node.event_mask != 0,
            Self::Focusable => is_focusable_node(node),
            Self::Interaction => layout_tree.is_interaction_target(node),
        }
    }
}

impl<L: LayoutEngine> SkiaBackend<L> {
    /// Scroll-aware and clip-aware interactive hit test at logical coordinates `(x, y)`.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_filtered(
            ROOT_NODE_ID,
            x,
            y,
            (0.0, 0.0),
            None,
            HitTargetFilter::Interactive,
            0,
        )
    }

    /// Scroll-aware, clip-aware, and z-order-aware focusable hit test (`TextInput` or any node
    /// subscribed to `EventMask::FOCUS | EventMask::TEXT_CHANGE`) at logical coordinates `(x, y)`.
    pub fn hit_test_focusable(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_filtered(
            ROOT_NODE_ID,
            x,
            y,
            (0.0, 0.0),
            None,
            HitTargetFilter::Focusable,
            0,
        )
    }

    /// Scroll-aware, clip-aware, and z-order-aware interaction hit test for hover/active/cursor
    /// state (matches interactive nodes, `Pressable`/`TextInput`, disabled nodes, and any node
    /// declaring `state_rules` or an explicit `cursor`).
    pub fn hit_test_interaction(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_filtered(
            ROOT_NODE_ID,
            x,
            y,
            (0.0, 0.0),
            None,
            HitTargetFilter::Interaction,
            0,
        )
    }

    /// Finds the deepest `ScrollView` (or node subscribed to `EventMask::SCROLL`) enclosing `(x, y)`.
    pub fn hit_test_scroll_view(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_scroll_rec(ROOT_NODE_ID, x, y, (0.0, 0.0), None, 0)
    }

    /// Returns `true` if any retained node subscribes to `EventMask::CUSTOM`.
    pub fn has_custom_event_listener(&self) -> bool {
        self.find_event_mask_rec(ROOT_NODE_ID, EventMask::CUSTOM, 0)
            .is_some()
    }

    /// Returns the first retained node subscribed to `EventMask::CUSTOM` (e.g. `onkeydown`),
    /// falling back to `ROOT_NODE_ID` when no specific node declares custom listeners.
    pub fn find_custom_event_target(&self) -> NodeId {
        self.find_event_mask_rec(ROOT_NODE_ID, EventMask::CUSTOM, 0)
            .unwrap_or(ROOT_NODE_ID)
    }

    /// Resolves the nearest ancestor (including `start`) subscribed to `EventMask::CUSTOM`,
    /// falling back to [`Self::find_custom_event_target`].
    pub fn find_custom_event_target_from(&self, start: NodeId) -> NodeId {
        let mut cursor = Some(start);
        let mut steps = 0usize;
        while let Some(id) = cursor {
            if steps > MAX_HIT_TEST_DEPTH {
                break;
            }
            steps += 1;
            let Some(node) = self.layout_tree.node(id) else {
                break;
            };
            if (node.event_mask & EventMask::CUSTOM) != 0 {
                return id;
            }
            cursor = node.parent;
        }
        self.find_custom_event_target()
    }

    fn find_event_mask_rec(&self, id: NodeId, mask: u64, depth: usize) -> Option<NodeId> {
        if depth > MAX_HIT_TEST_DEPTH {
            return None;
        }
        let node = self.layout_tree.node(id)?;
        for &child in node.children.iter().rev() {
            if let Some(found) = self.find_event_mask_rec(child, mask, depth + 1) {
                return Some(found);
            }
        }
        if (node.event_mask & mask) != 0 {
            Some(id)
        } else {
            None
        }
    }

    /// Invokes `visit` on `children` in reverse paint order (topmost z-index first), using a
    /// stack buffer for up to 64 children to avoid per-node heap allocations.
    fn visit_children_topmost_first<F, R>(&self, children: &[NodeId], mut visit: F) -> Option<R>
    where
        F: FnMut(NodeId) -> Option<R>,
    {
        let has_z_order = children.len() > 1
            && children
                .iter()
                .any(|&cid| self.layout_tree.node(cid).is_some_and(|c| c.style.z_index != 0));
        if !has_z_order {
            for &child in children.iter().rev() {
                if let Some(hit) = visit(child) {
                    return Some(hit);
                }
            }
            return None;
        }

        if children.len() <= INLINE_Z_SORT_CAPACITY {
            let mut stack_buf = [0u32; INLINE_Z_SORT_CAPACITY];
            let slice = &mut stack_buf[..children.len()];
            slice.copy_from_slice(children);
            slice.sort_by_key(|&cid| self.layout_tree.node(cid).map_or(0, |c| c.style.z_index));
            for &child in slice.iter().rev() {
                if let Some(hit) = visit(child) {
                    return Some(hit);
                }
            }
        } else {
            let mut sorted = Vec::with_capacity(children.len());
            sorted.extend_from_slice(children);
            sorted.sort_by_key(|&cid| self.layout_tree.node(cid).map_or(0, |c| c.style.z_index));
            for &child in sorted.iter().rev() {
                if let Some(hit) = visit(child) {
                    return Some(hit);
                }
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn hit_test_filtered(
        &self,
        id: NodeId,
        x: f32,
        y: f32,
        offset: (f32, f32),
        clip: Option<LayoutRect>,
        filter: HitTargetFilter,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > MAX_HIT_TEST_DEPTH {
            return None;
        }
        let node = self.layout_tree.node(id)?;
        if node.style.opacity <= 0.001 {
            return None;
        }

        let visual_rect = offset_rect(node.computed_rect, offset);
        let inside_self =
            point_in_rect(x, y, visual_rect) && clip.is_none_or(|c| point_in_rect(x, y, c));

        if !inside_self && id != ROOT_NODE_ID {
            return None;
        }

        let child_clip = if should_clip_children(node) {
            let next = match clip {
                Some(prev) => intersect_rects(prev, visual_rect),
                None => Some(visual_rect),
            };
            match next {
                Some(c) if point_in_rect(x, y, c) => Some(c),
                _ => {
                    // Point is outside clipped region; children cannot be hit.
                    return if inside_self && filter.matches(&self.layout_tree, node) {
                        Some(id)
                    } else {
                        None
                    };
                }
            }
        } else {
            clip
        };

        let (sx, sy) = self.visual_scroll_shift(node);
        let child_offset = (offset.0 - sx, offset.1 - sy);

        if let Some(hit) = self.visit_children_topmost_first(&node.children, |child| {
            self.hit_test_filtered(child, x, y, child_offset, child_clip, filter, depth + 1)
        }) {
            return Some(hit);
        }

        if inside_self && filter.matches(&self.layout_tree, node) {
            Some(id)
        } else {
            None
        }
    }

    fn hit_test_scroll_rec(
        &self,
        id: NodeId,
        x: f32,
        y: f32,
        offset: (f32, f32),
        clip: Option<LayoutRect>,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > MAX_HIT_TEST_DEPTH {
            return None;
        }
        let node = self.layout_tree.node(id)?;
        if node.style.opacity <= 0.001 {
            return None;
        }
        let visual_rect = offset_rect(node.computed_rect, offset);
        let inside_self =
            point_in_rect(x, y, visual_rect) && clip.is_none_or(|c| point_in_rect(x, y, c));
        if !inside_self && id != ROOT_NODE_ID {
            return None;
        }

        let is_scrollable =
            node.kind == ComponentKind::ScrollView || (node.event_mask & EventMask::SCROLL) != 0;

        let child_clip = if should_clip_children(node) {
            match clip {
                Some(prev) => intersect_rects(prev, visual_rect),
                None => Some(visual_rect),
            }
        } else {
            clip
        };

        let (sx, sy) = self.visual_scroll_shift(node);
        let child_offset = (offset.0 - sx, offset.1 - sy);

        if child_clip.is_none_or(|c| point_in_rect(x, y, c))
            && let Some(hit) = self.visit_children_topmost_first(&node.children, |child| {
                self.hit_test_scroll_rec(child, x, y, child_offset, child_clip, depth + 1)
            })
        {
            return Some(hit);
        }

        if inside_self && is_scrollable {
            Some(id)
        } else {
            None
        }
    }
}
