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
mod hit_test;
mod interaction;
pub mod reloc_scene;
mod snapshot;
pub mod taffy_store;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use nve_abi::table::NveTableView;
use nve_abi::{
    ColorRgba, FrameId, NativeEvent, NodeId, NveRenderNode, NveSceneTable, NveSceneView,
    SceneNodeInput, SurfaceId, NVE_NODE_CLIP_BOUNDS, NVE_NODE_INTERACTIVE, NVE_NODE_LEAF_TEXT,
};
use nve_components::tables::{Interaction, TextContent, Typography};
use nve_engine::{
    HeuristicTextMeasurer, LayoutRect, StoreLayoutEngine, TextMeasurer, Viewport,
};
use nve_store::{ChangeSet, NodeId as StoreNodeId, NodeStore, NveTable, TableSlot};

pub use nve_components::interaction::{
    CursorKind, FocusModality, InteractionState, InteractionTracker, StateStyleRule,
};
pub use nve_components::layout_ext::{
    GridItemPlacement, GridLayout, GridLine, GridSingleTrack, GridTrack, GridTrackPlacement,
    GridTrackUnit, LayoutDirection, LayoutExt, LogicalInlineEdges, PositionInsets, PositionMode,
};
pub use nve_components::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};
pub use nve_components::style_patch::{Display, LayoutSnapshot, StylePatch, VisualSnapshot};
pub use reloc_scene::{prerender_native_frame0, prerender_native_frame0_with_text_measurer};
pub(crate) use snapshot::{
    box_visual, is_display_contents, is_display_none, length_to_dim, node_clips_overflow,
    without_box_paint,
};
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
    pub(crate) layout: L,
    text_measurer: Arc<dyn TextMeasurer>,
    pub(crate) prev_rects: Vec<(u64, LayoutRect)>,
    pub(crate) rect_gen: u64,
    pub(crate) track_mutations: bool,
    pub(crate) interaction: InteractionTracker,
    pub(crate) scroll_offsets: HashMap<NodeId, (f32, f32)>,
    pub(crate) last_mutations: Vec<HeadlessMutation>,
    pub(crate) pending_events: Vec<NativeEvent>,
    pub(crate) scene_table: NveSceneTable,
    pub(crate) published_tables: Vec<NveTableView>,
    published_dirty_buf: Vec<u32>,
    published_children_scratch: Vec<NodeId>,
    text_dirty_marks: Vec<u64>,
    text_dirty_epoch: u64,
    pub(crate) walk_stack: Vec<(StoreNodeId, usize)>,
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
            surface_id: 1,
            viewport: Viewport::default(),
            layout,
            text_measurer,
            prev_rects: Vec::new(),
            rect_gen: 0,
            track_mutations: true,
            interaction: InteractionTracker::new(),
            scroll_offsets: HashMap::new(),
            last_mutations: Vec::new(),
            pending_events: Vec::new(),
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

    /// Recomputes layout for `store` immediately (e.g. after swapping layout engines or viewport);
    /// returns whether the engine ran a pass (it self-gates clean frames).
    pub fn recompute_layout(&mut self, store: &NodeStore) -> bool {
        self.layout
            .compute_layout(store, StoreNodeId::ROOT, self.viewport)
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
            id: StoreNodeId,
            depth: usize,
            out: &mut String,
        ) {
            for _ in 0..depth {
                out.push_str("  ");
            }
            let r = backend.layout_of(id.raw()).unwrap_or_default();
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
        walk(self, store, StoreNodeId::ROOT, 0, &mut out);
        out
    }

    #[inline]
    pub fn push_event(&mut self, event: NativeEvent) {
        self.pending_events.push(event);
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
        self.layout.layout_of(StoreNodeId(id))
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
            self.scene_table.remove_node(removed.raw());
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
        if store.is_live(StoreNodeId::ROOT) {
            self.walk_stack.push((StoreNodeId::ROOT, 0));
        }
        while let Some((id, depth)) = self.walk_stack.pop() {
            if !store.is_live(id) {
                continue;
            }
            let rect = self.layout.layout_of(id).unwrap_or_default();
            let (bg, border_color, border_width, border_radius, opacity, z_index) =
                if let Some(v) = box_visual(store, id) {
                    (
                        v.background().0,
                        v.border_color().0,
                        v.border_width(),
                        v.border_radius(),
                        v.opacity(),
                        v.z_index(),
                    )
                } else {
                    (0, 0, 0.0, 0.0, 1.0, 0)
                };
            let clip = node_clips_overflow(store, id);
            let event_mask = store
                .try_row::<Interaction>(id)
                .map_or(0, |i| i.event_mask());
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
            if event_mask != 0 {
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
                .extend(children.iter().take(u16::MAX as usize).map(|c| c.raw()));

            let text_dirty = has_text_changes
                && self.text_dirty_marks.get(id.index()).copied() == Some(self.text_dirty_epoch);
            let res = self.scene_table.upsert_node_external_text(
                SceneNodeInput {
                    node_id: id.raw(),
                    primary_slot: store.primary_slot(id).0,
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
                    z_index,
                    text: "",
                    children: &self.published_children_scratch,
                },
                text_offset,
                text_len,
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
        self.dump_rec(store, StoreNodeId::ROOT, 0, &mut out);
        out
    }

    fn dump_rec(&self, store: &NodeStore, id: StoreNodeId, depth: usize, out: &mut String) {
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
