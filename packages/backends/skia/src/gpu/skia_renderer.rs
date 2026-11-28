//! Skia `Canvas` display-list renderer (`--features skia`) with unified `CosmicTextEngine`
//! `TextBlob` shaping and per-presenter bounded `SlabLru` GPU/CPU caches.

use std::cell::RefCell;
use std::sync::Arc;

use nve_abi::{ColorRgba, EventBatchBuf};
use nve_store::NodeStore;
#[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
use skia_safe::gpu::DirectContext;
use skia_safe::{Canvas, Color, FontMgr, Matrix, Paint, PaintStyle, RRect, Rect};

#[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
pub(crate) use super::cpu_surface::read_surface_to_pixel_buffer;
pub use super::cpu_surface::{CpuSkiaSurface, GpuSurfaceBackend};
pub use super::renderer_cache::RendererCacheStats;
use super::renderer_cache::SkiaRendererCaches;
use super::sanitize_scale_factor;
use super::skia_graphics::draw_rounded_rect_gradient;
use super::skia_primitives::{
    apply_clip, draw_bitmap_text_skia, draw_fill_rrect, draw_image_cached, draw_plugin_cached,
    draw_shadow_cached, draw_stroke_rrect, draw_text_cached, FramePaints,
};
use crate::table_system::SkiaTableRegistry;
use crate::text::{CosmicTextEngine, TextAlign};
use crate::{DisplayList, PaintOp};

#[inline]
pub(crate) fn to_skia_color(c: ColorRgba) -> Color {
    Color::from_argb(c.a, c.r, c.g, c.b)
}

/// Paints an NVE retained [`DisplayList`] (or retained scene) directly onto any
/// [`skia_safe::Canvas`], sharing `CosmicTextEngine` HarfBuzz shaping with the software
/// rasterizer and caching `Typeface`, `TextBlob`, `Image`, plugin textures, and blur
/// `MaskFilter`s per presenter via bounded `SlabLru`s.
pub struct SkiaCanvasRenderer {
    font_mgr: FontMgr,
    text_engine: Arc<CosmicTextEngine>,
    caches: RefCell<SkiaRendererCaches>,
}

impl Default for SkiaCanvasRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SkiaCanvasRenderer {
    pub fn new() -> Self {
        Self::with_text_engine(CosmicTextEngine::shared())
    }

    pub fn with_text_engine(text_engine: Arc<CosmicTextEngine>) -> Self {
        Self {
            font_mgr: FontMgr::new(),
            text_engine,
            caches: RefCell::new(SkiaRendererCaches::new()),
        }
    }

    pub fn font_mgr(&self) -> &FontMgr {
        &self.font_mgr
    }

    pub fn text_engine(&self) -> &Arc<CosmicTextEngine> {
        &self.text_engine
    }

    /// Returns a snapshot of cache hit/miss/upload/eviction counters and live byte footprint.
    pub fn cache_stats(&self) -> RendererCacheStats {
        self.caches.borrow().stats()
    }

    /// Resets hit/miss/upload counters without evicting live cached entries.
    pub fn reset_cache_stats(&self) {
        self.caches.borrow_mut().reset_stats();
    }

    /// Immediately drops all GPU-backed textures retained by this renderer.
    pub fn evict_gpu_caches(&self) {
        self.caches.borrow_mut().evict_gpu_entries();
    }

    /// Drops all cached typefaces, text blobs, images, plugins, and blur filters.
    pub fn clear_caches(&self) {
        self.caches.borrow_mut().clear_all();
    }

    pub(crate) fn paint_display_list_opt_plugins(
        &self,
        display_list: &DisplayList,
        canvas: &Canvas,
        scale_factor: f32,
        plugins: Option<super::SkiaPluginContext<'_>>,
    ) {
        self.paint_display_list_inner(
            display_list,
            canvas,
            scale_factor,
            plugins,
            #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
            None,
        );
    }

    #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
    pub(crate) fn paint_display_list_with_gpu_context(
        &self,
        display_list: &DisplayList,
        canvas: &Canvas,
        scale_factor: f32,
        plugins: Option<super::SkiaPluginContext<'_>>,
        direct_context: Option<&mut DirectContext>,
    ) {
        self.paint_display_list_inner(display_list, canvas, scale_factor, plugins, direct_context);
    }

    pub(crate) fn paint_scene_opt_plugins(
        &self,
        scene: &crate::retained::CompositorScene,
        canvas: &Canvas,
        scale_factor: f32,
        plugins: Option<super::SkiaPluginContext<'_>>,
    ) {
        let mut caches = self.caches.borrow_mut();
        crate::retained::paint_scene_onto_skia_canvas(
            scene,
            canvas,
            &self.font_mgr,
            &self.text_engine,
            &mut caches,
            scale_factor,
            plugins,
            #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
            None,
        );
    }

    #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
    pub(crate) fn paint_scene_with_gpu_context(
        &self,
        scene: &crate::retained::CompositorScene,
        canvas: &Canvas,
        scale_factor: f32,
        plugins: Option<super::SkiaPluginContext<'_>>,
        direct_context: Option<&mut DirectContext>,
    ) {
        let mut caches = self.caches.borrow_mut();
        crate::retained::paint_scene_onto_skia_canvas(
            scene,
            canvas,
            &self.font_mgr,
            &self.text_engine,
            &mut caches,
            scale_factor,
            plugins,
            direct_context,
        );
    }

    fn paint_display_list_inner(
        &self,
        display_list: &DisplayList,
        canvas: &Canvas,
        scale_factor: f32,
        mut plugins: Option<(&NodeStore, &SkiaTableRegistry, &mut EventBatchBuf)>,
        #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
        mut direct_context: Option<&mut DirectContext>,
    ) {
        let scale = sanitize_scale_factor(scale_factor);
        let saved = canvas.save();
        if (scale - 1.0).abs() > f32::EPSILON {
            canvas.scale((scale, scale));
        }

        let mut caches = self.caches.borrow_mut();
        #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
        let context_tag = caches.sync_gpu_context(direct_context.as_deref_mut());
        #[cfg(not(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d")))]
        let context_tag = 0u32;

        let mut paints = FramePaints::new();
        let mut save_depth = 0usize;

        for op in &display_list.ops {
            match op {
                PaintOp::SaveOpacity(alpha) => {
                    let safe_alpha = if alpha.is_finite() {
                        alpha.clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    if safe_alpha >= 0.999 {
                        canvas.save();
                    } else {
                        canvas.save_layer_alpha_f(None, safe_alpha);
                    }
                    save_depth += 1;
                }
                PaintOp::Restore => {
                    if save_depth > 0 {
                        canvas.restore();
                        save_depth -= 1;
                    }
                }
                PaintOp::PushClip { rect, radius } => {
                    canvas.save();
                    save_depth += 1;
                    apply_clip(canvas, *rect, *radius);
                }
                PaintOp::PopClip => {
                    if save_depth > 0 {
                        canvas.restore();
                        save_depth -= 1;
                    }
                }
                PaintOp::PushTransform { matrix } => {
                    canvas.save();
                    save_depth += 1;
                    if matrix.iter().all(|v| v.is_finite()) {
                        let m = Matrix::new_all(
                            matrix[0], matrix[2], matrix[4],
                            matrix[1], matrix[3], matrix[5],
                            0.0, 0.0, 1.0,
                        );
                        canvas.concat(&m);
                    }
                }
                PaintOp::PopTransform => {
                    if save_depth > 0 {
                        canvas.restore();
                        save_depth -= 1;
                    }
                }
                PaintOp::DrawShadow {
                    rect,
                    radius,
                    offset_x,
                    offset_y,
                    blur_radius,
                    spread_radius,
                    color,
                    border_width,
                    inset,
                } => {
                    if *inset {
                        crate::gpu::skia_primitives::draw_inset_shadow_cached(
                            canvas,
                            &mut caches,
                            &mut paints.shadow,
                            *rect,
                            *radius,
                            *border_width,
                            *offset_x,
                            *offset_y,
                            *blur_radius,
                            *spread_radius,
                            *color,
                        );
                    } else {
                        draw_shadow_cached(
                            canvas,
                            &mut caches,
                            &mut paints.shadow,
                            *rect,
                            *radius,
                            *offset_x,
                            *offset_y,
                            *blur_radius,
                            *spread_radius,
                            *color,
                        );
                    }
                }
                PaintOp::FillRoundedRect {
                    rect,
                    radius,
                    color,
                } => {
                    draw_fill_rrect(canvas, &mut paints.fill, *rect, *radius, *color);
                }
                PaintOp::FillRoundedRectGradient {
                    rect,
                    radius,
                    gradient,
                } => {
                    draw_rounded_rect_gradient(
                        canvas,
                        *rect,
                        *radius,
                        gradient,
                        &mut paints.gradient,
                        &mut caches.grad_scratch,
                    );
                }
                PaintOp::StrokeRoundedRect {
                    rect,
                    radius,
                    width,
                    color,
                } => {
                    draw_stroke_rrect(canvas, &mut paints.stroke, *rect, *radius, *width, *color);
                }
                PaintOp::DrawText {
                    rect,
                    text,
                    font_size,
                    font_weight,
                    text_align,
                    color,
                } => {
                    draw_text_cached(
                        canvas,
                        &self.font_mgr,
                        &self.text_engine,
                        &mut caches,
                        &mut paints.text,
                        *rect,
                        text,
                        *font_size,
                        *font_weight,
                        *text_align,
                        *color,
                    );
                }
                PaintOp::DrawImage {
                    rect,
                    radius,
                    src,
                    fit,
                    resolved,
                } => {
                    draw_image_cached(
                        canvas,
                        &mut caches,
                        &paints.image,
                        *rect,
                        *radius,
                        src,
                        *resolved,
                        *fit,
                        context_tag,
                        #[cfg(any(
                            feature = "skia-metal",
                            feature = "skia-vulkan",
                            feature = "skia-d3d"
                        ))]
                        direct_context.as_deref_mut(),
                    );
                }
                PaintOp::Plugin {
                    node_id,
                    rect,
                    inherited_text_color,
                    plugin_epoch,
                } => {
                    if let Some((store, registry, ref mut events)) = plugins {
                        draw_plugin_cached(
                            canvas,
                            &mut caches,
                            &paints.image,
                            *node_id,
                            *plugin_epoch,
                            *rect,
                            scale,
                            *inherited_text_color,
                            store,
                            registry,
                            events,
                            context_tag,
                            #[cfg(any(
                                feature = "skia-metal",
                                feature = "skia-vulkan",
                                feature = "skia-d3d"
                            ))]
                            direct_context.as_deref_mut(),
                        );
                    }
                }
            }
        }

        canvas.restore_to_count(saved);
    }

    /// Paints a compact translucent developer HUD status banner directly on a Skia [`Canvas`]
    /// using the unified `CosmicTextEngine` + `TextBlob` pipeline (zero `draw_str`).
    pub fn paint_hud_banner(&self, canvas: &Canvas, scale_factor: f32, banner: &str) {
        if banner.is_empty() {
            return;
        }
        let scale = sanitize_scale_factor(scale_factor);
        let saved = canvas.save();
        if (scale - 1.0).abs() > f32::EPSILON {
            canvas.scale((scale, scale));
        }

        let font_size = 12.0;
        let (text_w, text_h) = self.text_engine.measure_text(banner, font_size, None);
        let pad_x = 10.0;
        let box_w = (text_w + pad_x * 2.0).max(96.0);
        let box_h = (text_h + 8.0).max(24.0);
        let rect = Rect::from_xywh(10.0, 10.0, box_w, box_h);

        let mut bg = Paint::default();
        bg.set_anti_alias(true);
        bg.set_color(Color::from_argb(218, 15, 23, 42));
        canvas.draw_rrect(RRect::new_rect_xy(rect, 6.0, 6.0), &bg);

        let mut border = Paint::default();
        border.set_anti_alias(true);
        border.set_style(PaintStyle::Stroke);
        border.set_stroke_width(1.0);
        border.set_color(Color::from_argb(180, 56, 189, 248));
        canvas.draw_rrect(RRect::new_rect_xy(rect, 6.0, 6.0), &border);

        let mut fg = Paint::default();
        fg.set_anti_alias(true);
        fg.set_style(PaintStyle::Fill);
        fg.set_color(Color::from_argb(255, 226, 232, 240));

        let text_y = 10.0 + ((box_h - text_h) * 0.5).max(2.0);
        let text_rect = nve_engine::LayoutRect {
            x: 10.0 + pad_x,
            y: text_y,
            width: text_w.max(1.0),
            height: text_h.max(1.0),
        };
        let drew_blob = if self.text_engine.has_system_fonts() {
            let mut caches = self.caches.borrow_mut();
            if let Some(blob) = caches.get_or_build_text_blob(
                &self.font_mgr,
                &self.text_engine,
                banner,
                font_size,
                400,
                TextAlign::Left,
                None,
                text_w.max(1.0),
            ) {
                canvas.draw_text_blob(&blob, (10.0 + pad_x, text_y), &fg);
                true
            } else {
                false
            }
        } else {
            false
        };
        if !drew_blob {
            draw_bitmap_text_skia(
                canvas,
                &mut fg,
                text_rect,
                banner,
                font_size,
                400,
                TextAlign::Left,
                ColorRgba::rgb(226, 232, 240),
            );
        }

        canvas.restore_to_count(saved);
    }

    /// Builds or fetches a cached `TextBlob` for `text` (exposes warm-path zero-alloc testing).
    pub fn build_cached_text_blob(
        &self,
        text: &str,
        font_size: f32,
        font_weight: u16,
        text_align: TextAlign,
        wrap_width: Option<f32>,
        align_width: f32,
    ) -> Option<skia_safe::TextBlob> {
        self.caches.borrow_mut().get_or_build_text_blob(
            &self.font_mgr,
            &self.text_engine,
            text,
            font_size,
            font_weight,
            text_align,
            wrap_width,
            align_width,
        )
    }

    #[cfg(test)]
    pub(crate) fn resolve_typeface_for_test(
        &self,
        face_id: cosmic_text::fontdb::ID,
        weight: u16,
    ) -> Option<skia_safe::Typeface> {
        self.caches.borrow_mut().resolve_typeface_for_test(
            &self.font_mgr,
            &self.text_engine,
            face_id,
            weight,
        )
    }

    #[cfg(test)]
    pub(crate) fn set_base_typefaces_cap_for_test(&self, max_entries: usize) {
        self.caches
            .borrow_mut()
            .set_base_typefaces_cap_for_test(max_entries);
    }

    #[cfg(test)]
    pub(crate) fn set_base_typefaces_byte_cap_for_test(&self, max_bytes: usize) {
        self.caches
            .borrow_mut()
            .set_base_typefaces_byte_cap_for_test(max_bytes);
    }

    #[cfg(test)]
    pub(crate) fn plugin_scratch_capacities(&self) -> (usize, usize) {
        self.caches.borrow().plugin_scratch_capacities()
    }
}
