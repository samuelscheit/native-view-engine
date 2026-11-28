//! Pure-Rust zero-allocation software replay sink for [`super::CompositorScene`].

use nve_abi::{ColorRgba, EventBatchBuf};
use nve_engine::LayoutRect;
use nve_store::NodeStore;

use super::chunks::LocalPaintOp;
use super::replay::{replay_scene_into, snap_world_rect, translate_rect, ReplaySink};
use super::CompositorScene;
use crate::raster::clip::ClipStack;
use crate::raster::text_blend::draw_bitmap_text;
use crate::raster::transform::{
    fill_rounded_rect_transformed, map_logical_rect_approx, stroke_rounded_rect_transformed,
    TransformStack, TransformedStroke,
};
use crate::raster::{
    draw_box_shadow_prim, draw_image_prim, draw_inset_box_shadow_prim, fill_rounded_rect,
    fill_rounded_rect_gradient_prim, fill_rounded_rect_gradient_transformed_prim, scale_rect,
    stroke_rounded_rect_prim, OpacityStack, PixelBuffer,
};
use crate::table_system::{SkiaSystemPaintCtx, SkiaTableRegistry};
use crate::text::CosmicTextEngine;

struct SoftwareReplaySink<'a> {
    buffer: &'a mut PixelBuffer,
    scale: f32,
    text_engine: Option<&'a CosmicTextEngine>,
    plugins: Option<(&'a NodeStore, &'a SkiaTableRegistry, &'a mut EventBatchBuf)>,
    opacity_stack: OpacityStack,
    clip_stack: ClipStack,
    transform_stack: TransformStack,
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
        let safe = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.opacity_stack.push_mul(safe);
    }

    #[inline]
    fn pop_opacity(&mut self) {
        self.opacity_stack.pop();
    }

    #[inline]
    fn push_transform(&mut self, matrix: [f32; 6]) {
        let safe = if matrix.iter().all(|v| v.is_finite()) {
            matrix
        } else {
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        };
        self.transform_stack.push_logical(safe, self.scale);
    }

    #[inline]
    fn pop_transform(&mut self) {
        self.transform_stack.pop();
    }

    #[inline]
    fn push_clip(&mut self, rect: LayoutRect, radius: f32) {
        let eff = if !self.transform_stack.is_identity() {
            map_logical_rect_approx(self.transform_stack.current(), rect, self.scale)
        } else {
            rect
        };
        self.clip_stack
            .push(scale_rect(eff, self.scale), radius * self.scale);
    }

    #[inline]
    fn pop_clip(&mut self) {
        self.clip_stack.pop();
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
        let cur_opacity = self.opacity_stack.current();
        let is_tf = !self.transform_stack.is_identity();
        let scale = self.scale;
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
                let eff = if is_tf {
                    map_logical_rect_approx(self.transform_stack.current(), rect, scale)
                } else {
                    rect
                };
                if inset {
                    draw_inset_box_shadow_prim(
                        self.buffer,
                        &self.clip_stack,
                        scale_rect(eff, scale),
                        radius * scale,
                        border_width * scale,
                        offset_x * scale,
                        offset_y * scale,
                        blur_radius * scale,
                        spread_radius * scale,
                        color,
                        cur_opacity,
                    );
                } else {
                    draw_box_shadow_prim(
                        self.buffer,
                        &self.clip_stack,
                        scale_rect(eff, scale),
                        radius * scale,
                        offset_x * scale,
                        offset_y * scale,
                        blur_radius * scale,
                        spread_radius * scale,
                        color,
                        cur_opacity,
                    );
                }
            }
            LocalPaintOp::FillRoundedRect {
                local_rect,
                radius,
                color,
            } => {
                let rect = translate_rect(local_rect, wx, wy);
                if is_tf {
                    fill_rounded_rect_transformed(
                        self.buffer,
                        &self.clip_stack,
                        self.transform_stack.current(),
                        scale_rect(rect, scale),
                        radius * scale,
                        color,
                        cur_opacity,
                    );
                } else {
                    fill_rounded_rect(
                        self.buffer,
                        &self.clip_stack,
                        scale_rect(rect, scale),
                        radius * scale,
                        color,
                        cur_opacity,
                    );
                }
            }
            LocalPaintOp::FillSnappedRect {
                local_rect,
                min_local_x,
                max_local_x,
                color,
            } => {
                let rect = snap_world_rect(local_rect, min_local_x, max_local_x, wx, wy, scale);
                if is_tf {
                    fill_rounded_rect_transformed(
                        self.buffer,
                        &self.clip_stack,
                        self.transform_stack.current(),
                        scale_rect(rect, scale),
                        0.0,
                        color,
                        cur_opacity,
                    );
                } else {
                    fill_rounded_rect(
                        self.buffer,
                        &self.clip_stack,
                        scale_rect(rect, scale),
                        0.0,
                        color,
                        cur_opacity,
                    );
                }
            }
            LocalPaintOp::FillRoundedRectGradient {
                local_rect,
                radius,
                gradient_idx,
            } => {
                let Some(gradient) = scene.arenas.gradient(gradient_idx) else {
                    return;
                };
                let rect = translate_rect(local_rect, wx, wy);
                if is_tf {
                    fill_rounded_rect_gradient_transformed_prim(
                        self.buffer,
                        &self.clip_stack,
                        self.transform_stack.current(),
                        scale_rect(rect, scale),
                        radius * scale,
                        gradient,
                        cur_opacity,
                    );
                } else {
                    fill_rounded_rect_gradient_prim(
                        self.buffer,
                        &self.clip_stack,
                        scale_rect(rect, scale),
                        radius * scale,
                        gradient,
                        cur_opacity,
                    );
                }
            }
            LocalPaintOp::StrokeRoundedRect {
                local_rect,
                radius,
                width: stroke_w,
                color,
            } => {
                let rect = translate_rect(local_rect, wx, wy);
                if is_tf {
                    stroke_rounded_rect_transformed(
                        self.buffer,
                        &self.clip_stack,
                        self.transform_stack.current(),
                        TransformedStroke {
                            rect: scale_rect(rect, scale),
                            radius: radius * scale,
                            width: stroke_w * scale,
                            color,
                            opacity: cur_opacity,
                        },
                    );
                } else {
                    stroke_rounded_rect_prim(
                        self.buffer,
                        &self.clip_stack,
                        scale_rect(rect, scale),
                        radius * scale,
                        stroke_w * scale,
                        color,
                        cur_opacity,
                    );
                }
            }
            LocalPaintOp::PushLocalClip { local_rect, radius } => {
                self.push_clip(translate_rect(local_rect, wx, wy), radius);
            }
            LocalPaintOp::PopLocalClip => {
                self.pop_clip();
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
                if self.clip_stack.is_empty() {
                    return;
                }
                let text = scene.arenas.str_slice(str_off, str_len);
                let rect = translate_rect(local_rect, wx, wy);
                let eff = if is_tf {
                    map_logical_rect_approx(self.transform_stack.current(), rect, scale)
                } else {
                    rect
                };
                let phys_clip = Some(self.clip_stack.bounds());
                if let Some(engine) = self.text_engine {
                    engine.rasterize_text_styled_clipped(
                        self.buffer,
                        eff,
                        text,
                        font_size,
                        font_weight,
                        text_align,
                        scale,
                        color,
                        cur_opacity,
                        phys_clip,
                    );
                } else {
                    draw_bitmap_text(
                        self.buffer,
                        eff,
                        text,
                        font_size,
                        font_weight,
                        text_align,
                        scale,
                        color,
                        cur_opacity,
                        phys_clip,
                    );
                }
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
                let rect = translate_rect(local_rect, wx, wy);
                let eff = if is_tf {
                    map_logical_rect_approx(self.transform_stack.current(), rect, scale)
                } else {
                    rect
                };
                draw_image_prim(
                    self.buffer,
                    &self.clip_stack,
                    eff,
                    radius,
                    src,
                    fit,
                    scale,
                    cur_opacity,
                );
            }
            LocalPaintOp::Plugin {
                node_id,
                local_rect,
                inherited_text_color,
                plugin_epoch: _,
            } => {
                if self.clip_stack.is_empty() {
                    return;
                }
                if let Some((store, registry, ref mut events)) = self.plugins {
                    let rect = translate_rect(local_rect, wx, wy);
                    let eff = if is_tf {
                        map_logical_rect_approx(self.transform_stack.current(), rect, scale)
                    } else {
                        rect
                    };
                    let scaled = scale_rect(eff, scale);
                    let [cx0, cy0, cx1, cy1] = self.clip_stack.bounds();
                    let clip_rect = LayoutRect {
                        x: cx0 as f32,
                        y: cy0 as f32,
                        width: (cx1 - cx0).max(0) as f32,
                        height: (cy1 - cy0).max(0) as f32,
                    };
                    let mut ctx = SkiaSystemPaintCtx {
                        buffer: self.buffer,
                        rect: scaled,
                        clip_rect: Some(clip_rect),
                        scale_factor: scale,
                        opacity: cur_opacity,
                        inherited_text_color,
                        events,
                    };
                    registry.paint_node(&mut ctx, store, node_id);
                }
            }
        }
    }

    #[inline]
    fn paint_scrollbar_thumb(&mut self, rect: LayoutRect, radius: f32, color: ColorRgba) {
        let cur_opacity = self.opacity_stack.current();
        if !self.transform_stack.is_identity() {
            fill_rounded_rect_transformed(
                self.buffer,
                &self.clip_stack,
                self.transform_stack.current(),
                scale_rect(rect, self.scale),
                radius * self.scale,
                color,
                cur_opacity,
            );
        } else {
            fill_rounded_rect(
                self.buffer,
                &self.clip_stack,
                scale_rect(rect, self.scale),
                radius * self.scale,
                color,
                cur_opacity,
            );
        }
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
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    buffer.clear(clear_color);
    let mut sink = SoftwareReplaySink {
        clip_stack: ClipStack::new(buffer.width(), buffer.height()),
        buffer,
        scale,
        text_engine,
        plugins,
        opacity_stack: OpacityStack::new(),
        transform_stack: TransformStack::new(),
    };
    replay_scene_into(scene, &mut sink);
}
