//! Web DOM & SSR adapter for `nve-canvas` (`WebTableSystem`).
//!
//! Unifies direct in-process [`DirectCanvasRegistry`] painting and recorded [`DrawList`] replay
//! through [`WebCanvas2dPainter`] (`GuardedPainter<WebCanvas2dSink>`), repainting in Step 6
//! (`sync_connected`) once elements are document-connected with true layout dimensions.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod painter;

#[cfg(target_arch = "wasm32")]
pub use painter::{WebCanvas2dPainter, WebCanvas2dSink};

use core::fmt::Write as _;

use nve_backend_web::{WebSystemCommitCtx, WebTableSystem};
use nve_canvas::{Canvas, DirectCanvasRegistry, DrawList};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};
use nve_tables::tables::Layout;

/// Stack-formats a decimal `usize` without heap allocation.
#[inline]
fn format_usize_ascii(mut v: usize, buf: &mut [u8; 20]) -> &str {
    if v == 0 {
        buf[19] = b'0';
        return core::str::from_utf8(&buf[19..]).unwrap_or("0");
    }
    let mut pos = buf.len();
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    core::str::from_utf8(&buf[pos..]).unwrap_or("0")
}

/// `WebTableSystem` plugin for [`Canvas`] (`nve.canvas`).
#[derive(Debug, Default, Clone)]
pub struct CanvasWebSystem {
    direct: Option<DirectCanvasRegistry>,
}

impl CanvasWebSystem {
    /// Creates a recorded-only Web canvas plugin (for SSR / wire hosts).
    #[inline]
    pub fn new() -> Self {
        Self { direct: None }
    }

    /// Creates a Web canvas plugin backed by an in-process [`DirectCanvasRegistry`], falling back
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

    #[inline]
    fn has_direct(&self, node: NodeId) -> bool {
        self.direct.as_ref().is_some_and(|reg| reg.contains(node))
    }
}

impl WebTableSystem for CanvasWebSystem {
    fn slot(&self) -> TableSlot {
        Canvas::slot()
    }

    fn priority(&self) -> u8 {
        115
    }

    fn tag_name<'a>(&self, _store: &'a NodeStore, _node: NodeId) -> Option<&'a str> {
        Some("canvas")
    }

    fn watches_slot(&self, slot: TableSlot) -> bool {
        slot == Layout::slot()
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        if self.has_direct(node) {
            out.push_str(" data-nve-canvas=\"direct\"");
            return;
        }
        if let Some(row) = store.try_row::<Canvas>(node) {
            let count = store
                .resource::<DrawList>(row.draw_list())
                .map_or(0, |dl| dl.len());
            if count > 0 {
                let _ = write!(out, " data-nve-draw-ops=\"{count}\"");
            }
        }
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        let Some(row) = store.try_row::<Canvas>(node) else {
            return;
        };
        if self.has_direct(node) {
            ctx.set_attr(node, "data-nve-canvas", "direct");
            ctx.remove_attr(node, "data-nve-draw-ops");
        } else {
            ctx.remove_attr(node, "data-nve-canvas");
            let count = store
                .resource::<DrawList>(row.draw_list())
                .map_or(0, |d| d.len());
            if count > 0 {
                let mut count_buf = [0u8; 20];
                ctx.set_attr(
                    node,
                    "data-nve-draw-ops",
                    format_usize_ascii(count, &mut count_buf),
                );
            } else {
                ctx.remove_attr(node, "data-nve-draw-ops");
            }
        }

        // Queue Step-6 connected repaint so responsive `<canvas>` elements measure true connected
        // layout dimensions (and coalesce commit + retag + Layout resize into one paint).
        // Guarded inside `WebSystemCommitCtx::request_connected_sync` when unmounted.
        ctx.request_connected_sync(node);
    }

    fn remove_node(&self, _ctx: &mut WebSystemCommitCtx<'_>, node: NodeId) {
        if let Some(reg) = &self.direct {
            reg.remove(node);
        }
    }

    fn reset(&self) {
        if let Some(reg) = &self.direct {
            reg.clear();
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn sync_connected(&self, store: &NodeStore, node: NodeId, el: &web_sys::Element) {
        painter::paint_canvas_dom(el, store, node, self.direct.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::painter::CssColorBuf;
    use nve_abi::ColorRgba;
    use nve_backend_web::WebBackend;
    use nve_canvas::{
        CanvasOp, CanvasSink, GuardedPainter, PolylinePoints, MAX_CLIP_DEPTH,
    };
    use nve_engine::StoreBackend;
    use nve_store::{ChangeSet, FlatResource, Length, ValidatedWords};

    #[derive(Default)]
    struct RecordingSink {
        log: Vec<&'static str>,
        saves: usize,
        restores: usize,
        live_depth: usize,
        max_live_depth: usize,
        one_point_polylines: usize,
    }

    impl CanvasSink for RecordingSink {
        fn clear(&mut self, _color: ColorRgba) {
            self.log.push("clear");
        }
        fn fill_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _color: ColorRgba) {
            self.log.push("fill_rect");
        }
        fn fill_rounded_rect(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _radius: f32,
            _color: ColorRgba,
        ) {
            self.log.push("fill_rounded_rect");
        }
        fn fill_circle(&mut self, _cx: f32, _cy: f32, _radius: f32, _color: ColorRgba) {
            self.log.push("fill_circle");
        }
        fn stroke_line(
            &mut self,
            _x0: f32,
            _y0: f32,
            _x1: f32,
            _y1: f32,
            _width: f32,
            _color: ColorRgba,
        ) {
            self.log.push("stroke_line");
        }
        fn stroke_polyline(&mut self, points: PolylinePoints<'_>, _width: f32, _color: ColorRgba) {
            if points.len() == 1 {
                self.one_point_polylines += 1;
            }
            self.log.push("stroke_polyline");
        }
        fn push_clip_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {
            self.saves += 1;
            self.live_depth += 1;
            self.max_live_depth = self.max_live_depth.max(self.live_depth);
            self.log.push("clip");
        }
        fn pop_clip(&mut self) {
            assert!(self.live_depth > 0, "underflow pop_clip");
            self.live_depth -= 1;
            self.restores += 1;
            self.log.push("restore");
        }
    }

    #[test]
    fn stack_css_color_formatter_matches_css_grammar() {
        let mut buf = CssColorBuf::new();
        assert_eq!(buf.format(ColorRgba::rgb(0, 128, 255)), "rgb(0,128,255)");
        assert_eq!(
            buf.format(ColorRgba::rgba(10, 20, 30, 0)),
            "rgba(10,20,30,0.0000)"
        );
        assert_eq!(
            buf.format(ColorRgba::rgba(10, 20, 30, 128)),
            "rgba(10,20,30,0.5020)"
        );
        assert_eq!(
            buf.format(ColorRgba::rgba(255, 255, 255, 254)),
            "rgba(255,255,255,0.9961)"
        );
    }

    #[test]
    fn hostile_wire_draw_list_enforces_max_clip_depth_66_clips_2_restores_and_unwind() {
        let mut ops = Vec::new();
        ops.push(CanvasOp::RestoreClip); // unmatched leading restore
        for i in 0..66 {
            ops.push(CanvasOp::ClipRect {
                x: i as f32,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            });
        }
        ops.push(CanvasOp::RestoreClip);
        ops.push(CanvasOp::RestoreClip);
        ops.push(CanvasOp::FillRect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            color: ColorRgba::WHITE,
        });
        ops.push(CanvasOp::StrokePolyline {
            points: vec![(4.0, 5.0)],
            width: 2.0,
            color: ColorRgba::BLACK,
        });

        let mut words = Vec::new();
        assert!(DrawList::from_ops(ops).write_words(&mut words));
        let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid raw draw list");

        let mut sink = RecordingSink::default();
        {
            let mut painter = GuardedPainter::new(&mut sink);
            DrawList::view(vw).replay(&mut painter);
        }

        assert_eq!(sink.saves, MAX_CLIP_DEPTH);
        assert_eq!(sink.max_live_depth, MAX_CLIP_DEPTH);
        assert_eq!(sink.restores, MAX_CLIP_DEPTH);
        assert_eq!(sink.live_depth, 0);
        assert_eq!(sink.one_point_polylines, 1);
        let fill_idx = sink
            .log
            .iter()
            .position(|&ev| ev == "fill_rect")
            .expect("fill_rect logged");
        assert_eq!(fill_idx, MAX_CLIP_DEPTH);
        assert!(sink.log[..fill_idx].iter().all(|&ev| ev == "clip"));
        assert!(sink.log[fill_idx + 2..].iter().all(|&ev| ev == "restore"));
    }

    #[test]
    #[allow(clippy::panic)]
    fn guarded_painter_unwinds_exact_64_restores_on_panic_while_overflowed() {
        struct PanickingSink<'a>(&'a mut RecordingSink);
        impl CanvasSink for PanickingSink<'_> {
            fn clear(&mut self, color: ColorRgba) {
                self.0.clear(color);
            }
            fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
                self.0.fill_rect(x, y, w, h, color);
                panic!("simulated web canvas panic");
            }
            fn fill_rounded_rect(
                &mut self,
                x: f32,
                y: f32,
                w: f32,
                h: f32,
                r: f32,
                c: ColorRgba,
            ) {
                self.0.fill_rounded_rect(x, y, w, h, r, c);
            }
            fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, c: ColorRgba) {
                self.0.fill_circle(cx, cy, r, c);
            }
            fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, c: ColorRgba) {
                self.0.stroke_line(x0, y0, x1, y1, w, c);
            }
            fn stroke_polyline(&mut self, pts: PolylinePoints<'_>, w: f32, c: ColorRgba) {
                self.0.stroke_polyline(pts, w, c);
            }
            fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
                self.0.push_clip_rect(x, y, w, h);
            }
            fn pop_clip(&mut self) {
                self.0.pop_clip();
            }
        }

        let mut ops = Vec::new();
        for _ in 0..(MAX_CLIP_DEPTH + 2) {
            ops.push(CanvasOp::ClipRect {
                x: 0.0,
                y: 0.0,
                w: 50.0,
                h: 50.0,
            });
        }
        ops.push(CanvasOp::FillRect {
            x: 1.0,
            y: 1.0,
            w: 4.0,
            h: 4.0,
            color: ColorRgba::WHITE,
        });
        let mut words = Vec::new();
        assert!(DrawList::from_ops(ops).write_words(&mut words));
        let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid raw draw list");

        let mut inner = RecordingSink::default();
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut painter = GuardedPainter::new(PanickingSink(&mut inner));
            DrawList::view(vw).replay(&mut painter);
        }));
        assert!(res.is_err(), "panic must propagate");
        assert_eq!(inner.saves, MAX_CLIP_DEPTH);
        assert_eq!(inner.restores, MAX_CLIP_DEPTH);
        assert_eq!(inner.live_depth, 0);
        assert_eq!(inner.log[MAX_CLIP_DEPTH], "fill_rect");
        assert_eq!(inner.log.len(), MAX_CLIP_DEPTH + 1 + MAX_CLIP_DEPTH);
    }

    #[test]
    fn web_backend_direct_lifecycle_watches_layout_and_cleans_up_without_leaks() {
        let reg = DirectCanvasRegistry::new();
        let sys = CanvasWebSystem::with_direct(reg.clone());
        assert!(sys.watches_slot(Layout::slot()));

        let mut backend = WebBackend::new();
        backend.register_table_system(sys);

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
                    .width(Length::points(100.0))
                    .height(Length::points(40.0));
                let g = Arc::clone(g);
                reg.bind(&mut tx, n, move |p, size| {
                    g.fetch_add(1, Ordering::Relaxed);
                    p.fill_rect(0.0, 0.0, size.width, size.height, ColorRgba::WHITE);
                });
            }
            tx.commit_into(&mut changes);
            (a, b, c)
        };
        backend.commit(&store, &changes).expect("initial commit");
        assert_eq!(reg.len(), 3);
        assert_eq!(
            backend.renderer().plugin_attrs(na),
            &[("data-nve-canvas".to_string(), "direct".to_string())]
        );

        // Detach Canvas from na.
        {
            let mut tx = store.tx();
            tx.detach::<Canvas>(na);
            tx.commit_into(&mut changes);
        }
        backend.commit(&store, &changes).expect("detach commit");
        assert_eq!(reg.len(), 2);
        assert!(!reg.contains(na));
        assert_eq!(Arc::strong_count(&guard_a), 1);

        // Remove nb.
        {
            let mut tx = store.tx();
            tx.remove(nb);
            tx.commit_into(&mut changes);
        }
        backend.commit(&store, &changes).expect("remove commit");
        assert_eq!(reg.len(), 1);
        assert!(!reg.contains(nb));
        assert_eq!(Arc::strong_count(&guard_b), 1);

        // Reset WebBackend -> clears nc and drops its Arc.
        backend.reset();
        assert_eq!(reg.len(), 0);
        assert!(!reg.contains(nc));
        assert_eq!(Arc::strong_count(&guard_c), 1);
    }

    #[test]
    fn connected_sync_queues_on_canvas_and_layout_edits_and_skips_when_unmounted() {
        use nve_backend_web::table_system::WebStoreRenderer;

        let reg = DirectCanvasRegistry::new();
        let mut renderer =
            WebStoreRenderer::new().with_table_system(CanvasWebSystem::with_direct(reg.clone()));
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();

        // 1. Unmounted/SSR default (`track_connected_sync == false`): zero entries queued.
        let node = {
            let mut tx = store.tx();
            let n = tx.create();
            tx.set_children(NodeId::ROOT, &[n]);
            tx.edit::<Layout>(n)
                .width(Length::points(120.0))
                .height(Length::points(60.0));
            reg.bind(&mut tx, n, |p, size| {
                p.fill_rect(0.0, 0.0, size.width, size.height, ColorRgba::WHITE);
            });
            tx.commit_into(&mut changes);
            n
        };
        renderer.commit(&store, &changes).expect("unmounted commit");
        assert!(
            renderer.drain_connected_sync_requests().is_empty(),
            "unmounted commits must never leak connected_sync entries"
        );

        // 2. Mounted mode (`track_connected_sync == true`): `Canvas` invalidate & watched `Layout`
        //    changes both queue `(Canvas::slot(), node)`, and steady-state `data-nve-canvas` stays stable.
        renderer.set_track_connected_sync(true);
        {
            let mut tx = store.tx();
            assert!(reg.invalidate(&mut tx, node).is_some());
            tx.commit_into(&mut changes);
        }
        renderer.commit(&store, &changes).expect("invalidate commit");
        assert_eq!(
            renderer.drain_connected_sync_requests(),
            vec![(Canvas::slot(), node)]
        );
        assert_eq!(
            renderer.plugin_attrs(node),
            &[("data-nve-canvas".to_string(), "direct".to_string())]
        );

        // Watched `Layout` resize with zero `Canvas` changes still queues connected sync!
        {
            let mut tx = store.tx();
            tx.edit::<Layout>(node).width(Length::points(240.0));
            tx.commit_into(&mut changes);
        }
        renderer.commit(&store, &changes).expect("layout resize commit");
        assert_eq!(
            renderer.drain_connected_sync_requests(),
            vec![(Canvas::slot(), node)]
        );

        // Recorded mode uses zero-alloc stack decimal formatter for `data-nve-draw-ops`.
        reg.remove(node);
        let dl = DrawList::record(|p| {
            p.clear(ColorRgba::BLACK);
            p.fill_rect(0.0, 0.0, 10.0, 10.0, ColorRgba::WHITE);
        });
        {
            let mut tx = store.tx();
            tx.edit::<Canvas>(node)
                .draw_list_ephemeral(dl)
                .generation(99);
            tx.commit_into(&mut changes);
        }
        renderer.commit(&store, &changes).expect("recorded commit");
        assert_eq!(
            renderer.plugin_attrs(node),
            &[("data-nve-draw-ops".to_string(), "2".to_string())]
        );
        assert_eq!(
            renderer.drain_connected_sync_requests(),
            vec![(Canvas::slot(), node)]
        );
    }
}
