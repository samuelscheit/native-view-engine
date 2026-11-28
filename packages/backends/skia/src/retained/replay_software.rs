//! Pure-Rust zero-allocation software replay sink for [`super::CompositorScene`], delegating
//! every primitive op to [`crate::raster::SoftwareRasterCtx`].

use nve_abi::{ColorRgba, EventBatchBuf};
use nve_engine::LayoutRect;
use nve_store::NodeStore;

use super::chunks::LocalPaintOp;
use super::replay::{replay_scene_into, snap_world_rect, translate_rect, ReplaySink};
use super::CompositorScene;
use crate::raster::{PixelBuffer, SoftwareRasterCtx};
use crate::table_system::SkiaTableRegistry;
use crate::text::CosmicTextEngine;

struct SoftwareReplaySink<'a> {
    ctx: SoftwareRasterCtx<'a>,
}

impl ReplaySink for SoftwareReplaySink<'_> {
    /// Multiplies per-primitive alpha (`OpacityStack::push_mul`) to preserve strict
    /// zero-allocation software replay (matching [`crate::raster::rasterize_display_list`]).
    ///
    /// Note: For non-overlapping translucent children this matches Skia's `save_layer_alpha_f`;
    /// for overlapping opaque children inside a translucent group, Skia performs CSS-spec
    /// offscreen group compositing (`save_layer_alpha_f`) whereas software multiplies per op.
    #[inline]
    fn push_opacity(&mut self, alpha: f32) {
        self.ctx.push_opacity(alpha);
    }

    #[inline]
    fn pop_opacity(&mut self) {
        self.ctx.pop_opacity();
    }

    #[inline]
    fn push_transform(&mut self, matrix: [f32; 6]) {
        self.ctx.push_transform(matrix);
    }

    #[inline]
    fn pop_transform(&mut self) {
        self.ctx.pop_transform();
    }

    #[inline]
    fn push_clip(&mut self, rect: LayoutRect, radius: f32) {
        self.ctx.push_clip(rect, radius);
    }

    #[inline]
    fn pop_clip(&mut self) {
        self.ctx.pop_clip();
    }

    fn paint_local_op(
        &mut self,
        scene: &CompositorScene,
        op: LocalPaintOp,
        wx: f32,
        wy: f32,
        width: f32,
        height: f32,
    ) {
        match op {
            LocalPaintOp::DrawShadow {
                radius,
                offset_x,
                offset_y,
                blur_radius,
                spread_radius,
                color,
                border_width,
                inset,
            } => {
                let rect = LayoutRect {
                    x: wx,
                    y: wy,
                    width,
                    height,
                };
                self.ctx.draw_shadow(
                    rect,
                    radius,
                    border_width,
                    offset_x,
                    offset_y,
                    blur_radius,
                    spread_radius,
                    color,
                    inset,
                );
            }
            LocalPaintOp::FillRoundedRect {
                local_rect,
                radius,
                color,
            } => {
                self.ctx
                    .fill_rrect(translate_rect(local_rect, wx, wy), radius, color);
            }
            LocalPaintOp::FillSnappedRect {
                local_rect,
                min_local_x,
                max_local_x,
                color,
            } => {
                let rect =
                    snap_world_rect(local_rect, min_local_x, max_local_x, wx, wy, self.ctx.scale());
                self.ctx.fill_rrect(rect, 0.0, color);
            }
            LocalPaintOp::FillRoundedRectGradient {
                local_rect,
                radius,
                gradient_idx,
            } => {
                if let Some(gradient) = scene.arenas.gradient(gradient_idx) {
                    self.ctx.fill_rrect_gradient(
                        translate_rect(local_rect, wx, wy),
                        radius,
                        gradient,
                    );
                }
            }
            LocalPaintOp::StrokeRoundedRect {
                local_rect,
                radius,
                width: stroke_w,
                color,
            } => {
                self.ctx
                    .stroke_rrect(translate_rect(local_rect, wx, wy), radius, stroke_w, color);
            }
            LocalPaintOp::PushLocalClip { local_rect, radius } => {
                self.ctx
                    .push_clip(translate_rect(local_rect, wx, wy), radius);
            }
            LocalPaintOp::PopLocalClip => {
                self.ctx.pop_clip();
            }
            LocalPaintOp::DrawText {
                local_rect,
                str_off,
                str_len,
                font_size,
                font_weight,
                text_align,
                color,
            } => {
                let text = scene.arenas.str_slice(str_off, str_len);
                self.ctx.draw_text(
                    translate_rect(local_rect, wx, wy),
                    text,
                    font_size,
                    font_weight,
                    text_align,
                    color,
                );
            }
            LocalPaintOp::DrawImage {
                local_rect,
                radius,
                str_off,
                str_len,
                fit,
                handle_idx: _,
            } => {
                let src = scene.arenas.str_slice(str_off, str_len);
                self.ctx
                    .draw_image(translate_rect(local_rect, wx, wy), radius, src, fit);
            }
            LocalPaintOp::Plugin {
                node_id,
                local_rect,
                inherited_text_color,
                plugin_epoch: _,
            } => {
                self.ctx.draw_plugin(
                    node_id,
                    translate_rect(local_rect, wx, wy),
                    inherited_text_color,
                );
            }
        }
    }

    #[inline]
    fn paint_scrollbar_thumb(&mut self, rect: LayoutRect, radius: f32, color: ColorRgba) {
        self.ctx.fill_rrect(rect, radius, color);
    }
}

/// Software-rasterizes a retained [`CompositorScene`] directly into `buffer` with zero
/// intermediate `DisplayList` allocation.
pub(crate) fn rasterize_scene_inner(
    scene: &CompositorScene,
    buffer: &mut PixelBuffer,
    scale_factor: f32,
    clear_color: ColorRgba,
    text_engine: Option<&CosmicTextEngine>,
    plugins: Option<(&NodeStore, &SkiaTableRegistry, &mut EventBatchBuf)>,
) {
    let mut sink = SoftwareReplaySink {
        ctx: SoftwareRasterCtx::new(buffer, scale_factor, clear_color, text_engine, plugins),
    };
    replay_scene_into(scene, &mut sink);
}
