//! Skia `Canvas` display-list renderer and zero-copy-readback CPU raster surface (`--features skia`).

use std::time::Instant;

use nve_abi::ColorRgba;
use nve_components::canvas_ir::CanvasOp;
use skia_safe::{
    canvas::SrcRectConstraint, font_style::{Slant, Weight as SkiaWeight, Width as SkiaWidth},
    images, AlphaType, BlurStyle, Canvas, ClipOp, Color, ColorType, Data, FilterMode, Font,
    FontMgr, FontStyle, ImageInfo, MaskFilter, MipmapMode, Paint, PaintCap, PaintJoin, PaintStyle,
    PathBuilder, RRect, Rect, SamplingOptions, Surface, Typeface,
};

use super::{
    sanitize_scale_factor, FrameLifecycleState, FrameTelemetry, GpuBackendKind, GpuFramePresenter,
    GpuSurfaceError, SwapchainExtent,
};
use crate::image_cache::{compute_image_placement, ImageCache};
use crate::raster::PixelBuffer;
use crate::text::{TextAlign, LINE_HEIGHT_MULTIPLIER};
use crate::{DisplayList, PaintOp};

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
                    resize_mode,
                } => {
                    if rect.width <= 0.0 || rect.height <= 0.0 {
                        continue;
                    }
                    let Ok(decoded) = ImageCache::shared().get_or_decode(src) else {
                        continue;
                    };
                    let Some(placement) =
                        compute_image_placement(*rect, decoded.width, decoded.height, *resize_mode)
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
                PaintOp::CustomSkiaCanvas { rect, ops, .. } => {
                    if rect.width <= 0.0 || rect.height <= 0.0 || ops.is_empty() {
                        continue;
                    }
                    let outer_saved = canvas.save();
                    let bounds = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
                    canvas.clip_rect(bounds, ClipOp::Intersect, true);
                    canvas.translate((rect.x, rect.y));

                    let mut vec_fill = Paint::default();
                    vec_fill.set_anti_alias(true);
                    vec_fill.set_style(PaintStyle::Fill);

                    let mut vec_stroke = Paint::default();
                    vec_stroke.set_anti_alias(true);
                    vec_stroke.set_style(PaintStyle::Stroke);
                    vec_stroke.set_stroke_cap(PaintCap::Round);
                    vec_stroke.set_stroke_join(PaintJoin::Round);

                    let mut local_clip_depth = 0usize;

                    for cop in ops {
                        match cop {
                            CanvasOp::Clear { color } => {
                                if color.a > 0 {
                                    vec_fill.set_color(to_skia_color(*color));
                                    canvas.draw_rect(
                                        Rect::from_xywh(0.0, 0.0, rect.width, rect.height),
                                        &vec_fill,
                                    );
                                }
                            }
                            CanvasOp::FillRect { x, y, w, h, color } => {
                                if *w > 0.0 && *h > 0.0 && color.a > 0 {
                                    vec_fill.set_color(to_skia_color(*color));
                                    canvas.draw_rect(Rect::from_xywh(*x, *y, *w, *h), &vec_fill);
                                }
                            }
                            CanvasOp::FillRoundedRect {
                                x,
                                y,
                                w,
                                h,
                                radius,
                                color,
                            } => {
                                if *w > 0.0 && *h > 0.0 && color.a > 0 {
                                    vec_fill.set_color(to_skia_color(*color));
                                    let r = Rect::from_xywh(*x, *y, *w, *h);
                                    let cr = radius.max(0.0).min(*w * 0.5).min(*h * 0.5);
                                    canvas.draw_rrect(RRect::new_rect_xy(r, cr, cr), &vec_fill);
                                }
                            }
                            CanvasOp::FillCircle {
                                cx,
                                cy,
                                radius,
                                color,
                            } => {
                                if *radius > 0.0 && color.a > 0 {
                                    vec_fill.set_color(to_skia_color(*color));
                                    canvas.draw_circle((*cx, *cy), *radius, &vec_fill);
                                }
                            }
                            CanvasOp::StrokeLine {
                                x0,
                                y0,
                                x1,
                                y1,
                                width,
                                color,
                            } => {
                                if *width > 0.0 && color.a > 0 {
                                    vec_stroke.set_stroke_width(*width);
                                    vec_stroke.set_color(to_skia_color(*color));
                                    canvas.draw_line((*x0, *y0), (*x1, *y1), &vec_stroke);
                                }
                            }
                            CanvasOp::StrokePolyline {
                                points,
                                width,
                                color,
                            } => {
                                if *width > 0.0 && color.a > 0 && !points.is_empty() {
                                    if points.len() == 1 {
                                        vec_fill.set_color(to_skia_color(*color));
                                        let (px, py) = points[0];
                                        canvas.draw_circle((px, py), (*width * 0.5).max(0.5), &vec_fill);
                                    } else {
                                        let mut pb = PathBuilder::new();
                                        pb.move_to(points[0]);
                                        for &pt in &points[1..] {
                                            pb.line_to(pt);
                                        }
                                        let path = pb.detach();
                                        vec_stroke.set_stroke_width(*width);
                                        vec_stroke.set_color(to_skia_color(*color));
                                        canvas.draw_path(&path, &vec_stroke);
                                    }
                                }
                            }
                            CanvasOp::ClipRect { x, y, w, h } => {
                                canvas.save();
                                canvas.clip_rect(
                                    Rect::from_xywh(*x, *y, w.max(0.0), h.max(0.0)),
                                    ClipOp::Intersect,
                                    true,
                                );
                                local_clip_depth += 1;
                            }
                            CanvasOp::RestoreClip => {
                                if local_clip_depth > 0 {
                                    canvas.restore();
                                    local_clip_depth -= 1;
                                }
                            }
                        }
                    }

                    canvas.restore_to_count(outer_saved);
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

/// Reads back a Skia [`Surface`] into a [`PixelBuffer`] reusing `scratch_rgba` to avoid
/// per-frame heap allocations.
pub(crate) fn read_surface_to_pixel_buffer(
    surface: &mut Surface,
    width: u32,
    height: u32,
    scratch_rgba: &mut Vec<u8>,
    target: &mut PixelBuffer,
) -> Result<(), GpuSurfaceError> {
    let needed = (width as usize) * (height as usize) * 4;
    if scratch_rgba.len() != needed {
        scratch_rgba.resize(needed, 0);
    }
    target.resize(width, height);
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    let row_bytes = (width as usize) * 4;
    let ok = surface.read_pixels(&info, scratch_rgba, row_bytes, (0, 0));
    if !ok {
        return Err(GpuSurfaceError::ReadbackFailed(
            "skia_safe::Surface::read_pixels returned false".to_string(),
        ));
    }
    let dst = target.pixels_mut();
    let (rgba_chunks, _) = scratch_rgba.as_chunks::<4>();
    for (px, rgba) in dst.iter_mut().zip(rgba_chunks) {
        *px = PixelBuffer::pack_rgb(rgba[0], rgba[1], rgba[2]);
    }
    Ok(())
}

/// Unified Skia surface abstraction implemented by CPU raster surfaces and native
/// GPU surface adapters (Metal on macOS, Direct3D 12 on Windows, Vulkan on Linux).
pub trait GpuSurfaceBackend {
    fn kind(&self) -> GpuBackendKind;
    fn size(&self) -> (u32, u32);
    fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuSurfaceError>;
    fn render_display_list(
        &mut self,
        renderer: &SkiaCanvasRenderer,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
    ) -> Result<(), GpuSurfaceError>;
    fn copy_to_pixel_buffer(&mut self, target: &mut PixelBuffer) -> Result<(), GpuSurfaceError>;
}

/// CPU-backed Skia raster surface (`skia_safe::surfaces::raster_n32_premul`).
///
/// Always available when `--features skia` is enabled and cleanly bridges into
/// `winit` + `softbuffer` or headless screenshot readback when a native GPU swapchain
/// is unavailable.
pub struct CpuSkiaSurface {
    extent: SwapchainExtent,
    surface: Surface,
    renderer: SkiaCanvasRenderer,
    readback_rgba: Vec<u8>,
    state: FrameLifecycleState,
    frame_index: u64,
    swapchain_recreations: u64,
    frame_start: Option<Instant>,
    acquire_us: u64,
    record_us: u64,
    submit_us: u64,
    last_telemetry: Option<FrameTelemetry>,
    fallback_reason: Option<String>,
}

impl CpuSkiaSurface {
    pub fn new(width: u32, height: u32) -> Result<Self, GpuSurfaceError> {
        Self::new_with_scale(width, height, 1.0)
    }

    pub fn new_with_scale(
        width: u32,
        height: u32,
        scale_factor: f32,
    ) -> Result<Self, GpuSurfaceError> {
        let extent = SwapchainExtent::new(width, height, scale_factor)?;
        let surface =
            skia_safe::surfaces::raster_n32_premul((extent.width as i32, extent.height as i32))
                .ok_or_else(|| {
                    GpuSurfaceError::SurfaceCreationFailed(format!(
                        "raster_n32_premul returned None for {}x{}",
                        extent.width, extent.height
                    ))
                })?;
        Ok(Self {
            extent,
            surface,
            renderer: SkiaCanvasRenderer::new(),
            readback_rgba: vec![0u8; (extent.width as usize) * (extent.height as usize) * 4],
            state: FrameLifecycleState::Idle,
            frame_index: 0,
            swapchain_recreations: 0,
            frame_start: None,
            acquire_us: 0,
            record_us: 0,
            submit_us: 0,
            last_telemetry: None,
            fallback_reason: None,
        })
    }

    pub fn set_fallback_reason(&mut self, reason: Option<String>) {
        self.fallback_reason = reason;
    }

    pub fn fallback_reason(&self) -> Option<&str> {
        self.fallback_reason.as_deref()
    }

    pub fn canvas(&mut self) -> &Canvas {
        self.surface.canvas()
    }
}

impl GpuSurfaceBackend for CpuSkiaSurface {
    fn kind(&self) -> GpuBackendKind {
        GpuBackendKind::CpuRaster
    }

    fn size(&self) -> (u32, u32) {
        (self.extent.width, self.extent.height)
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuSurfaceError> {
        let scale = self.extent.scale_factor;
        <Self as GpuFramePresenter>::resize(self, width, height, scale)
    }

    fn render_display_list(
        &mut self,
        renderer: &SkiaCanvasRenderer,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
    ) -> Result<(), GpuSurfaceError> {
        let canvas = self.surface.canvas();
        canvas.clear(to_skia_color(clear_color));
        renderer.paint_display_list(display_list, canvas, scale_factor);
        Ok(())
    }

    fn copy_to_pixel_buffer(&mut self, target: &mut PixelBuffer) -> Result<(), GpuSurfaceError> {
        read_surface_to_pixel_buffer(
            &mut self.surface,
            self.extent.width,
            self.extent.height,
            &mut self.readback_rgba,
            target,
        )
    }
}

impl GpuFramePresenter for CpuSkiaSurface {
    fn kind(&self) -> GpuBackendKind {
        GpuBackendKind::CpuRaster
    }

    fn extent(&self) -> SwapchainExtent {
        self.extent
    }

    fn lifecycle_state(&self) -> FrameLifecycleState {
        self.state
    }

    fn swapchain_recreations(&self) -> u64 {
        self.swapchain_recreations
    }

    fn last_telemetry(&self) -> Option<&FrameTelemetry> {
        self.last_telemetry.as_ref()
    }

    fn fallback_reason(&self) -> Option<&str> {
        self.fallback_reason.as_deref()
    }

    fn resize(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: f32,
    ) -> Result<(), GpuSurfaceError> {
        if self.state != FrameLifecycleState::Idle {
            return Err(GpuSurfaceError::InvalidFrameState {
                expected: FrameLifecycleState::Idle.as_str(),
                actual: self.state.as_str(),
            });
        }
        let next = SwapchainExtent::new(width, height, scale_factor)?;
        if next.width == self.extent.width
            && next.height == self.extent.height
            && (next.scale_factor - self.extent.scale_factor).abs() <= f32::EPSILON
        {
            return Ok(());
        }
        if next.width != self.extent.width || next.height != self.extent.height {
            let next_surface =
                skia_safe::surfaces::raster_n32_premul((next.width as i32, next.height as i32))
                    .ok_or_else(|| {
                        GpuSurfaceError::SurfaceCreationFailed(format!(
                            "raster_n32_premul returned None for {}x{}",
                            next.width, next.height
                        ))
                    })?;
            self.surface = next_surface;
            self.readback_rgba
                .resize((next.width as usize) * (next.height as usize) * 4, 0);
        }
        self.extent = next;
        self.swapchain_recreations += 1;
        Ok(())
    }

    fn begin_frame(&mut self) -> Result<(), GpuSurfaceError> {
        if self.state != FrameLifecycleState::Idle {
            return Err(GpuSurfaceError::InvalidFrameState {
                expected: FrameLifecycleState::Idle.as_str(),
                actual: self.state.as_str(),
            });
        }
        let t0 = Instant::now();
        self.frame_start = Some(t0);
        self.acquire_us = t0.elapsed().as_micros() as u64;
        self.record_us = 0;
        self.submit_us = 0;
        self.state = FrameLifecycleState::Begun;
        Ok(())
    }

    fn render_display_list(
        &mut self,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
        hud_banner: Option<&str>,
    ) -> Result<(), GpuSurfaceError> {
        if self.state != FrameLifecycleState::Begun {
            return Err(GpuSurfaceError::InvalidFrameState {
                expected: FrameLifecycleState::Begun.as_str(),
                actual: self.state.as_str(),
            });
        }
        let t0 = Instant::now();
        let canvas = self.surface.canvas();
        canvas.clear(to_skia_color(clear_color));
        self.renderer
            .paint_display_list(display_list, canvas, scale_factor);
        if let Some(banner) = hud_banner {
            self.renderer.paint_hud_banner(canvas, scale_factor, banner);
        }
        self.record_us = t0.elapsed().as_micros() as u64;
        self.state = FrameLifecycleState::Rendered;
        Ok(())
    }

    fn submit_frame(&mut self) -> Result<(), GpuSurfaceError> {
        if self.state != FrameLifecycleState::Rendered {
            return Err(GpuSurfaceError::InvalidFrameState {
                expected: FrameLifecycleState::Rendered.as_str(),
                actual: self.state.as_str(),
            });
        }
        let t0 = Instant::now();
        self.submit_us = t0.elapsed().as_micros() as u64;
        self.state = FrameLifecycleState::Submitted;
        Ok(())
    }

    fn present_frame(&mut self) -> Result<FrameTelemetry, GpuSurfaceError> {
        if self.state != FrameLifecycleState::Submitted {
            return Err(GpuSurfaceError::InvalidFrameState {
                expected: FrameLifecycleState::Submitted.as_str(),
                actual: self.state.as_str(),
            });
        }
        let t_present = Instant::now();
        self.frame_index += 1;
        let present_us = t_present.elapsed().as_micros() as u64;
        let total_us = self
            .frame_start
            .take()
            .map_or(self.acquire_us + self.record_us + self.submit_us + present_us, |t| {
                t.elapsed().as_micros() as u64
            });
        let telemetry = FrameTelemetry {
            backend: GpuBackendKind::CpuRaster,
            frame_index: self.frame_index,
            width: self.extent.width,
            height: self.extent.height,
            scale_factor: self.extent.scale_factor,
            acquire_us: self.acquire_us,
            record_us: self.record_us,
            submit_us: self.submit_us,
            present_us,
            total_us,
            swapchain_recreations: self.swapchain_recreations,
            fallback_reason: self.fallback_reason.clone(),
        };
        self.last_telemetry = Some(telemetry.clone());
        self.state = FrameLifecycleState::Idle;
        Ok(telemetry)
    }

    fn abort_frame(&mut self) {
        self.frame_start = None;
        self.state = FrameLifecycleState::Idle;
    }
}
