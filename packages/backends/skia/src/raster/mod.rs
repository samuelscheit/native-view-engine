//! Pure-Rust anti-aliased software rasterizer for NVE `DisplayList`.

mod canvas;
pub(crate) mod clip;
pub(crate) mod dispatch;
mod gradient;
mod hud;
mod pixel_buffer;
pub mod shadow;
mod shadow_transformed;
pub(crate) mod text_blend;
pub(crate) mod transform;

pub use hud::draw_hud_banner;

use nve_abi::{ColorRgba, EventBatchBuf};
use nve_engine::LayoutRect;
use nve_store::NodeStore;

use crate::table_system::SkiaTableRegistry;
use crate::text::CosmicTextEngine;
use crate::{DisplayList, PaintOp};

use clip::{rounded_rect_sdf, ClipStack};
pub(crate) use dispatch::SoftwareRasterCtx;
pub(crate) use text_blend::{
    blend_color_glyph, blend_mask_glyph, blend_subpixel_glyph, draw_bitmap_text,
};

pub use pixel_buffer::PixelBuffer;

pub(crate) fn fill_rounded_rect(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    rect: LayoutRect,
    radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || !opacity.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
        || color.a == 0
        || opacity <= 0.0
        || clip.is_empty()
    {
        return;
    }
    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (rect.x.floor() as i32).saturating_sub(1).clamp(cx0, cx1);
    let max_x = ((rect.x + rect.width).ceil() as i32)
        .saturating_add(1)
        .clamp(cx0, cx1);
    let min_y = (rect.y.floor() as i32).saturating_sub(1).clamp(cy0, cy1);
    let max_y = ((rect.y + rect.height).ceil() as i32)
        .saturating_add(1)
        .clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let hx = rect.width * 0.5;
    let hy = rect.height * 0.5;
    let cx = rect.x + hx;
    let cy = rect.y + hy;
    let safe_radius = if radius.is_finite() { radius } else { 0.0 };
    let r = safe_radius.clamp(0.0, hx.min(hy));
    let rounded_clip = clip.has_rounded();

    let inner_x0 = ((rect.x + r).ceil() as i32 + 1).max(min_x);
    let inner_x1 = ((rect.x + rect.width - r).floor() as i32 - 1).min(max_x);
    let inner_y0 = ((rect.y + 1.0).ceil() as i32).max(min_y);
    let inner_y1 = ((rect.y + rect.height - 1.0).floor() as i32).min(max_y);
    let full_band_y0 = ((rect.y + r).ceil() as i32 + 1).max(min_y);
    let full_band_y1 = ((rect.y + rect.height - r).floor() as i32 - 1).min(max_y);
    let edge_x0 = ((rect.x + 1.0).ceil() as i32).max(min_x);
    let edge_x1 = ((rect.x + rect.width - 1.0).floor() as i32).min(max_x);

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let (rect_l, rect_r) = if py >= full_band_y0 && py < full_band_y1 && edge_x0 < edge_x1 {
            (edge_x0, edge_x1)
        } else if py >= inner_y0 && py < inner_y1 && inner_x0 < inner_x1 {
            (inner_x0, inner_x1)
        } else {
            (max_x, max_x)
        };
        let (span_l, span_r) = if rounded_clip {
            clip.interior_span_for_row(py, rect_l, rect_r)
        } else {
            (rect_l, rect_r)
        };

        let left_end = span_l.min(max_x);
        for px in min_x..left_end {
            let fx = px as f32 + 0.5;
            let clip_cov = if rounded_clip {
                clip.rounded_coverage(fx, fy)
            } else {
                1.0
            };
            if clip_cov <= 0.0 {
                continue;
            }
            let sdf = rounded_rect_sdf(fx, fy, cx, cy, hx, hy, r);
            let cov = (0.5 - sdf).clamp(0.0, 1.0) * clip_cov;
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
        if span_l < span_r {
            buf.blend_span(py, span_l, span_r, color, opacity);
        }
        let right_start = if span_l < span_r { span_r } else { left_end };
        for px in right_start..max_x {
            let fx = px as f32 + 0.5;
            let clip_cov = if rounded_clip {
                clip.rounded_coverage(fx, fy)
            } else {
                1.0
            };
            if clip_cov <= 0.0 {
                continue;
            }
            let sdf = rounded_rect_sdf(fx, fy, cx, cy, hx, hy, r);
            let cov = (0.5 - sdf).clamp(0.0, 1.0) * clip_cov;
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}

pub(crate) fn stroke_rounded_rect_prim(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    rect: LayoutRect,
    radius: f32,
    stroke_width: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || !stroke_width.is_finite()
        || !opacity.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
        || stroke_width <= 0.0
        || color.a == 0
        || opacity <= 0.0
        || clip.is_empty()
    {
        return;
    }
    // Inset stroke centerline by `half` so borders sit strictly inside the border box `[0, bw]`,
    // matching `gpu::skia_primitives::draw_stroke_rrect` and meeting inset shadows seamlessly.
    let half = (stroke_width * 0.5)
        .min(rect.width * 0.5)
        .min(rect.height * 0.5);
    let hw = (stroke_width * 0.5).max(0.5);
    let pad = ((hw - half).max(0.0).ceil() as i32).saturating_add(1);
    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (rect.x.floor() as i32).saturating_sub(pad).clamp(cx0, cx1);
    let max_x = ((rect.x + rect.width).ceil() as i32)
        .saturating_add(pad)
        .clamp(cx0, cx1);
    let min_y = (rect.y.floor() as i32).saturating_sub(pad).clamp(cy0, cy1);
    let max_y = ((rect.y + rect.height).ceil() as i32)
        .saturating_add(pad)
        .clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let cx = rect.x + rect.width * 0.5;
    let cy = rect.y + rect.height * 0.5;
    let hx = (rect.width * 0.5 - half).max(0.0);
    let hy = (rect.height * 0.5 - half).max(0.0);
    let safe_radius = if radius.is_finite() {
        radius.max(0.0)
    } else {
        0.0
    };
    let r = (safe_radius - half).max(0.0).min(hx.min(hy));
    let rounded_clip = clip.has_rounded();

    let margin = half + hw + r + 1.5;
    let skip_x0 = ((rect.x + margin).ceil() as i32).max(min_x);
    let skip_x1 = ((rect.x + rect.width - margin).floor() as i32).min(max_x);
    let skip_y0 = ((rect.y + margin).ceil() as i32).max(min_y);
    let skip_y1 = ((rect.y + rect.height - margin).floor() as i32).min(max_y);

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let can_skip_mid = py >= skip_y0 && py < skip_y1 && skip_x0 < skip_x1;
        let mut px = min_x;
        while px < max_x {
            if can_skip_mid && px == skip_x0 {
                px = skip_x1;
                continue;
            }
            let fx = px as f32 + 0.5;
            let clip_cov = if rounded_clip {
                clip.rounded_coverage(fx, fy)
            } else {
                1.0
            };
            if clip_cov > 0.0 {
                let sdf = rounded_rect_sdf(fx, fy, cx, cy, hx, hy, r);
                let band_dist = sdf.abs() - hw;
                let cov = (0.5 - band_dist).clamp(0.0, 1.0) * clip_cov;
                if cov > 0.0 {
                    buf.blend_pixel(px, py, color, cov * opacity);
                }
            }
            px += 1;
        }
    }
}

/// Rasterizes a retained `DisplayList` into `buffer` using the shared [`CosmicTextEngine`].
pub fn rasterize_display_list(
    display_list: &DisplayList,
    buffer: &mut PixelBuffer,
    scale_factor: f32,
    clear_color: ColorRgba,
) {
    let engine = CosmicTextEngine::shared();
    rasterize_display_list_with_text_engine(
        display_list,
        buffer,
        scale_factor,
        clear_color,
        engine.as_ref(),
    );
}

/// Rasterizes a retained `DisplayList` into `buffer` using an explicit [`CosmicTextEngine`].
pub fn rasterize_display_list_with_text_engine(
    display_list: &DisplayList,
    buffer: &mut PixelBuffer,
    scale_factor: f32,
    clear_color: ColorRgba,
    text_engine: &CosmicTextEngine,
) {
    rasterize_display_list_inner(
        display_list,
        buffer,
        scale_factor,
        clear_color,
        Some(text_engine),
        None,
    );
}

/// Rasterizes a retained `DisplayList` into `buffer`, dispatching any `PaintOp::Plugin` ops
/// through `registry` against `store`.
#[allow(clippy::too_many_arguments)]
pub fn rasterize_display_list_with_plugins(
    display_list: &DisplayList,
    buffer: &mut PixelBuffer,
    scale_factor: f32,
    clear_color: ColorRgba,
    text_engine: &CosmicTextEngine,
    store: &NodeStore,
    registry: &SkiaTableRegistry,
    events: &mut EventBatchBuf,
) {
    rasterize_display_list_inner(
        display_list,
        buffer,
        scale_factor,
        clear_color,
        Some(text_engine),
        Some((store, registry, events)),
    );
}

/// Rasterizes a retained `DisplayList` into `buffer` with `text_engine` and optional plugin context.
pub(crate) fn rasterize_display_list_with_text_engine_and_plugins(
    display_list: &DisplayList,
    buffer: &mut PixelBuffer,
    scale_factor: f32,
    clear_color: ColorRgba,
    text_engine: &CosmicTextEngine,
    plugins: Option<(&NodeStore, &SkiaTableRegistry, &mut EventBatchBuf)>,
) {
    rasterize_display_list_inner(
        display_list,
        buffer,
        scale_factor,
        clear_color,
        Some(text_engine),
        plugins,
    );
}

/// Rasterizes a retained `DisplayList` into `buffer` using the deterministic embedded 5x7
/// bitmap font.
pub fn rasterize_display_list_bitmap(
    display_list: &DisplayList,
    buffer: &mut PixelBuffer,
    scale_factor: f32,
    clear_color: ColorRgba,
) {
    rasterize_display_list_inner(display_list, buffer, scale_factor, clear_color, None, None);
}

fn rasterize_display_list_inner(
    display_list: &DisplayList,
    buffer: &mut PixelBuffer,
    scale_factor: f32,
    clear_color: ColorRgba,
    text_engine: Option<&CosmicTextEngine>,
    plugins: Option<(&NodeStore, &SkiaTableRegistry, &mut EventBatchBuf)>,
) {
    let mut ctx = SoftwareRasterCtx::new(buffer, scale_factor, clear_color, text_engine, plugins);

    for op in &display_list.ops {
        match op {
            PaintOp::SaveOpacity(alpha) => ctx.push_opacity(*alpha),
            PaintOp::Restore => ctx.pop_opacity(),
            PaintOp::PushClip { rect, radius } => ctx.push_clip(*rect, *radius),
            PaintOp::PopClip => ctx.pop_clip(),
            PaintOp::PushTransform { matrix } => ctx.push_transform(*matrix),
            PaintOp::PopTransform => ctx.pop_transform(),
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
            } => ctx.draw_shadow(
                *rect,
                *radius,
                *border_width,
                *offset_x,
                *offset_y,
                *blur_radius,
                *spread_radius,
                *color,
                *inset,
            ),
            PaintOp::FillRoundedRect {
                rect,
                radius,
                color,
            } => ctx.fill_rrect(*rect, *radius, *color),
            PaintOp::FillRoundedRectGradient {
                rect,
                radius,
                gradient,
            } => ctx.fill_rrect_gradient(*rect, *radius, gradient),
            PaintOp::StrokeRoundedRect {
                rect,
                radius,
                width,
                color,
            } => ctx.stroke_rrect(*rect, *radius, *width, *color),
            PaintOp::DrawText {
                rect,
                text,
                font_size,
                font_weight,
                text_align,
                color,
            } => ctx.draw_text(*rect, text, *font_size, *font_weight, *text_align, *color),
            PaintOp::DrawImage {
                rect,
                radius,
                src,
                fit,
                resolved: _,
            } => ctx.draw_image(*rect, *radius, src, *fit),
            PaintOp::Plugin {
                node_id,
                rect,
                inherited_text_color,
                plugin_epoch: _,
            } => ctx.draw_plugin(*node_id, *rect, *inherited_text_color),
        }
    }
}

#[inline]
pub(crate) fn scale_rect(rect: LayoutRect, scale: f32) -> LayoutRect {
    LayoutRect {
        x: rect.x * scale,
        y: rect.y * scale,
        width: rect.width * scale,
        height: rect.height * scale,
    }
}
