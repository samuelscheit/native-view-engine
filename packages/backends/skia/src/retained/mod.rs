//! Local-space retained [`CompositorScene`] and property trees for 120 Hz `COMPOSITE` updates
//! and dirty-only `PAINT` chunk rebuilds.
//!
//! - **120 Hz `COMPOSITE` fast path**: scrolling, `Visual::opacity`, `Transform`, and `z_index`
//!   updates mutate [`PropertyTrees`] / in-place child order in O(dirty) with **zero** paint-op
//!   re-emission and **zero** heap allocations.
//! - **Dirty-only `PAINT` path**: background, border, shadow, image, text, focus, and plugin edits
//!   re-emit only the affected node chunks, with `xxh64`-keyed inherited-typography & visibility
//!   stop-pruning and automatic `display: none` transition detection.
//! - **Structural `LAYOUT` path**: preserves unchanged node-local ops across pure `(x, y)` moves.

mod bridge;
pub mod chunks;
mod emit;
pub(crate) mod emit_helpers;
pub mod property_tree;
pub mod replay;
#[cfg(feature = "skia")]
mod replay_skia;
mod replay_software;
mod sync;

#[cfg(test)]
mod inset_shadow_parity_tests;
#[cfg(test)]
mod inset_shadow_tests;
#[cfg(test)]
mod oracle_edge_more_tests;
#[cfg(test)]
mod oracle_edge_tests;
#[cfg(test)]
mod oracle_lifecycle_tests;
#[cfg(test)]
mod oracle_tests;

use nve_abi::table::effect;
use nve_abi::{ColorRgba, EventBatchBuf, NodeId};
use nve_engine::Viewport;
use nve_layout::StoreLayoutEngine;
use nve_store::{ChangeSet, NodeStore, NveTable};
use nve_tables::tables::{Responsive, Scroll, Typography};

pub use chunks::{ChunkArenas, LocalPaintOp, NodeChunkMeta, ResolvedChunkText};
use emit::SceneSyncEnv;
pub use property_tree::{
    ChunkIdx, ClipNode, EffectNode, PropertyTrees, ScrollPropIdx, ScrollPropertyNode, TransformNode,
};
pub use replay::materialize_scene_display_list;
#[cfg(feature = "skia")]
pub(crate) use replay_skia::paint_scene_onto_skia_canvas;
pub(crate) use replay_software::rasterize_scene_inner;

use crate::frame_hook::SkiaFrameHooks;
use crate::raster::PixelBuffer;
use crate::table_system::SkiaTableRegistry;
use crate::text::CosmicTextEngine;
use crate::DisplayList;

/// Classification of the minimal work required to synchronize [`CompositorScene`] for a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneDirtyKind {
    /// No visible scene changes occurred.
    None,
    /// Only property-tree inputs (`opacity`, `transform`, `scroll`, or `z_index`) changed.
    CompositeOnly,
    /// One or more nodes need local paint-chunk re-emission (no layout/tree changes).
    PaintOnly,
    /// Tree topology, layout, viewport, or `display: none` changed.
    LayoutOrTree,
}

/// Cumulative diagnostic counters recording which synchronization paths [`CompositorScene`] took.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneSyncStats {
    pub last_dirty_kind: Option<SceneDirtyKind>,
    pub composite_fast_paths: u64,
    pub paint_incremental_paths: u64,
    pub layout_tree_syncs: u64,
    pub chunks_emitted: u64,
    pub scroll_fast_paths: u64,
}

/// Local-space retained compositor scene owned by [`crate::SkiaBackend`].
#[derive(Debug, Clone, Default)]
pub struct CompositorScene {
    pub(crate) initialized: bool,
    pub(crate) root: Option<ChunkIdx>,
    pub(crate) viewport_width: f32,
    pub(crate) viewport_height: f32,
    pub(crate) scale_factor: f32,
    /// Direct dense lookup indexed by `NodeId::index()` (`ChunkIdx::NONE` if absent).
    /// All lookups verify `chunks[cidx].is_live() && chunks[cidx].node_id == id`.
    pub(crate) node_to_chunk: Vec<ChunkIdx>,
    pub(crate) chunks: Vec<NodeChunkMeta>,
    pub(crate) free_chunks: Vec<ChunkIdx>,
    pub(crate) props: PropertyTrees,
    pub(crate) arenas: ChunkArenas,
    pub(crate) stats: SceneSyncStats,
    pub(super) walk_scratch: Vec<(NodeId, ChunkIdx, bool, Option<ResolvedChunkText>)>,
    pub(super) dirty_nodes_scratch: Vec<NodeId>,
    pub(super) z_parents_scratch: Vec<NodeId>,
    pub(super) live_epoch_marks: Vec<u64>,
    pub(super) live_epoch: u64,
    pub(super) touched_epoch_marks: Vec<u64>,
    pub(super) touched_epoch: u64,
    pub(super) dirty_queued_marks: Vec<u64>,
    pub(super) dirty_done_marks: Vec<u64>,
    pub(super) dirty_epoch: u64,
    pub(super) z_parent_marks: Vec<u64>,
    pub(super) z_parent_epoch: u64,
}

impl CompositorScene {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    #[inline]
    pub fn stats(&self) -> SceneSyncStats {
        self.stats
    }

    #[inline]
    pub fn reset_stats(&mut self) {
        self.stats = SceneSyncStats::default();
    }

    #[inline]
    pub fn live_op_count(&self) -> usize {
        self.arenas.live_op_count
    }

    #[inline]
    pub fn arena_compactions(&self) -> u64 {
        self.arenas.compactions
    }

    #[inline]
    pub fn ordered_children_len(&self) -> usize {
        self.arenas.ordered_children.len()
    }

    #[inline]
    pub fn viewport_size(&self) -> (f32, f32) {
        (self.viewport_width, self.viewport_height)
    }

    /// Generation-checked lookup returning the live [`ChunkIdx`] for `id`, or `None` if absent,
    /// retired, or occupied by a different generation at the recycled slot index.
    #[inline]
    pub fn live_chunk_idx(&self, id: NodeId) -> Option<ChunkIdx> {
        let &cidx = self.node_to_chunk.get(id.index())?;
        if cidx.is_none() {
            return None;
        }
        let meta = self.chunks.get(cidx.index())?;
        (meta.is_live() && meta.node_id == id).then_some(cidx)
    }

    #[inline]
    pub fn chunk_for_node(&self, id: NodeId) -> Option<&NodeChunkMeta> {
        let cidx = self.live_chunk_idx(id)?;
        self.chunks.get(cidx.index())
    }

    #[inline]
    pub fn transform_for_node(&self, id: NodeId) -> Option<&TransformNode> {
        let cidx = self.live_chunk_idx(id)?;
        self.props.transforms.get(cidx.index())
    }

    #[inline]
    pub fn effect_for_node(&self, id: NodeId) -> Option<&EffectNode> {
        let cidx = self.live_chunk_idx(id)?;
        self.props.effects.get(cidx.index())
    }

    /// O(1) cached `(viewport_w, viewport_h, content_w, content_h)` for a live `Scroll` container.
    #[inline]
    pub(crate) fn cached_scroll_bounds(&self, id: NodeId) -> Option<(f32, f32, f32, f32)> {
        if !self.initialized {
            return None;
        }
        let cidx = self.live_chunk_idx(id)?;
        let tf = self.props.transforms.get(cidx.index())?;
        if tf.scroll_idx.is_none() {
            return None;
        }
        let sc = self.props.scrolls.get(tf.scroll_idx.index())?;
        Some((tf.width, tf.height, sc.content_w, sc.content_h))
    }

    pub fn clear(&mut self) {
        self.initialized = false;
        self.root = None;
        self.node_to_chunk.clear();
        self.chunks.clear();
        self.free_chunks.clear();
        self.props.clear();
        self.arenas.clear();
        self.walk_scratch.clear();
        self.dirty_nodes_scratch.clear();
        self.z_parents_scratch.clear();
        self.live_epoch_marks.clear();
        self.live_epoch = 0;
        self.touched_epoch_marks.clear();
        self.touched_epoch = 0;
        self.dirty_queued_marks.clear();
        self.dirty_done_marks.clear();
        self.dirty_epoch = 0;
        self.z_parent_marks.clear();
        self.z_parent_epoch = 0;
    }

    /// Flattens this retained scene into `out` at `scale_factor`.
    pub fn materialize_display_list(&self, scale_factor: f32, out: &mut DisplayList) {
        materialize_scene_display_list(self, scale_factor, out);
    }

    /// Software-rasterizes this scene directly into `buffer` using `text_engine`.
    pub fn rasterize_with_text_engine(
        &self,
        buffer: &mut PixelBuffer,
        scale_factor: f32,
        clear_color: ColorRgba,
        text_engine: &CosmicTextEngine,
    ) {
        rasterize_scene_inner(
            self,
            buffer,
            scale_factor,
            clear_color,
            Some(text_engine),
            None,
            None,
        );
    }

    /// Software-rasterizes this scene including registered [`SkiaTableRegistry`] plugins.
    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_with_plugins(
        &self,
        buffer: &mut PixelBuffer,
        scale_factor: f32,
        clear_color: ColorRgba,
        text_engine: &CosmicTextEngine,
        store: &NodeStore,
        registry: &SkiaTableRegistry,
        events: &mut EventBatchBuf,
    ) {
        rasterize_scene_inner(
            self,
            buffer,
            scale_factor,
            clear_color,
            Some(text_engine),
            Some((store, registry, events)),
            None,
        );
    }

    /// Software-rasterizes this scene with registered plugins reusing a caller-owned RGBA scratch
    /// buffer (zero allocations on steady-state frames).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rasterize_with_plugins_and_scratch(
        &self,
        buffer: &mut PixelBuffer,
        scale_factor: f32,
        clear_color: ColorRgba,
        text_engine: &CosmicTextEngine,
        store: &NodeStore,
        registry: &SkiaTableRegistry,
        events: &mut EventBatchBuf,
        plugin_scratch: &mut PixelBuffer,
    ) {
        rasterize_scene_inner(
            self,
            buffer,
            scale_factor,
            clear_color,
            Some(text_engine),
            Some((store, registry, events)),
            Some(plugin_scratch),
        );
    }

    /// Software-rasterizes this scene using the deterministic 5x7 bitmap font.
    pub fn rasterize_bitmap(
        &self,
        buffer: &mut PixelBuffer,
        scale_factor: f32,
        clear_color: ColorRgba,
    ) {
        rasterize_scene_inner(self, buffer, scale_factor, clear_color, None, None, None);
    }

    /// Classifies the minimal sync tier needed for `changes`.
    pub(crate) fn classify_commit(
        &self,
        changes: &ChangeSet,
        vp_changed: bool,
        has_plugins: bool,
        frame_hooks: &SkiaFrameHooks,
    ) -> SceneDirtyKind {
        if !self.initialized
            || vp_changed
            || !changes.created.is_empty()
            || !changes.removed.is_empty()
            || !changes.children.is_empty()
            || changes.effects_union() & (effect::LAYOUT | effect::MEASURE | effect::REALIZE) != 0
            || changes.table(Responsive::slot()).is_some()
        {
            return SceneDirtyKind::LayoutOrTree;
        }
        if let Some(tc) = changes.table(Scroll::slot()) {
            let needs_tree_rebuild = !tc.detached.is_empty()
                || tc.nodes.iter().any(|&id| {
                    self.live_chunk_idx(id)
                        .is_none_or(|cidx| self.props.transforms[cidx.index()].scroll_idx.is_none())
                });
            if needs_tree_rebuild {
                return SceneDirtyKind::LayoutOrTree;
            }
        }

        if changes.is_empty() {
            return SceneDirtyKind::None;
        }

        let fx_union = changes.effects_union();
        let has_paint = (fx_union & effect::PAINT) != 0
            || !changes.commands.is_empty()
            || !changes.resources_defined.is_empty()
            || !changes.resources_released.is_empty()
            || frame_hooks.watches_changes(changes)
            || changes.table(Typography::slot()).is_some();

        if has_paint || (has_plugins && !changes.tables().is_empty() && fx_union == 0) {
            return SceneDirtyKind::PaintOnly;
        }

        if (fx_union & effect::COMPOSITE) != 0 {
            return SceneDirtyKind::CompositeOnly;
        }

        SceneDirtyKind::None
    }

    /// Synchronizes this retained scene for a committed frame using the cheapest valid path.
    pub(crate) fn sync_commit<L: StoreLayoutEngine>(
        &mut self,
        env: &SceneSyncEnv<'_, L>,
        changes: &ChangeSet,
        viewport: Viewport,
        viewport_dirty: bool,
        plugin_dirty_nodes: &[NodeId],
        plugin_base_epoch_changed: bool,
    ) -> SceneDirtyKind {
        let vp_changed = viewport_dirty
            || (self.viewport_width - viewport.width).abs() > f32::EPSILON
            || (self.viewport_height - viewport.height).abs() > f32::EPSILON
            || (self.scale_factor - viewport.scale_factor).abs() > f32::EPSILON;

        let mut kind = self.classify_commit(
            changes,
            vp_changed,
            !env.table_registry.iter().is_empty(),
            env.frame_hooks,
        );
        if plugin_base_epoch_changed && kind != SceneDirtyKind::LayoutOrTree {
            kind = SceneDirtyKind::LayoutOrTree;
        } else if !plugin_dirty_nodes.is_empty()
            && matches!(kind, SceneDirtyKind::None | SceneDirtyKind::CompositeOnly)
        {
            kind = SceneDirtyKind::PaintOnly;
        }

        self.viewport_width = viewport.width;
        self.viewport_height = viewport.height;
        self.scale_factor = viewport.scale_factor;
        self.stats.last_dirty_kind = Some(kind);

        match kind {
            SceneDirtyKind::None => {
                if !env.layout_tree.scene().last_reconciled_scroll_nodes().is_empty() {
                    self.sync_reconciled_scrolls(env);
                    self.stats.last_dirty_kind = Some(SceneDirtyKind::CompositeOnly);
                }
            }
            SceneDirtyKind::CompositeOnly => {
                self.stats.composite_fast_paths += 1;
                self.sync_composite_from_changes(env, changes);
                self.sync_reconciled_scrolls(env);
            }
            SceneDirtyKind::PaintOnly => {
                self.sync_paint_from_changes(env, changes, plugin_dirty_nodes);
                if self.stats.last_dirty_kind != Some(SceneDirtyKind::LayoutOrTree) {
                    self.sync_reconciled_scrolls(env);
                }
            }
            SceneDirtyKind::LayoutOrTree => {
                self.stats.layout_tree_syncs += 1;
                self.sync_full_tree(
                    env,
                    changes,
                    plugin_dirty_nodes,
                    vp_changed || plugin_base_epoch_changed,
                    false,
                );
            }
        }
        self.stats.last_dirty_kind.unwrap_or(kind)
    }

    /// O(1) 120 Hz scroll update: mutates the scroll container's sparse scroll property node
    /// (plus any sticky nodes in O(#sticky)) with zero paint-op re-emission and zero allocations.
    pub(crate) fn update_scroll<L: StoreLayoutEngine>(
        &mut self,
        env: &SceneSyncEnv<'_, L>,
        scroll_id: NodeId,
        clamped_x: f32,
        clamped_y: f32,
    ) -> bool {
        if !self.initialized {
            return false;
        }
        let Some(cidx) = self.live_chunk_idx(scroll_id) else {
            return false;
        };
        let scroll_idx = self.props.transforms[cidx.index()].scroll_idx;
        if scroll_idx.is_none() {
            return false;
        }
        let (shift_x, shift_y) = env.layout_tree.scene().visual_scroll_shift(env.store, scroll_id);
        let sc = &mut self.props.scrolls[scroll_idx.index()];
        sc.scroll_raw_x = clamped_x;
        sc.scroll_raw_y = clamped_y;
        sc.scroll_shift_x = shift_x;
        sc.scroll_shift_y = shift_y;

        for i in 0..self.props.sticky_chunks.len() {
            let (sid, scidx) = self.props.sticky_chunks[i];
            if scidx.is_some()
                && let Some(meta) = self.chunks.get(scidx.index())
                && meta.is_live()
                && meta.node_id == sid
            {
                let (stx, sty) = env.layout_tree.compute_sticky_delta(env.store, sid);
                let stf = &mut self.props.transforms[scidx.index()];
                stf.sticky_x = stx;
                stf.sticky_y = sty;
            }
        }
        self.refresh_top_layer_overlays(env);
        self.stats.last_dirty_kind = Some(SceneDirtyKind::CompositeOnly);
        self.stats.scroll_fast_paths += 1;
        true
    }

    /// Updates top-layer overlay positions after scroll or `CompositeOnly` transforms, and re-emits
    /// any overlay chunk whose `AUTO_SIZE` bounds (`width`/`height`) changed.
    pub(super) fn refresh_top_layer_overlays<L: StoreLayoutEngine>(
        &mut self,
        env: &SceneSyncEnv<'_, L>,
    ) {
        if self.props.top_layer_chunks.is_empty() {
            return;
        }
        let mut resized = false;
        for i in 0..self.props.top_layer_chunks.len() {
            let tcidx = self.props.top_layer_chunks[i];
            let Some(node_id) = self
                .chunks
                .get(tcidx.index())
                .filter(|m| m.is_live())
                .map(|m| m.node_id)
            else {
                continue;
            };
            if let Some(ro) = env.layout_tree.scene().resolved_overlay(node_id)
                && let Some(base) = env.layout_tree.layout_of(node_id)
            {
                let tf = &mut self.props.transforms[tcidx.index()];
                tf.sticky_x = ro.origin_x - base.x;
                tf.sticky_y = ro.origin_y - base.y;
                if tf.width.to_bits() != ro.width.to_bits()
                    || tf.height.to_bits() != ro.height.to_bits()
                {
                    tf.width = ro.width;
                    tf.height = ro.height;
                    if !resized {
                        self.begin_dirty_queue();
                        resized = true;
                    }
                    self.enqueue_dirty(node_id);
                }
            }
        }
        if resized {
            self.reemit_dirty_nodes_with_inheritance(env);
        }
    }

    /// O(dirty) 120 Hz property update for opacity/transform animations (zero op emission, 0 alloc).
    pub(crate) fn update_composite_nodes<L: StoreLayoutEngine>(
        &mut self,
        env: &SceneSyncEnv<'_, L>,
        nodes: &[NodeId],
    ) {
        if !self.initialized || nodes.is_empty() {
            return;
        }
        for &id in nodes {
            let Some(cidx) = self.live_chunk_idx(id) else {
                continue;
            };
            let mut style = env.layout_tree.effective_visual(env.store, id);
            env.frame_hooks.apply_visual(id, &mut style);
            self.props.effects[cidx.index()].opacity = style.opacity;

            let eff_tf = env.frame_hooks.resolve_transform(
                id,
                env.layout_tree.effective_transform(env.store, id),
            );
            self.props.transforms[cidx.index()].set_transform(eff_tf);
        }
        self.refresh_top_layer_overlays(env);
        self.stats.last_dirty_kind = Some(SceneDirtyKind::CompositeOnly);
        self.stats.composite_fast_paths += 1;
    }

    /// O(dirty) incremental paint update (focus, caret, IME, interaction hover/active, color anims).
    pub(crate) fn update_dirty_paint_nodes<L: StoreLayoutEngine>(
        &mut self,
        env: &SceneSyncEnv<'_, L>,
        nodes: &[NodeId],
    ) {
        if !self.initialized || nodes.is_empty() {
            return;
        }
        self.begin_dirty_queue();
        for &id in nodes {
            if env.store.is_live(id) {
                self.enqueue_dirty(id);
            }
        }
        if self.has_display_none_transition(env) {
            self.stats.last_dirty_kind = Some(SceneDirtyKind::LayoutOrTree);
            self.stats.layout_tree_syncs += 1;
            self.sync_full_tree(env, &ChangeSet::default(), &[], false, true);
            return;
        }
        self.stats.last_dirty_kind = Some(SceneDirtyKind::PaintOnly);
        self.stats.paint_incremental_paths += 1;
        self.reemit_dirty_nodes_with_inheritance(env);
    }

    /// Zero-allocation fast path for plugin-only repaints (e.g. 120 Hz direct `Canvas::generation`
    /// invalidations): patches `NodeChunkMeta::plugin_epoch` and its trailing `LocalPaintOp::Plugin`
    /// in-place instead of retiring and re-appending arena ops every frame.
    pub(super) fn try_update_plugin_epochs_in_place<L: StoreLayoutEngine>(
        &mut self,
        env: &SceneSyncEnv<'_, L>,
        changes: &ChangeSet,
    ) -> bool {
        if !changes.resources_defined.is_empty()
            || !changes.resources_released.is_empty()
            || !changes.commands.is_empty()
            || env.frame_hooks.watches_changes(changes)
            || !changes
                .tables()
                .iter()
                .all(|tc| tc.detached.is_empty() && env.table_registry.get(tc.slot).is_some())
        {
            return false;
        }
        for &id in &self.dirty_nodes_scratch {
            let Some(cidx) = self.live_chunk_idx(id) else {
                return false;
            };
            let meta = self.chunks[cidx.index()];
            let Some(last) = meta
                .op_len
                .checked_sub(1)
                .and_then(|d| self.arenas.ops.get(meta.op_start as usize + d as usize))
            else {
                return false;
            };
            if !matches!(*last, LocalPaintOp::Plugin { node_id, .. } if node_id == id) {
                return false;
            }
        }
        for i in 0..self.dirty_nodes_scratch.len() {
            let id = self.dirty_nodes_scratch[i];
            let Some(cidx) = self.live_chunk_idx(id) else {
                continue;
            };
            let epoch = env.plugin_epoch_of(id);
            let meta = &mut self.chunks[cidx.index()];
            meta.plugin_epoch = epoch;
            let op_idx = meta.op_start as usize + meta.op_len as usize - 1;
            if let LocalPaintOp::Plugin { plugin_epoch, .. } = &mut self.arenas.ops[op_idx] {
                *plugin_epoch = epoch;
            }
            self.stats.chunks_emitted += 1;
        }
        true
    }
}
