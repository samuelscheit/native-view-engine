//! Z-ordered, clip-aware, scroll-shifted, sticky-clamped, `Transform2D`-mapped hit testing for
//! [`crate::SkiaBackend`] directly over `&NodeStore`.

use nve_abi::NodeId;
use nve_components::tables::{is_node_display_none, Interaction, Scroll, Visual};
use nve_engine::{LayoutRect, StoreLayoutEngine};
use nve_store::NodeStore;

use crate::display_list::{intersect_rects, offset_rect, point_in_rect, should_clip_children};
use crate::{SkiaBackend, MAX_HIT_TEST_DEPTH};

#[derive(Clone, Copy)]
enum HitTargetFilter {
    EventSubscribed,
    InteractionEligible,
}

impl HitTargetFilter {
    fn matches<L: StoreLayoutEngine>(
        self,
        backend: &SkiaBackend<L>,
        store: &NodeStore,
        id: NodeId,
    ) -> bool {
        match self {
            Self::EventSubscribed => store
                .try_row::<Interaction>(id)
                .is_some_and(|i| !i.listeners().is_empty()),
            Self::InteractionEligible => backend.layout_tree.is_interaction_target(store, id),
        }
    }
}

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    pub fn hit_test(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_filtered(
            store,
            NodeId::ROOT,
            x,
            y,
            (0.0, 0.0),
            None,
            false,
            HitTargetFilter::EventSubscribed,
            0,
        )
    }

    pub fn hit_test_interaction(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_filtered(
            store,
            NodeId::ROOT,
            x,
            y,
            (0.0, 0.0),
            None,
            false,
            HitTargetFilter::InteractionEligible,
            0,
        )
    }

    pub fn hit_test_scroll_container(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_scroll_rec(store, NodeId::ROOT, x, y, (0.0, 0.0), None, false, 0)
    }

    fn visit_children_topmost_first(
        store: &NodeStore,
        children: &[NodeId],
        mut visit: impl FnMut(NodeId) -> Option<NodeId>,
    ) -> Option<NodeId> {
        let has_z = children.len() > 1
            && children
                .iter()
                .any(|&cid| store.try_row::<Visual>(cid).is_some_and(|v| v.z_index() != 0));
        if !has_z {
            for &cid in children.iter().rev() {
                if let Some(hit) = visit(cid) {
                    return Some(hit);
                }
            }
        } else {
            let mut sorted: Vec<NodeId> = children.to_vec();
            sorted.sort_by_key(|&cid| store.try_row::<Visual>(cid).map_or(0, |v| v.z_index()));
            for cid in sorted.into_iter().rev() {
                if let Some(hit) = visit(cid) {
                    return Some(hit);
                }
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn hit_test_filtered(
        &self,
        store: &NodeStore,
        id: NodeId,
        x: f32,
        y: f32,
        offset: (f32, f32),
        clip: Option<LayoutRect>,
        inherited_hidden: bool,
        filter: HitTargetFilter,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > MAX_HIT_TEST_DEPTH || !store.is_live(id) || is_node_display_none(store, id) {
            return None;
        }
        let is_vis_hidden = store
            .try_row::<Visual>(id)
            .map_or(inherited_hidden, |v| v.visibility().resolve(inherited_hidden));
        let rect = self.layout_tree.layout_of(id)?;
        let (stx, sty) = self.layout_tree.compute_sticky_delta(store, id);
        let node_offset = (offset.0 + stx, offset.1 + sty);
        let visual_rect = offset_rect(rect, node_offset);
        if let Some(c) = clip
            && !point_in_rect(x, y, c)
        {
            return None;
        }

        let effective_tf = self
            .frame_hooks
            .resolve_transform(id, self.layout_tree.effective_transform(store, id));
        let (px, py) = if let Some(tf) = effective_tf
            && !tf.is_identity()
        {
            tf.inverse_map_surface_point(
                x,
                y,
                visual_rect.x,
                visual_rect.y,
                visual_rect.width,
                visual_rect.height,
            )?
        } else {
            (x, y)
        };
        let inside_self = point_in_rect(px, py, visual_rect);

        let clips = should_clip_children(store, id);
        if !inside_self && id != NodeId::ROOT && clips {
            return None;
        }

        let child_clip = if clips {
            let next = match clip {
                Some(prev) => intersect_rects(prev, visual_rect),
                None => Some(visual_rect),
            };
            match next {
                Some(c) if point_in_rect(px, py, c) => Some(c),
                _ => {
                    return if inside_self && !is_vis_hidden && filter.matches(self, store, id) {
                        Some(id)
                    } else {
                        None
                    };
                }
            }
        } else if effective_tf.is_some_and(|tf| !tf.is_identity()) {
            None
        } else {
            clip
        };

        let (sx, sy) = self.visual_scroll_shift(store, id);
        let child_offset = (node_offset.0 - sx, node_offset.1 - sy);

        if let Some(hit) = Self::visit_children_topmost_first(store, store.children(id), |child| {
            self.hit_test_filtered(
                store,
                child,
                px,
                py,
                child_offset,
                child_clip,
                is_vis_hidden,
                filter,
                depth + 1,
            )
        }) {
            return Some(hit);
        }

        if inside_self && !is_vis_hidden && filter.matches(self, store, id) {
            Some(id)
        } else {
            None
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn hit_test_scroll_rec(
        &self,
        store: &NodeStore,
        id: NodeId,
        x: f32,
        y: f32,
        offset: (f32, f32),
        clip: Option<LayoutRect>,
        inherited_hidden: bool,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > MAX_HIT_TEST_DEPTH || !store.is_live(id) || is_node_display_none(store, id) {
            return None;
        }
        let is_vis_hidden = store
            .try_row::<Visual>(id)
            .map_or(inherited_hidden, |v| v.visibility().resolve(inherited_hidden));
        let rect = self.layout_tree.layout_of(id)?;
        let visual_rect = offset_rect(rect, offset);
        let inside_self =
            point_in_rect(x, y, visual_rect) && clip.is_none_or(|c| point_in_rect(x, y, c));
        if !inside_self && id != NodeId::ROOT {
            return None;
        }

        let is_scrollable = store.has::<Scroll>(id);

        let child_clip = if should_clip_children(store, id) {
            match clip {
                Some(prev) => intersect_rects(prev, visual_rect),
                None => Some(visual_rect),
            }
        } else {
            clip
        };

        let (sx, sy) = self.visual_scroll_shift(store, id);
        let child_offset = (offset.0 - sx, offset.1 - sy);

        if child_clip.is_none_or(|c| point_in_rect(x, y, c))
            && let Some(hit) =
                Self::visit_children_topmost_first(store, store.children(id), |child| {
                    self.hit_test_scroll_rec(
                        store,
                        child,
                        x,
                        y,
                        child_offset,
                        child_clip,
                        is_vis_hidden,
                        depth + 1,
                    )
                })
        {
            return Some(hit);
        }

        if inside_self && !is_vis_hidden && is_scrollable {
            Some(id)
        } else {
            None
        }
    }
}
