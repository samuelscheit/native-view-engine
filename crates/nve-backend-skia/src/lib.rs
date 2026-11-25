//! Cross-platform Skia RenderBackend targeting macOS, Windows, Linux, iOS, and Android.
//!
//! Consumes the exact same `nve-abi` binary `CommandBuffer` as the React Native Fabric backend,
//! computes Flexbox layout, compiles a culled retained `DisplayList`, rasterizes cleanly via a
//! zero-dependency software rasterizer (and `--features desktop` `winit` + `softbuffer` live
//! preview window with 2-tier WebSocket hot reload), and (with `--features skia`) paints directly
//! onto a `skia_safe::Canvas`.

pub mod font;
pub mod gpu;
pub mod raster;
pub mod text;
#[cfg(feature = "desktop")]
pub mod window;

use std::sync::Arc;
use std::time::Instant;

use nve_abi::{ColorRgba, ComponentKind, NativeEvent, NodeId, SurfaceId, ROOT_NODE_ID};
use nve_backend_headless::{HeadlessBackend, TaffyLayoutEngine};
use nve_engine::{
    BackendApplyStats, BackendError, LayoutEngine, LayoutRect, RenderBackend, TextMeasurer,
    Viewport,
};

pub use font::{bitmap_font_metrics, BitmapFontMetrics, BitmapFontTextMeasurer};
pub use gpu::{GpuBackendKind, GpuSurfaceConfig, GpuSurfaceError};
#[cfg(feature = "skia")]
pub use gpu::{CpuSkiaSurface, GpuSurface, GpuSurfaceBackend, SkiaCanvasRenderer};
#[cfg(feature = "yoga")]
pub use nve_layout_yoga::{YogaLayoutEngine, YogaUnavailableError};
pub use raster::{
    draw_hud_banner, rasterize_display_list, rasterize_display_list_bitmap,
    rasterize_display_list_with_text_engine, PixelBuffer,
};
pub use text::CosmicTextEngine;
#[cfg(feature = "desktop")]
pub use window::{
    run_desktop_preview, run_desktop_preview_with_options, DesktopApp, DesktopHostError,
    DesktopPreviewOptions, DesktopSession, DesktopWindowHost, HotAppSlot, HotReloadClient,
    DEFAULT_WS_URL,
};

#[derive(Debug, Clone, PartialEq)]
pub enum PaintOp {
    SaveOpacity(f32),
    Restore,
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
    CustomSkiaCanvas {
        node_id: NodeId,
        rect: LayoutRect,
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
        let mut layout_tree = HeadlessBackend::with_layout(layout);
        layout_tree.set_text_measurer(Arc::clone(&text_engine) as Arc<dyn TextMeasurer>);
        Self {
            layout_tree,
            display_list: DisplayList::default(),
            viewport: Viewport::default(),
            text_engine,
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
        };
        swapped.rebuild_display_list();
        swapped
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn display_list(&self) -> &DisplayList {
        &self.display_list
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        self.layout_tree.hit_test(x, y, true)
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

    fn intersects_viewport(&self, rect: LayoutRect, outset: f32) -> bool {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return false;
        }
        let x0 = rect.x - outset;
        let y0 = rect.y - outset;
        let x1 = rect.x + rect.width + outset;
        let y1 = rect.y + rect.height + outset;
        x1 > 0.0 && y1 > 0.0 && x0 < self.viewport.width && y0 < self.viewport.height
    }

    fn rebuild_display_list(&mut self) {
        self.display_list.ops.clear();
        self.emit_node(ROOT_NODE_ID);
    }

    fn emit_node(&mut self, id: NodeId) {
        let Some(node) = self.layout_tree.node(id).cloned() else {
            return;
        };

        let opacity = node.style.opacity.clamp(0.0, 1.0);
        if opacity <= 0.001 {
            // Fully transparent subtree produces no visible pixels — cull completely.
            return;
        }

        let pushed_opacity = opacity < 0.999;
        let opacity_op_idx = self.display_list.ops.len();
        if pushed_opacity {
            self.display_list.ops.push(PaintOp::SaveOpacity(opacity));
        }

        if node.style.background.a > 0 && self.intersects_viewport(node.computed_rect, 0.0) {
            self.display_list.ops.push(PaintOp::FillRoundedRect {
                rect: node.computed_rect,
                radius: node.style.border_radius.max(0.0),
                color: node.style.background,
            });
        }
        if node.style.border_width > 0.0
            && node.style.border_color.a > 0
            && self.intersects_viewport(node.computed_rect, node.style.border_width * 0.5)
        {
            self.display_list.ops.push(PaintOp::StrokeRoundedRect {
                rect: node.computed_rect,
                radius: node.style.border_radius.max(0.0),
                width: node.style.border_width,
                color: node.style.border_color,
            });
        }
        if let Some(text) = node.text {
            if !text.is_empty()
                && node.style.font_size > 0.0
                && node.style.text_color.a > 0
                && self.intersects_viewport(node.computed_rect, 0.0)
            {
                self.display_list.ops.push(PaintOp::DrawText {
                    rect: node.computed_rect,
                    text,
                    font_size: node.style.font_size,
                    color: node.style.text_color,
                });
            }
        }
        if node.kind == ComponentKind::SkiaCanvas
            && self.intersects_viewport(node.computed_rect, 0.0)
        {
            self.display_list.ops.push(PaintOp::CustomSkiaCanvas {
                node_id: id,
                rect: node.computed_rect,
            });
        }

        // Paint children in z_index order when any child specifies a non-zero z_index.
        let mut children = node.children;
        let has_z_order = children.iter().any(|cid| {
            self.layout_tree
                .node(*cid)
                .is_some_and(|c| c.style.z_index != 0)
        });
        if has_z_order {
            children.sort_by_key(|cid| {
                self.layout_tree
                    .node(*cid)
                    .map_or(0, |c| c.style.z_index)
            });
        }
        for child in children {
            self.emit_node(child);
        }

        if pushed_opacity {
            if self.display_list.ops.len() == opacity_op_idx + 1 {
                // Elide redundant SaveOpacity/Restore around non-drawing subtrees.
                self.display_list.ops.pop();
            } else {
                self.display_list.ops.push(PaintOp::Restore);
            }
        }
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
        } else if cfg!(feature = "desktop") {
            "skia-desktop-soft"
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
}
