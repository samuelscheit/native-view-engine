//! Cross-platform Skia RenderBackend targeting macOS, Windows, Linux, iOS, and Android.
//!
//! Consumes the exact same `nve-abi` binary `CommandBuffer` as the React Native Fabric backend,
//! computes Flexbox layout, compiles a culled retained `DisplayList`, rasterizes cleanly via a
//! zero-dependency software rasterizer, and (with `--features skia` / `skia-metal` / `skia-vulkan` /
//! `skia-d3d`) paints directly onto a `skia_safe::Canvas` or hardware GPU swapchain.
//!
//! For the runnable desktop `winit` + `softbuffer` / GPU window host, see `hosts/skia-shell`
//! (`nve-skia-shell`).

mod display_list;
mod focus;
pub mod font;
pub mod gpu;
mod hit_test;
pub mod image_cache;
mod interaction;
mod motion;
pub mod raster;
mod reloc_scene;
mod scroll;
pub mod text;
pub mod text_bidi;
mod text_cache;
pub mod text_grapheme;
mod text_input;

#[cfg(test)]
mod media_tests;
#[cfg(test)]
mod motion_tests;
#[cfg(test)]
mod polish_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_bidi_tests;
#[cfg(test)]
mod text_input_tests;
#[cfg(test)]
mod text_tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use nve_abi::{
    ColorRgba, ComponentKind, FrameId, LayoutProps, NativeEvent, NodeId, RawPropValueRef,
    StyleProps, SurfaceId, ROOT_NODE_ID,
};
use nve_backend_headless::{HeadlessBackend, TaffyLayoutEngine};
use nve_components::canvas_ir::{DrawList, ImageResizeMode};
use nve_components::native_view::{NativeCommand, TypedNativeViewCommand, TypedNativeViewProps};
use nve_engine::{
    replay_command_buffer_into, BackendApplyStats, BackendError, LayoutEngine, LayoutRect,
    RenderBackend, RenderSink, TextMeasurer, Viewport,
};

use display_list::{emit_node_into, InheritedTextStyle};

pub use focus::TextFocusState;
pub use font::{bitmap_font_metrics, BitmapFontMetrics, BitmapFontTextMeasurer};
pub use gpu::d3d12::D3D12SwapchainConfig;
#[cfg(all(feature = "skia-d3d", target_os = "windows"))]
pub use gpu::d3d12::D3D12Swapchain;
pub use gpu::metal::MetalSwapchainConfig;
#[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
pub use gpu::metal::MetalSwapchain;
pub use gpu::vulkan::{VulkanSemaphoreSyncHooks, VulkanSwapchainConfig};
#[cfg(feature = "skia-vulkan")]
pub use gpu::vulkan::VulkanSwapchain;
pub use gpu::{
    FrameLifecycleState, FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceConfig,
    GpuSurfaceError, GpuSwapchain, SoftwareFramePresenter, SwapchainExtent,
};
#[cfg(feature = "skia")]
pub use gpu::{CpuSkiaSurface, GpuSurface, GpuSurfaceBackend, SkiaCanvasRenderer};
pub use image_cache::{
    compute_image_placement, decode_image_uri, decode_png, encode_rgb_png, DecodedImage,
    ImageCache, ImageDecodeError, ImageDrawPlacement,
};
pub use motion::MotionDriver;
pub use nve_components::interaction::{
    CursorKind, FocusModality, InteractionState, InteractionTracker, StateStyleRule,
};
pub use nve_components::layout_ext::{
    compute_sticky_offset, GridItemPlacement, GridLayout, GridLine, GridSingleTrack, GridTrack,
    GridTrackPlacement, GridTrackUnit, LayoutDirection, LayoutExt, LogicalInlineEdges,
    PositionInsets, PositionMode,
};
pub use nve_components::motion::{EasingCurve, MotionPreference, TransitionProperty, TransitionSpec};
pub use nve_components::paint::{
    lerp_color_premul, sample_gradient_stops, write_css_gradient, BackgroundFill, GradientStop,
    LinearGradient, RadialGradient,
};
pub use nve_components::style_patch::{Display, StylePatch};
pub use nve_components::svg::{
    compute_svg_content_hash, parse_svg_path_data, FillRule, SvgPicture, VectorPathCmd, VectorShape,
};
pub use nve_components::transform::Transform2D;
#[cfg(feature = "yoga")]
pub use nve_layout_yoga::{YogaLayoutEngine, YogaUnavailableError};
pub use raster::{
    draw_hud_banner, draw_svg_to_buffer, parse_box_shadow, rasterize_display_list,
    rasterize_display_list_bitmap, rasterize_display_list_with_text_engine, rasterize_svg_picture,
    BoxShadow, CachedSvgBitmap, PixelBuffer, SvgCache, SvgCacheKey, SvgCacheStats,
    DEFAULT_SVG_CACHE_MAX_BYTES, DEFAULT_SVG_CACHE_MAX_ENTRIES,
};
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
/// Subtle translucent fill painted for `ScrollView` overlay scrollbar thumbs.
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
        resize_mode: ImageResizeMode,
    },
    DrawSvg {
        rect: LayoutRect,
        picture: SvgPicture,
        tint: Option<ColorRgba>,
    },
    CustomSkiaCanvas {
        node_id: NodeId,
        rect: LayoutRect,
        ops: DrawList,
    },
}

#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    pub ops: Vec<PaintOp>,
}

pub struct SkiaBackend<L: LayoutEngine = TaffyLayoutEngine> {
    pub(crate) layout_tree: HeadlessBackend<L>,
    pub(crate) display_list: DisplayList,
    pub(crate) viewport: Viewport,
    pub(crate) text_engine: Arc<CosmicTextEngine>,
    pub(crate) scroll_offsets: HashMap<NodeId, (f32, f32)>,
    pub(crate) last_virtual_scroll_y: HashMap<NodeId, f32>,
    pub(crate) focus: TextFocusState,
    pub(crate) motion: MotionDriver,
    pub(crate) newly_tracked_transitions: Vec<NodeId>,
    pub(crate) sort_scratch: Vec<NodeId>,
}

impl Default for SkiaBackend<TaffyLayoutEngine> {
    fn default() -> Self {
        Self::new()
    }
}

impl SkiaBackend<TaffyLayoutEngine> {
    pub fn new() -> Self {
        Self::with_layout(TaffyLayoutEngine::new())
    }
}

#[cfg(feature = "yoga")]
impl SkiaBackend<YogaLayoutEngine> {
    /// Creates a [`SkiaBackend`] backed by real React Native Yoga C++20 layout.
    pub fn new_yoga() -> Result<Self, YogaUnavailableError> {
        Ok(Self::with_layout(YogaLayoutEngine::new()?))
    }
}

impl<L: LayoutEngine> SkiaBackend<L> {
    pub fn with_layout(layout: L) -> Self {
        Self::with_layout_and_text_engine(layout, CosmicTextEngine::shared())
    }

    pub fn with_layout_and_text_engine(layout: L, text_engine: Arc<CosmicTextEngine>) -> Self {
        let mut layout_tree = HeadlessBackend::with_layout(layout).with_mutation_tracking(false);
        layout_tree.set_text_measurer(Arc::clone(&text_engine) as Arc<dyn TextMeasurer>);
        Self {
            layout_tree,
            display_list: DisplayList::default(),
            viewport: Viewport::default(),
            text_engine,
            scroll_offsets: HashMap::new(),
            last_virtual_scroll_y: HashMap::new(),
            focus: TextFocusState::default(),
            motion: MotionDriver::new(),
            newly_tracked_transitions: Vec::new(),
            sort_scratch: Vec::new(),
        }
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

    pub fn swap_layout_engine<NewL: LayoutEngine>(self, next: NewL) -> SkiaBackend<NewL> {
        let layout_tree = self.layout_tree.swap_layout_engine(next);
        let mut swapped = SkiaBackend {
            layout_tree,
            display_list: DisplayList::default(),
            viewport: self.viewport,
            text_engine: self.text_engine,
            scroll_offsets: self.scroll_offsets,
            last_virtual_scroll_y: self.last_virtual_scroll_y,
            focus: self.focus,
            motion: self.motion,
            newly_tracked_transitions: self.newly_tracked_transitions,
            sort_scratch: self.sort_scratch,
        };
        swapped.reconcile_scroll_offsets();
        swapped.reconcile_focus_state();
        swapped.reconcile_motion_after_frame();
        swapped.rebuild_display_list();
        swapped
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn display_list(&self) -> &DisplayList {
        &self.display_list
    }

    pub fn layout_tree(&self) -> &HeadlessBackend<L> {
        &self.layout_tree
    }

    /// Attaches or clears a zero-serialization [`DrawList`] (`Arc<[CanvasOp]>`) directly on
    /// `node_id` and rebuilds the display list if changed (zero base64, zero `NVE_DRAW1` decode).
    pub fn set_node_draw_list(&mut self, node_id: NodeId, draw_list: Option<DrawList>) {
        if self.layout_tree.set_node_draw_list(node_id, draw_list) {
            self.rebuild_display_list();
        }
    }

    /// Directly attaches or clears a zero-serialization [`BackgroundFill`] on `node_id` and
    /// rebuilds the display list if changed.
    pub fn set_node_background_fill(&mut self, node_id: NodeId, fill: Option<BackgroundFill>) {
        if self.layout_tree.set_node_background_fill(node_id, fill) {
            self.rebuild_display_list();
        }
    }

    /// Directly attaches or clears a zero-serialization [`SvgPicture`] on `node_id` and
    /// rebuilds the display list if changed.
    pub fn set_node_svg(&mut self, node_id: NodeId, svg: Option<SvgPicture>) {
        if self.layout_tree.set_node_svg(node_id, svg) {
            self.rebuild_display_list();
        }
    }

    /// Attaches or clears zero-serialization [`TypedNativeViewProps`] directly on `node_id`.
    pub fn set_native_view_props(
        &mut self,
        node_id: NodeId,
        props: Option<TypedNativeViewProps>,
    ) {
        let _ = self.layout_tree.set_native_view_props(node_id, props);
    }

    /// Records a zero-serialization [`NativeCommand`] dispatched directly in-process.
    pub fn dispatch_native_command(&mut self, node_id: NodeId, cmd: NativeCommand) {
        self.layout_tree.dispatch_native_command(node_id, cmd);
    }

    /// Records a zero-serialization [`TypedNativeViewCommand`] dispatched directly in-process.
    pub fn dispatch_typed_native_command(
        &mut self,
        node_id: NodeId,
        cmd: TypedNativeViewCommand,
    ) {
        self.layout_tree
            .dispatch_typed_native_command(node_id, cmd);
    }

    /// Computes the visual (scroll-shifted + sticky-clamped) layout rectangle of `node_id` in
    /// logical viewport coordinates.
    pub fn visual_rect_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        let node = self.layout_tree.node(node_id)?;
        let mut rect = node.computed_rect;
        let (own_sx, own_sy) = self
            .layout_tree
            .compute_sticky_delta(node_id, &self.scroll_offsets);
        rect.x += own_sx;
        rect.y += own_sy;
        let mut ancestor = node.parent;
        let mut depth = 0usize;
        while let Some(pid) = ancestor {
            if depth > MAX_HIT_TEST_DEPTH {
                break;
            }
            depth += 1;
            let Some(pnode) = self.layout_tree.node(pid) else {
                break;
            };
            let (sx, sy) = self.visual_scroll_shift(pnode);
            rect.x -= sx;
            rect.y -= sy;
            let (stx, sty) = self
                .layout_tree
                .compute_sticky_delta(pid, &self.scroll_offsets);
            rect.x += stx;
            rect.y += sty;
            ancestor = pnode.parent;
        }
        Some(rect)
    }

    /// Directly attaches or clears zero-serialization [`LayoutExt`] on `id`, recomputing layout
    /// and rebuilding the display list when changed.
    pub fn set_node_layout_ext(&mut self, id: NodeId, ext: Option<LayoutExt>) {
        if self.layout_tree.set_node_layout_ext(id, ext) {
            self.layout_tree.recompute_layout();
            self.reconcile_scroll_offsets();
            self.rebuild_display_list();
        }
    }

    /// Directly attaches or clears zero-serialization [`Transform2D`] on `id`, syncing transitions
    /// and rebuilding the display list when changed.
    pub fn set_node_transform(&mut self, id: NodeId, transform: Option<Transform2D>) {
        if self.layout_tree.set_node_transform(id, transform) {
            self.sync_motion_targets(&[id]);
            self.rebuild_display_list();
        }
    }

    pub fn rasterize(&self, buffer: &mut PixelBuffer, clear_color: ColorRgba) {
        rasterize_display_list_with_text_engine(
            &self.display_list,
            buffer,
            self.viewport.scale_factor,
            clear_color,
            self.text_engine.as_ref(),
        );
    }

    pub fn rasterize_at_scale(
        &self,
        buffer: &mut PixelBuffer,
        scale_factor: f32,
        clear_color: ColorRgba,
    ) {
        rasterize_display_list_with_text_engine(
            &self.display_list,
            buffer,
            scale_factor,
            clear_color,
            self.text_engine.as_ref(),
        );
    }

    pub(crate) fn rebuild_display_list(&mut self) {
        self.display_list.ops.clear();
        self.sort_scratch.clear();
        let viewport_rect = LayoutRect {
            x: 0.0,
            y: 0.0,
            width: self.viewport.width,
            height: self.viewport.height,
        };
        emit_node_into(
            &self.layout_tree,
            &self.scroll_offsets,
            &self.focus,
            &self.motion,
            self.text_engine.as_ref(),
            viewport_rect,
            (0.0, 0.0),
            false,
            InheritedTextStyle::default(),
            &mut self.sort_scratch,
            &mut self.display_list.ops,
            ROOT_NODE_ID,
        );
    }

    #[cfg(feature = "skia")]
    pub fn paint_to_canvas(&self, canvas: &skia_safe::Canvas) {
        SkiaCanvasRenderer::new().paint_display_list(&self.display_list, canvas, 1.0);
    }

    #[cfg(feature = "skia")]
    pub fn paint_to_surface<S: GpuSurfaceBackend>(
        &self,
        renderer: &SkiaCanvasRenderer,
        surface: &mut S,
        clear_color: ColorRgba,
    ) -> Result<(), GpuSurfaceError> {
        surface.render_display_list(
            renderer,
            &self.display_list,
            self.viewport.scale_factor,
            clear_color,
        )
    }
}

impl<L: LayoutEngine> SkiaBackend<L> {
    fn finish_skia_frame(&mut self, mut stats: BackendApplyStats) -> BackendApplyStats {
        let t_mount = Instant::now();
        self.reconcile_scroll_offsets();
        self.reconcile_focus_state();
        if !self.newly_tracked_transitions.is_empty() {
            let seeded = std::mem::take(&mut self.newly_tracked_transitions);
            for id in seeded {
                if let Some(style) = self.layout_tree.effective_style(id) {
                    self.motion.seed_baseline(id, style);
                }
                let tf = self
                    .layout_tree
                    .effective_transform(id)
                    .unwrap_or(Transform2D::IDENTITY);
                self.motion.seed_transform_baseline(id, tf);
            }
        }
        self.reconcile_motion_after_frame();
        self.rebuild_display_list();
        stats.mount_us = t_mount.elapsed().as_micros() as u64;
        stats
    }
}

impl<L: LayoutEngine> RenderSink for SkiaBackend<L> {
    fn begin_frame(&mut self, surface_id: SurfaceId, frame_id: FrameId) {
        self.newly_tracked_transitions.clear();
        self.layout_tree.begin_frame(surface_id, frame_id);
    }

    fn create_node(&mut self, node_id: NodeId, kind: ComponentKind, custom_name: Option<&str>) {
        self.layout_tree.create_node(node_id, kind, custom_name);
    }

    fn remove_node(&mut self, node_id: NodeId) {
        self.layout_tree.remove_node(node_id);
    }

    fn set_children(&mut self, parent_id: NodeId, children: &[NodeId]) {
        self.layout_tree.set_children(parent_id, children);
    }

    fn set_text(&mut self, node_id: NodeId, text: &str) {
        self.layout_tree.set_text(node_id, text);
    }

    fn set_layout(&mut self, node_id: NodeId, layout: &LayoutProps) {
        self.layout_tree.set_layout(node_id, layout);
    }

    fn set_style(&mut self, node_id: NodeId, style: &StyleProps) {
        self.layout_tree.set_style(node_id, style);
    }

    fn set_raw_prop(&mut self, node_id: NodeId, key: &str, value: RawPropValueRef<'_>) {
        self.layout_tree.set_raw_prop(node_id, key, value);
    }

    fn set_event_mask(&mut self, node_id: NodeId, mask: u64) {
        self.layout_tree.set_event_mask(node_id, mask);
    }

    fn dispatch_command(&mut self, node_id: NodeId, command_name: &str, payload: &[u8]) {
        self.layout_tree
            .dispatch_command(node_id, command_name, payload);
    }

    fn set_layout_ext(&mut self, node_id: NodeId, ext: Option<&LayoutExt>) {
        self.layout_tree.set_layout_ext(node_id, ext);
    }

    fn set_transform(&mut self, node_id: NodeId, transform: Option<Transform2D>) {
        self.layout_tree.set_transform(node_id, transform);
    }

    fn set_cursor(&mut self, node_id: NodeId, cursor: Option<CursorKind>) {
        self.layout_tree.set_cursor(node_id, cursor);
    }

    fn set_state_rules(&mut self, node_id: NodeId, rules: &[StateStyleRule]) {
        self.layout_tree.set_state_rules(node_id, rules);
    }

    fn set_transitions(&mut self, node_id: NodeId, transitions: &[TransitionSpec]) {
        let was_tracked = self.motion.has_transitions(node_id);
        self.motion
            .set_node_transitions(node_id, transitions.to_vec());
        if !was_tracked && !transitions.is_empty() {
            self.newly_tracked_transitions.push(node_id);
        }
    }

    fn set_background_fill(&mut self, node_id: NodeId, fill: Option<&BackgroundFill>) {
        self.layout_tree.set_background_fill(node_id, fill);
    }

    fn set_svg(&mut self, node_id: NodeId, svg: Option<&SvgPicture>) {
        self.layout_tree.set_svg(node_id, svg);
    }

    fn set_draw_list(&mut self, node_id: NodeId, draw_list: Option<&DrawList>) {
        self.layout_tree.set_draw_list(node_id, draw_list);
    }

    fn set_native_view_props(&mut self, node_id: NodeId, props: Option<&TypedNativeViewProps>) {
        RenderSink::set_native_view_props(&mut self.layout_tree, node_id, props);
    }

    fn commit_frame(&mut self, sync_mount: bool) -> Result<BackendApplyStats, BackendError> {
        let stats = self.layout_tree.commit_frame(sync_mount)?;
        Ok(self.finish_skia_frame(stats))
    }

    fn finish_uncommitted(&mut self) -> Result<BackendApplyStats, BackendError> {
        let stats = self.layout_tree.finish_uncommitted()?;
        Ok(self.finish_skia_frame(stats))
    }
}

impl<L: LayoutEngine> RenderBackend for SkiaBackend<L> {
    fn name(&self) -> &'static str {
        if cfg!(feature = "skia") {
            "skia-gpu"
        } else {
            "skia-displaylist"
        }
    }

    fn set_viewport(&mut self, surface_id: SurfaceId, viewport: Viewport) {
        self.viewport = viewport;
        self.layout_tree.set_viewport(surface_id, viewport);
    }

    fn submit_frame(&mut self, command_bytes: &[u8]) -> Result<BackendApplyStats, BackendError> {
        replay_command_buffer_into(command_bytes, self)
    }

    fn drain_events(&mut self) -> Vec<NativeEvent> {
        self.layout_tree.drain_events()
    }

    fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        self.layout_tree.layout_of(node_id)
    }
}
