//! Skia `Canvas` replay sink (`--features skia`) for [`super::CompositorScene`], reusing the
//! unified `CosmicTextEngine` `TextBlob` + `SlabLru` caches and GPU `DirectContext`.
//!
//! ## Group Opacity Semantics
//! Skia uses CSS-spec isolated group compositing via `Canvas::save_layer_alpha_f` (deferred until
//! the first actual draw op inside the scope so empty or culled translucent subtrees never allocate
//! an offscreen layer). Contrast with [`super::replay_software`], which multiplies per-primitive
//! alpha via `OpacityStack::push_mul` to stay strictly zero-allocation without offscreen layer
//! buffers.

use nve_abi::{ColorRgba, EventBatchBuf};
use nve_engine::LayoutRect;
use nve_store::NodeStore;
#[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
use skia_safe::gpu::DirectContext;
use skia_safe::{Canvas, FontMgr, Matrix};

use super::chunks::LocalPaintOp;
use super::replay::{replay_scene_into, snap_world_rect, translate_rect, ReplaySink};
use super::CompositorScene;
use crate::gpu::renderer_cache::SkiaRendererCaches;
use crate::gpu::skia_graphics::draw_rounded_rect_gradient;
use crate::gpu::skia_primitives::{
    apply_clip, draw_fill_rrect, draw_image_cached, draw_inset_shadow_cached, draw_plugin_cached,
    draw_shadow_cached, draw_stroke_rrect, draw_text_cached, FramePaints,
};
use crate::table_system::SkiaTableRegistry;
use crate::text::CosmicTextEngine;

const INLINE_SCOPE_DEPTH: usize = 64;

#[derive(Clone, Copy)]
enum DeferredScope {
    Opacity(f32),
    Transform([f32; 6]),
    Clip(LayoutRect, f32),
}

struct ScopeStack {
    inline: [DeferredScope; INLINE_SCOPE_DEPTH],
    len: usize,
    flushed: usize,
    overflow: Vec<DeferredScope>,
}

impl ScopeStack {
    #[inline]
    fn new() -> Self {
        Self {
            inline: [DeferredScope::Opacity(1.0); INLINE_SCOPE_DEPTH],
            len: 0,
            flushed: 0,
            overflow: Vec::new(),
        }
    }

    #[inline]
    fn push(&mut self, scope: DeferredScope) {
        if self.len < INLINE_SCOPE_DEPTH {
            self.inline[self.len] = scope;
        } else {
            self.overflow.push(scope);
        }
        self.len += 1;
    }

    #[inline]
    fn get(&self, idx: usize) -> DeferredScope {
        if idx < INLINE_SCOPE_DEPTH {
            self.inline[idx]
        } else {
            self.overflow[idx - INLINE_SCOPE_DEPTH]
        }
    }

    #[inline]
    fn flush_to_canvas(&mut self, canvas: &Canvas) {
        while self.flushed < self.len {
            match self.get(self.flushed) {
                DeferredScope::Opacity(alpha) => {
                    if alpha >= 0.999 {
                        canvas.save();
                    } else {
                        canvas.save_layer_alpha_f(None, alpha);
                    }
                }
                DeferredScope::Transform(matrix) => {
                    canvas.save();
                    if matrix.iter().all(|v| v.is_finite()) {
                        let m = Matrix::new_all(
                            matrix[0], matrix[2], matrix[4], matrix[1], matrix[3], matrix[5], 0.0,
                            0.0, 1.0,
                        );
                        canvas.concat(&m);
                    }
                }
                DeferredScope::Clip(rect, radius) => {
                    canvas.save();
                    apply_clip(canvas, rect, radius);
                }
            }
            self.flushed += 1;
        }
    }

    #[inline]
    fn pop(&mut self, canvas: &Canvas) {
        if self.len == 0 {
            return;
        }
        self.len -= 1;
        if self.len >= INLINE_SCOPE_DEPTH {
            self.overflow.pop();
        }
        if self.flushed > self.len {
            canvas.restore();
            self.flushed = self.len;
        }
    }
}

struct SkiaReplaySink<'a> {
    canvas: &'a Canvas,
    font_mgr: &'a FontMgr,
    text_engine: &'a CosmicTextEngine,
    caches: &'a mut SkiaRendererCaches,
    paints: FramePaints,
    scale: f32,
    context_tag: u32,
    scopes: ScopeStack,
    plugins: Option<(&'a NodeStore, &'a SkiaTableRegistry, &'a mut EventBatchBuf)>,
    #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
    direct_context: Option<&'a mut DirectContext>,
}

impl ReplaySink for SkiaReplaySink<'_> {
    /// Defers `Canvas::save_layer_alpha_f` until the first actual draw op inside the scope so
    /// empty or culled translucent subtrees allocate zero offscreen Skia layers.
    #[inline]
    fn push_opacity(&mut self, alpha: f32) {
        let safe = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.scopes.push(DeferredScope::Opacity(safe));
    }

    #[inline]
    fn pop_opacity(&mut self) {
        self.scopes.pop(self.canvas);
    }

    #[inline]
    fn push_transform(&mut self, matrix: [f32; 6]) {
        self.scopes.push(DeferredScope::Transform(matrix));
    }

    #[inline]
    fn pop_transform(&mut self) {
        self.scopes.pop(self.canvas);
    }

    #[inline]
    fn push_clip(&mut self, rect: LayoutRect, radius: f32) {
        self.scopes.push(DeferredScope::Clip(rect, radius));
    }

    #[inline]
    fn pop_clip(&mut self) {
        self.scopes.pop(self.canvas);
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
            LocalPaintOp::PushLocalClip { local_rect, radius } => {
                self.push_clip(translate_rect(local_rect, wx, wy), radius);
                return;
            }
            LocalPaintOp::PopLocalClip => {
                self.pop_clip();
                return;
            }
            _ => {}
        }

        self.scopes.flush_to_canvas(self.canvas);

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
                if inset {
                    draw_inset_shadow_cached(
                        self.canvas,
                        self.caches,
                        &mut self.paints.shadow,
                        rect,
                        radius,
                        border_width,
                        offset_x,
                        offset_y,
                        blur_radius,
                        spread_radius,
                        color,
                    );
                } else {
                    draw_shadow_cached(
                        self.canvas,
                        self.caches,
                        &mut self.paints.shadow,
                        rect,
                        radius,
                        offset_x,
                        offset_y,
                        blur_radius,
                        spread_radius,
                        color,
                    );
                }
            }
            LocalPaintOp::FillRoundedRect {
                local_rect,
                radius,
                color,
            } => {
                draw_fill_rrect(
                    self.canvas,
                    &mut self.paints.fill,
                    translate_rect(local_rect, wx, wy),
                    radius,
                    color,
                );
            }
            LocalPaintOp::FillSnappedRect {
                local_rect,
                min_local_x,
                max_local_x,
                color,
            } => {
                draw_fill_rrect(
                    self.canvas,
                    &mut self.paints.fill,
                    snap_world_rect(local_rect, min_local_x, max_local_x, wx, wy, self.scale),
                    0.0,
                    color,
                );
            }
            LocalPaintOp::FillRoundedRectGradient {
                local_rect,
                radius,
                gradient_idx,
            } => {
                if let Some(gradient) = scene.arenas.gradient(gradient_idx) {
                    draw_rounded_rect_gradient(
                        self.canvas,
                        translate_rect(local_rect, wx, wy),
                        radius,
                        gradient,
                        &mut self.paints.gradient,
                        &mut self.caches.grad_scratch,
                    );
                }
            }
            LocalPaintOp::StrokeRoundedRect {
                local_rect,
                radius,
                width: stroke_w,
                color,
            } => {
                draw_stroke_rrect(
                    self.canvas,
                    &mut self.paints.stroke,
                    translate_rect(local_rect, wx, wy),
                    radius,
                    stroke_w,
                    color,
                );
            }
            LocalPaintOp::PushLocalClip { .. } | LocalPaintOp::PopLocalClip => {}
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
                draw_text_cached(
                    self.canvas,
                    self.font_mgr,
                    self.text_engine,
                    self.caches,
                    &mut self.paints.text,
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
                handle_idx,
            } => {
                let src = scene.arenas.str_slice(str_off, str_len);
                let prev_handle = scene.arenas.image_handle(handle_idx);
                let next_handle = draw_image_cached(
                    self.canvas,
                    self.caches,
                    &self.paints.image,
                    translate_rect(local_rect, wx, wy),
                    radius,
                    src,
                    prev_handle,
                    fit,
                    self.context_tag,
                    #[cfg(any(
                        feature = "skia-metal",
                        feature = "skia-vulkan",
                        feature = "skia-d3d"
                    ))]
                    self.direct_context.as_deref_mut(),
                );
                if next_handle != prev_handle {
                    scene.arenas.set_image_handle(handle_idx, next_handle);
                }
            }
            LocalPaintOp::Plugin {
                node_id,
                local_rect,
                inherited_text_color,
                plugin_epoch,
            } => {
                if let Some((store, registry, ref mut events)) = self.plugins {
                    draw_plugin_cached(
                        self.canvas,
                        self.caches,
                        &self.paints.image,
                        node_id,
                        plugin_epoch,
                        translate_rect(local_rect, wx, wy),
                        self.scale,
                        inherited_text_color,
                        store,
                        registry,
                        events,
                        self.context_tag,
                        #[cfg(any(
                            feature = "skia-metal",
                            feature = "skia-vulkan",
                            feature = "skia-d3d"
                        ))]
                        self.direct_context.as_deref_mut(),
                    );
                }
            }
        }
    }

    #[inline]
    fn paint_scrollbar_thumb(&mut self, rect: LayoutRect, radius: f32, color: ColorRgba) {
        self.scopes.flush_to_canvas(self.canvas);
        draw_fill_rrect(self.canvas, &mut self.paints.fill, rect, radius, color);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_scene_onto_skia_canvas(
    scene: &CompositorScene,
    canvas: &Canvas,
    font_mgr: &FontMgr,
    text_engine: &CosmicTextEngine,
    caches: &mut SkiaRendererCaches,
    scale_factor: f32,
    plugins: Option<(&NodeStore, &SkiaTableRegistry, &mut EventBatchBuf)>,
    #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
    mut direct_context: Option<&mut DirectContext>,
) {
    let scale = crate::gpu::sanitize_scale_factor(scale_factor);
    let saved = canvas.save();
    if (scale - 1.0).abs() > f32::EPSILON {
        canvas.scale((scale, scale));
    }

    #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
    let context_tag = caches.sync_gpu_context(direct_context.as_deref_mut());
    #[cfg(not(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d")))]
    let context_tag = 0u32;

    let mut sink = SkiaReplaySink {
        canvas,
        font_mgr,
        text_engine,
        caches,
        paints: FramePaints::new(),
        scale,
        context_tag,
        scopes: ScopeStack::new(),
        plugins,
        #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
        direct_context,
    };
    replay_scene_into(scene, &mut sink);
    canvas.restore_to_count(saved);
}
