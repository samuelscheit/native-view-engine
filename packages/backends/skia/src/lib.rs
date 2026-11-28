//! Cross-platform Skia [`StoreBackend`] targeting macOS, Windows, Linux, iOS, and Android.
//!
//! Consumes `(&NodeStore, &ChangeSet)` directly with zero copies, computes layout via any
//! [`StoreLayoutEngine`] (defaulting to [`TaffyStoreLayout`]), maintains a local-space retained
//! [`CompositorScene`] with flat `Copy` property trees, rasterizes via a pure-Rust software
//! rasterizer (plus optional GPU `skia` / `skia-metal` / `skia-vulkan` / `skia-d3d`), and
//! dispatches optional capability modules (`nve-svg-skia`, `nve-canvas-skia`, 3P plugins) through
//! [`SkiaTableRegistry`] with zero compile-time module dependencies.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod display_list;
mod focus;
pub mod font;
pub mod frame_hook;
pub mod gpu;
mod hit_test;
pub mod image_cache;
mod interaction;
pub mod raster;
pub mod retained;
mod scroll;
pub mod slab_lru;
pub mod table_system;
pub mod text;
pub mod text_bidi;
mod text_cache;
pub mod text_grapheme;
mod text_input;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_bidi_tests;
#[cfg(test)]
mod text_tests;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use nve_abi::{ColorRgba, EventBatchBuf, NodeId, SurfaceId};
use nve_backend_headless::HeadlessBackend;
use nve_engine::{BackendApplyStats, BackendError, LayoutRect, StoreBackend, Viewport};
use nve_layout::{StoreLayoutEngine, TaffyStoreLayout, TextMeasurer};
use nve_store::{ChangeSet, NodeStore, Rgba};
use nve_tables::tables::ImageFit;

pub use nve_engine::InteractionTracker;

static NEXT_PLUGIN_EPOCH: AtomicU64 = AtomicU64::new(1);

#[inline]
pub(crate) fn next_plugin_epoch() -> u64 {
    NEXT_PLUGIN_EPOCH.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
use display_list::emit_node_into;

pub use focus::TextFocusState;
pub use font::{bitmap_font_metrics, BitmapFontMetrics, BitmapFontTextMeasurer};
pub use frame_hook::{NodeTargets, SkiaFrameHook, SkiaFrameHooks, TargetsOf};
pub use gpu::d3d12::D3D12SwapchainConfig;
#[cfg(all(feature = "skia-d3d", target_os = "windows"))]
pub use gpu::d3d12::D3D12Swapchain;
pub use gpu::metal::MetalSwapchainConfig;
#[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
pub use gpu::metal::MetalSwapchain;
pub use gpu::vulkan::VulkanSwapchainConfig;
#[cfg(feature = "skia-vulkan")]
pub use gpu::vulkan::VulkanSwapchain;
pub use gpu::{
    FrameLifecycleState, FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceConfig,
    GpuSurfaceError, GpuSwapchain, SoftwareFramePresenter, SwapchainExtent,
};
#[cfg(feature = "skia")]
pub use gpu::{
    CpuSkiaSurface, GpuSurface, GpuSurfaceBackend, RendererCacheStats, SkiaCanvasRenderer,
};
#[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
pub use gpu::{GaneshPresenter, NativeSwapchain};
pub use image_cache::{
    compute_image_placement, decode_image_uri, decode_png, encode_rgb_png, DecodedImage,
    ImageCache, ImageDecodeError, ImageDrawPlacement, ImageSlot, ImageSlotHandle,
};
pub use nve_tables::interaction::{
    CursorKind, FocusModality, InteractionState, StateStyleRule,
};
pub use nve_tables::layout_ext::{
    compute_sticky_offset, GridItemPlacement, GridLayout, GridLine, GridSingleTrack, GridTrack,
    GridTrackPlacement, GridTrackUnit, LayoutDirection, LayoutExt, LogicalInlineEdges,
    PositionInsets, PositionMode,
};
pub use nve_tables::paint::{
    lerp_color_premul, sample_gradient_stops, BackgroundFill, GradientStop, LinearGradient,
    RadialGradient,
};
pub use nve_tables::snapshots::{LayoutSnapshot, VisualSnapshot};
pub use nve_tables::tables::{BoxShadow, Display};
pub use nve_tables::transform::Transform2D;
pub use nve_tables::{RowPatch, StylePatchExt};
pub use raster::{
    draw_hud_banner, rasterize_display_list, rasterize_display_list_bitmap,
    rasterize_display_list_with_plugins, rasterize_display_list_with_text_engine, PixelBuffer,
};
pub use retained::{CompositorScene, SceneDirtyKind, SceneSyncStats};
pub use table_system::{SkiaSystemPaintCtx, SkiaTableRegistry, SkiaTableSystem};
pub use text::{parse_font_weight, CosmicTextEngine, TextAlign, DEFAULT_FONT_WEIGHT};
pub use text_bidi::{ShapedLineLayout, VisualGraphemeCluster};
pub use text_grapheme::{
    clamp_grapheme_boundary, grapheme_boundaries, grapheme_count, is_grapheme_boundary,
    next_grapheme_boundary, normalize_line_endings, paragraph_bounds_at, prev_grapheme_boundary,
    word_bounds_at,
};

pub(crate) const MAX_HIT_TEST_DEPTH: usize = 256;

/// Accent stroke color painted around the focused `TextInput`.
pub const FOCUS_RING_COLOR: ColorRgba = ColorRgba::rgb(56, 189, 248);
/// Translucent fill painted behind selected text ranges in a focused `TextInput`.
pub const FOCUS_SELECTION_COLOR: ColorRgba = ColorRgba::rgba(56, 189, 248, 88);
/// Accent underline painted beneath active IME composition (`Preedit`) text.
pub const IME_PREEDIT_UNDERLINE_COLOR: ColorRgba = ColorRgba::rgb(56, 189, 248);
/// Subtle translucent fill painted for `Scroll` overlay scrollbar thumbs.
pub const SCROLLBAR_THUMB_COLOR: ColorRgba = ColorRgba::rgba(148, 163, 184, 150);

#[derive(Debug, Clone, PartialEq)]
pub enum PaintOp {
    SaveOpacity(f32),
    Restore,
    PushClip {
        rect: LayoutRect,
        radius: f32,
    },
    PopClip,
    PushTransform {
        matrix: [f32; 6],
    },
    PopTransform,
    DrawShadow {
        rect: LayoutRect,
        radius: f32,
        offset_x: f32,
        offset_y: f32,
        blur_radius: f32,
        spread_radius: f32,
        color: ColorRgba,
        border_width: f32,
        inset: bool,
    },
    FillRoundedRect {
        rect: LayoutRect,
        radius: f32,
        color: ColorRgba,
    },
    FillRoundedRectGradient {
        rect: LayoutRect,
        radius: f32,
        gradient: BackgroundFill,
    },
    StrokeRoundedRect {
        rect: LayoutRect,
        radius: f32,
        width: f32,
        color: ColorRgba,
    },
    DrawText {
        rect: LayoutRect,
        text: String,
        font_size: f32,
        font_weight: u16,
        text_align: TextAlign,
        color: ColorRgba,
    },
    DrawImage {
        rect: LayoutRect,
        radius: f32,
        src: String,
        fit: ImageFit,
        /// Optional pre-resolved `(slot, decoded_id)` handle from `ImageCache`
        /// enabling lock-free, zero-hashing warm Skia image lookups without pinning `Arc<DecodedImage>`.
        resolved: Option<ImageSlotHandle>,
    },
    /// Delegated capability plugin paint op (`nve-svg-skia`, `nve-canvas-skia`, 3P crates).
    Plugin {
        node_id: NodeId,
        rect: LayoutRect,
        inherited_text_color: Rgba,
        /// Process-wide monotonic mutation epoch bumped when plugin table rows, commands,
        /// referenced resources, registered systems, or the backend instance change.
        plugin_epoch: u64,
    },
}

#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    pub ops: Vec<PaintOp>,
}

pub struct SkiaBackend<L: StoreLayoutEngine = TaffyStoreLayout> {
    pub(crate) layout_tree: HeadlessBackend<L>,
    pub(crate) display_list: DisplayList,
    pub(crate) scene: CompositorScene,
    pub(crate) viewport: Viewport,
    pub(crate) viewport_dirty: bool,
    pub(crate) text_engine: Arc<CosmicTextEngine>,
    pub(crate) focus: TextFocusState,
    pub(crate) frame_hooks: SkiaFrameHooks,
    pub(crate) table_registry: SkiaTableRegistry,
    pub(crate) table_events: EventBatchBuf,
    pub(crate) plugin_epochs: Vec<u64>,
    pub(crate) base_plugin_epoch: u64,
    pub(crate) plugin_base_epoch_dirty: bool,
    #[cfg(test)]
    pub(crate) sort_scratch: Vec<NodeId>,
    pub(crate) motion_scratch: Vec<NodeId>,
    pub(crate) motion_paint_scratch: Vec<NodeId>,
    pub(crate) plugin_dirty_scratch: Vec<NodeId>,
}

impl Default for SkiaBackend<TaffyStoreLayout> {
    fn default() -> Self {
        Self::new()
    }
}

impl SkiaBackend<TaffyStoreLayout> {
    pub fn new() -> Self {
        Self::with_layout(TaffyStoreLayout::new())
    }
}

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    pub fn with_layout(layout: L) -> Self {
        Self::with_layout_and_text_engine(layout, CosmicTextEngine::shared())
    }

    pub fn with_layout_and_text_engine(layout: L, text_engine: Arc<CosmicTextEngine>) -> Self {
        let mut layout_tree = HeadlessBackend::with_layout(layout).with_mutation_tracking(false);
        layout_tree.set_text_measurer(Arc::clone(&text_engine) as Arc<dyn TextMeasurer>);
        Self {
            layout_tree,
            display_list: DisplayList::default(),
            scene: CompositorScene::new(),
            viewport: Viewport::default(),
            viewport_dirty: true,
            text_engine,
            focus: TextFocusState::default(),
            frame_hooks: SkiaFrameHooks::default(),
            table_registry: SkiaTableRegistry::new(),
            table_events: EventBatchBuf::new(),
            plugin_epochs: Vec::new(),
            base_plugin_epoch: next_plugin_epoch(),
            plugin_base_epoch_dirty: false,
            #[cfg(test)]
            sort_scratch: Vec::new(),
            motion_scratch: Vec::new(),
            motion_paint_scratch: Vec::new(),
            plugin_dirty_scratch: Vec::new(),
        }
    }

    #[inline]
    pub fn register_table_system<S: SkiaTableSystem>(&mut self, system: S) -> &mut Self {
        self.table_registry.register(system);
        self.base_plugin_epoch = next_plugin_epoch();
        self.plugin_base_epoch_dirty = true;
        self
    }

    #[inline]
    pub fn table_registry(&self) -> &SkiaTableRegistry {
        &self.table_registry
    }

    #[inline]
    pub fn table_registry_mut(&mut self) -> &mut SkiaTableRegistry {
        self.base_plugin_epoch = next_plugin_epoch();
        self.plugin_base_epoch_dirty = true;
        &mut self.table_registry
    }

    #[inline]
    pub fn table_events(&self) -> &EventBatchBuf {
        &self.table_events
    }

    #[inline]
    pub fn table_events_mut(&mut self) -> &mut EventBatchBuf {
        &mut self.table_events
    }

    pub fn text_engine(&self) -> &Arc<CosmicTextEngine> {
        &self.text_engine
    }

    pub fn layout_engine(&self) -> &L {
        self.layout_tree.layout_engine()
    }

    pub fn layout_engine_mut(&mut self) -> &mut L {
        self.layout_tree.layout_engine_mut()
    }

    pub fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.layout_tree.set_text_measurer(measurer);
    }

    pub fn swap_layout_engine<NewL: StoreLayoutEngine>(self, next: NewL) -> SkiaBackend<NewL> {
        let layout_tree = self.layout_tree.swap_layout_engine(next);
        SkiaBackend {
            layout_tree,
            display_list: DisplayList::default(),
            scene: CompositorScene::new(),
            viewport: self.viewport,
            viewport_dirty: true,
            text_engine: self.text_engine,
            focus: self.focus,
            frame_hooks: self.frame_hooks,
            table_registry: self.table_registry,
            table_events: self.table_events,
            plugin_epochs: self.plugin_epochs,
            base_plugin_epoch: self.base_plugin_epoch,
            plugin_base_epoch_dirty: self.plugin_base_epoch_dirty,
            #[cfg(test)]
            sort_scratch: self.sort_scratch,
            motion_scratch: self.motion_scratch,
            motion_paint_scratch: self.motion_paint_scratch,
            plugin_dirty_scratch: self.plugin_dirty_scratch,
        }
    }

    #[inline]
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    #[inline]
    pub fn layout_tree(&self) -> &HeadlessBackend<L> {
        &self.layout_tree
    }

    #[inline]
    pub fn layout_tree_mut(&mut self) -> &mut HeadlessBackend<L> {
        &mut self.layout_tree
    }

    /// Computes the visual (scroll-shifted + sticky-clamped) layout rectangle of `node_id` in
    /// logical viewport coordinates.
    pub fn visual_rect_of(&self, store: &NodeStore, node_id: NodeId) -> Option<LayoutRect> {
        let mut rect = self.layout_tree.layout_of(node_id)?;
        let (own_sx, own_sy) = self.layout_tree.compute_sticky_delta(store, node_id);
        rect.x += own_sx;
        rect.y += own_sy;
        let mut ancestor = store.parent(node_id);
        let mut depth = 0usize;
        while let Some(pid) = ancestor {
            if depth > MAX_HIT_TEST_DEPTH {
                break;
            }
            depth += 1;
            let (sx, sy) = self.visual_scroll_shift(store, pid);
            rect.x -= sx;
            rect.y -= sy;
            let (stx, sty) = self.layout_tree.compute_sticky_delta(store, pid);
            rect.x += stx;
            rect.y += sty;
            ancestor = store.parent(pid);
        }
        Some(rect)
    }

    /// Compiles a fresh ground-truth [`DisplayList`] directly from `store` (test oracle only).
    #[cfg(test)]
    pub(crate) fn rebuild_display_list(&mut self, store: &NodeStore) -> &DisplayList {
        self.display_list.ops.clear();
        if !store.is_live(NodeId::ROOT) {
            return &self.display_list;
        }
        let clip_bounds = LayoutRect {
            x: 0.0,
            y: 0.0,
            width: self.viewport.width.max(1.0),
            height: self.viewport.height.max(1.0),
        };
        emit_node_into(
            store,
            &self.layout_tree,
            &self.focus,
            &self.frame_hooks,
            &self.table_registry,
            &self.plugin_epochs,
            self.base_plugin_epoch,
            self.viewport.scale_factor,
            self.text_engine.as_ref(),
            clip_bounds,
            (0.0, 0.0),
            false,
            false,
            None,
            &mut self.sort_scratch,
            &mut self.display_list.ops,
            NodeId::ROOT,
        );
        &self.display_list
    }

    #[inline]
    pub(crate) fn bump_plugin_epoch(&mut self, id: NodeId) {
        let idx = id.index();
        if idx >= self.plugin_epochs.len() {
            self.plugin_epochs.resize(idx + 1, 0);
        }
        self.plugin_epochs[idx] = next_plugin_epoch();
    }

    #[inline]
    pub fn plugin_epoch_of(&self, id: NodeId) -> u64 {
        self.plugin_epochs
            .get(id.index())
            .copied()
            .unwrap_or(0)
            .max(self.base_plugin_epoch)
    }

    /// Software-rasterizes the retained [`CompositorScene`] (without plugin dispatch) with zero
    /// intermediate `DisplayList` allocation.
    pub fn rasterize(&self, buffer: &mut PixelBuffer, clear_color: ColorRgba) {
        self.scene.rasterize_with_text_engine(
            buffer,
            self.viewport.scale_factor,
            clear_color,
            self.text_engine.as_ref(),
        );
    }

    /// Software-rasterizes the retained [`CompositorScene`] including registered
    /// [`SkiaTableSystem`] plugins with zero intermediate `DisplayList` allocation.
    pub fn rasterize_with_store(
        &mut self,
        store: &NodeStore,
        buffer: &mut PixelBuffer,
        clear_color: ColorRgba,
    ) {
        self.scene.rasterize_with_plugins(
            buffer,
            self.viewport.scale_factor,
            clear_color,
            self.text_engine.as_ref(),
            store,
            &self.table_registry,
            &mut self.table_events,
        );
    }
}

impl<L: StoreLayoutEngine> StoreBackend for SkiaBackend<L> {
    fn name(&self) -> &'static str {
        "skia"
    }

    fn set_viewport(&mut self, surface_id: SurfaceId, viewport: Viewport) {
        if self.viewport != viewport {
            self.viewport = viewport;
            self.viewport_dirty = true;
        }
        self.layout_tree.set_viewport(surface_id, viewport);
    }

    fn commit(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
    ) -> Result<BackendApplyStats, BackendError> {
        let vp_dirty = self.viewport_dirty;
        let needs_layout = vp_dirty || changes.needs_layout();
        let mut stats = self.layout_tree.commit(store, changes)?;
        self.viewport_dirty = false;
        let t_mount = Instant::now();
        self.reconcile_scroll_offsets(store, changes, needs_layout);
        self.reconcile_focus_state(store);
        self.frame_hooks_after_commit(store, changes);

        let mut plugin_dirty = std::mem::take(&mut self.plugin_dirty_scratch);
        plugin_dirty.clear();
        if !self.table_registry.iter().is_empty() {
            for &id in changes.created.iter().chain(changes.removed.iter()) {
                self.bump_plugin_epoch(id);
                plugin_dirty.push(id);
            }
            let slots_len = self.table_registry.iter().len();
            let total_res_changed =
                changes.resources_defined.len() + changes.resources_released.len();
            let changed_res_set = (total_res_changed > 4).then(|| {
                let mut set = std::collections::HashSet::with_capacity_and_hasher(
                    total_res_changed,
                    crate::slab_lru::FastBuildHasher,
                );
                set.extend(changes.resources_defined.iter().copied());
                set.extend(changes.resources_released.iter().copied());
                set
            });
            for i in 0..slots_len {
                let slot = self.table_registry.iter()[i].slot();
                if let Some(tc) = changes.table(slot) {
                    for &id in tc.nodes.iter().chain(tc.detached.iter()) {
                        self.bump_plugin_epoch(id);
                        plugin_dirty.push(id);
                    }
                }
                if total_res_changed > 0 {
                    self.bump_plugin_nodes_referencing_changed_resources(
                        store,
                        slot,
                        changes,
                        changed_res_set.as_ref(),
                        &mut plugin_dirty,
                    );
                }
            }
        }
        for cmd in &changes.commands {
            self.bump_plugin_epoch(cmd.node);
            plugin_dirty.push(cmd.node);
            let _ = self.table_registry.dispatch_command(
                store,
                cmd.node,
                cmd.table,
                cmd.op,
                changes.command_payload(cmd),
                &mut self.table_events,
            );
        }
        let base_dirty = std::mem::replace(&mut self.plugin_base_epoch_dirty, false);
        let chunks_before = self.scene.stats().chunks_emitted;
        let dirty_kind =
            self.sync_scene_for_commit(store, changes, vp_dirty, &plugin_dirty, base_dirty);
        let chunks_delta = self
            .scene
            .stats()
            .chunks_emitted
            .saturating_sub(chunks_before);
        self.plugin_dirty_scratch = plugin_dirty;
        stats.mount_us = t_mount.elapsed().as_micros() as u64;
        stats.mutation_count = match dirty_kind {
            SceneDirtyKind::None => 0,
            SceneDirtyKind::CompositeOnly => changes.touched.len() as u32,
            SceneDirtyKind::PaintOnly | SceneDirtyKind::LayoutOrTree => {
                chunks_delta.max(changes.removed.len() as u64) as u32
            }
        };
        Ok(stats)
    }

    fn drain_events(&mut self, out: &mut EventBatchBuf) {
        self.layout_tree.drain_events(out);
        if !self.table_events.is_empty() {
            out.extend_from_validated(self.table_events.as_validated());
            self.table_events.clear();
        }
    }

    fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        self.layout_tree.layout_of(node_id)
    }

    fn reset(&mut self) {
        self.layout_tree.reset();
        self.display_list.ops.clear();
        self.scene.clear();
        self.viewport_dirty = true;
        self.focus = TextFocusState::default();
        self.frame_hooks.reset();
        self.table_events.clear();
        self.plugin_epochs.clear();
        self.base_plugin_epoch = next_plugin_epoch();
        self.plugin_base_epoch_dirty = false;
    }
}
