//! Hit-test geometry shared by event dispatch ([`HeadlessBackend::hit_test`]) and interaction
//! state tracking ([`HeadlessBackend::hit_test_interaction`]): sticky deltas, `Transform2D`
//! inverse mapping, scroll offsets, overflow clipping, and `display: contents` pass-through.
//!
//! **Bubbling**: a node is a hit candidate whenever the point lands on a visible box of its
//! subtree (its own box, or a descendant box its clip does not cut away), as DOM events bubble
//! from the hit box to every ancestor regardless of the ancestor's own extent or `visibility`;
//! the deepest accepting candidate wins. A `display: contents` node generates no box (no rect,
//! transform, scroll offset, sticky delta, or clip): the walk passes the parent's coordinate
//! space straight to its children, and it is a candidate only through its descendants.

use nve_abi::NodeId;
use nve_components::layout_ext::compute_sticky_offset;
use nve_components::tables::{
    Interaction, Layout as NveLayout, Overflow, Position, Transform as TransformTable, Visual,
};
use nve_components::transform::Transform2D;
use nve_engine::{LayoutRect, StoreLayoutEngine};
use nve_store::{NodeId as StoreNodeId, NodeStore};

use crate::{
    HeadlessBackend, MAX_TREE_DEPTH, generates_box, is_display_contents, is_display_none,
    length_to_dim, node_clips_overflow,
};

/// Result of hit-testing one subtree.
#[derive(Clone, Copy)]
struct Hit {
    /// Deepest accepting node under the point.
    target: Option<NodeId>,
    /// The point lies on a visible box of this subtree, so the hit bubbles to this node.
    covered: bool,
}

const MISS: Hit = Hit {
    target: None,
    covered: false,
};

impl<L: StoreLayoutEngine> HeadlessBackend<L> {
    /// Nearest ancestor of `id` that generates a box (skips `display: contents` ancestors).
    fn box_parent(store: &NodeStore, id: StoreNodeId) -> Option<StoreNodeId> {
        let mut cur = store.parent(id);
        for _ in 0..MAX_TREE_DEPTH {
            let anc = cur?;
            if anc == StoreNodeId::ROOT || !is_display_contents(store, anc) {
                return Some(anc);
            }
            cur = store.parent(anc);
        }
        None
    }

    /// Computes sticky offset `(dx, dy)` for `id` against its nearest scroll ancestor. Containing
    /// block and scroll container are the nearest *box* ancestors (`display: contents` ancestors
    /// have neither a rect nor a scroll port); a contents node itself never sticks.
    pub fn compute_sticky_delta(&self, store: &NodeStore, id: StoreNodeId) -> (f32, f32) {
        let Some(l) = store.try_row::<NveLayout>(id) else {
            return (0.0, 0.0);
        };
        if l.position() != Position::Sticky || is_display_contents(store, id) {
            return (0.0, 0.0);
        }
        let Some(node_rect) = self.layout.layout_of(id) else {
            return (0.0, 0.0);
        };
        let Some(parent_id) = Self::box_parent(store, id) else {
            return (0.0, 0.0);
        };
        let Some(cb_rect) = self.layout.layout_of(parent_id) else {
            return (0.0, 0.0);
        };

        // Walk up box ancestors to the nearest scroll container (overflow scroll/auto or a
        // non-zero scroll offset).
        let mut scroller = parent_id;
        let mut cur = Some(parent_id);
        let mut scroll = (0.0f32, 0.0f32);
        let mut steps = 0usize;
        while let Some(anc) = cur {
            if steps > MAX_TREE_DEPTH {
                break;
            }
            steps += 1;
            let raw = anc.raw();
            let has_offset = self.scroll_offsets.contains_key(&raw);
            let is_scroll = store.try_row::<Visual>(anc).is_some_and(|v| {
                matches!(v.overflow_x(), Overflow::Scroll | Overflow::Auto)
                    || matches!(v.overflow_y(), Overflow::Scroll | Overflow::Auto)
            });
            if has_offset || is_scroll {
                scroller = anc;
                scroll = self.scroll_offset(raw);
                break;
            }
            cur = Self::box_parent(store, anc);
        }

        let vp_rect = self.layout.layout_of(scroller).unwrap_or(LayoutRect {
            x: 0.0,
            y: 0.0,
            width: self.viewport.width,
            height: self.viewport.height,
        });
        let insets = nve_components::layout_ext::PositionInsets {
            top: length_to_dim(l.inset_top()),
            right: length_to_dim(l.inset_right()),
            bottom: length_to_dim(l.inset_bottom()),
            left: length_to_dim(l.inset_left()),
        };
        compute_sticky_offset(
            (node_rect.x, node_rect.y, node_rect.width, node_rect.height),
            (cb_rect.x, cb_rect.y, cb_rect.width, cb_rect.height),
            (vp_rect.x, vp_rect.y, vp_rect.width, vp_rect.height),
            scroll,
            insets,
        )
    }

    /// Reads the `Transform2D` of `id` from `NodeStore`'s `Transform` table, if attached and `id`
    /// generates a box (a `display: contents` node has nothing to transform).
    pub fn effective_transform(&self, store: &NodeStore, id: StoreNodeId) -> Option<Transform2D> {
        if !generates_box(store, id) {
            return None;
        }
        let t = store.try_row::<TransformTable>(id)?;
        let tf = Transform2D {
            translate_x: t.translate_x(),
            translate_y: t.translate_y(),
            scale_x: t.scale_x(),
            scale_y: t.scale_y(),
            rotate_deg: t.rotate_deg(),
            origin_x: t.origin_x(),
            origin_y: t.origin_y(),
        };
        (!tf.is_identity()).then_some(tf)
    }

    /// Hit-tests `(x, y)` in reverse paint order for the deepest node subscribing to events.
    pub fn hit_test(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_where(store, x, y, |id| {
            store
                .try_row::<Interaction>(id)
                .is_some_and(|i| i.event_mask() != 0)
        })
    }

    /// Hit-tests `(x, y)` in reverse paint order for the deepest node `accepts` admits.
    pub(crate) fn hit_test_where(
        &self,
        store: &NodeStore,
        x: f32,
        y: f32,
        accepts: impl Fn(StoreNodeId) -> bool,
    ) -> Option<NodeId> {
        HitWalk {
            backend: self,
            store,
            accepts,
        }
        .walk(StoreNodeId::ROOT, (x, y), (0.0, 0.0), false, 0)
        .target
    }
}

/// One hit-test query: the backend's geometry, the store, and the target predicate.
struct HitWalk<'a, L: StoreLayoutEngine, F> {
    backend: &'a HeadlessBackend<L>,
    store: &'a NodeStore,
    accepts: F,
}

impl<L: StoreLayoutEngine, F: Fn(StoreNodeId) -> bool> HitWalk<'_, L, F> {
    fn walk(
        &self,
        id: StoreNodeId,
        point: (f32, f32),
        offset: (f32, f32),
        inherited_hidden: bool,
        depth: usize,
    ) -> Hit {
        let (store, backend) = (self.store, self.backend);
        if depth > MAX_TREE_DEPTH || !store.is_live(id) || is_display_none(store, id) {
            return MISS;
        }
        let hidden = store.try_row::<Visual>(id).map_or(inherited_hidden, |v| {
            v.visibility().resolve(inherited_hidden)
        });
        let (child_point, child_offset, inside) = if !generates_box(store, id) {
            (point, offset, false)
        } else {
            let Some(rect) = backend.layout.layout_of(id) else {
                return MISS;
            };
            let (stx, sty) = backend.compute_sticky_delta(store, id);
            let r_x = rect.x + offset.0 + stx;
            let r_y = rect.y + offset.1 + sty;
            let (px, py) = match backend.effective_transform(store, id) {
                Some(tf) => match tf.inverse_map_surface_point(
                    point.0,
                    point.1,
                    r_x,
                    r_y,
                    rect.width,
                    rect.height,
                ) {
                    Some(p) => p,
                    None => return MISS,
                },
                None => point,
            };
            let inside = rect.width > 0.0
                && rect.height > 0.0
                && px >= r_x
                && px <= r_x + rect.width
                && py >= r_y
                && py <= r_y + rect.height;
            if !inside && id != StoreNodeId::ROOT && node_clips_overflow(store, id) {
                return MISS;
            }
            let (sx, sy) = backend.scroll_offset(id.raw());
            let child_offset = (offset.0 + stx - sx, offset.1 + sty - sy);
            ((px, py), child_offset, inside)
        };

        let mut covered = inside && !hidden;
        for &cid in store.children(id).iter().rev() {
            let hit = self.walk(cid, child_point, child_offset, hidden, depth + 1);
            if hit.target.is_some() {
                return hit;
            }
            covered |= hit.covered;
        }
        Hit {
            target: (covered && (self.accepts)(id)).then_some(id.raw()),
            covered,
        }
    }
}

#[cfg(test)]
#[path = "hit_test_tests.rs"]
mod tests;
