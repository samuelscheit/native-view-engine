//! Skia `Canvas` display-list renderer (`--features skia`).

use nve_abi::{ColorRgba, EventBatchBuf};
use skia_safe::{
    canvas::SrcRectConstraint,
    font_style::{Slant, Weight as SkiaWeight, Width as SkiaWidth},
    images, AlphaType, BlurStyle, Canvas, ClipOp, Color, ColorType, Data, FilterMode, Font,
    FontMgr, FontStyle, ImageInfo, MaskFilter, Matrix, MipmapMode, Paint, PaintStyle, RRect, Rect,
    SamplingOptions, Typeface,
};

#[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
pub(crate) use super::cpu_surface::read_surface_to_pixel_buffer;
pub use super::cpu_surface::{CpuSkiaSurface, GpuSurfaceBackend};
use super::sanitize_scale_factor;
use super::skia_graphics::draw_rounded_rect_gradient;
use crate::image_cache::{compute_image_placement, ImageCache};
use crate::raster::PixelBuffer;
use crate::table_system::{SkiaSystemPaintCtx, SkiaTableRegistry};
use crate::text::{TextAlign, LINE_HEIGHT_MULTIPLIER};
use crate::{DisplayList, PaintOp};
use nve_store::NodeStore;

const PREFERRED_SANS_FAMILIES: &[&str] = &[
    ".AppleSystemUIFont",
    "SF Pro Text",
    "Helvetica Neue",
    "Segoe UI",
    "DejaVu Sans",
    "Noto Sans",
    "Liberation Sans",
    "Arial",
    "",
];

#[inline]
pub(crate) fn to_skia_color(c: ColorRgba) -> Color {
    Color::from_argb(c.a, c.r, c.g, c.b)
}

/// Paints an NVE retained [`DisplayList`] directly onto any [`skia_safe::Canvas`].
///
/// Resolves a native system sans-serif [`Typeface`] once via [`FontMgr`] and uses
/// Skia's `RRect`, `Paint`, clipping, and `save_layer_alpha_f` primitives.
pub struct SkiaCanvasRenderer {
    font_mgr: FontMgr,
    default_typeface: Option<Typeface>,
}

impl Default for SkiaCanvasRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SkiaCanvasRenderer {
    pub fn new() -> Self {
        let font_mgr = FontMgr::new();
        let style = FontStyle::normal();
        let default_typeface = PREFERRED_SANS_FAMILIES
            .iter()
            .find_map(|family| font_mgr.match_family_style(*family, style));
        Self {
            font_mgr,
            default_typeface,
        }
    }

    pub fn font_mgr(&self) -> &FontMgr {
        &self.font_mgr
    }

    pub fn default_typeface(&self) -> Option<&Typeface> {
        self.default_typeface.as_ref()
    }

    fn make_font(&self, font_size: f32) -> Font {
        self.make_font_with_weight(font_size, 400)
    }

    fn make_font_with_weight(&self, font_size: f32, font_weight: u16) -> Font {
        let size = if font_size.is_finite() && font_size > 0.0 {
            font_size
        } else {
            16.0
        };
        let typeface = if font_weight != 400 && font_weight != 0 {
            let style = FontStyle::new(
                SkiaWeight::from(font_weight.clamp(100, 900) as i32),
                SkiaWidth::NORMAL,
                Slant::Upright,
            );
            PREFERRED_SANS_FAMILIES
                .iter()
                .find_map(|family| self.font_mgr.match_family_style(*family, style))
                .or_else(|| self.default_typeface.clone())
        } else {
            self.default_typeface.clone()
        };
        let mut font = match typeface {
            Some(tf) => Font::from_typeface(tf, size),
            None => Font::default().with_size(size).unwrap_or_default(),
        };
        font.set_subpixel(true);
        font.set_edging(skia_safe::font::Edging::SubpixelAntiAlias);
        font
    }

    /// Paints `display_list` onto `canvas`, scaling logical coordinates by `scale_factor`.
    pub fn paint_display_list(
        &self,
        display_list: &DisplayList,
        canvas: &Canvas,
        scale_factor: f32,
    ) {
        self.paint_display_list_inner(display_list, canvas, scale_factor, None);
    }

    /// Paints `display_list` onto `canvas`, also dispatching [`PaintOp::Plugin`] nodes through
    /// `registry` against `store` (rasterizing plugin rects and compositing onto `canvas`).
    pub fn paint_display_list_with_plugins(
        &self,
        display_list: &DisplayList,
        canvas: &Canvas,
        scale_factor: f32,
        store: &NodeStore,
        registry: &SkiaTableRegistry,
        events: &mut EventBatchBuf,
    ) {
        self.paint_display_list_inner(
            display_list,
            canvas,
            scale_factor,
            Some((store, registry, events)),
        );
    }

    pub(crate) fn paint_display_list_opt_plugins(
        &self,
        display_list: &DisplayList,
        canvas: &Canvas,
        scale_factor: f32,
        plugins: Option<super::SkiaPluginContext<'_>>,
    ) {
        self.paint_display_list_inner(display_list, canvas, scale_factor, plugins);
    }

    fn paint_display_list_inner(
        &self,
        display_list: &DisplayList,
        canvas: &Canvas,
        scale_factor: f32,
        mut plugins: Option<(&NodeStore, &SkiaTableRegistry, &mut EventBatchBuf)>,
    ) {
        let scale = sanitize_scale_factor(scale_factor);
        let saved = canvas.save();
        if (scale - 1.0).abs() > f32::EPSILON {
            canvas.scale((scale, scale));
        }

        let mut fill_paint = Paint::default();
        fill_paint.set_anti_alias(true);
        fill_paint.set_style(PaintStyle::Fill);

        let mut stroke_paint = Paint::default();
        stroke_paint.set_anti_alias(true);
        stroke_paint.set_style(PaintStyle::Stroke);

        let mut text_paint = Paint::default();
        text_paint.set_anti_alias(true);
        text_paint.set_style(PaintStyle::Fill);

        let mut image_paint = Paint::default();
        image_paint.set_anti_alias(true);

        let mut save_depth = 0usize;
        let mut plugin_buf = PixelBuffer::new_transparent_rgba(0, 0);
        let mut plugin_rgba: Vec<u8> = Vec::new();

        for op in &display_list.ops {
            match op {
                PaintOp::SaveOpacity(alpha) => {
                    canvas.save_layer_alpha_f(None, alpha.clamp(0.0, 1.0));
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
                    let r = Rect::from_xywh(rect.x, rect.y, rect.width.max(0.0), rect.height.max(0.0));
                    let clamped_r = radius
                        .max(0.0)
                        .min(rect.width.max(0.0) * 0.5)
                        .min(rect.height.max(0.0) * 0.5);
                    if clamped_r > 0.01 {
                        canvas.clip_rrect(RRect::new_rect_xy(r, clamped_r, clamped_r), ClipOp::Intersect, true);
                    } else {
                        canvas.clip_rect(r, ClipOp::Intersect, true);
                    }
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
                    let m = Matrix::new_all(
                        matrix[0], matrix[2], matrix[4],
                        matrix[1], matrix[3], matrix[5],
                        0.0, 0.0, 1.0,
                    );
                    canvas.concat(&m);
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
                } => {
                    let sw = rect.width + *spread_radius * 2.0;
                    let sh = rect.height + *spread_radius * 2.0;
                    if sw <= 0.0 || sh <= 0.0 || color.a == 0 {
                        continue;
                    }
                    let mut shadow_paint = Paint::default();
                    shadow_paint.set_anti_alias(true);
                    shadow_paint.set_style(PaintStyle::Fill);
                    shadow_paint.set_color(to_skia_color(*color));
                    if *blur_radius > 0.5 {
                        let sigma = (*blur_radius * 0.5).max(0.5);
                        if let Some(filter) = MaskFilter::blur(BlurStyle::Normal, sigma, false) {
                            shadow_paint.set_mask_filter(filter);
                        }
                    }
                    let sr = Rect::from_xywh(
                        rect.x + *offset_x - *spread_radius,
                        rect.y + *offset_y - *spread_radius,
                        sw,
                        sh,
                    );
                    let base_r = if *radius > 0.01 {
                        (*radius + *spread_radius).max(0.0)
                    } else {
                        0.0
                    };
                    let clamped_r = base_r.min(sw * 0.5).min(sh * 0.5);
                    canvas.draw_rrect(RRect::new_rect_xy(sr, clamped_r, clamped_r), &shadow_paint);
                }
                PaintOp::FillRoundedRect {
                    rect,
                    radius,
                    color,
                } => {
                    fill_paint.set_color(to_skia_color(*color));
                    let r = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
                    let clamped_r = radius.max(0.0).min(rect.width * 0.5).min(rect.height * 0.5);
                    canvas.draw_rrect(RRect::new_rect_xy(r, clamped_r, clamped_r), &fill_paint);
                }
                PaintOp::FillRoundedRectGradient {
                    rect,
                    radius,
                    gradient,
                } => {
                    draw_rounded_rect_gradient(canvas, *rect, *radius, gradient);
                }
                PaintOp::StrokeRoundedRect {
                    rect,
                    radius,
                    width,
                    color,
                } => {
                    if *width <= 0.0 || color.a == 0 {
                        continue;
                    }
                    stroke_paint.set_stroke_width(*width);
                    stroke_paint.set_color(to_skia_color(*color));
                    // Inset by half stroke width so the stroke occupies the exact interior edge
                    // matching the software rasterizer's border-box semantics.
                    let half = (*width * 0.5).min(rect.width * 0.5).min(rect.height * 0.5);
                    let inset_w = (rect.width - half * 2.0).max(0.0);
                    let inset_h = (rect.height - half * 2.0).max(0.0);
                    let r = Rect::from_xywh(rect.x + half, rect.y + half, inset_w, inset_h);
                    let clamped_r = (*radius - half).max(0.0).min(inset_w * 0.5).min(inset_h * 0.5);
                    canvas.draw_rrect(RRect::new_rect_xy(r, clamped_r, clamped_r), &stroke_paint);
                }
                PaintOp::DrawText {
                    rect,
                    text,
                    font_size,
                    font_weight,
                    text_align,
                    color,
                } => {
                    if text.is_empty() || rect.width <= 0.0 || rect.height <= 0.0 {
                        continue;
                    }
                    text_paint.set_color(to_skia_color(*color));
                    let font = self.make_font_with_weight(*font_size, *font_weight);
                    let (_, metrics) = font.metrics();
                    let ascent = -metrics.ascent;
                    let descent = metrics.descent;
                    let line_height = (*font_size * LINE_HEIGHT_MULTIPLIER).max(*font_size + 2.0);
                    let glyph_h = (ascent + descent).max(*font_size);
                    let baseline_offset = ((line_height - glyph_h) * 0.5).max(0.0) + ascent;

                    let clip = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
                    canvas.save();
                    canvas.clip_rect(clip, ClipOp::Intersect, true);
                    for (line_idx, line) in text.split('\n').enumerate() {
                        let y = rect.y + (line_idx as f32) * line_height + baseline_offset;
                        let align_x = match text_align {
                            TextAlign::Left => 0.0,
                            TextAlign::Center => {
                                let (line_w, _) = font.measure_str(line, None);
                                ((rect.width - line_w) * 0.5).max(0.0)
                            }
                            TextAlign::Right => {
                                let (line_w, _) = font.measure_str(line, None);
                                (rect.width - line_w).max(0.0)
                            }
                        };
                        canvas.draw_str(line, (rect.x + align_x, y), &font, &text_paint);
                    }
                    canvas.restore();
                }
                PaintOp::DrawImage {
                    rect,
                    radius,
                    src,
                    fit,
                } => {
                    if rect.width <= 0.0 || rect.height <= 0.0 {
                        continue;
                    }
                    let Ok(decoded) = ImageCache::shared().get_or_decode(src) else {
                        continue;
                    };
                    let Some(placement) =
                        compute_image_placement(*rect, decoded.width, decoded.height, *fit)
                    else {
                        continue;
                    };
                    let info = ImageInfo::new(
                        (decoded.width as i32, decoded.height as i32),
                        ColorType::RGBA8888,
                        AlphaType::Unpremul,
                        None,
                    );
                    let row_bytes = (decoded.width as usize) * 4;
                    let data = Data::new_copy(&decoded.rgba);
                    let Some(sk_img) = images::raster_from_data(&info, data, row_bytes) else {
                        continue;
                    };

                    let outer = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
                    let clamped_r = radius
                        .max(0.0)
                        .min(rect.width * 0.5)
                        .min(rect.height * 0.5);
                    canvas.save();
                    if clamped_r > 0.01 {
                        canvas.clip_rrect(
                            RRect::new_rect_xy(outer, clamped_r, clamped_r),
                            ClipOp::Intersect,
                            true,
                        );
                    } else {
                        canvas.clip_rect(outer, ClipOp::Intersect, true);
                    }

                    let src_r = Rect::from_xywh(
                        placement.src_x,
                        placement.src_y,
                        placement.src_w,
                        placement.src_h,
                    );
                    let dst_r = Rect::from_xywh(
                        placement.dst_x,
                        placement.dst_y,
                        placement.dst_w,
                        placement.dst_h,
                    );
                    let filter = if (placement.dst_w - placement.src_w).abs() < 0.01
                        && (placement.dst_h - placement.src_h).abs() < 0.01
                    {
                        FilterMode::Nearest
                    } else {
                        FilterMode::Linear
                    };
                    canvas.draw_image_rect_with_sampling_options(
                        &sk_img,
                        Some((&src_r, SrcRectConstraint::Strict)),
                        dst_r,
                        SamplingOptions::new(filter, MipmapMode::None),
                        &image_paint,
                    );
                    canvas.restore();
                }
                PaintOp::Plugin {
                    node_id,
                    rect,
                    inherited_text_color,
                } => {
                    if let Some((store, registry, ref mut events)) = plugins
                        && rect.width.is_finite()
                        && rect.height.is_finite()
                        && rect.width > 0.0
                        && rect.height > 0.0
                    {
                        let max_dim = (rect.width * scale).max(rect.height * scale);
                        let eff_scale = if max_dim > 4096.0 {
                            scale * (4096.0 / max_dim)
                        } else {
                            scale
                        };
                        let pw = (rect.width * eff_scale).round().clamp(1.0, 4096.0) as u32;
                        let ph = (rect.height * eff_scale).round().clamp(1.0, 4096.0) as u32;
                        plugin_buf.reset_transparent_rgba(pw, ph);
                        let mut ctx = SkiaSystemPaintCtx {
                            buffer: &mut plugin_buf,
                            rect: nve_engine::LayoutRect {
                                x: 0.0,
                                y: 0.0,
                                width: pw as f32,
                                height: ph as f32,
                            },
                            clip_rect: None,
                            scale_factor: eff_scale,
                            opacity: 1.0,
                            inherited_text_color: *inherited_text_color,
                            events,
                        };
                        if registry.paint_node(&mut ctx, store, *node_id) > 0 {
                            plugin_buf.write_rgba8_into(&mut plugin_rgba);
                            let info = ImageInfo::new(
                                (pw as i32, ph as i32),
                                ColorType::RGBA8888,
                                AlphaType::Unpremul,
                                None,
                            );
                            if let Some(img) = images::raster_from_data(
                                &info,
                                Data::new_copy(&plugin_rgba),
                                (pw as usize) * 4,
                            ) {
                                let dst = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
                                canvas.draw_image_rect_with_sampling_options(
                                    &img,
                                    None,
                                    dst,
                                    SamplingOptions::new(FilterMode::Linear, MipmapMode::None),
                                    &image_paint,
                                );
                            }
                        }
                    }
                }
            }
        }

        canvas.restore_to_count(saved);
    }

    /// Paints a compact translucent developer HUD status banner directly on a Skia [`Canvas`]
    /// (used by direct hardware GPU swapchains so HUD rendering stays 100% GPU-resident).
    pub fn paint_hud_banner(&self, canvas: &Canvas, scale_factor: f32, banner: &str) {
        if banner.is_empty() {
            return;
        }
        let scale = sanitize_scale_factor(scale_factor);
        let saved = canvas.save();
        if (scale - 1.0).abs() > f32::EPSILON {
            canvas.scale((scale, scale));
        }

        let font = self.make_font(12.0);
        let (text_w, _) = font.measure_str(banner, None);
        let pad_x = 10.0;
        let box_w = (text_w + pad_x * 2.0).max(96.0);
        let box_h = 24.0;
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
        fg.set_color(Color::from_argb(255, 226, 232, 240));
        canvas.draw_str(banner, (10.0 + pad_x, 26.0), &font, &fg);

        canvas.restore_to_count(saved);
    }
}
