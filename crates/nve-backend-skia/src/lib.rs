//! Cross-platform Skia RenderBackend targeting macOS, Windows, Linux, iOS, and Android.
//!
//! Consumes the exact same `nve-abi` binary `CommandBuffer` as the React Native Fabric backend,
//! computes Flexbox layout, compiles a culled retained `DisplayList`, rasterizes cleanly via a
//! zero-dependency software rasterizer, and (with `--features skia` / `skia-metal` / `skia-vulkan` /
//! `skia-d3d`) paints directly onto a `skia_safe::Canvas` or hardware GPU swapchain.
//!
//! For the runnable desktop `winit` + `softbuffer` / GPU window host, see `hosts/skia-shell`
//! (`nve-skia-shell`).

pub mod font;
pub mod gpu;
pub mod image_cache;
pub mod raster;
pub mod text;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use nve_abi::{
    ColorRgba, ComponentKind, EventMask, NativeEvent, NodeId, RawPropValue, SurfaceId, ROOT_NODE_ID,
};
use nve_backend_headless::{HeadlessBackend, HeadlessNode, TaffyLayoutEngine};
use nve_components::canvas_ir::{CanvasOp, DrawListDecoder, ImageResizeMode};
use nve_engine::{
    BackendApplyStats, BackendError, LayoutEngine, LayoutRect, RenderBackend, TextMeasurer,
    Viewport,
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
    compute_image_placement, decode_image_uri, decode_png, DecodedImage, ImageCache,
    ImageDecodeError, ImageDrawPlacement,
};
#[cfg(feature = "yoga")]
pub use nve_layout_yoga::{YogaLayoutEngine, YogaUnavailableError};
pub use raster::{
    draw_hud_banner, rasterize_display_list, rasterize_display_list_bitmap,
    rasterize_display_list_with_text_engine, PixelBuffer,
};
pub use text::CosmicTextEngine;

const MAX_HIT_TEST_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub enum PaintOp {
    SaveOpacity(f32),
    Restore,
    PushClip {
        rect: LayoutRect,
        radius: f32,
    },
    PopClip,
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
    layout_tree: HeadlessBackend<L>,
    display_list: DisplayList,
    viewport: Viewport,
    text_engine: Arc<CosmicTextEngine>,
    scroll_offsets: HashMap<NodeId, (f32, f32)>,
    last_virtual_scroll_y: HashMap<NodeId, f32>,
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
    /// Creates a [`SkiaBackend`] using any concrete or boxed [`LayoutEngine`], defaulting to
    /// the shared native [`CosmicTextEngine`] for both layout measurement and rasterization.
    pub fn with_layout(layout: L) -> Self {
        Self::with_layout_and_text_engine(layout, CosmicTextEngine::shared())
    }

    /// Creates a [`SkiaBackend`] with an explicit [`LayoutEngine`] and [`CosmicTextEngine`].
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

    /// Replaces the active [`TextMeasurer`] used for intrinsic text layout.
    pub fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.layout_tree.set_text_measurer(measurer);
    }

    /// Hot-swaps the active [`LayoutEngine`] to `next`, replaying the retained tree and
    /// rebuilding the retained [`DisplayList`] against the new layout rectangles.
    pub fn swap_layout_engine<NewL: LayoutEngine>(self, next: NewL) -> SkiaBackend<NewL> {
        let layout_tree = self.layout_tree.swap_layout_engine(next);
        let mut swapped = SkiaBackend {
            layout_tree,
            display_list: DisplayList::default(),
            viewport: self.viewport,
            text_engine: self.text_engine,
            scroll_offsets: self.scroll_offsets,
            last_virtual_scroll_y: self.last_virtual_scroll_y,
        };
        swapped.reconcile_scroll_offsets();
        swapped.rebuild_display_list();
        swapped
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn display_list(&self) -> &DisplayList {
        &self.display_list
    }

    /// Returns the stored `(scroll_x, scroll_y)` offset for a `ScrollView` node.
    pub fn scroll_offset(&self, node_id: NodeId) -> (f32, f32) {
        self.scroll_offsets.get(&node_id).copied().unwrap_or((0.0, 0.0))
    }

    /// Computes `(content_width, content_height)` for a `ScrollView` node (taking into account
    /// both measured descendant extents and optional virtualized `virtual-total-height`).
    pub fn scroll_content_size(&self, node_id: NodeId) -> Option<(f32, f32)> {
        compute_scroll_content_size(&self.layout_tree, node_id)
    }

    /// Sets the scroll offset of `node_id`, clamping it against `[0..=content - viewport]`,
    /// rebuilds the retained [`DisplayList`] if changed, and returns the clamped `(x, y)`.
    pub fn set_scroll_offset(&mut self, node_id: NodeId, x: f32, y: f32) -> (f32, f32) {
        let Some(node) = self.layout_tree.node(node_id) else {
            return (0.0, 0.0);
        };
        let rect = node.computed_rect;
        let (cw, ch) = compute_scroll_content_size(&self.layout_tree, node_id)
            .unwrap_or((rect.width, rect.height));
        let max_x = (cw - rect.width).max(0.0);
        let max_y = (ch - rect.height).max(0.0);
        let clamped_x = if x.is_finite() { x.clamp(0.0, max_x) } else { 0.0 };
        let clamped_y = if y.is_finite() { y.clamp(0.0, max_y) } else { 0.0 };

        let prev = self.scroll_offsets.get(&node_id).copied().unwrap_or((0.0, 0.0));
        self.scroll_offsets.insert(node_id, (clamped_x, clamped_y));
        if (prev.0 - clamped_x).abs() > 1e-4 || (prev.1 - clamped_y).abs() > 1e-4 {
            self.rebuild_display_list();
        }
        (clamped_x, clamped_y)
    }

    /// Applies an incremental scroll delta `(dx, dy)` to `node_id`, clamping against content
    /// bounds and rebuilding the display list when the offset changes.
    pub fn apply_scroll_delta(&mut self, node_id: NodeId, dx: f32, dy: f32) -> (f32, f32) {
        let (cur_x, cur_y) = self.scroll_offset(node_id);
        let nx = cur_x + if dx.is_finite() { dx } else { 0.0 };
        let ny = cur_y + if dy.is_finite() { dy } else { 0.0 };
        self.set_scroll_offset(node_id, nx, ny)
    }

    /// Scroll-aware and clip-aware interactive hit test at logical coordinates `(x, y)`.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_rec(ROOT_NODE_ID, x, y, (0.0, 0.0), None, true, 0)
    }

    /// Finds the deepest `ScrollView` (or node subscribed to `EventMask::SCROLL`) enclosing `(x, y)`.
    pub fn hit_test_scroll_view(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_scroll_rec(ROOT_NODE_ID, x, y, (0.0, 0.0), None, 0)
    }

    /// Returns the first retained node subscribed to `EventMask::CUSTOM` (e.g. `onkeydown`),
    /// falling back to `ROOT_NODE_ID` when no specific node declares custom listeners.
    pub fn find_custom_event_target(&self) -> NodeId {
        self.find_event_mask_rec(ROOT_NODE_ID, EventMask::CUSTOM, 0)
            .unwrap_or(ROOT_NODE_ID)
    }

    fn find_event_mask_rec(&self, id: NodeId, mask: u64, depth: usize) -> Option<NodeId> {
        if depth > MAX_HIT_TEST_DEPTH {
            return None;
        }
        let node = self.layout_tree.node(id)?;
        for &child in node.children.iter().rev() {
            if let Some(found) = self.find_event_mask_rec(child, mask, depth + 1) {
                return Some(found);
            }
        }
        if (node.event_mask & mask) != 0 {
            Some(id)
        } else {
            None
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn hit_test_rec(
        &self,
        id: NodeId,
        x: f32,
        y: f32,
        offset: (f32, f32),
        clip: Option<LayoutRect>,
        interactive_only: bool,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > MAX_HIT_TEST_DEPTH {
            return None;
        }
        let node = self.layout_tree.node(id)?;
        if node.style.opacity <= 0.001 {
            return None;
        }

        let visual_rect = offset_rect(node.computed_rect, offset);
        let inside_self = point_in_rect(x, y, visual_rect)
            && clip.is_none_or(|c| point_in_rect(x, y, c));

        if !inside_self && id != ROOT_NODE_ID {
            return None;
        }

        let child_clip = if should_clip_children(node) {
            let next = match clip {
                Some(prev) => intersect_rects(prev, visual_rect),
                None => Some(visual_rect),
            };
            match next {
                Some(c) if point_in_rect(x, y, c) => Some(c),
                _ => {
                    // Point is outside clipped region; children cannot be hit.
                    return if inside_self && (!interactive_only || node.event_mask != 0) {
                        Some(id)
                    } else {
                        None
                    };
                }
            }
        } else {
            clip
        };

        let (sx, sy) = self.visual_scroll_shift(node);
        let child_offset = (offset.0 - sx, offset.1 - sy);

        let children = &node.children;
        let has_z_order = children.len() > 1
            && children
                .iter()
                .any(|&cid| self.layout_tree.node(cid).is_some_and(|c| c.style.z_index != 0));
        if has_z_order {
            let mut sorted = children.clone();
            sorted.sort_by_key(|&cid| self.layout_tree.node(cid).map_or(0, |c| c.style.z_index));
            for child in sorted.into_iter().rev() {
                if let Some(hit) = self.hit_test_rec(
                    child,
                    x,
                    y,
                    child_offset,
                    child_clip,
                    interactive_only,
                    depth + 1,
                ) {
                    return Some(hit);
                }
            }
        } else {
            for &child in children.iter().rev() {
                if let Some(hit) = self.hit_test_rec(
                    child,
                    x,
                    y,
                    child_offset,
                    child_clip,
                    interactive_only,
                    depth + 1,
                ) {
                    return Some(hit);
                }
            }
        }

        if inside_self && (!interactive_only || node.event_mask != 0) {
            Some(id)
        } else {
            None
        }
    }

    fn hit_test_scroll_rec(
        &self,
        id: NodeId,
        x: f32,
        y: f32,
        offset: (f32, f32),
        clip: Option<LayoutRect>,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > MAX_HIT_TEST_DEPTH {
            return None;
        }
        let node = self.layout_tree.node(id)?;
        if node.style.opacity <= 0.001 {
            return None;
        }
        let visual_rect = offset_rect(node.computed_rect, offset);
        let inside_self = point_in_rect(x, y, visual_rect)
            && clip.is_none_or(|c| point_in_rect(x, y, c));
        if !inside_self && id != ROOT_NODE_ID {
            return None;
        }

        let is_scrollable =
            node.kind == ComponentKind::ScrollView || (node.event_mask & EventMask::SCROLL) != 0;

        let child_clip = if should_clip_children(node) {
            match clip {
                Some(prev) => intersect_rects(prev, visual_rect),
                None => Some(visual_rect),
            }
        } else {
            clip
        };

        let (sx, sy) = self.visual_scroll_shift(node);
        let child_offset = (offset.0 - sx, offset.1 - sy);

        if child_clip.is_none_or(|c| point_in_rect(x, y, c)) {
            for &child in node.children.iter().rev() {
                if let Some(hit) =
                    self.hit_test_scroll_rec(child, x, y, child_offset, child_clip, depth + 1)
                {
                    return Some(hit);
                }
            }
        }

        if inside_self && is_scrollable {
            Some(id)
        } else {
            None
        }
    }

    fn visual_scroll_shift(&self, node: &HeadlessNode) -> (f32, f32) {
        if node.kind != ComponentKind::ScrollView {
            return (0.0, 0.0);
        }
        let (sx, sy) = self.scroll_offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        if let Some(top_spacer) = parse_px_raw_prop(node.raw_props.get("virtual-top-spacer")) {
            // Virtualized ScrollView mounts children starting at `virtual-top-spacer`.
            (sx, (sy - top_spacer).max(0.0))
        } else {
            (sx, sy)
        }
    }

    fn reconcile_scroll_offsets(&mut self) {
        // Prune scroll state for removed nodes.
        self.scroll_offsets
            .retain(|id, _| self.layout_tree.node(*id).is_some());
        self.last_virtual_scroll_y
            .retain(|id, _| self.layout_tree.node(*id).is_some());

        // Sync virtual-scroll-y updates initiated by the app (e.g. Jump 50k buttons) and re-clamp.
        let mut updates: Vec<(NodeId, f32, f32, Option<f32>)> = Vec::new();
        self.collect_scroll_reconcile(ROOT_NODE_ID, &mut updates, 0);
        for (id, x, y, v_scroll) in updates {
            if let Some(vs) = v_scroll {
                self.last_virtual_scroll_y.insert(id, vs);
            }
            if x != 0.0 || y != 0.0 || self.scroll_offsets.contains_key(&id) {
                self.scroll_offsets.insert(id, (x, y));
            }
        }
    }

    fn collect_scroll_reconcile(
        &self,
        id: NodeId,
        out: &mut Vec<(NodeId, f32, f32, Option<f32>)>,
        depth: usize,
    ) {
        if depth > MAX_HIT_TEST_DEPTH {
            return;
        }
        let Some(node) = self.layout_tree.node(id) else {
            return;
        };
        if node.kind == ComponentKind::ScrollView {
            let rect = node.computed_rect;
            let (cw, ch) = compute_scroll_content_size(&self.layout_tree, id)
                .unwrap_or((rect.width, rect.height));
            let max_x = (cw - rect.width).max(0.0);
            let max_y = (ch - rect.height).max(0.0);
            let (mut sx, mut sy) = self.scroll_offsets.get(&id).copied().unwrap_or((0.0, 0.0));
            let v_scroll = parse_px_raw_prop(node.raw_props.get("virtual-scroll-y"));
            if let Some(app_vy) = v_scroll {
                let prev_vy = self.last_virtual_scroll_y.get(&id).copied();
                if prev_vy.is_none_or(|p| (p - app_vy).abs() > 0.5) {
                    sy = app_vy;
                }
            }
            sx = sx.clamp(0.0, max_x);
            sy = sy.clamp(0.0, max_y);
            out.push((id, sx, sy, v_scroll));
        }
        for &child in &node.children {
            self.collect_scroll_reconcile(child, out, depth + 1);
        }
    }

    /// Rasterizes the retained `DisplayList` into `buffer` using the current viewport scale factor.
    pub fn rasterize(&self, buffer: &mut PixelBuffer, clear_color: ColorRgba) {
        rasterize_display_list_with_text_engine(
            &self.display_list,
            buffer,
            self.viewport.scale_factor,
            clear_color,
            self.text_engine.as_ref(),
        );
    }

    /// Rasterizes the retained `DisplayList` into `buffer` at an explicit pixel scale factor.
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

    fn rebuild_display_list(&mut self) {
        self.display_list.ops.clear();
        let viewport_rect = LayoutRect {
            x: 0.0,
            y: 0.0,
            width: self.viewport.width,
            height: self.viewport.height,
        };
        emit_node_into(
            &self.layout_tree,
            &self.scroll_offsets,
            viewport_rect,
            (0.0, 0.0),
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

#[inline]
fn offset_rect(rect: LayoutRect, offset: (f32, f32)) -> LayoutRect {
    LayoutRect {
        x: rect.x + offset.0,
        y: rect.y + offset.1,
        width: rect.width,
        height: rect.height,
    }
}

#[inline]
fn point_in_rect(x: f32, y: f32, r: LayoutRect) -> bool {
    r.width > 0.0 && r.height > 0.0 && x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height
}

#[inline]
fn intersect_rects(a: LayoutRect, b: LayoutRect) -> Option<LayoutRect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);
    if x1 > x0 && y1 > y0 {
        Some(LayoutRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    } else {
        None
    }
}

#[inline]
fn intersects_clip(clip: LayoutRect, rect: LayoutRect, outset: f32) -> bool {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return false;
    }
    let x0 = rect.x - outset;
    let y0 = rect.y - outset;
    let x1 = rect.x + rect.width + outset;
    let y1 = rect.y + rect.height + outset;
    x1 > clip.x && y1 > clip.y && x0 < clip.x + clip.width && y0 < clip.y + clip.height
}

fn parse_px_raw_prop(val: Option<&RawPropValue>) -> Option<f32> {
    match val? {
        RawPropValue::F64(v) if v.is_finite() => Some(*v as f32),
        RawPropValue::I64(v) => Some(*v as f32),
        RawPropValue::String(s) => {
            let trimmed = s.trim().trim_end_matches("px").trim();
            let v = trimmed.parse::<f32>().ok()?;
            v.is_finite().then_some(v)
        }
        _ => None,
    }
}

fn should_clip_children(node: &HeadlessNode) -> bool {
    if node.kind == ComponentKind::ScrollView {
        return true;
    }
    if let Some(RawPropValue::String(ov)) = node.raw_props.get("overflow") {
        let lower = ov.trim().to_ascii_lowercase();
        if matches!(lower.as_str(), "hidden" | "clip" | "scroll" | "auto") {
            return true;
        }
    }
    if let Some(RawPropValue::Bool(true)) = node.raw_props.get("clip") {
        return true;
    }
    false
}

fn compute_scroll_content_size<L: LayoutEngine>(
    layout_tree: &HeadlessBackend<L>,
    scroll_id: NodeId,
) -> Option<(f32, f32)> {
    let node = layout_tree.node(scroll_id)?;
    let origin_x = node.computed_rect.x;
    let origin_y = node.computed_rect.y;
    let mut max_w = node.computed_rect.width.max(0.0);
    let mut max_h = node.computed_rect.height.max(0.0);

    fn walk<L: LayoutEngine>(
        tree: &HeadlessBackend<L>,
        id: NodeId,
        origin_x: f32,
        origin_y: f32,
        max_w: &mut f32,
        max_h: &mut f32,
        depth: usize,
    ) {
        if depth > MAX_HIT_TEST_DEPTH {
            return;
        }
        let Some(n) = tree.node(id) else {
            return;
        };
        let r = n.computed_rect;
        let right = (r.x + r.width) - origin_x;
        let bottom = (r.y + r.height) - origin_y;
        if right.is_finite() && right > *max_w {
            *max_w = right;
        }
        if bottom.is_finite() && bottom > *max_h {
            *max_h = bottom;
        }
        // Do not descend through nested ScrollViews for outer content bounds (their own content clips).
        if n.kind != ComponentKind::ScrollView {
            for &child in &n.children {
                walk(tree, child, origin_x, origin_y, max_w, max_h, depth + 1);
            }
        }
    }

    for &child in &node.children {
        walk(
            layout_tree,
            child,
            origin_x,
            origin_y,
            &mut max_w,
            &mut max_h,
            0,
        );
    }

    if !node.children.is_empty() {
        max_w = (max_w + node.layout.padding.right.max(0.0)).max(node.computed_rect.width);
        max_h = (max_h + node.layout.padding.bottom.max(0.0)).max(node.computed_rect.height);
    }

    if let Some(virtual_h) = parse_px_raw_prop(node.raw_props.get("virtual-total-height")) {
        if virtual_h > max_h {
            max_h = virtual_h;
        }
    }
    if let Some(virtual_w) = parse_px_raw_prop(node.raw_props.get("virtual-total-width")) {
        if virtual_w > max_w {
            max_w = virtual_w;
        }
    }

    Some((max_w, max_h))
}

fn extract_image_src(node: &HeadlessNode) -> Option<String> {
    let raw = node.raw_props.get("src")?;
    match raw {
        RawPropValue::String(s) if !s.is_empty() => Some(s.clone()),
        RawPropValue::Bytes(bytes) if !bytes.is_empty() => {
            ImageCache::shared().register_png_bytes(bytes).ok()
        }
        _ => None,
    }
}

fn extract_image_resize_mode(node: &HeadlessNode) -> ImageResizeMode {
    let raw = node
        .raw_props
        .get("resizeMode")
        .or_else(|| node.raw_props.get("resize_mode"));
    match raw {
        Some(RawPropValue::String(s)) => ImageResizeMode::parse(s).unwrap_or_default(),
        Some(RawPropValue::I64(n)) => ImageResizeMode::from_u8(*n as u8).unwrap_or_default(),
        _ => ImageResizeMode::default(),
    }
}

fn extract_canvas_ops(node: &HeadlessNode) -> Vec<CanvasOp> {
    let Some(raw) = node
        .raw_props
        .get("drawList")
        .or_else(|| node.raw_props.get("draw_list"))
    else {
        return Vec::new();
    };
    DrawListDecoder::decode_raw_prop(raw).unwrap_or_default()
}

fn emit_node_into<L: LayoutEngine>(
    layout_tree: &HeadlessBackend<L>,
    scroll_offsets: &HashMap<NodeId, (f32, f32)>,
    clip_bounds: LayoutRect,
    offset: (f32, f32),
    ops: &mut Vec<PaintOp>,
    id: NodeId,
) {
    let Some(node) = layout_tree.node(id) else {
        return;
    };

    let opacity = node.style.opacity.clamp(0.0, 1.0);
    if opacity <= 0.001 {
        // Fully transparent subtree produces no visible pixels — cull completely.
        return;
    }

    let pushed_opacity = opacity < 0.999;
    let opacity_op_idx = ops.len();
    if pushed_opacity {
        ops.push(PaintOp::SaveOpacity(opacity));
    }

    let rect = offset_rect(node.computed_rect, offset);
    let in_clip = intersects_clip(clip_bounds, rect, 0.0);

    if node.style.background.a > 0 && in_clip {
        ops.push(PaintOp::FillRoundedRect {
            rect,
            radius: node.style.border_radius.max(0.0),
            color: node.style.background,
        });
    }
    if node.style.border_width > 0.0
        && node.style.border_color.a > 0
        && intersects_clip(clip_bounds, rect, node.style.border_width * 0.5)
    {
        ops.push(PaintOp::StrokeRoundedRect {
            rect,
            radius: node.style.border_radius.max(0.0),
            width: node.style.border_width,
            color: node.style.border_color,
        });
    }
    if in_clip {
        if node.kind == ComponentKind::Image {
            if let Some(src) = extract_image_src(node) {
                ops.push(PaintOp::DrawImage {
                    rect,
                    radius: node.style.border_radius.max(0.0),
                    src,
                    resize_mode: extract_image_resize_mode(node),
                });
            }
        }
        if let Some(text) = node.text.as_deref() {
            if !text.is_empty() && node.style.font_size > 0.0 && node.style.text_color.a > 0 {
                ops.push(PaintOp::DrawText {
                    rect,
                    text: text.to_owned(),
                    font_size: node.style.font_size,
                    color: node.style.text_color,
                });
            }
        }
        if node.kind == ComponentKind::SkiaCanvas {
            let canvas_ops = extract_canvas_ops(node);
            ops.push(PaintOp::CustomSkiaCanvas {
                node_id: id,
                rect,
                ops: canvas_ops,
            });
        }
    }

    // Paint children (optionally clipped for ScrollView / overflow:hidden, and scroll-translated).
    let children = &node.children;
    if !children.is_empty() {
        let clips = should_clip_children(node);
        let next_clip = if clips {
            intersect_rects(clip_bounds, rect)
        } else {
            Some(clip_bounds)
        };

        if let Some(child_clip) = next_clip {
            let clip_op_idx = ops.len();
            if clips {
                ops.push(PaintOp::PushClip {
                    rect,
                    radius: node.style.border_radius.max(0.0),
                });
            }

            let child_offset = if node.kind == ComponentKind::ScrollView {
                let (sx, sy) = scroll_offsets.get(&id).copied().unwrap_or((0.0, 0.0));
                let visual_sy = if let Some(top_spacer) =
                    parse_px_raw_prop(node.raw_props.get("virtual-top-spacer"))
                {
                    (sy - top_spacer).max(0.0)
                } else {
                    sy
                };
                (offset.0 - sx, offset.1 - visual_sy)
            } else {
                offset
            };

            let has_z_order = children.len() > 1
                && children
                    .iter()
                    .any(|&cid| layout_tree.node(cid).is_some_and(|c| c.style.z_index != 0));
            if has_z_order {
                let mut sorted = children.clone();
                sorted.sort_by_key(|&cid| layout_tree.node(cid).map_or(0, |c| c.style.z_index));
                for child in sorted {
                    emit_node_into(
                        layout_tree,
                        scroll_offsets,
                        child_clip,
                        child_offset,
                        ops,
                        child,
                    );
                }
            } else {
                for &child in children {
                    emit_node_into(
                        layout_tree,
                        scroll_offsets,
                        child_clip,
                        child_offset,
                        ops,
                        child,
                    );
                }
            }

            if clips {
                if ops.len() == clip_op_idx + 1 {
                    // Elide redundant PushClip/PopClip around non-drawing subtrees.
                    ops.pop();
                } else {
                    ops.push(PaintOp::PopClip);
                }
            }
        }
    }

    if pushed_opacity {
        if ops.len() == opacity_op_idx + 1 {
            // Elide redundant SaveOpacity/Restore around non-drawing subtrees.
            ops.pop();
        } else {
            ops.push(PaintOp::Restore);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{CommandBufferBuilder, Dimension, EdgeInsets, FlexDirection, LayoutProps, StyleProps};

    #[test]
    fn emits_retained_display_list_from_command_buffer() {
        let mut skia = SkiaBackend::new();
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(200.0),
                height: Dimension::Points(80.0),
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgb(30, 41, 59),
                border_radius: 12.0,
                ..Default::default()
            },
        );
        b.create_node(3, ComponentKind::Text, None);
        b.set_text(3, "Skia Ready");
        b.set_children(2, &[3]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);

        skia.submit_frame(b.finish()).unwrap();
        assert_eq!(skia.display_list().ops.len(), 2);
    }

    #[test]
    fn culls_zero_opacity_empty_opacity_groups_and_offscreen_nodes() {
        let mut skia = SkiaBackend::new();
        skia.set_viewport(
            1,
            Viewport {
                width: 200.0,
                height: 200.0,
                scale_factor: 1.0,
            },
        );

        let mut b = CommandBufferBuilder::new(1, 1);
        // Node 2: opacity 0.0 -> whole subtree (including child 3) must be culled.
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(80.0),
                height: Dimension::Points(40.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgb(255, 0, 0),
                opacity: 0.0,
                ..Default::default()
            },
        );
        b.create_node(3, ComponentKind::Text, None);
        b.set_text(3, "Hidden");
        b.set_children(2, &[3]);

        // Node 4: opacity 0.5 but transparent background and no drawable children -> elided.
        b.create_node(4, ComponentKind::View, None);
        b.set_layout(
            4,
            &LayoutProps {
                width: Dimension::Points(80.0),
                height: Dimension::Points(40.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
        );
        b.set_style(
            4,
            &StyleProps {
                background: ColorRgba::TRANSPARENT,
                opacity: 0.5,
                ..Default::default()
            },
        );

        // Node 5: visible card inside viewport.
        b.create_node(5, ComponentKind::View, None);
        b.set_layout(
            5,
            &LayoutProps {
                width: Dimension::Points(100.0),
                height: Dimension::Points(50.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
        );
        b.set_style(
            5,
            &StyleProps {
                background: ColorRgba::rgb(16, 185, 129),
                ..Default::default()
            },
        );

        // Node 6: pushed past y=200 via margin -> offscreen, culled.
        b.create_node(6, ComponentKind::View, None);
        b.set_layout(
            6,
            &LayoutProps {
                width: Dimension::Points(100.0),
                height: Dimension::Points(50.0),
                flex_shrink: 0.0,
                margin: EdgeInsets {
                    top: 250.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        b.set_style(
            6,
            &StyleProps {
                background: ColorRgba::rgb(239, 68, 68),
                ..Default::default()
            },
        );

        b.set_children(ROOT_NODE_ID, &[2, 4, 5, 6]);
        b.commit(true);
        skia.submit_frame(b.finish()).unwrap();

        // Only node 5's FillRoundedRect should survive culling.
        assert_eq!(skia.display_list().ops.len(), 1);
        match &skia.display_list().ops[0] {
            PaintOp::FillRoundedRect { color, .. } => {
                assert_eq!(*color, ColorRgba::rgb(16, 185, 129));
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }

    #[test]
    fn rasterizes_rounded_rect_corners_stroke_and_opacity_stack() {
        let list = DisplayList {
            ops: vec![
                // Solid rounded card at (20, 20, 80x80) with radius 20.
                PaintOp::FillRoundedRect {
                    rect: LayoutRect {
                        x: 20.0,
                        y: 20.0,
                        width: 80.0,
                        height: 80.0,
                    },
                    radius: 20.0,
                    color: ColorRgba::rgb(37, 99, 235),
                },
                // White 2px stroke around a box at (120, 20, 60x60).
                PaintOp::StrokeRoundedRect {
                    rect: LayoutRect {
                        x: 120.0,
                        y: 20.0,
                        width: 60.0,
                        height: 60.0,
                    },
                    radius: 8.0,
                    width: 2.0,
                    color: ColorRgba::WHITE,
                },
                // 50% opacity red square over black background -> ~(128, 0, 0).
                PaintOp::SaveOpacity(0.5),
                PaintOp::FillRoundedRect {
                    rect: LayoutRect {
                        x: 20.0,
                        y: 120.0,
                        width: 40.0,
                        height: 40.0,
                    },
                    radius: 0.0,
                    color: ColorRgba::rgb(255, 0, 0),
                },
                PaintOp::Restore,
                // After Restore, opacity returns to 1.0 -> full green (0, 255, 0).
                PaintOp::FillRoundedRect {
                    rect: LayoutRect {
                        x: 80.0,
                        y: 120.0,
                        width: 40.0,
                        height: 40.0,
                    },
                    radius: 0.0,
                    color: ColorRgba::rgb(0, 255, 0),
                },
            ],
        };

        let mut buf = PixelBuffer::new(200, 180);
        rasterize_display_list(&list, &mut buf, 1.0, ColorRgba::BLACK);

        // Center of rounded rect is exact blue.
        assert_eq!(buf.get_rgb(60, 60), Some((37, 99, 235)));
        // Far outer corner pixel (20, 20) of radius-20 rounded rect is clipped out -> black.
        assert_eq!(buf.get_rgb(20, 20), Some((0, 0, 0)));

        // Stroke top border pixel is white, hollow center remains black.
        assert_eq!(buf.get_rgb(150, 20), Some((255, 255, 255)));
        assert_eq!(buf.get_rgb(150, 50), Some((0, 0, 0)));

        // 50% opacity red over black.
        let (r, g, b) = buf.get_rgb(40, 140).unwrap();
        assert!((126..=130).contains(&r), "expected ~128 red, got {r}");
        assert_eq!((g, b), (0, 0));

        // Restored full opacity green.
        assert_eq!(buf.get_rgb(100, 140), Some((0, 255, 0)));
    }

    #[test]
    fn rasterizes_full_ui_tree_with_text_and_expected_color_placement() {
        let mut skia = SkiaBackend::new();
        skia.set_viewport(
            1,
            Viewport {
                width: 320.0,
                height: 200.0,
                scale_factor: 1.0,
            },
        );

        let mut b = CommandBufferBuilder::new(1, 1);
        // Root container (#0f172a) with padding 16
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Percent(100.0),
                height: Dimension::Percent(100.0),
                padding: EdgeInsets::all(16.0),
                gap: 12.0,
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgb(15, 23, 42),
                ..Default::default()
            },
        );

        // Text header (#38bdf8)
        b.create_node(3, ComponentKind::Text, None);
        b.set_style(
            3,
            &StyleProps {
                text_color: ColorRgba::rgb(56, 189, 248),
                font_size: 20.0,
                ..Default::default()
            },
        );
        b.set_text(3, "HELLO NVE");

        // Blue button (#2563eb)
        b.create_node(4, ComponentKind::Pressable, None);
        b.set_layout(
            4,
            &LayoutProps {
                width: Dimension::Points(140.0),
                height: Dimension::Points(48.0),
                ..Default::default()
            },
        );
        b.set_style(
            4,
            &StyleProps {
                background: ColorRgba::rgb(37, 99, 235),
                border_radius: 10.0,
                ..Default::default()
            },
        );

        b.set_children(2, &[3, 4]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);

        skia.submit_frame(b.finish()).unwrap();

        let mut buf = PixelBuffer::new(320, 200);
        skia.rasterize(&mut buf, ColorRgba::BLACK);

        // Background color in padding margin.
        assert_eq!(buf.get_rgb(4, 4), Some((15, 23, 42)));

        // Verify text rendered non-empty cyan pixels inside node 3's rect.
        let text_rect = skia.layout_of(3).unwrap();
        let mut cyan_pixels = 0usize;
        for y in (text_rect.y as u32)..((text_rect.y + text_rect.height) as u32).min(200) {
            for x in (text_rect.x as u32)..((text_rect.x + text_rect.width) as u32).min(320) {
                if buf.get_rgb(x, y) == Some((56, 189, 248)) {
                    cyan_pixels += 1;
                }
            }
        }
        assert!(
            cyan_pixels > 40,
            "expected crisp glyph pixels for 'HELLO NVE', found {cyan_pixels}"
        );

        // Verify blue button center pixel matches #2563eb.
        let btn_rect = skia.layout_of(4).unwrap();
        let bx = (btn_rect.x + btn_rect.width * 0.5) as u32;
        let by = (btn_rect.y + btn_rect.height * 0.5) as u32;
        assert_eq!(buf.get_rgb(bx, by), Some((37, 99, 235)));
    }

    #[test]
    fn sorts_children_by_z_index_in_display_list() {
        let mut skia = SkiaBackend::new();
        skia.set_viewport(
            1,
            Viewport {
                width: 200.0,
                height: 200.0,
                scale_factor: 1.0,
            },
        );
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(50.0),
                height: Dimension::Points(50.0),
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgb(255, 0, 0),
                z_index: 10,
                ..Default::default()
            },
        );
        b.create_node(3, ComponentKind::View, None);
        b.set_layout(
            3,
            &LayoutProps {
                width: Dimension::Points(50.0),
                height: Dimension::Points(50.0),
                ..Default::default()
            },
        );
        b.set_style(
            3,
            &StyleProps {
                background: ColorRgba::rgb(0, 0, 255),
                z_index: 1,
                ..Default::default()
            },
        );
        b.set_children(ROOT_NODE_ID, &[2, 3]);
        b.commit(true);
        skia.submit_frame(b.finish()).unwrap();

        assert_eq!(skia.display_list().ops.len(), 2);
        // z_index 1 (blue) paints first, z_index 10 (red) paints on top.
        match (&skia.display_list().ops[0], &skia.display_list().ops[1]) {
            (
                PaintOp::FillRoundedRect { color: c0, .. },
                PaintOp::FillRoundedRect { color: c1, .. },
            ) => {
                assert_eq!(*c0, ColorRgba::rgb(0, 0, 255));
                assert_eq!(*c1, ColorRgba::rgb(255, 0, 0));
            }
            other => panic!("unexpected ops: {other:?}"),
        }
    }

    struct FixedOffsetLayout {
        x_offset: f32,
        y_step: f32,
        children: std::collections::HashMap<NodeId, Vec<NodeId>>,
        kinds: std::collections::HashMap<NodeId, ComponentKind>,
        texts: std::collections::HashMap<NodeId, String>,
        font_sizes: std::collections::HashMap<NodeId, f32>,
        rects: std::collections::HashMap<NodeId, LayoutRect>,
        measurer: Arc<dyn TextMeasurer>,
    }

    impl FixedOffsetLayout {
        fn new(x_offset: f32, y_step: f32) -> Self {
            Self {
                x_offset,
                y_step,
                children: std::collections::HashMap::new(),
                kinds: std::collections::HashMap::new(),
                texts: std::collections::HashMap::new(),
                font_sizes: std::collections::HashMap::new(),
                rects: std::collections::HashMap::new(),
                measurer: Arc::new(BitmapFontTextMeasurer),
            }
        }

        fn place(&mut self, id: NodeId, depth: usize, slot: usize) {
            let is_text = self.kinds.get(&id) == Some(&ComponentKind::Text);
            let (w, h) = if is_text {
                let text = self.texts.get(&id).map(String::as_str).unwrap_or("");
                let fs = self.font_sizes.get(&id).copied().unwrap_or(16.0);
                self.measurer.measure(text, fs, None)
            } else {
                (90.0, 36.0)
            };
            self.rects.insert(
                id,
                LayoutRect {
                    x: self.x_offset + (depth as f32) * 10.0,
                    y: (slot as f32) * self.y_step,
                    width: w,
                    height: h,
                },
            );
            if let Some(kids) = self.children.get(&id).cloned() {
                for (i, child) in kids.into_iter().enumerate() {
                    self.place(child, depth + 1, slot + i + 1);
                }
            }
        }
    }

    impl LayoutEngine for FixedOffsetLayout {
        fn name(&self) -> &'static str {
            "fixed-offset"
        }

        fn upsert_node(&mut self, id: NodeId, kind: ComponentKind) {
            self.kinds.insert(id, kind);
        }

        fn remove_node(&mut self, id: NodeId) {
            self.kinds.remove(&id);
            self.children.remove(&id);
            self.texts.remove(&id);
            self.font_sizes.remove(&id);
            self.rects.remove(&id);
        }

        fn set_layout(&mut self, _id: NodeId, _layout: &LayoutProps) {}

        fn set_style(&mut self, id: NodeId, style: &StyleProps) {
            self.font_sizes.insert(id, style.font_size);
        }

        fn set_text(&mut self, id: NodeId, text: &str) {
            self.texts.insert(id, text.to_owned());
        }

        fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
            self.children.insert(parent, children.to_vec());
        }

        fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
            self.measurer = measurer;
        }

        fn compute_layout(&mut self, root: NodeId, viewport: Viewport) {
            self.rects.clear();
            self.place(root, 0, 0);
            if let Some(r) = self.rects.get_mut(&root) {
                r.x = 0.0;
                r.y = 0.0;
                r.width = viewport.width;
                r.height = viewport.height;
            }
        }

        fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
            self.rects.get(&id).copied()
        }

        fn reset(&mut self) {
            self.children.clear();
            self.kinds.clear();
            self.texts.clear();
            self.font_sizes.clear();
            self.rects.clear();
        }
    }

    #[test]
    fn skia_supports_custom_layout_dyn_box_swap_and_custom_text_measurer() {
        use nve_abi::{EventMask, FlexAlign};
        use nve_engine::DynLayoutEngine;

        let viewport = Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        };

        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::Pressable, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(100.0),
                height: Dimension::Points(50.0),
                align_items: FlexAlign::FlexStart,
                margin: EdgeInsets {
                    left: 10.0,
                    top: 10.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgb(37, 99, 235),
                ..Default::default()
            },
        );
        b.set_event_mask(2, EventMask::PRESS);

        b.create_node(3, ComponentKind::Text, None);
        b.set_style(
            3,
            &StyleProps {
                text_color: ColorRgba::WHITE,
                font_size: 16.0,
                ..Default::default()
            },
        );
        b.set_text(3, "Swap");

        b.set_children(2, &[3]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        let cmd = b.into_bytes();

        // Start with runtime-switchable DynLayoutEngine (Box<dyn LayoutEngine>) wrapping Taffy.
        let mut skia: SkiaBackend<DynLayoutEngine> =
            SkiaBackend::with_layout(Box::new(TaffyLayoutEngine::new()) as DynLayoutEngine);
        skia.set_viewport(1, viewport);
        skia.set_text_measurer(Arc::new(|_text: &str, _fs: f32, _mw: Option<f32>| {
            (77.0, 29.0)
        }));
        skia.submit_frame(&cmd).unwrap();

        assert_eq!(skia.layout_engine().name(), "taffy");
        assert_eq!(skia.layout_of(2).unwrap().x, 10.0);
        assert_eq!(skia.layout_of(3).unwrap().width, 77.0);
        assert_eq!(skia.layout_of(3).unwrap().height, 29.0);
        assert_eq!(skia.hit_test(50.0, 30.0), Some(2));

        // Live-swap to FixedOffsetLayout -> display list, hit-testing, raster output, and text measurer all update.
        let swapped = skia.swap_layout_engine(FixedOffsetLayout::new(120.0, 40.0));
        assert_eq!(swapped.layout_engine().name(), "fixed-offset");
        let r2 = swapped.layout_of(2).unwrap();
        assert_eq!(r2.x, 130.0);
        assert_eq!(r2.y, 40.0);
        assert_eq!(swapped.layout_of(3).unwrap().width, 77.0);
        assert_eq!(swapped.layout_of(3).unwrap().height, 29.0);
        assert_eq!(swapped.hit_test(50.0, 30.0), None);
        assert_eq!(swapped.hit_test(150.0, 50.0), Some(2));

        // Verify rasterized blue button moved to the new layout engine's rect.
        let mut buf = PixelBuffer::new(300, 200);
        swapped.rasterize(&mut buf, ColorRgba::BLACK);
        assert_eq!(buf.get_rgb(50, 30), Some((0, 0, 0)));
        assert_eq!(buf.get_rgb(150, 55), Some((37, 99, 235)));
    }

    #[cfg(feature = "yoga")]
    #[test]
    fn skia_new_yoga_matches_availability_and_renders_with_real_rn_yoga() {
        if !YogaLayoutEngine::is_available() {
            let err = SkiaBackend::new_yoga()
                .err()
                .expect("expected YogaUnavailableError when RN_REACT_COMMON_DIR is unset");
            assert!(err.to_string().contains("RN_REACT_COMMON_DIR"));
            return;
        }

        let viewport = Viewport {
            width: 320.0,
            height: 200.0,
            scale_factor: 1.0,
        };
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Percent(100.0),
                height: Dimension::Percent(100.0),
                padding: EdgeInsets::all(16.0),
                gap: 12.0,
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgb(15, 23, 42),
                ..Default::default()
            },
        );
        b.create_node(3, ComponentKind::Pressable, None);
        b.set_layout(
            3,
            &LayoutProps {
                width: Dimension::Points(140.0),
                height: Dimension::Points(48.0),
                ..Default::default()
            },
        );
        b.set_style(
            3,
            &StyleProps {
                background: ColorRgba::rgb(37, 99, 235),
                border_radius: 10.0,
                ..Default::default()
            },
        );
        b.set_children(2, &[3]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        let cmd = b.into_bytes();

        let mut taffy_skia = SkiaBackend::new();
        taffy_skia.set_viewport(1, viewport);
        taffy_skia.submit_frame(&cmd).unwrap();

        let mut yoga_skia = SkiaBackend::new_yoga().expect("real RN Yoga should initialize");
        assert_eq!(yoga_skia.layout_engine().name(), "yoga");
        yoga_skia.set_viewport(1, viewport);
        yoga_skia.submit_frame(&cmd).unwrap();

        for id in [ROOT_NODE_ID, 2, 3] {
            let tr = taffy_skia.layout_of(id).unwrap();
            let yr = yoga_skia.layout_of(id).unwrap();
            assert!((tr.x - yr.x).abs() <= 1.0, "node {id} x: {tr:?} vs {yr:?}");
            assert!((tr.y - yr.y).abs() <= 1.0, "node {id} y: {tr:?} vs {yr:?}");
            assert!(
                (tr.width - yr.width).abs() <= 1.0,
                "node {id} w: {tr:?} vs {yr:?}"
            );
            assert!(
                (tr.height - yr.height).abs() <= 1.0,
                "node {id} h: {tr:?} vs {yr:?}"
            );
        }

        let mut buf = PixelBuffer::new(320, 200);
        yoga_skia.rasterize(&mut buf, ColorRgba::BLACK);
        let btn = yoga_skia.layout_of(3).unwrap();
        assert_eq!(
            buf.get_rgb((btn.x + 20.0) as u32, (btn.y + 20.0) as u32),
            Some((37, 99, 235))
        );
    }

    const STUDIO_AVATAR_MAYA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEWBxd4fKHggFOB1JD4MAKjRfBnTmv6zAAAAAElFTkSuQmCC";

    #[test]
    fn renders_studio_png_avatar_under_all_four_resize_modes_and_rounded_clip() {
        let viewport = Viewport {
            width: 120.0,
            height: 80.0,
            scale_factor: 1.0,
        };

        for mode in [
            ImageResizeMode::Cover,
            ImageResizeMode::Contain,
            ImageResizeMode::Stretch,
            ImageResizeMode::Center,
        ] {
            let mut skia = SkiaBackend::new();
            skia.set_viewport(1, viewport);

            let mut b = CommandBufferBuilder::new(1, 1);
            b.create_node(2, ComponentKind::Image, None);
            b.set_layout(
                2,
                &LayoutProps {
                    width: Dimension::Points(80.0),
                    height: Dimension::Points(40.0),
                    margin: EdgeInsets {
                        left: 10.0,
                        top: 10.0,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            b.set_style(
                2,
                &StyleProps {
                    border_radius: 12.0,
                    ..Default::default()
                },
            );
            b.set_raw_prop(2, "src", &RawPropValue::String(STUDIO_AVATAR_MAYA.to_string()));
            b.set_raw_prop(
                2,
                "resizeMode",
                &RawPropValue::String(mode.as_str().to_string()),
            );
            b.set_children(ROOT_NODE_ID, &[2]);
            b.commit(true);

            skia.submit_frame(b.finish()).unwrap();
            assert!(
                skia.display_list()
                    .ops
                    .iter()
                    .any(|op| matches!(op, PaintOp::DrawImage { resize_mode, .. } if *resize_mode == mode)),
                "expected DrawImage op for {mode:?}"
            );

            let mut buf = PixelBuffer::new(120, 80);
            skia.rasterize(&mut buf, ColorRgba::BLACK);

            let mut non_black_inside = 0usize;
            for y in 10..50 {
                for x in 10..90 {
                    if buf.get_rgb(x, y) != Some((0, 0, 0)) {
                        non_black_inside += 1;
                    }
                }
            }
            assert!(
                non_black_inside > 0,
                "mode {mode:?}: expected non-zero avatar pixels inside image rect"
            );

            // Far outer corner at (10, 10) must stay clipped to black due to border_radius=12.
            assert_eq!(
                buf.get_rgb(10, 10),
                Some((0, 0, 0)),
                "mode {mode:?}: rounded corner should clip pixel at (10, 10)"
            );
            // Outside image bounds must remain untouched black.
            assert_eq!(buf.get_rgb(5, 5), Some((0, 0, 0)));
            assert_eq!(buf.get_rgb(105, 60), Some((0, 0, 0)));

            // Contain leaves horizontal letterbox bars black on left/right edges of 80x40 box
            // because the avatar is 8x8 square (centered 40x40 inside 80x40 -> x=30..70).
            if mode == ImageResizeMode::Contain {
                assert_eq!(buf.get_rgb(15, 30), Some((0, 0, 0)));
                assert_eq!(buf.get_rgb(85, 30), Some((0, 0, 0)));
            }
            // Center places the 8x8 image at center (46..54, 26..34), leaving (20, 30) black.
            if mode == ImageResizeMode::Center {
                assert_eq!(buf.get_rgb(20, 30), Some((0, 0, 0)));
                assert!(non_black_inside <= 64, "Center 8x8 must paint <= 64 pixels, got {non_black_inside}");
            }
        }
    }

    #[test]
    fn executes_nve_draw1_sparkline_primitives_and_survives_corrupt_buffers() {
        use nve_components::canvas_ir::DrawListBuilder;

        let viewport = Viewport {
            width: 340.0,
            height: 160.0,
            scale_factor: 1.0,
        };

        // 1. Real telemetry sparkline + FillCircle + StrokeLine + ClipRect/RestoreClip.
        let stroke_cyan = ColorRgba::rgb(56, 189, 248);
        let bg_slate = ColorRgba::rgb(17, 24, 39);
        let dot_green = ColorRgba::rgb(16, 185, 129);
        let line_red = ColorRgba::rgb(239, 68, 68);

        let mut draw = DrawListBuilder::new();
        draw.sparkline(&[120.0, 180.0, 95.0, 240.0, 210.0, 310.0], 320.0, 72.0, stroke_cyan, bg_slate);
        draw.fill_circle(40.0, 110.0, 12.0, dot_green);
        draw.stroke_line(80.0, 110.0, 180.0, 110.0, 4.0, line_red);
        // ClipRect restricts the yellow FillRect to x=200..230 even though rect spans x=200..300.
        draw.clip_rect(200.0, 95.0, 30.0, 30.0);
        draw.fill_rect(200.0, 95.0, 100.0, 30.0, ColorRgba::rgb(250, 204, 21));
        draw.restore_clip();
        let draw_bytes = draw.finish();

        let mut skia = SkiaBackend::new();
        skia.set_viewport(1, viewport);

        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::SkiaCanvas, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(320.0),
                height: Dimension::Points(140.0),
                margin: EdgeInsets {
                    left: 10.0,
                    top: 10.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        b.set_raw_prop(2, "drawList", &RawPropValue::Bytes(draw_bytes));
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);

        skia.submit_frame(b.finish()).unwrap();

        let mut buf = PixelBuffer::new(340, 160);
        skia.rasterize(&mut buf, ColorRgba::BLACK);

        // Sparkline background fills inside (10..330, 10..82).
        assert_eq!(buf.get_rgb(20, 20), Some((17, 24, 39)));

        // Sparkline cyan polyline + terminal dot painted inside the sparkline band.
        let mut cyan_hits = 0usize;
        for y in 10..82 {
            for x in 10..330 {
                let (r, g, b) = buf.get_rgb(x, y).unwrap();
                if r <= 80 && g >= 160 && b >= 220 {
                    cyan_hits += 1;
                }
            }
        }
        assert!(
            cyan_hits >= 40,
            "expected cyan sparkline stroke + dot pixels, found {cyan_hits}"
        );

        // FillCircle center at canvas (40, 110) -> buffer (50, 120).
        assert_eq!(buf.get_rgb(50, 120), Some((16, 185, 129)));
        // StrokeLine midpoint at canvas (130, 110) -> buffer (140, 120).
        assert_eq!(buf.get_rgb(140, 120), Some((239, 68, 68)));
        // ClipRect allows yellow at canvas (215, 110) -> buffer (225, 120),
        // and clips out canvas (260, 110) -> buffer (270, 120) leaving it black.
        assert_eq!(buf.get_rgb(225, 120), Some((250, 204, 21)));
        assert_eq!(buf.get_rgb(270, 120), Some((0, 0, 0)));

        // 2. Corrupt & truncated NVE_DRAW1 buffers never panic.
        for bad in [
            RawPropValue::Bytes(vec![]),
            RawPropValue::Bytes(b"NVE_DRAW1".to_vec()),
            RawPropValue::Bytes(b"BAD_MAGIC_1234567890".to_vec()),
            RawPropValue::Bytes(vec![b'N', b'V', b'E', b'_', b'D', b'R', b'A', b'W', b'1', 1, 0, 255, 255, 0, 0]),
            RawPropValue::String("nve-draw1;b64,!!!invalid!!!".to_string()),
        ] {
            let mut b = CommandBufferBuilder::new(1, 2);
            b.set_raw_prop(2, "drawList", &bad);
            b.commit(false);
            skia.submit_frame(b.finish()).unwrap();
            let mut tmp = PixelBuffer::new(340, 160);
            skia.rasterize(&mut tmp, ColorRgba::BLACK);
        }
    }

    #[test]
    fn scrollview_clips_overflow_shifts_pixels_and_updates_hit_testing_on_scroll() {
        use nve_abi::EventMask;

        let viewport = Viewport {
            width: 200.0,
            height: 200.0,
            scale_factor: 1.0,
        };
        let mut skia = SkiaBackend::new();
        skia.set_viewport(1, viewport);

        // ScrollView at (10, 10, 160x80) containing 3 stacked 60px pressable rows (total height 180px):
        // Row 10: red   (y=0..60)
        // Row 11: green (y=60..120)
        // Row 12: blue  (y=120..180)
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::ScrollView, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(160.0),
                height: Dimension::Points(80.0),
                flex_direction: FlexDirection::Column,
                margin: EdgeInsets {
                    left: 10.0,
                    top: 10.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        b.set_event_mask(2, EventMask::SCROLL);

        for (id, color) in [
            (10u32, ColorRgba::rgb(239, 68, 68)),
            (11u32, ColorRgba::rgb(16, 185, 129)),
            (12u32, ColorRgba::rgb(37, 99, 235)),
        ] {
            b.create_node(id, ComponentKind::Pressable, None);
            b.set_layout(
                id,
                &LayoutProps {
                    width: Dimension::Points(160.0),
                    height: Dimension::Points(60.0),
                    flex_shrink: 0.0,
                    ..Default::default()
                },
            );
            b.set_style(
                id,
                &StyleProps {
                    background: color,
                    ..Default::default()
                },
            );
            b.set_event_mask(id, EventMask::PRESS);
        }
        b.set_children(2, &[10, 11, 12]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        skia.submit_frame(b.finish()).unwrap();

        // Verify PushClip/PopClip emitted for ScrollView.
        assert!(skia.display_list().ops.iter().any(|op| matches!(op, PaintOp::PushClip { .. })));
        assert!(skia.display_list().ops.iter().any(|op| matches!(op, PaintOp::PopClip)));
        assert_eq!(skia.hit_test_scroll_view(50.0, 50.0), Some(2));
        assert_eq!(skia.hit_test_scroll_view(190.0, 190.0), None);

        // At scroll=0:
        // - y=30 (inside viewport 10..90) hits Row 10 (red).
        // - y=80 (inside viewport 10..90) hits Row 11 (green).
        // - y=110 (below viewport bottom 90) must be CLIPPED (black pixel and no hit!).
        assert_eq!(skia.hit_test(50.0, 30.0), Some(10));
        assert_eq!(skia.hit_test(50.0, 80.0), Some(11));
        assert_eq!(skia.hit_test(50.0, 110.0), None);

        let mut buf = PixelBuffer::new(200, 200);
        skia.rasterize(&mut buf, ColorRgba::BLACK);
        assert_eq!(buf.get_rgb(50, 30), Some((239, 68, 68)));
        assert_eq!(buf.get_rgb(50, 80), Some((16, 185, 129)));
        assert_eq!(buf.get_rgb(50, 110), Some((0, 0, 0))); // clipped!

        // Scroll down by 90px (clamped against 180 - 80 = 100px max):
        // Row 10 is now at y=10-90=-80..-20 (completely clipped above).
        // Row 11 is at y=70-90=-20..40 (visible in y=10..40).
        // Row 12 is at y=130-90=40..100 (visible in y=40..90, clipped at 90).
        let (sx, sy) = skia.apply_scroll_delta(2, 0.0, 90.0);
        assert_eq!((sx, sy), (0.0, 90.0));

        assert_eq!(skia.hit_test(50.0, 25.0), Some(11));
        assert_eq!(skia.hit_test(50.0, 60.0), Some(12));
        assert_eq!(skia.hit_test(50.0, 95.0), None); // clipped below viewport

        skia.rasterize(&mut buf, ColorRgba::BLACK);
        assert_eq!(buf.get_rgb(50, 25), Some((16, 185, 129))); // green shifted up
        assert_eq!(buf.get_rgb(50, 60), Some((37, 99, 235)));  // blue scrolled into view
        assert_eq!(buf.get_rgb(50, 5), Some((0, 0, 0)));       // nothing leaks above
        assert_eq!(buf.get_rgb(50, 95), Some((0, 0, 0)));      // nothing leaks below

        // Over-scrolling clamps cleanly to content_height - viewport_height (100px), and negative clamps to 0.
        assert_eq!(skia.apply_scroll_delta(2, 0.0, 500.0), (0.0, 100.0));
        assert_eq!(skia.apply_scroll_delta(2, 0.0, -999.0), (0.0, 0.0));
    }
}
