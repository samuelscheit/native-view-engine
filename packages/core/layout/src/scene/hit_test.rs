//! Shared zero-allocation hit-test geometry over [`LaidOutScene`]: sticky clamping, `Transform2D`
//! inverse mapping (with optional animated transform override), scroll offsets, overflow clipping,
//! `z_index` reverse paint order, and `display: contents` pass-through.

use nve_abi::{EventMask, NodeId as AbiNodeId};
use nve_components::layout_ext::compute_sticky_offset;
use nve_components::tables::{
    Interaction, Layout as NveLayout, Overflow, Position, Scroll, Transform as TransformTable,
    Visual, generates_box, is_node_display_none, visual_clips_overflow,
};
use nve_components::transform::Transform2D;
use nve_engine::LayoutRect;
use nve_store::{NodeId as StoreNodeId, NodeStore};

use crate::{MAX_TREE_DEPTH, StoreLayoutEngine};

use super::{LaidOutScene, length_to_dim};

/// Result of hit-testing one subtree.
#[derive(Clone, Copy)]
struct Hit {
    /// Deepest accepting node under the point.
    target: Option<AbiNodeId>,
    /// The point lies on a visible box of this subtree, so the hit bubbles to this node.
    covered: bool,
}

const MISS: Hit = Hit {
    target: None,
    covered: false,
};

impl<L: StoreLayoutEngine> LaidOutScene<L> {
    /// Nearest ancestor of `id` that generates a box (skips `display: contents` ancestors).
    fn box_parent(store: &NodeStore, id: StoreNodeId) -> Option<StoreNodeId> {
        let mut cur = store.parent(id);
        for _ in 0..MAX_TREE_DEPTH {
            let anc = cur?;
            if generates_box(store, anc) {
                return Some(anc);
            }
            cur = store.parent(anc);
        }
        None
    }

    /// Computes sticky offset `(dx, dy)` for `id` against its nearest scroll ancestor.
    pub fn compute_sticky_delta(&self, store: &NodeStore, id: StoreNodeId) -> (f32, f32) {
        if !generates_box(store, id) {
            return (0.0, 0.0);
        }
        self.sticky_delta_of_box(store, id)
    }

    fn sticky_delta_of_box(&self, store: &NodeStore, id: StoreNodeId) -> (f32, f32) {
        let Some(l) = store.try_row::<NveLayout>(id) else {
            return (0.0, 0.0);
        };
        if l.position() != Position::Sticky {
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
            let has_offset = self.scroll.contains(raw);
            let is_scroll = store.has::<Scroll>(anc)
                || store.try_row::<Visual>(anc).is_some_and(|v| {
                    matches!(v.overflow_x(), Overflow::Scroll | Overflow::Auto)
                        || matches!(v.overflow_y(), Overflow::Scroll | Overflow::Auto)
                });
            if has_offset || is_scroll {
                scroller = anc;
                scroll = self.scroll.get(raw);
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
    /// generates a box.
    pub fn effective_transform(&self, store: &NodeStore, id: StoreNodeId) -> Option<Transform2D> {
        if !generates_box(store, id) {
            return None;
        }
        Self::transform_of_box(store, id)
    }

    fn transform_of_box(store: &NodeStore, id: StoreNodeId) -> Option<Transform2D> {
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

    /// Returns `true` when `id` subscribes to any UI event via its `Interaction` table.
    #[inline]
    pub fn is_event_target(store: &NodeStore, id: StoreNodeId) -> bool {
        store
            .try_row::<Interaction>(id)
            .is_some_and(|i| i.event_mask() != 0)
    }

    /// Returns `true` when `id` is a scroll container (`Scroll` table or `EventMask::SCROLL`).
    #[inline]
    pub fn is_scroll_container(store: &NodeStore, id: StoreNodeId) -> bool {
        store.has::<Scroll>(id)
            || store
                .try_row::<Interaction>(id)
                .is_some_and(|i| (u64::from(i.event_mask()) & EventMask::SCROLL) != 0)
    }

    /// Hit-tests `(x, y)` in reverse paint order for the deepest node subscribing to events.
    pub fn hit_test(&self, store: &NodeStore, x: f32, y: f32) -> Option<AbiNodeId> {
        self.hit_test_where(store, x, y, |id| Self::is_event_target(store, id))
    }

    /// Hit-tests `(x, y)` in reverse paint order for the deepest scrollable node (`Scroll` table
    /// or `EventMask::SCROLL`).
    pub fn hit_test_scroll_container(
        &self,
        store: &NodeStore,
        x: f32,
        y: f32,
    ) -> Option<AbiNodeId> {
        self.hit_test_where(store, x, y, |id| Self::is_scroll_container(store, id))
    }

    /// Hit-tests `(x, y)` in reverse paint order for the deepest node `accepts` admits.
    pub fn hit_test_where(
        &self,
        store: &NodeStore,
        x: f32,
        y: f32,
        accepts: impl Fn(StoreNodeId) -> bool,
    ) -> Option<AbiNodeId> {
        self.hit_test_where_with_transform(store, x, y, |_, base| base, accepts)
    }

    /// Hit-tests `(x, y)` allowing backends (such as `SkiaBackend`'s `MotionDriver`) to supply an
    /// animated `Transform2D` override per node while sharing the zero-alloc walk.
    pub fn hit_test_where_with_transform(
        &self,
        store: &NodeStore,
        x: f32,
        y: f32,
        resolve_transform: impl Fn(StoreNodeId, Option<Transform2D>) -> Option<Transform2D>,
        accepts: impl Fn(StoreNodeId) -> bool,
    ) -> Option<AbiNodeId> {
        let mut walk = HitWalk {
            scene: self,
            store,
            resolve_transform,
            accepts,
            scratch: self.hit_scratch.take(),
        };
        let target = walk
            .walk(StoreNodeId::ROOT, (x, y), (0.0, 0.0), false, 0)
            .target;
        debug_assert!(walk.scratch.is_empty(), "every level truncates its keys");
        self.hit_scratch.set(walk.scratch);
        target
    }
}

#[inline]
fn paint_z(store: &NodeStore, id: StoreNodeId) -> i32 {
    match store.try_row::<Visual>(id).map_or(0, |v| v.z_index()) {
        0 => 0,
        z if generates_box(store, id) => z,
        _ => 0,
    }
}

fn single_z_level(store: &NodeStore, children: &[StoreNodeId]) -> bool {
    match children {
        [] | [_] => true,
        [first, rest @ ..] => {
            let z = paint_z(store, *first);
            rest.iter().all(|&c| paint_z(store, c) == z)
        }
    }
}

const STACK_SORT_LEN: usize = 64;

fn fill_z_keys(store: &NodeStore, children: &[StoreNodeId], keys: &mut [(i32, u32)]) {
    for (i, (key, &c)) in keys.iter_mut().zip(children).enumerate() {
        *key = (paint_z(store, c), i as u32);
    }
    keys.sort_unstable();
}

struct HitWalk<'a, L: StoreLayoutEngine, T, F> {
    scene: &'a LaidOutScene<L>,
    store: &'a NodeStore,
    resolve_transform: T,
    accepts: F,
    scratch: Vec<(i32, u32)>,
}

struct EnteredNode {
    child_point: (f32, f32),
    child_offset: (f32, f32),
    inside: bool,
    hidden: bool,
}

impl<
    'a,
    L: StoreLayoutEngine,
    T: Fn(StoreNodeId, Option<Transform2D>) -> Option<Transform2D>,
    F: Fn(StoreNodeId) -> bool,
> HitWalk<'a, L, T, F>
{
    fn walk_children(
        &mut self,
        children: &'a [StoreNodeId],
        point: (f32, f32),
        offset: (f32, f32),
        hidden: bool,
        depth: usize,
    ) -> Option<Hit> {
        if single_z_level(self.store, children) {
            return children
                .iter()
                .rev()
                .find_map(|&cid| self.child_hit(cid, point, offset, hidden, depth));
        }
        if children.len() <= STACK_SORT_LEN {
            self.walk_sorted_on_stack(children, point, offset, hidden, depth)
        } else {
            self.walk_sorted_in_scratch(children, point, offset, hidden, depth)
        }
    }

    fn child_hit(
        &mut self,
        cid: StoreNodeId,
        point: (f32, f32),
        offset: (f32, f32),
        hidden: bool,
        depth: usize,
    ) -> Option<Hit> {
        let hit = self.walk(cid, point, offset, hidden, depth);
        (hit.target.is_some() || hit.covered).then_some(hit)
    }

    #[inline(never)]
    fn walk_sorted_on_stack(
        &mut self,
        children: &'a [StoreNodeId],
        point: (f32, f32),
        offset: (f32, f32),
        hidden: bool,
        depth: usize,
    ) -> Option<Hit> {
        let mut buf = [(0, 0); STACK_SORT_LEN];
        let keys = &mut buf[..children.len()];
        fill_z_keys(self.store, children, keys);
        keys.iter()
            .rev()
            .find_map(|&(_, i)| self.child_hit(children[i as usize], point, offset, hidden, depth))
    }

    #[inline(never)]
    fn walk_sorted_in_scratch(
        &mut self,
        children: &'a [StoreNodeId],
        point: (f32, f32),
        offset: (f32, f32),
        hidden: bool,
        depth: usize,
    ) -> Option<Hit> {
        let start = self.scratch.len();
        let end = start + children.len();
        self.scratch.resize(end, (0, 0));
        fill_z_keys(self.store, children, &mut self.scratch[start..]);
        let mut found = None;
        for k in (start..end).rev() {
            let cid = children[self.scratch[k].1 as usize];
            found = self.child_hit(cid, point, offset, hidden, depth);
            if found.is_some() {
                break;
            }
        }
        self.scratch.truncate(start);
        found
    }

    #[inline(never)]
    fn enter_node(
        &self,
        id: StoreNodeId,
        point: (f32, f32),
        offset: (f32, f32),
        inherited_hidden: bool,
    ) -> Option<EnteredNode> {
        let (store, scene) = (self.store, self.scene);
        if !store.is_live(id) || is_node_display_none(store, id) {
            return None;
        }
        let visual = store.try_row::<Visual>(id);
        let hidden = visual
            .as_ref()
            .map_or(inherited_hidden, |v| v.visibility().resolve(inherited_hidden));
        if !generates_box(store, id) {
            return Some(EnteredNode {
                child_point: point,
                child_offset: offset,
                inside: false,
                hidden,
            });
        }
        let rect = scene.layout.layout_of(id)?;
        let (stx, sty) = scene.sticky_delta_of_box(store, id);
        let r_x = rect.x + offset.0 + stx;
        let r_y = rect.y + offset.1 + sty;
        let base_tf = LaidOutScene::<L>::transform_of_box(store, id);
        let effective_tf = (self.resolve_transform)(id, base_tf);
        let (px, py) = match effective_tf {
            Some(tf) if !tf.is_identity() => tf.inverse_map_surface_point(
                point.0,
                point.1,
                r_x,
                r_y,
                rect.width,
                rect.height,
            )?,
            _ => point,
        };
        let inside = rect.width > 0.0
            && rect.height > 0.0
            && px >= r_x
            && px <= r_x + rect.width
            && py >= r_y
            && py <= r_y + rect.height;
        let clips = store.has::<Scroll>(id) || visual.as_ref().is_some_and(visual_clips_overflow);
        if !inside && id != StoreNodeId::ROOT && clips {
            return None;
        }
        let (sx, sy) = scene.scroll.visual_shift(store, id);
        let child_offset = (offset.0 + stx - sx, offset.1 + sty - sy);
        Some(EnteredNode {
            child_point: (px, py),
            child_offset,
            inside,
            hidden,
        })
    }

    fn walk(
        &mut self,
        id: StoreNodeId,
        point: (f32, f32),
        offset: (f32, f32),
        inherited_hidden: bool,
        depth: usize,
    ) -> Hit {
        if depth > MAX_TREE_DEPTH {
            return MISS;
        }
        let Some(entered) = self.enter_node(id, point, offset, inherited_hidden) else {
            return MISS;
        };

        let mut covered = entered.inside && !entered.hidden;
        let children = self.store.children(id);
        match self.walk_children(
            children,
            entered.child_point,
            entered.child_offset,
            entered.hidden,
            depth + 1,
        ) {
            Some(hit) if hit.target.is_some() => return hit,
            Some(_) => covered = true,
            None => {}
        }
        Hit {
            target: (covered && (self.accepts)(id)).then_some(id.raw()),
            covered,
        }
    }
}

#[cfg(test)]
mod tests {
    use nve_abi::EventMask;
    use nve_components::tables::{Interaction, Layout, Scroll};
    use nve_engine::Viewport;
    use nve_store::{ChangeSet, Length, NodeId, NodeStore};

    use crate::{LaidOutScene, TaffyStoreLayout};

    #[test]
    fn scroll_table_clips_hit_tests_without_explicit_visual_overflow() {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let (scroller, child) = {
            let mut tx = store.tx();
            let scroller = tx.create();
            tx.edit::<Layout>(scroller)
                .width(Length::points(120.0))
                .height(Length::points(80.0));
            // Attach `Scroll` with NO `Visual` row — matches Skia's `should_clip_children`.
            tx.edit::<Scroll>(scroller);

            let child = tx.create();
            tx.edit::<Layout>(child)
                .width(Length::points(120.0))
                .height(Length::points(200.0));
            tx.edit::<Interaction>(child)
                .event_mask(EventMask::PRESS as u32);

            tx.set_children(scroller, &[child]);
            tx.set_children(NodeId::ROOT, &[scroller]);
            tx.commit_into(&mut changes);
            (scroller, child)
        };

        let mut scene = LaidOutScene::new(TaffyStoreLayout::new());
        scene.set_viewport(Viewport {
            width: 400.0,
            height: 400.0,
            scale_factor: 1.0,
        });
        assert!(scene.commit(&store, &changes));

        // Inside the 120x80 scroll viewport hits the interactive child.
        assert_eq!(scene.hit_test(&store, 40.0, 40.0), Some(child.raw()));
        // Below the 80px scroll viewport (inside child's 200px overflow) MUST be clipped.
        assert_eq!(scene.hit_test(&store, 40.0, 140.0), None);
        assert_eq!(
            scene.hit_test_scroll_container(&store, 40.0, 40.0),
            Some(scroller.raw())
        );
    }
}
