//! Shared zero-allocation software primitive raster dispatch used by both flat `DisplayList`
//! rasterization (`raster/mod.rs`) and retained `CompositorScene` replay (`retained/replay_software.rs`).

use nve_abi::{ColorRgba, EventBatchBuf};
use nve_engine::LayoutRect;
use nve_store::{NodeId, NodeStore, Rgba};
use nve_tables::paint::BackgroundFill;
use nve_tables::tables::ImageFit;

use super::canvas::draw_image;
use super::clip::ClipStack;
use super::gradient::{fill_rounded_rect_gradient, fill_rounded_rect_gradient_transformed};
use super::shadow::{draw_box_shadow, draw_inset_box_shadow};
use super::shadow_transformed::{draw_box_shadow_transformed, draw_inset_box_shadow_transformed};
use super::text_blend::draw_bitmap_text;
use super::transform::{
    fill_rounded_rect_transformed, map_logical_rect_approx, stroke_rounded_rect_transformed,
    OpacityStack, TransformStack, TransformedStroke,
};
use super::{fill_rounded_rect, scale_rect, stroke_rounded_rect_prim, PixelBuffer};
use crate::table_system::{SkiaSystemPaintCtx, SkiaTableRegistry};
use crate::text::{CosmicTextEngine, TextAlign};

/// Shared stateful software rasterization context holding active opacity, clip, and transform
/// stacks so `DisplayList` and `CompositorScene` share one exact primitive dispatch table.
pub(crate) struct SoftwareRasterCtx<'a> {
    buffer: &'a mut PixelBuffer,
    scale: f32,
    text_engine: Option<&'a CosmicTextEngine>,
    plugins: Option<(&'a NodeStore, &'a SkiaTableRegistry, &'a mut EventBatchBuf)>,
    opacity_stack: OpacityStack,
    clip_stack: ClipStack,
    transform_stack: TransformStack,
}

impl<'a> SoftwareRasterCtx<'a> {
    #[inline]
    pub(crate) fn new(
        buffer: &'a mut PixelBuffer,
        scale_factor: f32,
        clear_color: ColorRgba,
        text_engine: Option<&'a CosmicTextEngine>,
        plugins: Option<(&'a NodeStore, &'a SkiaTableRegistry, &'a mut EventBatchBuf)>,
    ) -> Self {
        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        buffer.clear(clear_color);
        let clip_stack = ClipStack::new(buffer.width(), buffer.height());
        Self {
            buffer,
            scale,
            text_engine,
            plugins,
            opacity_stack: OpacityStack::new(),
            clip_stack,
            transform_stack: TransformStack::new(),
        }
    }

    #[inline]
    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }

    #[inline]
    pub(crate) fn push_opacity(&mut self, alpha: f32) {
        let safe = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.opacity_stack.push_mul(safe);
    }

    #[inline]
    pub(crate) fn pop_opacity(&mut self) {
        self.opacity_stack.pop();
    }

    #[inline]
    pub(crate) fn push_transform(&mut self, matrix: [f32; 6]) {
        let safe = if matrix.iter().all(|v| v.is_finite()) {
            matrix
        } else {
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        };
        self.transform_stack.push_logical(safe, self.scale);
    }

    #[inline]
    pub(crate) fn pop_transform(&mut self) {
        self.transform_stack.pop();
    }

    #[inline]
    pub(crate) fn push_clip(&mut self, rect: LayoutRect, radius: f32) {
        let eff = if !self.transform_stack.is_identity() {
            map_logical_rect_approx(self.transform_stack.current(), rect, self.scale)
        } else {
            rect
        };
        self.clip_stack
            .push(scale_rect(eff, self.scale), radius * self.scale);
    }

    #[inline]
    pub(crate) fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn draw_shadow(
        &mut self,
        rect: LayoutRect,
        radius: f32,
        border_width: f32,
        offset_x: f32,
        offset_y: f32,
        blur_radius: f32,
        spread_radius: f32,
        color: ColorRgba,
        inset: bool,
    ) {
        let cur_opacity = self.opacity_stack.current();
        let scale = self.scale;
        if !self.transform_stack.is_identity() {
            let m = self.transform_stack.current();
            if inset {
                draw_inset_box_shadow_transformed(
                    self.buffer,
                    &self.clip_stack,
                    m,
                    rect,
                    radius,
                    border_width,
                    offset_x,
                    offset_y,
                    blur_radius,
                    spread_radius,
                    scale,
                    color,
                    cur_opacity,
                );
            } else {
                draw_box_shadow_transformed(
                    self.buffer,
                    &self.clip_stack,
                    m,
                    rect,
                    radius,
                    offset_x,
                    offset_y,
                    blur_radius,
                    spread_radius,
                    scale,
                    color,
                    cur_opacity,
                );
            }
        } else if inset {
            draw_inset_box_shadow(
                self.buffer,
                &self.clip_stack,
                rect,
                radius,
                border_width,
                offset_x,
                offset_y,
                blur_radius,
                spread_radius,
                scale,
                color,
                cur_opacity,
            );
        } else {
            draw_box_shadow(
                self.buffer,
                &self.clip_stack,
                rect,
                radius,
                offset_x,
                offset_y,
                blur_radius,
                spread_radius,
                scale,
                color,
                cur_opacity,
            );
        }
    }

    #[inline]
    pub(crate) fn fill_rrect(&mut self, rect: LayoutRect, radius: f32, color: ColorRgba) {
        let cur_opacity = self.opacity_stack.current();
        let scale = self.scale;
        if !self.transform_stack.is_identity() {
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

    #[inline]
    pub(crate) fn fill_rrect_gradient(
        &mut self,
        rect: LayoutRect,
        radius: f32,
        gradient: &BackgroundFill,
    ) {
        let cur_opacity = self.opacity_stack.current();
        let scale = self.scale;
        if !self.transform_stack.is_identity() {
            fill_rounded_rect_gradient_transformed(
                self.buffer,
                &self.clip_stack,
                self.transform_stack.current(),
                scale_rect(rect, scale),
                radius * scale,
                gradient,
                cur_opacity,
            );
        } else {
            fill_rounded_rect_gradient(
                self.buffer,
                &self.clip_stack,
                scale_rect(rect, scale),
                radius * scale,
                gradient,
                cur_opacity,
            );
        }
    }

    #[inline]
    pub(crate) fn stroke_rrect(
        &mut self,
        rect: LayoutRect,
        radius: f32,
        width: f32,
        color: ColorRgba,
    ) {
        let cur_opacity = self.opacity_stack.current();
        let scale = self.scale;
        if !self.transform_stack.is_identity() {
            stroke_rounded_rect_transformed(
                self.buffer,
                &self.clip_stack,
                self.transform_stack.current(),
                TransformedStroke {
                    rect: scale_rect(rect, scale),
                    radius: radius * scale,
                    width: width * scale,
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
                width * scale,
                color,
                cur_opacity,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn draw_text(
        &mut self,
        rect: LayoutRect,
        text: &str,
        font_size: f32,
        font_weight: u16,
        text_align: TextAlign,
        color: ColorRgba,
    ) {
        if self.clip_stack.is_empty() {
            return;
        }
        let cur_opacity = self.opacity_stack.current();
        let scale = self.scale;
        let eff = if !self.transform_stack.is_identity() {
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

    #[inline]
    pub(crate) fn draw_image(&mut self, rect: LayoutRect, radius: f32, src: &str, fit: ImageFit) {
        let cur_opacity = self.opacity_stack.current();
        let scale = self.scale;
        let eff = if !self.transform_stack.is_identity() {
            map_logical_rect_approx(self.transform_stack.current(), rect, scale)
        } else {
            rect
        };
        draw_image(
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

    #[inline]
    pub(crate) fn draw_plugin(
        &mut self,
        node_id: NodeId,
        rect: LayoutRect,
        inherited_text_color: Rgba,
    ) {
        if self.clip_stack.is_empty() {
            return;
        }
        if let Some((store, registry, ref mut events)) = self.plugins {
            let cur_opacity = self.opacity_stack.current();
            let scale = self.scale;
            let eff = if !self.transform_stack.is_identity() {
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
