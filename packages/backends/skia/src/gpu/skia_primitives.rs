//! Reusable cached Skia drawing primitives shared by `SkiaCanvasRenderer` (flat `DisplayList`)
//! and the retained `CompositorScene` Skia replay (`--features skia`).

use nve_abi::{ColorRgba, EventBatchBuf};
use nve_engine::LayoutRect;
use nve_store::{NodeId, NodeStore, Rgba};
use nve_tables::tables::ImageFit;
#[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
use skia_safe::gpu::DirectContext;
use skia_safe::{
    canvas::SrcRectConstraint, Canvas, ClipOp, FilterMode, FontMgr, MipmapMode, Paint, PaintStyle,
    RRect, Rect, SamplingOptions,
};

use super::renderer_cache::SkiaRendererCaches;
use super::skia_renderer::to_skia_color;
use crate::font::{for_each_bitmap_cell_in_glyph, for_each_bitmap_glyph_pass};
use crate::image_cache::{compute_image_placement, ImageCache, ImageSlotHandle};
use crate::raster::shadow::{
    linear_ctm_scales, physical_blur_sigma, resolve_inset_geometry, InsetGeometry, MAX_BLUR_SIGMA,
};
use crate::table_system::SkiaTableRegistry;
use crate::text::{CosmicTextEngine, TextAlign};
use crate::text_cache::{sanitize_font_size, sanitize_font_weight, wrap_width_from_rect};

/// Returns `(geom_scale, min_axis_scale)` of `canvas`'s current local-to-device 2D affine CTM.
/// Delegates to [`linear_ctm_scales`] (`sx.sqrt() * sy.sqrt()`) to avoid `sx * sy` overflow.
#[inline]
fn canvas_ctm_blur_scales(canvas: &Canvas) -> (f32, f32) {
    let m = canvas.local_to_device_as_3x3();
    linear_ctm_scales(&[m.scale_x(), m.skew_y(), m.skew_x(), m.scale_y(), 0.0, 0.0])
}

#[inline]
pub(crate) fn rect_is_valid(rect: LayoutRect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width > 0.0
        && rect.height > 0.0
}

#[inline]
pub(crate) fn safe_radius(radius: f32) -> f32 {
    if radius.is_finite() {
        radius.max(0.0)
    } else {
        0.0
    }
}

pub(crate) struct FramePaints {
    pub(crate) fill: Paint,
    pub(crate) stroke: Paint,
    pub(crate) text: Paint,
    pub(crate) image: Paint,
    pub(crate) shadow: Paint,
    pub(crate) gradient: Paint,
}

impl FramePaints {
    pub(crate) fn new() -> Self {
        let mut fill = Paint::default();
        fill.set_anti_alias(true);
        fill.set_style(PaintStyle::Fill);

        let mut stroke = Paint::default();
        stroke.set_anti_alias(true);
        stroke.set_style(PaintStyle::Stroke);

        let mut text = Paint::default();
        text.set_anti_alias(true);
        text.set_style(PaintStyle::Fill);

        let mut image = Paint::default();
        image.set_anti_alias(true);

        let mut shadow = Paint::default();
        shadow.set_anti_alias(true);
        shadow.set_style(PaintStyle::Fill);

        let mut gradient = Paint::default();
        gradient.set_anti_alias(true);
        gradient.set_style(PaintStyle::Fill);

        Self {
            fill,
            stroke,
            text,
            image,
            shadow,
            gradient,
        }
    }
}

#[inline]
pub(crate) fn apply_clip(canvas: &Canvas, rect: LayoutRect, radius: f32) {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
    {
        canvas.clip_rect(Rect::from_xywh(0.0, 0.0, 0.0, 0.0), ClipOp::Intersect, false);
        return;
    }
    let w = rect.width.max(0.0);
    let h = rect.height.max(0.0);
    let r = Rect::from_xywh(rect.x, rect.y, w, h);
    let clamped_r = safe_radius(radius).min(w * 0.5).min(h * 0.5);
    if clamped_r > 0.01 {
        canvas.clip_rrect(
            RRect::new_rect_xy(r, clamped_r, clamped_r),
            ClipOp::Intersect,
            true,
        );
    } else {
        canvas.clip_rect(r, ClipOp::Intersect, true);
    }
}

#[inline]
pub(crate) fn draw_fill_rrect(
    canvas: &Canvas,
    paint: &mut Paint,
    rect: LayoutRect,
    radius: f32,
    color: ColorRgba,
) {
    if !rect_is_valid(rect) || color.a == 0 {
        return;
    }
    paint.set_color(to_skia_color(color));
    let r = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
    let clamped_r = safe_radius(radius)
        .min(rect.width * 0.5)
        .min(rect.height * 0.5);
    canvas.draw_rrect(RRect::new_rect_xy(r, clamped_r, clamped_r), paint);
}

#[inline]
pub(crate) fn draw_stroke_rrect(
    canvas: &Canvas,
    paint: &mut Paint,
    rect: LayoutRect,
    radius: f32,
    width: f32,
    color: ColorRgba,
) {
    if !rect_is_valid(rect) || !width.is_finite() || width <= 0.0 || color.a == 0 {
        return;
    }
    paint.set_stroke_width(width);
    paint.set_color(to_skia_color(color));
    let half = (width * 0.5).min(rect.width * 0.5).min(rect.height * 0.5);
    let inset_w = (rect.width - half * 2.0).max(0.0);
    let inset_h = (rect.height - half * 2.0).max(0.0);
    let r = Rect::from_xywh(rect.x + half, rect.y + half, inset_w, inset_h);
    let clamped_r = (safe_radius(radius) - half)
        .max(0.0)
        .min(inset_w * 0.5)
        .min(inset_h * 0.5);
    canvas.draw_rrect(RRect::new_rect_xy(r, clamped_r, clamped_r), paint);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_shadow_cached(
    canvas: &Canvas,
    caches: &mut SkiaRendererCaches,
    shadow_paint: &mut Paint,
    rect: LayoutRect,
    radius: f32,
    offset_x: f32,
    offset_y: f32,
    blur_radius: f32,
    spread_radius: f32,
    color: ColorRgba,
) {
    if !rect_is_valid(rect)
        || !offset_x.is_finite()
        || !offset_y.is_finite()
        || !blur_radius.is_finite()
        || !spread_radius.is_finite()
        || color.a == 0
    {
        return;
    }
    let sw = rect.width + spread_radius * 2.0;
    let sh = rect.height + spread_radius * 2.0;
    if !sw.is_finite() || !sh.is_finite() || sw <= 0.0 || sh <= 0.0 {
        return;
    }
    shadow_paint.set_color(to_skia_color(color));
    let (ctm_scale, _) = canvas_ctm_blur_scales(canvas);
    if let Some(phys_sigma) = physical_blur_sigma(blur_radius, ctm_scale) {
        let logical_sigma = phys_sigma / ctm_scale;
        shadow_paint.set_mask_filter(caches.get_or_create_blur_filter(logical_sigma));
    } else {
        shadow_paint.set_mask_filter(None);
    }
    let sx = rect.x + offset_x - spread_radius;
    let sy = rect.y + offset_y - spread_radius;
    if !sx.is_finite() || !sy.is_finite() {
        return;
    }
    let sr = Rect::from_xywh(sx, sy, sw, sh);
    let r = safe_radius(radius);
    let base_r = if r > 0.01 {
        (r + spread_radius).max(0.0)
    } else {
        0.0
    };
    let clamped_r = base_r.min(sw * 0.5).min(sh * 0.5);
    canvas.draw_rrect(RRect::new_rect_xy(sr, clamped_r, clamped_r), shadow_paint);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_inset_shadow_cached(
    canvas: &Canvas,
    caches: &mut SkiaRendererCaches,
    shadow_paint: &mut Paint,
    rect: LayoutRect,
    radius: f32,
    border_width: f32,
    offset_x: f32,
    offset_y: f32,
    blur_radius: f32,
    spread_radius: f32,
    color: ColorRgba,
) {
    if color.a == 0 {
        return;
    }
    let (ctm_scale, min_axis_scale) = canvas_ctm_blur_scales(canvas);
    if ctm_scale <= 0.0 {
        return;
    }
    let phys_sigma = physical_blur_sigma(blur_radius, ctm_scale);
    let eff_min_axis = min_axis_scale.max(1e-3);
    let capped_phys_sigma = phys_sigma.unwrap_or(0.0).min(MAX_BLUR_SIGMA);
    let blur_margin = if phys_sigma.is_some() {
        (capped_phys_sigma * 4.0 + 2.0) / eff_min_axis
    } else {
        1.0 / eff_min_axis
    };

    let (inner_rect, inner_r, hole_rect, base_hole_r) = match resolve_inset_geometry(
        rect,
        radius,
        border_width,
        offset_x,
        offset_y,
        blur_radius,
        spread_radius,
        1.0,
        blur_margin,
    ) {
        InsetGeometry::Empty => return,
        InsetGeometry::SolidFill {
            inner_rect,
            inner_radius,
        } => {
            let padding_rrect = RRect::new_rect_xy(
                Rect::from_xywh(inner_rect.x, inner_rect.y, inner_rect.width, inner_rect.height),
                inner_radius,
                inner_radius,
            );
            shadow_paint.set_color(to_skia_color(color));
            shadow_paint.set_mask_filter(None);
            canvas.draw_rrect(padding_rrect, shadow_paint);
            return;
        }
        InsetGeometry::Ring {
            inner_rect,
            inner_radius,
            hole_rect,
            base_hole_r,
        } => (inner_rect, inner_radius, hole_rect, base_hole_r),
    };

    let padding_rrect = RRect::new_rect_xy(
        Rect::from_xywh(inner_rect.x, inner_rect.y, inner_rect.width, inner_rect.height),
        inner_r,
        inner_r,
    );
    canvas.save();
    canvas.clip_rrect(padding_rrect, ClipOp::Intersect, true);
    shadow_paint.set_color(to_skia_color(color));
    if let Some(sigma) = phys_sigma {
        let logical_sigma = sigma / ctm_scale;
        shadow_paint.set_mask_filter(caches.get_or_create_blur_filter(logical_sigma));
    } else {
        shadow_paint.set_mask_filter(None);
    }

    // Outset bounded by `MAX_BLUR_SIGMA` converted into logical units via `min_axis_scale`.
    let outset = (capped_phys_sigma * 4.0 + 4.0) / eff_min_axis;
    let outer_l = inner_rect.x.min(hole_rect.x) - outset;
    let outer_t = inner_rect.y.min(hole_rect.y) - outset;
    let outer_r = (inner_rect.x + inner_rect.width).max(hole_rect.x + hole_rect.width) + outset;
    let outer_b = (inner_rect.y + inner_rect.height).max(hole_rect.y + hole_rect.height) + outset;
    if !outer_l.is_finite()
        || !outer_t.is_finite()
        || !outer_r.is_finite()
        || !outer_b.is_finite()
        || !(outer_r - outer_l).is_finite()
        || !(outer_b - outer_t).is_finite()
    {
        canvas.restore();
        return;
    }
    let outer_rrect = RRect::new_rect(Rect::from_ltrb(outer_l, outer_t, outer_r, outer_b));
    let clamped_hole_r = base_hole_r
        .min(hole_rect.width * 0.5)
        .min(hole_rect.height * 0.5);
    let hole_rrect = RRect::new_rect_xy(
        Rect::from_xywh(hole_rect.x, hole_rect.y, hole_rect.width, hole_rect.height),
        clamped_hole_r,
        clamped_hole_r,
    );
    canvas.draw_drrect(outer_rrect, hole_rrect, shadow_paint);
    canvas.restore();
}

/// Renders embedded 5x7 bitmap fallback glyphs onto a Skia `Canvas` when zero system fonts are
/// installed or when a font file fails to decode (sharing exact layout & faux-bold `+1.0` geometry
/// with software `draw_bitmap_text` via `font::for_each_bitmap_glyph_pass`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_bitmap_text_skia(
    canvas: &Canvas,
    paint: &mut Paint,
    rect: LayoutRect,
    text: &str,
    font_size: f32,
    font_weight: u16,
    text_align: TextAlign,
    color: ColorRgba,
) {
    if text.is_empty() || !rect_is_valid(rect) || color.a == 0 {
        return;
    }
    let fs = sanitize_font_size(font_size);
    let weight = sanitize_font_weight(font_weight);

    let prev_aa = paint.is_anti_alias();
    paint.set_anti_alias(false);
    paint.set_color(to_skia_color(color));
    let clip = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
    canvas.save();
    canvas.clip_rect(clip, ClipOp::Intersect, false);

    for_each_bitmap_glyph_pass(rect, text, fs, weight, text_align, |gx, gy, gw, gh, bitmap| {
        for_each_bitmap_cell_in_glyph(gx, gy, gw, gh, bitmap, |rx, ry, cw, ch| {
            canvas.draw_rect(Rect::from_xywh(rx, ry, cw, ch), paint);
        });
    });

    canvas.restore();
    paint.set_anti_alias(prev_aa);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_text_cached(
    canvas: &Canvas,
    font_mgr: &FontMgr,
    text_engine: &CosmicTextEngine,
    caches: &mut SkiaRendererCaches,
    text_paint: &mut Paint,
    rect: LayoutRect,
    text: &str,
    font_size: f32,
    font_weight: u16,
    text_align: TextAlign,
    color: ColorRgba,
) {
    if text.is_empty() || !rect_is_valid(rect) || color.a == 0 {
        return;
    }
    if !text_engine.has_system_fonts() {
        draw_bitmap_text_skia(
            canvas, text_paint, rect, text, font_size, font_weight, text_align, color,
        );
        return;
    }
    let wrap_width = wrap_width_from_rect(rect.width);
    let Some(blob) = caches.get_or_build_text_blob(
        font_mgr,
        text_engine,
        text,
        font_size,
        font_weight,
        text_align,
        wrap_width,
        rect.width,
    ) else {
        // If typeface decoding failed (e.g. corrupt font file), degrade gracefully to the
        // embedded 5x7 bitmap fallback so text never silently vanishes.
        draw_bitmap_text_skia(
            canvas, text_paint, rect, text, font_size, font_weight, text_align, color,
        );
        return;
    };
    text_paint.set_color(to_skia_color(color));
    let clip = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
    canvas.save();
    canvas.clip_rect(clip, ClipOp::Intersect, true);
    canvas.draw_text_blob(&blob, (rect.x, rect.y), text_paint);
    canvas.restore();
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_image_cached(
    canvas: &Canvas,
    caches: &mut SkiaRendererCaches,
    image_paint: &Paint,
    rect: LayoutRect,
    radius: f32,
    src: &str,
    resolved: Option<ImageSlotHandle>,
    fit: ImageFit,
    context_tag: u32,
    #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
    direct_context: Option<&mut DirectContext>,
) -> Option<ImageSlotHandle> {
    if !rect_is_valid(rect) {
        return resolved;
    }
    let shared_cache = ImageCache::shared();
    // Fast path: pre-resolved `ImageSlotHandle` validated by one per-slot atomic `Acquire` load
    // (+ CLOCK second-chance mark) avoids BOTH the global mutex lock AND URI hashing completely.
    let (sk_img, img_w, img_h, effective_handle) = if let Some(handle) = resolved
        && shared_cache.is_handle_live(handle)
        && let Some((sk_img, w, h)) =
            caches.get_cached_image_by_slot(handle.slot(), handle.decoded_id(), context_tag)
    {
        (sk_img, w, h, Some(handle))
    } else {
        let Ok((handle_opt, decoded)) = shared_cache.get_or_decode_with_handle(src) else {
            return None;
        };
        let Some(sk_img) = caches.get_or_upload_image_for_slot(
            handle_opt.map(ImageSlotHandle::slot),
            &decoded,
            context_tag,
            #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
            direct_context,
        ) else {
            return handle_opt;
        };
        (sk_img, decoded.width(), decoded.height(), handle_opt)
    };
    let Some(placement) = compute_image_placement(rect, img_w, img_h, fit) else {
        return effective_handle;
    };

    canvas.save();
    apply_clip(canvas, rect, radius);
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
        image_paint,
    );
    canvas.restore();
    effective_handle
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_plugin_cached(
    canvas: &Canvas,
    caches: &mut SkiaRendererCaches,
    image_paint: &Paint,
    node_id: NodeId,
    plugin_epoch: u64,
    rect: LayoutRect,
    scale: f32,
    inherited_text_color: Rgba,
    store: &NodeStore,
    registry: &SkiaTableRegistry,
    events: &mut EventBatchBuf,
    context_tag: u32,
    #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
    direct_context: Option<&mut DirectContext>,
) {
    if !rect_is_valid(rect) || !scale.is_finite() || scale <= 0.0 {
        return;
    }
    if registry.try_paint_skia_canvas(store, node_id, canvas, rect, scale) {
        return;
    }
    let max_dim = (rect.width * scale).max(rect.height * scale);
    let eff_scale = if max_dim > 4096.0 {
        scale * (4096.0 / max_dim)
    } else {
        scale
    };
    let pw = (rect.width * eff_scale).round().clamp(1.0, 4096.0) as u32;
    let ph = (rect.height * eff_scale).round().clamp(1.0, 4096.0) as u32;
    let Some(img) = caches.get_or_render_plugin(
        node_id,
        plugin_epoch,
        pw,
        ph,
        eff_scale,
        inherited_text_color,
        store,
        registry,
        events,
        context_tag,
        #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
        direct_context,
    ) else {
        return;
    };
    let dst = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
    canvas.draw_image_rect_with_sampling_options(
        &img,
        None,
        dst,
        SamplingOptions::new(FilterMode::Linear, MipmapMode::None),
        image_paint,
    );
}
