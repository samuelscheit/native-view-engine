//! Skia / software rasterizer adapter for `nve-canvas` (`SkiaTableSystem`).
//!
//! Executes [`DrawList`] vector ops (`Clear`, `FillRect`, `FillRoundedRect`, `FillCircle`,
//! `StrokeLine`, `StrokePolyline`, `ClipRect`, `RestoreClip`) onto `PixelBuffer`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use nve_abi::ColorRgba;
use nve_backend_skia::{PixelBuffer, SkiaSystemPaintCtx, SkiaTableSystem};
use nve_canvas::{Canvas, CanvasOpRef, ClipDepthTracker, DrawList, MAX_CLIP_DEPTH};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};

/// `SkiaTableSystem` plugin painting [`Canvas`] (`nve.canvas`) draw lists onto `PixelBuffer`.
#[derive(Debug, Default, Clone, Copy)]
pub struct CanvasSkiaSystem;

#[derive(Clone, Copy, Default)]
struct ClipRectPx {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl ClipRectPx {
    fn intersect(self, other: Self) -> Self {
        Self {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
    }
}

/// Zero-heap-allocation clip stack enforcing [`MAX_CLIP_DEPTH`] (`64`) with overflow balance.
struct ClipStackInline {
    stack: [ClipRectPx; MAX_CLIP_DEPTH + 1],
    tracker: ClipDepthTracker,
}

impl ClipStackInline {
    fn new(base: ClipRectPx) -> Self {
        let mut stack = [ClipRectPx::default(); MAX_CLIP_DEPTH + 1];
        stack[0] = base;
        Self {
            stack,
            tracker: ClipDepthTracker::new(),
        }
    }

    #[inline]
    fn top(&self) -> ClipRectPx {
        self.stack[self.tracker.depth()]
    }

    #[inline]
    fn push(&mut self, c: ClipRectPx) {
        if self.tracker.push() {
            self.stack[self.tracker.depth()] = c;
        }
    }

    #[inline]
    fn pop(&mut self) {
        let _ = self.tracker.pop();
    }
}

impl SkiaTableSystem for CanvasSkiaSystem {
    fn slot(&self) -> TableSlot {
        Canvas::slot()
    }

    fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, store: &NodeStore, node: NodeId) {
        let Some(row) = store.try_row::<Canvas>(node) else {
            return;
        };
        let Some(dl) = store.resource::<DrawList>(row.draw_list()) else {
            return;
        };
        if !ctx.rect.x.is_finite()
            || !ctx.rect.y.is_finite()
            || !ctx.rect.width.is_finite()
            || !ctx.rect.height.is_finite()
            || ctx.rect.width <= 0.0
            || ctx.rect.height <= 0.0
            || !ctx.opacity.is_finite()
            || ctx.opacity <= 0.0
        {
            return;
        }
        let bw = ctx.buffer.width() as i32;
        let bh = ctx.buffer.height() as i32;
        let mut base_clip = ClipRectPx {
            x0: (ctx.rect.x.floor() as i32).clamp(0, bw),
            y0: (ctx.rect.y.floor() as i32).clamp(0, bh),
            x1: ((ctx.rect.x + ctx.rect.width).ceil() as i32).clamp(0, bw),
            y1: ((ctx.rect.y + ctx.rect.height).ceil() as i32).clamp(0, bh),
        };
        if let Some(cr) = ctx.clip_rect
            && cr.x.is_finite()
            && cr.y.is_finite()
            && cr.width.is_finite()
            && cr.height.is_finite()
        {
            let outer = ClipRectPx {
                x0: (cr.x.floor() as i32).clamp(0, bw),
                y0: (cr.y.floor() as i32).clamp(0, bh),
                x1: ((cr.x + cr.width).ceil() as i32).clamp(0, bw),
                y1: ((cr.y + cr.height).ceil() as i32).clamp(0, bh),
            };
            base_clip = base_clip.intersect(outer);
        }
        if base_clip.x0 >= base_clip.x1 || base_clip.y0 >= base_clip.y1 {
            return;
        }
        let scale = if ctx.scale_factor.is_finite() && ctx.scale_factor > 0.0 {
            ctx.scale_factor
        } else {
            1.0
        };
        let mut clips = ClipStackInline::new(base_clip);
        let ox = ctx.rect.x;
        let oy = ctx.rect.y;

        for op in dl.ops() {
            let clip = clips.top();
            match op {
                CanvasOpRef::Clear { color } => {
                    if color.a == 0 {
                        clear_rect_overwrite(ctx.buffer, clip, color);
                    } else {
                        fill_rect(
                            ctx.buffer,
                            clip,
                            ox,
                            oy,
                            ctx.rect.width,
                            ctx.rect.height,
                            0.0,
                            color,
                            ctx.opacity,
                        );
                    }
                }
                CanvasOpRef::FillRect { x, y, w, h, color } => {
                    fill_rect(
                        ctx.buffer,
                        clip,
                        ox + x * scale,
                        oy + y * scale,
                        w * scale,
                        h * scale,
                        0.0,
                        color,
                        ctx.opacity,
                    );
                }
                CanvasOpRef::FillRoundedRect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    color,
                } => {
                    fill_rect(
                        ctx.buffer,
                        clip,
                        ox + x * scale,
                        oy + y * scale,
                        w * scale,
                        h * scale,
                        radius * scale,
                        color,
                        ctx.opacity,
                    );
                }
                CanvasOpRef::FillCircle {
                    cx,
                    cy,
                    radius,
                    color,
                } => {
                    fill_circle(
                        ctx.buffer,
                        clip,
                        ox + cx * scale,
                        oy + cy * scale,
                        radius * scale,
                        color,
                        ctx.opacity,
                    );
                }
                CanvasOpRef::StrokeLine {
                    x0,
                    y0,
                    x1,
                    y1,
                    width,
                    color,
                } => {
                    stroke_line(
                        ctx.buffer,
                        clip,
                        ox + x0 * scale,
                        oy + y0 * scale,
                        ox + x1 * scale,
                        oy + y1 * scale,
                        width * scale,
                        color,
                        ctx.opacity,
                    );
                }
                CanvasOpRef::StrokePolyline {
                    points,
                    width,
                    color,
                } => {
                    if points.len() == 1 {
                        if let Some((px, py)) = points.get(0) {
                            fill_circle(
                                ctx.buffer,
                                clip,
                                ox + px * scale,
                                oy + py * scale,
                                (width * scale * 0.5).max(0.5),
                                color,
                                ctx.opacity,
                            );
                        }
                    } else {
                        let mut iter = points.iter();
                        if let Some(mut prev) = iter.next() {
                            for cur in iter {
                                stroke_line(
                                    ctx.buffer,
                                    clip,
                                    ox + prev.0 * scale,
                                    oy + prev.1 * scale,
                                    ox + cur.0 * scale,
                                    oy + cur.1 * scale,
                                    width * scale,
                                    color,
                                    ctx.opacity,
                                );
                                prev = cur;
                            }
                        }
                    }
                }
                CanvasOpRef::ClipRect { x, y, w, h } => {
                    let next = if x.is_finite()
                        && y.is_finite()
                        && w.is_finite()
                        && h.is_finite()
                        && w > 0.0
                        && h > 0.0
                    {
                        ClipRectPx {
                            x0: ((ox + x * scale).floor() as i32).clamp(0, bw),
                            y0: ((oy + y * scale).floor() as i32).clamp(0, bh),
                            x1: ((ox + (x + w) * scale).ceil() as i32).clamp(0, bw),
                            y1: ((oy + (y + h) * scale).ceil() as i32).clamp(0, bh),
                        }
                    } else {
                        ClipRectPx::default()
                    };
                    clips.push(clip.intersect(next));
                }
                CanvasOpRef::RestoreClip => {
                    clips.pop();
                }
            }
        }
    }
}

fn clear_rect_overwrite(buf: &mut PixelBuffer, clip: ClipRectPx, color: ColorRgba) {
    buf.fill_rect_rgba(clip.x0, clip.y0, clip.x1, clip.y1, color);
}

#[allow(clippy::too_many_arguments)]
fn fill_rect(
    buf: &mut PixelBuffer,
    clip: ClipRectPx,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if !x.is_finite()
        || !y.is_finite()
        || !w.is_finite()
        || !h.is_finite()
        || w <= 0.0
        || h <= 0.0
        || color.a == 0
        || !opacity.is_finite()
        || opacity <= 0.0
    {
        return;
    }
    let x0 = (x.floor() as i32).max(clip.x0);
    let x1 = ((x + w).ceil() as i32).min(clip.x1);
    let y0 = (y.floor() as i32).max(clip.y0);
    let y1 = ((y + h).ceil() as i32).min(clip.y1);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let raw_r = if radius.is_finite() { radius } else { 0.0 };
    let r = raw_r.clamp(0.0, (w * 0.5).min(h * 0.5));
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let hx = w * 0.5 - r;
    let hy = h * 0.5 - r;

    for py in y0..y1 {
        let fy = py as f32 + 0.5;
        for px in x0..x1 {
            let fx = px as f32 + 0.5;
            let cov = if r <= 0.0 {
                1.0
            } else {
                let dx = (fx - cx).abs() - hx;
                let dy = (fy - cy).abs() - hy;
                let dist = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r;
                (0.5 - dist).clamp(0.0, 1.0)
            };
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}

fn fill_circle(
    buf: &mut PixelBuffer,
    clip: ClipRectPx,
    cx: f32,
    cy: f32,
    radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if !cx.is_finite()
        || !cy.is_finite()
        || !radius.is_finite()
        || radius <= 0.0
        || color.a == 0
        || !opacity.is_finite()
        || opacity <= 0.0
    {
        return;
    }
    let x0 = ((cx - radius - 1.0).floor() as i32).max(clip.x0);
    let x1 = ((cx + radius + 1.0).ceil() as i32).min(clip.x1);
    let y0 = ((cy - radius - 1.0).floor() as i32).max(clip.y0);
    let y1 = ((cy + radius + 1.0).ceil() as i32).min(clip.y1);
    for py in y0..y1 {
        let dy = (py as f32 + 0.5) - cy;
        for px in x0..x1 {
            let dx = (px as f32 + 0.5) - cx;
            let dist = dx.hypot(dy) - radius;
            let cov = (0.5 - dist).clamp(0.0, 1.0);
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn stroke_line(
    buf: &mut PixelBuffer,
    clip: ClipRectPx,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    width: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if !x0.is_finite()
        || !y0.is_finite()
        || !x1.is_finite()
        || !y1.is_finite()
        || !width.is_finite()
        || width <= 0.0
        || color.a == 0
        || !opacity.is_finite()
        || opacity <= 0.0
    {
        return;
    }
    let half = (width * 0.5).max(0.5);
    let min_x = ((x0.min(x1) - half - 1.0).floor() as i32).max(clip.x0);
    let max_x = ((x0.max(x1) + half + 1.0).ceil() as i32).min(clip.x1);
    let min_y = ((y0.min(y1) - half - 1.0).floor() as i32).max(clip.y0);
    let max_y = ((y0.max(y1) + half + 1.0).ceil() as i32).min(clip.y1);
    let vx = x1 - x0;
    let vy = y1 - y0;
    let len2 = vx * vx + vy * vy;

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        for px in min_x..max_x {
            let fx = px as f32 + 0.5;
            let t = if len2 > 1e-6 {
                (((fx - x0) * vx + (fy - y0) * vy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let dist = (fx - (x0 + t * vx)).hypot(fy - (y0 + t * vy)) - half;
            let cov = (0.5 - dist).clamp(0.0, 1.0);
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::EventBatchBuf;
    use nve_canvas::{CanvasOp, DrawListRecorder};
    use nve_engine::LayoutRect;
    use nve_store::{ChangeSet, FlatResource, Rgba};

    #[test]
    fn canvas_skia_system_draws_shapes_and_clips() {
        let mut rec = DrawListRecorder::new();
        rec.clear(ColorRgba::BLACK)
            .clip_rect(4.0, 4.0, 12.0, 12.0)
            .fill_rect(0.0, 0.0, 32.0, 32.0, ColorRgba::rgb(0, 255, 0))
            .restore_clip()
            .fill_circle(24.0, 24.0, 4.0, ColorRgba::rgb(0, 0, 255));
        let dl = rec.finish();

        // Raw non-finite / negative-radius geometry is rejected at FlatResource ingress.
        let mut bad_words = Vec::new();
        let bad_dl = DrawList::from_ops(vec![
            CanvasOp::FillRoundedRect {
                x: f32::NAN,
                y: 0.0,
                w: 10.0,
                h: 10.0,
                radius: f32::NAN,
                color: ColorRgba::WHITE,
            },
            CanvasOp::StrokeLine {
                x0: 0.0,
                y0: 0.0,
                x1: f32::INFINITY,
                y1: 10.0,
                width: 2.0,
                color: ColorRgba::WHITE,
            },
        ]);
        assert!(!bad_dl.write_words(&mut bad_words));
        assert!(bad_words.is_empty());

        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Canvas>(n).draw_list_ephemeral(dl).generation(1);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let mut buf = PixelBuffer::new(32, 32);
        let mut events = EventBatchBuf::new();
        let mut ctx = SkiaSystemPaintCtx {
            buffer: &mut buf,
            rect: LayoutRect {
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
            },
            clip_rect: None,
            scale_factor: 1.0,
            opacity: 1.0,
            inherited_text_color: Rgba::WHITE,
            events: &mut events,
        };
        CanvasSkiaSystem.paint_node(&mut ctx, &store, n);
        assert_eq!(buf.get_rgb(8, 8), Some((0, 255, 0)));
        assert_eq!(buf.get_rgb(1, 1), Some((0, 0, 0))); // clipped out
        assert_eq!(buf.get_rgb(24, 24), Some((0, 0, 255)));

        // Transparent Clear overwrites prior pixels instead of silently no-oping.
        let mut rec2 = DrawListRecorder::new();
        rec2.clear(ColorRgba::TRANSPARENT);
        let mut tx = store.tx();
        tx.edit::<Canvas>(n)
            .draw_list_ephemeral(rec2.finish())
            .generation(2);
        tx.commit_into(&mut changes);
        let mut ctx2 = SkiaSystemPaintCtx {
            buffer: &mut buf,
            rect: LayoutRect {
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
            },
            clip_rect: None,
            scale_factor: 1.0,
            opacity: 1.0,
            inherited_text_color: Rgba::WHITE,
            events: &mut events,
        };
        CanvasSkiaSystem.paint_node(&mut ctx2, &store, n);
        assert_eq!(buf.get_rgb(8, 8), Some((0, 0, 0)));
    }

    #[test]
    fn canvas_skia_system_enforces_max_clip_depth_66_clips_2_restores() {
        let mut rec = DrawListRecorder::new();
        rec.clear(ColorRgba::BLACK);
        for _ in 0..(MAX_CLIP_DEPTH - 1) {
            rec.clip_rect(0.0, 0.0, 32.0, 32.0);
        }
        // 64th clip is active; 65th and 66th overflow and must not narrow the clip.
        rec.clip_rect(4.0, 4.0, 8.0, 8.0);
        rec.clip_rect(0.0, 0.0, 1.0, 1.0);
        rec.clip_rect(0.0, 0.0, 1.0, 1.0);
        // Two restores cancel the two overflowed clips; 64th clip stays active.
        rec.restore_clip().restore_clip();
        rec.fill_rect(0.0, 0.0, 32.0, 32.0, ColorRgba::rgb(0, 255, 0));
        // Third restore pops the 64th clip back to full 32x32.
        rec.restore_clip();
        rec.fill_rect(16.0, 16.0, 8.0, 8.0, ColorRgba::rgb(0, 0, 255));

        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Canvas>(n)
            .draw_list_ephemeral(rec.finish())
            .generation(1);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let mut buf = PixelBuffer::new(32, 32);
        let mut events = EventBatchBuf::new();
        let mut ctx = SkiaSystemPaintCtx {
            buffer: &mut buf,
            rect: LayoutRect {
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
            },
            clip_rect: None,
            scale_factor: 1.0,
            opacity: 1.0,
            inherited_text_color: Rgba::WHITE,
            events: &mut events,
        };
        CanvasSkiaSystem.paint_node(&mut ctx, &store, n);
        assert_eq!(buf.get_rgb(6, 6), Some((0, 255, 0)));
        assert_eq!(buf.get_rgb(1, 1), Some((0, 0, 0)));
        assert_eq!(buf.get_rgb(20, 20), Some((0, 0, 255)));
    }
}
