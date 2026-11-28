//! Skia / software rasterizer adapter for `nve-canvas` (`SkiaTableSystem`).
//!
//! Unifies direct in-process [`DirectCanvasRegistry`] painting and flat [`DrawList`] replay
//! through one [`SkiaCanvasPainter`] (`GuardedPainter<SkiaPixelSink>`) across both software
//! compositing and `--features skia` GPU/CPU plugin texture caching.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

#[cfg(feature = "skia")]
mod native_skia;
mod painter;
mod raster;

#[cfg(feature = "skia")]
pub use native_skia::{SkiaNativeCanvasPainter, SkiaNativeSink};
pub use painter::{SkiaCanvasPainter, SkiaPixelSink};

#[cfg(feature = "skia")]
use nve_backend_skia::NativeSkiaSurface;
use nve_backend_skia::{SkiaSystemPaintCtx, SkiaTableSystem};
use nve_canvas::{Canvas, DirectCanvasRegistry, DrawList};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};

/// `SkiaTableSystem` plugin painting [`Canvas`] (`nve.canvas`) nodes via direct in-process
/// [`DirectCanvasRegistry`] callbacks or recorded [`DrawList`] streams.
#[derive(Debug, Default, Clone)]
pub struct CanvasSkiaSystem {
    direct: Option<DirectCanvasRegistry>,
}

impl CanvasSkiaSystem {
    /// Creates a recorded-only Skia canvas plugin (for out-of-process / wire hosts).
    #[inline]
    pub fn new() -> Self {
        Self { direct: None }
    }

    /// Creates a Skia canvas plugin backed by an in-process [`DirectCanvasRegistry`], falling back
    /// to recorded [`DrawList`] replay for nodes without a bound direct closure.
    #[inline]
    pub fn with_direct(registry: DirectCanvasRegistry) -> Self {
        Self {
            direct: Some(registry),
        }
    }

    /// Returns the attached [`DirectCanvasRegistry`], if configured.
    #[inline]
    pub fn direct(&self) -> Option<&DirectCanvasRegistry> {
        self.direct.as_ref()
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
        let Some((mut painter, size)) = SkiaCanvasPainter::try_from_ctx(ctx) else {
            return;
        };
        if let Some(reg) = &self.direct
            && reg.paint(node, &mut painter, size)
        {
            return;
        }
        if let Some(dl) = store.resource::<DrawList>(row.draw_list()) {
            dl.replay(&mut painter);
        }
    }

    fn remove_node(&self, node: NodeId) {
        if let Some(reg) = &self.direct {
            reg.remove(node);
        }
    }

    fn reset(&self) {
        if let Some(reg) = &self.direct {
            reg.clear();
        }
    }

    #[cfg(feature = "skia")]
    fn prefers_skia_canvas(&self, _store: &NodeStore, node: NodeId) -> bool {
        self.direct.as_ref().is_some_and(|reg| reg.contains(node))
    }

    #[cfg(feature = "skia")]
    fn paint_skia_canvas(
        &self,
        _store: &NodeStore,
        node: NodeId,
        surface: &mut NativeSkiaSurface<'_>,
    ) -> bool {
        let Some(cb) = self.direct.as_ref().and_then(|reg| reg.lookup(node)) else {
            return false;
        };
        let (width, height, scale) = (surface.width(), surface.height(), surface.scale());
        let canvas = surface.begin();
        let (mut painter, size) = SkiaNativeCanvasPainter::new(canvas, width, height, scale);
        cb(&mut painter, size);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use super::*;
    use nve_abi::{ColorRgba, EventBatchBuf, SurfaceId};
    use nve_backend_skia::{PixelBuffer, SkiaBackend};
    use nve_canvas::{
        CanvasPaintSize, CanvasPainter, CanvasOp, DrawListRecorder, MAX_CLIP_DEPTH,
    };
    use nve_engine::{LayoutRect, StoreBackend, Viewport};
    use nve_store::{ChangeSet, FlatResource, Length, Rgba, ValidatedWords};
    use nve_tables::tables::Layout;

    fn paint_all_edge_cases(p: &mut impl CanvasPainter) {
        p.clear(ColorRgba::rgb(12, 16, 24));
        // Valid primitives across all 8 ops.
        p.fill_rect(2.0, 2.0, 20.0, 12.0, ColorRgba::rgb(220, 40, 40));
        p.fill_rounded_rect(6.0, 18.0, 24.0, 16.0, 5.0, ColorRgba::rgb(40, 200, 110));
        p.fill_circle(48.0, 16.0, 8.0, ColorRgba::rgb(50, 120, 240));
        p.stroke_line(4.0, 40.0, 58.0, 52.0, 2.5, ColorRgba::rgb(245, 190, 40));
        // 1-point polyline dot.
        p.stroke_polyline(&[(50.0, 44.0)], 4.0, ColorRgba::rgb(255, 90, 180));
        // Multi-point polyline containing NaN / Inf gaps (must split cleanly without drawing to 0,0).
        p.stroke_polyline(
            &[
                (6.0, 56.0),
                (20.0, 48.0),
                (f32::NAN, 10.0),
                (30.0, 58.0),
                (44.0, 50.0),
                (f32::INFINITY, f32::NEG_INFINITY),
                (56.0, 58.0),
            ],
            2.0,
            ColorRgba::rgb(180, 240, 255),
        );
        // Degenerate / non-finite inputs that must be skipped identically.
        p.fill_rect(f32::NAN, 0.0, 10.0, 10.0, ColorRgba::WHITE);
        p.fill_rect(0.0, 0.0, -5.0, 10.0, ColorRgba::WHITE);
        p.fill_rounded_rect(4.0, 4.0, 10.0, 10.0, -3.0, ColorRgba::rgba(255, 255, 255, 80));
        p.fill_circle(10.0, 10.0, -4.0, ColorRgba::WHITE);
        p.fill_circle(10.0, f32::INFINITY, 4.0, ColorRgba::WHITE);
        p.stroke_line(0.0, 0.0, f32::NAN, 10.0, 2.0, ColorRgba::WHITE);
        p.stroke_line(0.0, 0.0, 10.0, 10.0, -1.0, ColorRgba::WHITE);
        // 66 nested clips + 2 restores -> 64th clip stays active; 3rd restore pops 64th.
        for _ in 0..(MAX_CLIP_DEPTH - 1) {
            p.clip_rect(0.0, 0.0, 64.0, 64.0);
        }
        p.clip_rect(10.0, 10.0, 14.0, 14.0);
        p.clip_rect(0.0, 0.0, 1.0, 1.0);
        p.clip_rect(0.0, 0.0, 1.0, 1.0);
        p.restore_clip();
        p.restore_clip();
        p.fill_rect(0.0, 0.0, 64.0, 64.0, ColorRgba::rgba(0, 255, 128, 140));
        p.restore_clip();
        p.fill_rect(34.0, 26.0, 10.0, 10.0, ColorRgba::rgb(255, 255, 0));
        // Intentionally leave 63 clips unclosed to exercise Drop / finish unwind parity.
    }

    #[test]
    fn pixel_for_pixel_parity_direct_vs_recorded_across_all_ops_and_edge_cases() {
        let rect = LayoutRect {
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 64.0,
        };

        // 1. Direct SkiaCanvasPainter on an isolated RGBA buffer (matching `draw_plugin` / GPU cache)
        let mut direct_buf = PixelBuffer::new_transparent_rgba(64, 64);
        {
            let (mut painter, size) =
                SkiaCanvasPainter::new(&mut direct_buf, rect, None, 1.0, 1.0)
                    .expect("valid painter");
            assert_eq!(size, CanvasPaintSize::new(64.0, 64.0, 1.0));
            paint_all_edge_cases(&mut painter);
        }

        // 2. Recorded DrawList -> flat wire words -> ValidatedWords -> DrawListView::replay
        let dl = DrawList::record(paint_all_edge_cases);
        let mut words = Vec::new();
        assert!(dl.write_words(&mut words));
        let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid wire DrawList");
        let mut recorded_buf = PixelBuffer::new_transparent_rgba(64, 64);
        {
            let (mut painter, _) =
                SkiaCanvasPainter::new(&mut recorded_buf, rect, None, 1.0, 1.0)
                    .expect("valid painter");
            DrawList::view(vw).replay(&mut painter);
        }

        assert_eq!(
            direct_buf.pixels(),
            recorded_buf.pixels(),
            "direct SkiaCanvasPainter and recorded DrawListView::replay must be byte-identical"
        );
        // Verify transparent clear overwrites prior pixels + alpha to 0 identically in both paths.
        {
            let (mut p, _) =
                SkiaCanvasPainter::new(&mut direct_buf, rect, None, 1.0, 1.0).expect("painter");
            p.clear(ColorRgba::TRANSPARENT);
        }
        let clear_dl = DrawList::record(|rec| {
            rec.clear(ColorRgba::TRANSPARENT);
        });
        let mut clear_words = Vec::new();
        assert!(clear_dl.write_words(&mut clear_words));
        let clear_vw = ValidatedWords::<DrawList>::try_new(&clear_words).expect("valid clear");
        {
            let (mut p, _) =
                SkiaCanvasPainter::new(&mut recorded_buf, rect, None, 1.0, 1.0).expect("painter");
            DrawList::view(clear_vw).replay(&mut p);
        }
        assert_eq!(direct_buf.pixels(), recorded_buf.pixels());
        assert_eq!(direct_buf.get_rgb(12, 12), Some((0, 0, 0)));
        assert_eq!(direct_buf.get_alpha(12, 12), Some(0));
        assert_eq!(recorded_buf.get_alpha(12, 12), Some(0));
    }

    #[test]
    fn software_rasterize_with_store_isolates_clear_transparent_and_translucent_over_backdrop() {
        use nve_tables::tables::Visual;

        let reg = DirectCanvasRegistry::new();
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        {
            let mut tx = store.tx();
            let bg = tx.create();
            let canvas_node = tx.create();
            tx.set_children(NodeId::ROOT, &[bg]);
            tx.set_children(bg, &[canvas_node]);

            tx.edit::<Layout>(bg)
                .width(Length::points(64.0))
                .height(Length::points(64.0));
            tx.edit::<Visual>(bg).background(Rgba::rgb(200, 40, 20));

            tx.edit::<Layout>(canvas_node)
                .width(Length::points(64.0))
                .height(Length::points(64.0));
            reg.bind(&mut tx, canvas_node, |p, _| {
                // Initial transparent clear (like `paint_sparkline`) must preserve backdrop!
                p.clear(ColorRgba::TRANSPARENT);
                // Solid white left half, then post-draw transparent clear over top-left quadrant.
                p.fill_rect(0.0, 0.0, 32.0, 64.0, ColorRgba::WHITE);
                p.clip_rect(0.0, 0.0, 32.0, 32.0);
                p.clear(ColorRgba::TRANSPARENT);
                p.restore_clip();
                // Yellow bottom-right replaced by translucent blue `Src` clear over red backdrop.
                p.clip_rect(32.0, 32.0, 32.0, 32.0);
                p.fill_rect(32.0, 32.0, 32.0, 32.0, ColorRgba::rgb(255, 255, 0));
                p.clear(ColorRgba::rgba(0, 80, 240, 128));
                p.restore_clip();
            });
            tx.commit_into(&mut changes);
        }

        let mut backend = SkiaBackend::new();
        backend.register_table_system(CanvasSkiaSystem::with_direct(reg));
        backend.set_viewport(
            SurfaceId::ROOT,
            Viewport {
                width: 64.0,
                height: 64.0,
                scale_factor: 1.0,
            },
        );
        backend.commit(&store, &changes).expect("commit");

        let mut buf = PixelBuffer::new(64, 64);
        backend.rasterize_with_store(&store, &mut buf, ColorRgba::BLACK);

        // Top-right untouched after initial `clear(TRANSPARENT)` -> red backdrop `(200, 40, 20)`.
        assert_eq!(buf.get_rgb(48, 16), Some((200, 40, 20)));
        // Top-left post-draw `clear(TRANSPARENT)` -> red backdrop `(200, 40, 20)` (never black!).
        assert_eq!(buf.get_rgb(16, 16), Some((200, 40, 20)));
        // Bottom-left white fill -> `(255, 255, 255)`.
        assert_eq!(buf.get_rgb(16, 48), Some((255, 255, 255)));
        // Bottom-right translucent `clear(0, 80, 240, 128)` replaced yellow -> ~`(100, 60, 130)`.
        let (r, g, b) = buf.get_rgb(48, 48).expect("bottom-right pixel");
        assert!(
            (i16::from(r) - 100).abs() <= 2
                && (i16::from(g) - 60).abs() <= 2
                && (i16::from(b) - 130).abs() <= 2,
            "expected translucent clear over red backdrop ~(100,60,130), got ({r},{g},{b})"
        );
    }

    #[test]
    fn hostile_wire_draw_list_from_ops_balances_66_clips_and_unmatched_restores() {
        let mut ops = vec![
            CanvasOp::RestoreClip, // unmatched leading restore
            CanvasOp::Clear {
                color: ColorRgba::BLACK,
            },
        ];
        for _ in 0..(MAX_CLIP_DEPTH - 1) {
            ops.push(CanvasOp::ClipRect {
                x: 0.0,
                y: 0.0,
                w: 32.0,
                h: 32.0,
            });
        }
        // 64th live clip, then 65th & 66th overflow clips.
        ops.push(CanvasOp::ClipRect {
            x: 4.0,
            y: 4.0,
            w: 8.0,
            h: 8.0,
        });
        ops.push(CanvasOp::ClipRect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        });
        ops.push(CanvasOp::ClipRect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        });
        // Two restores absorb overflow; green fill stays inside 64th clip.
        ops.push(CanvasOp::RestoreClip);
        ops.push(CanvasOp::RestoreClip);
        ops.push(CanvasOp::FillRect {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: 32.0,
            color: ColorRgba::rgb(0, 255, 0),
        });
        // Third restore pops 64th clip; blue fill paints outside (4..12).
        ops.push(CanvasOp::RestoreClip);
        ops.push(CanvasOp::FillRect {
            x: 16.0,
            y: 16.0,
            w: 8.0,
            h: 8.0,
            color: ColorRgba::rgb(0, 0, 255),
        });

        let raw_dl = DrawList::from_ops(ops);
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Canvas>(n).draw_list_ephemeral(raw_dl).generation(1);
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
        CanvasSkiaSystem::new().paint_node(&mut ctx, &store, n);
        assert_eq!(buf.get_rgb(6, 6), Some((0, 255, 0)));
        assert_eq!(buf.get_rgb(1, 1), Some((0, 0, 0)));
        assert_eq!(buf.get_rgb(20, 20), Some((0, 0, 255)));
    }

    #[test]
    fn skia_backend_direct_lifecycle_cleans_up_on_detach_remove_recycle_and_reset() {
        let reg = DirectCanvasRegistry::new();
        let mut backend = SkiaBackend::new();
        backend.register_table_system(CanvasSkiaSystem::with_direct(reg.clone()));
        backend.set_viewport(
            SurfaceId::ROOT,
            Viewport {
                width: 64.0,
                height: 64.0,
                scale_factor: 1.0,
            },
        );

        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let guard_a = Arc::new(AtomicU32::new(0));
        let guard_b = Arc::new(AtomicU32::new(0));
        let guard_c = Arc::new(AtomicU32::new(0));

        let (na, nb, nc) = {
            let mut tx = store.tx();
            let a = tx.create();
            let b = tx.create();
            let c = tx.create();
            tx.set_children(NodeId::ROOT, &[a, b, c]);
            for (n, g) in [(a, &guard_a), (b, &guard_b), (c, &guard_c)] {
                tx.edit::<Layout>(n)
                    .width(Length::points(16.0))
                    .height(Length::points(16.0));
                let g = Arc::clone(g);
                reg.bind(&mut tx, n, move |p, size| {
                    g.fetch_add(1, Ordering::Relaxed);
                    p.fill_rect(0.0, 0.0, size.width, size.height, ColorRgba::rgb(0, 200, 100));
                });
            }
            tx.commit_into(&mut changes);
            (a, b, c)
        };
        backend.commit(&store, &changes).expect("initial commit");
        assert_eq!(reg.len(), 3);
        assert_eq!(Arc::strong_count(&guard_a), 2);

        // 1. Detach `Canvas` from `na` -> drops closure immediately.
        {
            let mut tx = store.tx();
            tx.detach::<Canvas>(na);
            tx.commit_into(&mut changes);
        }
        backend.commit(&store, &changes).expect("detach commit");
        assert_eq!(reg.len(), 2);
        assert!(!reg.contains(na));
        assert_eq!(Arc::strong_count(&guard_a), 1);

        // 2. Remove `nb` -> drops closure immediately & recycled NodeId has no stale entry.
        {
            let mut tx = store.tx();
            tx.remove(nb);
            tx.commit_into(&mut changes);
        }
        backend.commit(&store, &changes).expect("remove commit");
        assert_eq!(reg.len(), 1);
        assert!(!reg.contains(nb));
        assert_eq!(Arc::strong_count(&guard_b), 1);

        let recycled = {
            let mut tx = store.tx();
            let n = tx.create();
            tx.set_children(NodeId::ROOT, &[na, nc, n]);
            tx.commit_into(&mut changes);
            n
        };
        assert_eq!(recycled.index(), nb.index());
        assert_ne!(recycled, nb);
        assert!(!reg.contains(recycled));

        // 3. `backend.reset()` -> clears all remaining direct entries (`nc`) and drops its Arc.
        backend.reset();
        assert_eq!(reg.len(), 0);
        assert!(!reg.contains(nc));
        assert_eq!(Arc::strong_count(&guard_c), 1);

        // Recorded DrawList fallback still works when `CanvasSkiaSystem` has an empty registry.
        let mut rec = DrawListRecorder::new();
        rec.clear(ColorRgba::rgb(30, 40, 50));
        let mut store2 = NodeStore::new();
        let mut tx = store2.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Canvas>(n)
            .draw_list_ephemeral(rec.finish())
            .generation(1);
        tx.commit_into(&mut changes);
        let mut buf = PixelBuffer::new(16, 16);
        let mut events = EventBatchBuf::new();
        let mut ctx = SkiaSystemPaintCtx {
            buffer: &mut buf,
            rect: LayoutRect {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 16.0,
            },
            clip_rect: None,
            scale_factor: 1.0,
            opacity: 1.0,
            inherited_text_color: Rgba::WHITE,
            events: &mut events,
        };
        CanvasSkiaSystem::with_direct(reg).paint_node(&mut ctx, &store2, n);
        assert_eq!(buf.get_rgb(4, 4), Some((30, 40, 50)));
    }
}
