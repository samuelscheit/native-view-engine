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
mod scroll;
pub mod text;
mod text_cache;
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
mod text_input_tests;
#[cfg(test)]
mod text_tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use nve_abi::{ColorRgba, NativeEvent, NodeId, SurfaceId, ROOT_NODE_ID};
use nve_backend_headless::{HeadlessBackend, TaffyLayoutEngine};
use nve_components::canvas_ir::{CanvasOp, ImageResizeMode};
use nve_engine::{
    BackendApplyStats, BackendError, LayoutEngine, LayoutRect, RenderBackend, TextMeasurer,
    Viewport,
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
pub use nve_components::motion::{EasingCurve, MotionPreference, TransitionProperty, TransitionSpec};
pub use nve_components::style_patch::{Display, StylePatch};
#[cfg(feature = "yoga")]
pub use nve_layout_yoga::{YogaLayoutEngine, YogaUnavailableError};
pub use raster::{
    draw_hud_banner, parse_box_shadow, rasterize_display_list, rasterize_display_list_bitmap,
    rasterize_display_list_with_text_engine, BoxShadow, PixelBuffer,
};
pub use text::{parse_font_weight, CosmicTextEngine, TextAlign, DEFAULT_FONT_WEIGHT};

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
    CustomSkiaCanvas {
        node_id: NodeId,
        rect: LayoutRect,
        ops: Vec<CanvasOp>,
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

    /// Computes the visual (scroll-shifted) layout rectangle of `node_id` in logical viewport coordinates.
    pub fn visual_rect_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        let node = self.layout_tree.node(node_id)?;
        let mut rect = node.computed_rect;
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
            ancestor = pnode.parent;
        }
        Some(rect)
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
        let mut stats = self.layout_tree.submit_frame(command_bytes)?;
        let t_mount = Instant::now();
        self.reconcile_scroll_offsets();
        self.reconcile_focus_state();
        self.reconcile_motion_after_frame();
        self.rebuild_display_list();
        stats.mount_us = t_mount.elapsed().as_micros() as u64;
        Ok(stats)
    }

    fn drain_events(&mut self) -> Vec<NativeEvent> {
        self.layout_tree.drain_events()
    }

    fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        self.layout_tree.layout_of(node_id)
    }
}
