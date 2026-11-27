//! Pure-Rust headless [`StoreBackend`] generic over any [`StoreLayoutEngine`] (defaulting to
//! [`TaffyStoreLayout`] directly over `NodeStore` word columns with zero `TaffyTree` duplication).
//!
//! Provides <50ms edit->test verification of any Rust UI frontend, mutation diffing, coordinate
//! hit-testing (with scroll offsets, sticky clamping, overflow clipping, and `Transform2D`),
//! interactive pseudo-state & cursor tracking (`InteractionTracker`), live `NveSceneView`
//! (`primary_slot` + `NveTableView`) publication, live layout-engine swapping, and an ASCII
//! tree + rect inspector.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod commit;
mod interaction;
pub mod reloc_scene;
mod snapshot;
pub mod taffy_store;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use nve_abi::table::NveTableView;
use nve_abi::{
    ColorRgba, EventBatchBuf, FrameId, NodeId, NveRenderNode, NveSceneTable, NveSceneView,
    PrimarySlot, SceneNodeInput, ShortLen, SurfaceId, ZIndex, NVE_NODE_CLIP_BOUNDS,
    NVE_NODE_INTERACTIVE, NVE_NODE_LEAF_TEXT,
};
use nve_components::layout_ext::compute_sticky_offset;
use nve_components::tables::{
    Interaction, Layout as NveLayout, Overflow, Position, TextContent,
    Transform as TransformTable, Typography, Visual,
};
use nve_components::transform::Transform2D;
use nve_engine::{
    HeuristicTextMeasurer, LayoutRect, StoreLayoutEngine, TextMeasurer, Viewport,
};
use nve_store::{ChangeSet, NodeStore, NveTable, TableSlot};

pub use nve_components::interaction::{
    CursorKind, FocusModality, InteractionState, InteractionTracker, StateStyleRule,
};
pub use nve_components::layout_ext::{
    GridItemPlacement, GridLayout, GridLine, GridSingleTrack, GridTrack, GridTrackPlacement,
    GridTrackUnit, LayoutDirection, LayoutExt, LogicalInlineEdges, PositionInsets, PositionMode,
};
pub use nve_components::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};
pub use nve_components::snapshots::{LayoutSnapshot, VisualSnapshot};
pub use nve_components::tables::Display;
pub use nve_components::{RowPatch, StylePatchExt};
pub use reloc_scene::{prerender_native_frame0, prerender_native_frame0_with_text_measurer};
pub(crate) use snapshot::{is_display_none, length_to_dim, node_clips_overflow};
pub use snapshot::{layout_snapshot_of, visual_snapshot_of};
pub use taffy_store::{ResolvedTextStyle, TaffyStoreLayout};

pub(crate) const MAX_TREE_DEPTH: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub enum HeadlessMutation {
    Create(NodeId, TableSlot),
    Remove(NodeId),
    UpdateLayout(NodeId, LayoutRect),
    UpdateText(NodeId, String),
    UpdateStyle(NodeId),
}

/// Pure-Rust headless [`StoreBackend`] backed by a zero-copy [`StoreLayoutEngine`].
///
/// Holds ZERO duplicate copies of the UI tree, strings, or style props — reads directly from
/// `&NodeStore` during `commit` and inspection methods.
pub struct HeadlessBackend<L: StoreLayoutEngine = TaffyStoreLayout> {
    pub(crate) surface_id: SurfaceId,
    pub(crate) viewport: Viewport,
    pub(crate) viewport_dirty: bool,
    pub(crate) layout: L,
    text_measurer: Arc<dyn TextMeasurer>,
    pub(crate) prev_rects: Vec<(u64, LayoutRect)>,
    pub(crate) rect_gen: u64,
    pub(crate) track_mutations: bool,
    pub(crate) interaction: InteractionTracker,
    pub(crate) scroll_offsets: HashMap<NodeId, (f32, f32)>,
    pub(crate) last_mutations: Vec<HeadlessMutation>,
    pub(crate) pending_events: EventBatchBuf,
    pub(crate) scene_table: NveSceneTable,
    pub(crate) published_tables: Vec<NveTableView>,
    published_dirty_buf: Vec<u32>,
    published_children_scratch: Vec<NodeId>,
    text_dirty_marks: Vec<u64>,
    text_dirty_epoch: u64,
    pub(crate) walk_stack: Vec<(NodeId, usize)>,
}

impl Default for HeadlessBackend<TaffyStoreLayout> {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessBackend<TaffyStoreLayout> {
    pub fn new() -> Self {
        Self::with_layout(TaffyStoreLayout::new())
    }
}

impl<L: StoreLayoutEngine> HeadlessBackend<L> {
    pub fn with_layout(mut layout: L) -> Self {
        let text_measurer: Arc<dyn TextMeasurer> = Arc::new(HeuristicTextMeasurer);
        layout.set_text_measurer(Arc::clone(&text_measurer));
        Self {
            surface_id: SurfaceId::ROOT,
            viewport: Viewport::default(),
            viewport_dirty: true,
            layout,
            text_measurer,
            prev_rects: Vec::new(),
            rect_gen: 0,
            track_mutations: true,
            interaction: InteractionTracker::new(),
            scroll_offsets: HashMap::new(),
            last_mutations: Vec::new(),
            pending_events: EventBatchBuf::new(),
            scene_table: NveSceneTable::new(),
            published_tables: Vec::new(),
            published_dirty_buf: Vec::new(),
            published_children_scratch: Vec::new(),
            text_dirty_marks: Vec::new(),
            text_dirty_epoch: 0,
            walk_stack: Vec::new(),
        }
    }

    pub fn with_mutation_tracking(mut self, enabled: bool) -> Self {
        self.track_mutations = enabled;
        self
    }

    #[inline]
    pub fn set_mutation_tracking(&mut self, enabled: bool) {
        self.track_mutations = enabled;
    }

    #[inline]
    pub fn layout_engine_name(&self) -> &'static str {
        self.layout.name()
    }

    #[inline]
    pub fn layout_engine(&self) -> &L {
        &self.layout
    }

    #[inline]
    pub fn layout_engine_mut(&mut self) -> &mut L {
        &mut self.layout
    }

    #[inline]
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    #[inline]
    pub fn surface_id(&self) -> SurfaceId {
        self.surface_id
    }

    pub fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.text_measurer = Arc::clone(&measurer);
        self.layout.set_text_measurer(measurer);
        self.viewport_dirty = true;
    }

    #[inline]
    pub fn text_measurer(&self) -> &Arc<dyn TextMeasurer> {
        &self.text_measurer
    }

    /// Hot-swaps the active layout engine while preserving viewport, scroll, and interaction state.
    /// The caller should follow with `recompute_layout(store)` if an immediate re-layout is needed.
    pub fn swap_layout_engine<L2: StoreLayoutEngine>(
        self,
        mut new_layout: L2,
    ) -> HeadlessBackend<L2> {
        new_layout.set_text_measurer(Arc::clone(&self.text_measurer));
        HeadlessBackend {
            surface_id: self.surface_id,
            viewport: self.viewport,
            viewport_dirty: true,
            layout: new_layout,
            text_measurer: self.text_measurer,
            prev_rects: self.prev_rects,
            rect_gen: self.rect_gen,
            track_mutations: self.track_mutations,
            interaction: self.interaction,
            scroll_offsets: self.scroll_offsets,
            last_mutations: self.last_mutations,
            pending_events: self.pending_events,
            scene_table: self.scene_table,
            published_tables: self.published_tables,
            published_dirty_buf: self.published_dirty_buf,
            published_children_scratch: self.published_children_scratch,
            text_dirty_marks: self.text_dirty_marks,
            text_dirty_epoch: self.text_dirty_epoch,
            walk_stack: self.walk_stack,
        }
    }

    /// Recomputes layout for `store` immediately (e.g. after swapping layout engines or viewport).
    pub fn recompute_layout(&mut self, store: &NodeStore) {
        self.layout
            .compute_layout(store, NodeId::ROOT, self.viewport);
        self.viewport_dirty = false;
    }

    #[inline]
    pub fn last_mutations(&self) -> &[HeadlessMutation] {
        &self.last_mutations
    }

    /// Renders a human-readable indented tree of live nodes, layout rects, test IDs, and text.
    pub fn render_ascii_tree(&self, store: &NodeStore) -> String {
        use core::fmt::Write as _;
        fn walk<L: StoreLayoutEngine>(
            backend: &HeadlessBackend<L>,
            store: &NodeStore,
            id: NodeId,
            depth: usize,
            out: &mut String,
        ) {
            for _ in 0..depth {
                out.push_str("  ");
            }
            let r = backend.layout_of(id).unwrap_or_default();
            let _ = write!(
                out,
                "Node#{} ({:.0},{:.0} {:.0}×{:.0})",
                id.raw(),
                r.x,
                r.y,
                r.width,
                r.height
            );
            if let Some(dbg) = store.try_row::<nve_components::tables::DebugId>(id) {
                let tid = dbg.test_id();
                if !tid.is_empty() {
                    let _ = write!(out, " [{tid}]");
                }
            }
            if let Some(tc) = store.try_row::<nve_components::tables::TextContent>(id) {
                let t = tc.text();
                if !t.is_empty() {
                    let _ = write!(out, " \"{t}\"");
                }
            }
            if let Some(ti) = store.try_row::<nve_components::tables::TextInput>(id) {
                let v = ti.value();
                if !v.is_empty() {
                    let _ = write!(out, " value=\"{v}\"");
                }
            }
            out.push('\n');
            for &child in store.children(id) {
                walk(backend, store, child, depth + 1, out);
            }
        }
        let mut out = String::new();
        walk(self, store, NodeId::ROOT, 0, &mut out);
        out
    }

    #[inline]
    pub fn pending_events_mut(&mut self) -> &mut EventBatchBuf {
        &mut self.pending_events
    }

    #[inline]
    pub fn set_scroll_offset(&mut self, id: NodeId, dx: f32, dy: f32) {
        self.scroll_offsets.insert(id, (dx, dy));
    }

    #[inline]
    pub fn scroll_offset(&self, id: NodeId) -> (f32, f32) {
        self.scroll_offsets.get(&id).copied().unwrap_or((0.0, 0.0))
    }

    #[inline]
    pub fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        self.layout.layout_of(id)
    }

    /// Computes sticky offset `(dx, dy)` for `id` against its nearest scroll ancestor.
    pub fn compute_sticky_delta(&self, store: &NodeStore, id: NodeId) -> (f32, f32) {
        let Some(l) = store.try_row::<NveLayout>(id) else {
            return (0.0, 0.0);
        };
        if l.position() != Position::Sticky {
            return (0.0, 0.0);
        }
        let Some(node_rect) = self.layout.layout_of(id) else {
            return (0.0, 0.0);
        };
        let Some(parent_id) = store.parent(id) else {
            return (0.0, 0.0);
        };
        let Some(cb_rect) = self.layout.layout_of(parent_id) else {
            return (0.0, 0.0);
        };

        // Walk up to find nearest scroll container (overflow scroll/auto or non-zero scroll offset).
        let mut scroller = parent_id;
        let mut cur = Some(parent_id);
        let mut scroll = (0.0f32, 0.0f32);
        let mut steps = 0usize;
        while let Some(anc) = cur {
            if steps > MAX_TREE_DEPTH {
                break;
            }
            steps += 1;
            let has_offset = self.scroll_offsets.contains_key(&anc);
            let is_scroll = store.try_row::<Visual>(anc).is_some_and(|v| {
                matches!(v.overflow_x(), Overflow::Scroll | Overflow::Auto)
                    || matches!(v.overflow_y(), Overflow::Scroll | Overflow::Auto)
            });
            if has_offset || is_scroll {
                scroller = anc;
                scroll = self.scroll_offset(anc);
                break;
            }
            cur = store.parent(anc);
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

    /// Reads the `Transform2D` of `id` from `NodeStore`'s `Transform` table, if attached.
    pub fn effective_transform(&self, store: &NodeStore, id: NodeId) -> Option<Transform2D> {
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
        self.hit_test_rec(store, NodeId::ROOT, x, y, (0.0, 0.0), false, 0)
    }

    #[allow(clippy::too_many_arguments)]
    fn hit_test_rec(
        &self,
        store: &NodeStore,
        id: NodeId,
        x: f32,
        y: f32,
        offset: (f32, f32),
        inherited_hidden: bool,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > MAX_TREE_DEPTH || !store.is_live(id) || is_display_none(store, id) {
            return None;
        }
        let effective_hidden = store
            .try_row::<Visual>(id)
            .map_or(inherited_hidden, |v| v.visibility().resolve(inherited_hidden));
        let rect = self.layout.layout_of(id)?;
        let (stx, sty) = self.compute_sticky_delta(store, id);
        let node_offset = (offset.0 + stx, offset.1 + sty);
        let r_x = rect.x + node_offset.0;
        let r_y = rect.y + node_offset.1;
        let (px, py) = if let Some(tf) = self.effective_transform(store, id) {
            tf.inverse_map_surface_point(x, y, r_x, r_y, rect.width, rect.height)?
        } else {
            (x, y)
        };
        let inside = rect.width > 0.0
            && rect.height > 0.0
            && px >= r_x
            && px <= r_x + rect.width
            && py >= r_y
            && py <= r_y + rect.height;
        if !inside && id != NodeId::ROOT && node_clips_overflow(store, id) {
            return None;
        }
        let (sx, sy) = self.scroll_offset(id);
        let child_offset = (node_offset.0 - sx, node_offset.1 - sy);
        for &cid in store.children(id).iter().rev() {
            if let Some(hit) = self.hit_test_rec(
                store,
                cid,
                px,
                py,
                child_offset,
                effective_hidden,
                depth + 1,
            ) {
                return Some(hit);
            }
        }
        let has_listeners = store
            .try_row::<Interaction>(id)
            .is_some_and(|i| !i.listeners().is_empty());
        if inside && !effective_hidden && has_listeners {
            Some(id)
        } else {
            None
        }
    }

    /// Publishes the latest laid-out `NodeStore` into a zero-copy `NveSceneView` with
    /// `primary_slot` and dense `NveTableView` descriptors ready for native C++/Swift/Kotlin/Dart hosts.
    ///
    /// Unifies `NveSceneView::text_slab` directly with `store.text_slab()` so both
    /// `NveRenderNode::{text_offset,text_len}` and every published `NveTableView` `Str` word decode
    /// against the single zero-copy UTF-8 slab with zero text duplication.
    pub fn publish_scene_view(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        frame_id: FrameId,
    ) -> NveSceneView {
        self.scene_table.begin_frame(frame_id);
        for &removed in &changes.removed {
            self.scene_table.remove_node(removed);
        }
        let has_text_changes = if let Some(td) = changes.table(TextContent::slot()) {
            self.text_dirty_epoch = self.text_dirty_epoch.wrapping_add(1);
            if self.text_dirty_epoch == 0 {
                self.text_dirty_marks.fill(0);
                self.text_dirty_epoch = 1;
            }
            if self.text_dirty_marks.len() < store.capacity() {
                self.text_dirty_marks.resize(store.capacity(), 0);
            }
            let epoch = self.text_dirty_epoch;
            for &nid in td.nodes.iter().chain(td.detached.iter()) {
                let idx = nid.index();
                if idx < self.text_dirty_marks.len() {
                    self.text_dirty_marks[idx] = epoch;
                }
            }
            true
        } else {
            false
        };
        self.walk_stack.clear();
        if store.is_live(NodeId::ROOT) {
            self.walk_stack.push((NodeId::ROOT, 0));
        }
        while let Some((id, depth)) = self.walk_stack.pop() {
            if !store.is_live(id) {
                continue;
            }
            let rect = self.layout.layout_of(id).unwrap_or_default();
            let (bg, border_color, border_width, border_radius, opacity, z_index, clip) =
                if let Some(v) = store.try_row::<Visual>(id) {
                    (
                        v.background().0,
                        v.border_color().0,
                        v.border_width(),
                        v.border_radius(),
                        v.opacity(),
                        v.z_index(),
                        matches!(
                            v.overflow_x(),
                            Overflow::Hidden | Overflow::Scroll | Overflow::Auto
                        ) || matches!(
                            v.overflow_y(),
                            Overflow::Hidden | Overflow::Scroll | Overflow::Auto
                        ),
                    )
                } else {
                    (0, 0, 0.0, 0.0, 1.0, 0, false)
                };
            let has_listeners = store
                .try_row::<Interaction>(id)
                .is_some_and(|i| !i.listeners().is_empty());
            let (text_offset, text_len) = if let Some(tc) = store.try_row::<TextContent>(id) {
                let span = tc.text_span();
                let s = tc.text();
                let clamped_len = if s.len() <= u16::MAX as usize {
                    s.len() as u16
                } else {
                    let mut end = u16::MAX as usize;
                    while !s.is_char_boundary(end) {
                        end -= 1;
                    }
                    end as u16
                };
                if clamped_len == 0 {
                    (0u32, 0u16)
                } else {
                    (span.off(), clamped_len)
                }
            } else {
                (0u32, 0u16)
            };
            let children = if depth < MAX_TREE_DEPTH {
                store.children(id)
            } else {
                &[]
            };
            let mut flags = 0u8;
            if text_len > 0 && children.is_empty() {
                flags |= NVE_NODE_LEAF_TEXT;
            }
            if has_listeners {
                flags |= NVE_NODE_INTERACTIVE;
            }
            if clip {
                flags |= NVE_NODE_CLIP_BOUNDS;
            }
            let text_rgba = store
                .try_row::<Typography>(id)
                .map_or(ColorRgba::BLACK.to_u32(), |t| t.color().0);
            let font_size = store
                .try_row::<Typography>(id)
                .map_or(16.0, |t| t.font_size());
            let opacity_u8 = NveRenderNode::encode_opacity_u8(opacity);

            self.published_children_scratch.clear();
            self.published_children_scratch
                .extend(children.iter().take(u16::MAX as usize).copied());

            let text_dirty = has_text_changes
                && self.text_dirty_marks.get(id.index()).copied() == Some(self.text_dirty_epoch);
            let res = self.scene_table.upsert_node_external_text(
                SceneNodeInput {
                    node_id: id,
                    primary_slot: PrimarySlot(store.primary_slot(id).0),
                    flags,
                    opacity_u8,
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                    bg_rgba: bg,
                    text_rgba,
                    border_rgba: border_color,
                    border_radius,
                    border_width,
                    font_size,
                    z_index: ZIndex(z_index),
                    text: "",
                    children: &self.published_children_scratch,
                },
                text_offset,
                ShortLen(text_len),
                text_dirty,
            );
            debug_assert!(res.is_ok(), "scene_table upsert failed: {res:?}");

            if depth < MAX_TREE_DEPTH {
                for &cid in children.iter().take(u16::MAX as usize).rev() {
                    self.walk_stack.push((cid, depth + 1));
                }
            }
        }

        self.scene_table.prune_unseen();
        self.scene_table.maybe_compact_slabs();

        store.publish_table_views(
            changes,
            &mut self.published_tables,
            &mut self.published_dirty_buf,
        );
        let mut view = self.scene_table.as_view_with_external_text(store.text_slab());
        view.table_count = self.published_tables.len() as u32;
        view.tables = if self.published_tables.is_empty() {
            std::ptr::null()
        } else {
            self.published_tables.as_ptr()
        };
        view
    }

    /// Formats a human-readable ASCII tree + layout rect dump of `store` for debugging and snapshot tests.
    pub fn dump_tree(&self, store: &NodeStore) -> String {
        let mut out = String::new();
        self.dump_rec(store, NodeId::ROOT, 0, &mut out);
        out
    }

    fn dump_rec(&self, store: &NodeStore, id: NodeId, depth: usize, out: &mut String) {
        if depth > MAX_TREE_DEPTH || !store.is_live(id) {
            return;
        }
        for _ in 0..depth {
            out.push_str("  ");
        }
        let rect = self.layout.layout_of(id).unwrap_or_default();
        let slot = store.primary_slot(id);
        let _ = write!(
            out,
            "#{} [slot={}] ({:.1}, {:.1}, {:.1}x{:.1})",
            id.raw(),
            slot.0,
            rect.x,
            rect.y,
            rect.width,
            rect.height
        );
        if let Some(t) = store.try_row::<TextContent>(id) {
            let s = t.text();
            if !s.is_empty() {
                let _ = write!(out, " {:?}", s);
            }
        }
        out.push('\n');
        for &cid in store.children(id) {
            self.dump_rec(store, cid, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests;
