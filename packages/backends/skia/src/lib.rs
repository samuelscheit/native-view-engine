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
pub mod font;
pub mod gpu;
mod hit_test;
pub mod image_cache;
mod interaction;
pub mod raster;
mod reloc_scene;
mod scroll;
pub mod text;
mod text_cache;
mod text_input;

#[cfg(test)]
mod media_tests;
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

use nve_abi::{ColorRgba, ComponentKind, NativeEvent, NodeId, SurfaceId, ROOT_NODE_ID};
use nve_backend_headless::{HeadlessBackend, TaffyLayoutEngine};
use nve_components::canvas_ir::{CanvasOp, ImageResizeMode};
use nve_engine::{
    BackendApplyStats, BackendError, LayoutEngine, LayoutRect, RenderBackend, TextMeasurer,
    Viewport,
};

use display_list::{emit_node_into, InheritedTextStyle};
use text_input::{
    clamp_char_boundary, compute_text_input_inner_rect, compute_text_input_scroll_x,
    cursor_byte_index_at_local_x, is_secure_node, raw_text_input_value,
};

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
pub use nve_components::interaction::{
    CursorKind, FocusModality, InteractionState, InteractionTracker, StateStyleRule,
};
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

/// Active focus, caret, range selection, and IME composition state tracked by [`SkiaBackend`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextFocusState {
    pub focused_node: Option<NodeId>,
    /// Caret (active end) UTF-8 byte offset within the focused `TextInput`'s committed value
    /// (always clamped to a valid UTF-8 char boundary).
    pub caret_byte_index: usize,
    /// Selection anchor UTF-8 byte offset (`selection_anchor != caret_byte_index` indicates an
    /// active range selection).
    pub selection_anchor: usize,
    /// Active IME preedit (composition) string inserted visually at `caret_byte_index`.
    pub ime_preedit: Option<String>,
    /// Optional byte range `(start, end)` of cursor/selection inside `ime_preedit`.
    pub ime_preedit_cursor: Option<(usize, usize)>,
    /// Local value override for uncontrolled `TextInput`s; cleared automatically whenever
    /// `submit_frame` supplies a `value` prop or focus moves.
    pub(crate) uncontrolled_value: Option<String>,
}

impl TextFocusState {
    /// Returns the ordered `(start, end)` UTF-8 byte range when a non-empty selection is active.
    #[inline]
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        if self.selection_anchor != self.caret_byte_index {
            Some((
                self.selection_anchor.min(self.caret_byte_index),
                self.selection_anchor.max(self.caret_byte_index),
            ))
        } else {
            None
        }
    }
}

pub struct SkiaBackend<L: LayoutEngine = TaffyLayoutEngine> {
    pub(crate) layout_tree: HeadlessBackend<L>,
    pub(crate) display_list: DisplayList,
    pub(crate) viewport: Viewport,
    pub(crate) text_engine: Arc<CosmicTextEngine>,
    pub(crate) scroll_offsets: HashMap<NodeId, (f32, f32)>,
    pub(crate) last_virtual_scroll_y: HashMap<NodeId, f32>,
    pub(crate) focus: TextFocusState,
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
            sort_scratch: self.sort_scratch,
        };
        swapped.reconcile_scroll_offsets();
        swapped.reconcile_focus_state();
        swapped.rebuild_display_list();
        swapped
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn display_list(&self) -> &DisplayList {
        &self.display_list
    }

    pub fn focused_node(&self) -> Option<NodeId> {
        self.focus.focused_node
    }

    pub fn focus_state(&self) -> &TextFocusState {
        &self.focus
    }

    pub fn is_text_input(&self, node_id: NodeId) -> bool {
        self.layout_tree
            .node(node_id)
            .is_some_and(|n| n.kind == ComponentKind::TextInput)
    }

    pub fn is_secure_text_input(&self, node_id: NodeId) -> bool {
        self.layout_tree
            .node(node_id)
            .is_some_and(|n| n.kind == ComponentKind::TextInput && is_secure_node(n))
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

    /// Hit-tests a logical viewport X coordinate against a `TextInput` node and returns the
    /// closest UTF-8 byte boundary inside its current committed value.
    pub fn cursor_index_at_x(&self, node_id: NodeId, logical_x: f32) -> Option<usize> {
        let node = self.layout_tree.node(node_id)?;
        if node.kind != ComponentKind::TextInput {
            return None;
        }
        let committed = self.text_input_value(node_id)?;
        let visual_rect = self.visual_rect_of(node_id)?;
        let font_size = if node.style.font_size > 0.0 {
            node.style.font_size
        } else {
            13.0
        };
        let text_rect = compute_text_input_inner_rect(node, visual_rect, font_size);
        let is_secure = is_secure_node(node);
        let scroll_x = if self.focus.focused_node == Some(node_id) {
            compute_text_input_scroll_x(
                self.text_engine.as_ref(),
                &committed,
                self.focus.caret_byte_index,
                font_size,
                is_secure,
                text_rect.width,
            )
        } else {
            0.0
        };
        let local_x = logical_x - text_rect.x + scroll_x;
        Some(cursor_byte_index_at_local_x(
            self.text_engine.as_ref(),
            &committed,
            font_size,
            is_secure,
            local_x,
        ))
    }

    pub fn text_input_value(&self, node_id: NodeId) -> Option<String> {
        let node = self.layout_tree.node(node_id)?;
        if node.kind != ComponentKind::TextInput {
            return None;
        }
        if self.focus.focused_node == Some(node_id)
            && let Some(override_val) = &self.focus.uncontrolled_value
        {
            return Some(override_val.clone());
        }
        Some(raw_text_input_value(node).to_owned())
    }

    pub(crate) fn sync_text_focus_without_rebuild(&mut self, resolved: Option<NodeId>) -> bool {
        if self.focus.focused_node == resolved {
            return false;
        }
        self.focus.focused_node = resolved;
        self.focus.ime_preedit = None;
        self.focus.ime_preedit_cursor = None;
        self.focus.uncontrolled_value = None;
        let end = resolved
            .and_then(|id| self.text_input_value(id))
            .map_or(0, |v| v.len());
        self.focus.caret_byte_index = end;
        self.focus.selection_anchor = end;
        true
    }

    pub fn set_focused_node(&mut self, node_id: Option<NodeId>) -> bool {
        let dirty = self.layout_tree.focus_pointer(node_id);
        let resolved = self.layout_tree.interaction_tracker().focused();
        let changed = self.sync_text_focus_without_rebuild(resolved);
        if changed || !dirty.is_empty() {
            self.rebuild_display_list();
            true
        } else {
            false
        }
    }

    pub fn caret_byte_index(&self) -> usize {
        self.focus.caret_byte_index
    }

    pub fn selection_anchor_byte_index(&self) -> usize {
        self.focus.selection_anchor
    }

    /// Returns the ordered `(start, end)` UTF-8 byte range when a non-empty selection is active.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.focus.selection_range()
    }

    /// Returns the currently selected substring of the focused `TextInput`, if any.
    pub fn selected_text(&self) -> Option<String> {
        let id = self.focus.focused_node?;
        let (s, e) = self.focus.selection_range()?;
        let val = self.text_input_value(id)?;
        let cs = clamp_char_boundary(&val, s);
        let ce = clamp_char_boundary(&val, e);
        (cs < ce).then(|| val[cs..ce].to_owned())
    }

    pub fn caret_char_index(&self) -> usize {
        let Some(id) = self.focus.focused_node else {
            return 0;
        };
        let Some(val) = self.text_input_value(id) else {
            return 0;
        };
        let clamped = clamp_char_boundary(&val, self.focus.caret_byte_index);
        val[..clamped].chars().count()
    }

    /// Moves the caret to `byte_index` (clamped to a valid char boundary), collapsing any active
    /// selection (`selection_anchor = caret_byte_index`).
    pub fn set_caret_byte_index(&mut self, byte_index: usize) {
        self.set_selection_range(byte_index, byte_index);
    }

    /// Sets the selection anchor and active caret byte indices (both clamped to valid UTF-8 char
    /// boundaries) and rebuilds the display list when changed.
    pub fn set_selection_range(&mut self, anchor: usize, caret: usize) {
        let (clamped_anchor, clamped_caret) = self
            .focus
            .focused_node
            .and_then(|id| self.text_input_value(id))
            .map_or((0, 0), |v| {
                (
                    clamp_char_boundary(&v, anchor),
                    clamp_char_boundary(&v, caret),
                )
            });
        let changed = self.focus.selection_anchor != clamped_anchor
            || self.focus.caret_byte_index != clamped_caret;
        self.focus.selection_anchor = clamped_anchor;
        self.focus.caret_byte_index = clamped_caret;
        if changed {
            self.rebuild_display_list();
        }
    }

    /// Selects the entire committed value of the focused `TextInput` (`0..val.len()`).
    pub fn select_all_text(&mut self) {
        let Some(id) = self.focus.focused_node else {
            return;
        };
        let len = self.text_input_value(id).map_or(0, |v| v.len());
        self.set_selection_range(0, len);
    }

    /// Collapses any active selection to the current caret position.
    pub fn clear_selection(&mut self) {
        let caret = self.focus.caret_byte_index;
        self.set_selection_range(caret, caret);
    }

    pub fn ime_preedit(&self) -> Option<&str> {
        self.focus.ime_preedit.as_deref()
    }

    pub fn set_ime_preedit(
        &mut self,
        preedit: Option<String>,
        cursor: Option<(usize, usize)>,
    ) {
        let normalized = preedit.filter(|s| !s.is_empty());
        let normalized_cursor = normalized.as_deref().and_then(|text| {
            cursor.map(|(s, e)| {
                let cs = clamp_char_boundary(text, s);
                let ce = clamp_char_boundary(text, e.max(s));
                (cs, ce)
            })
        });
        if self.focus.ime_preedit != normalized
            || self.focus.ime_preedit_cursor != normalized_cursor
        {
            self.focus.ime_preedit = normalized;
            self.focus.ime_preedit_cursor = normalized_cursor;
            self.rebuild_display_list();
        }
    }

    pub fn set_focused_text_override(&mut self, value: Option<String>, caret_byte_index: usize) {
        let Some(id) = self.focus.focused_node else {
            return;
        };
        let clamped = match &value {
            Some(v) => clamp_char_boundary(v, caret_byte_index),
            None => self.layout_tree.node(id).map_or(0, |n| {
                clamp_char_boundary(raw_text_input_value(n), caret_byte_index)
            }),
        };
        self.focus.uncontrolled_value = value;
        self.focus.caret_byte_index = clamped;
        self.focus.selection_anchor = clamped;
        self.rebuild_display_list();
    }

    pub(crate) fn reconcile_focus_state(&mut self) {
        let Some(id) = self.focus.focused_node else {
            return;
        };
        let Some(node) = self.layout_tree.node(id) else {
            self.focus = TextFocusState::default();
            return;
        };
        if node.kind == ComponentKind::TextInput {
            if node.raw_props.contains_key("value") {
                self.focus.uncontrolled_value = None;
            }
            let current_val = self
                .focus
                .uncontrolled_value
                .as_deref()
                .unwrap_or_else(|| raw_text_input_value(node));
            self.focus.caret_byte_index =
                clamp_char_boundary(current_val, self.focus.caret_byte_index);
            self.focus.selection_anchor =
                clamp_char_boundary(current_val, self.focus.selection_anchor);
        } else {
            self.focus.caret_byte_index = 0;
            self.focus.selection_anchor = 0;
            self.focus.ime_preedit = None;
            self.focus.ime_preedit_cursor = None;
            self.focus.uncontrolled_value = None;
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
