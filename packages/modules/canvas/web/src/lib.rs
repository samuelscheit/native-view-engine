//! Web DOM & SSR adapter for `nve-canvas` (`WebTableSystem`).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use core::fmt::Write as _;

use nve_backend_web::{WebSystemCommitCtx, WebTableSystem};
use nve_canvas::{Canvas, DrawList};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};

/// `WebTableSystem` plugin for [`Canvas`] (`nve.canvas`).
#[derive(Debug, Default, Clone, Copy)]
pub struct CanvasWebSystem;

impl CanvasWebSystem {
    #[cfg(target_arch = "wasm32")]
    fn repaint_live(&self, store: &NodeStore, node: NodeId, el: &web_sys::Element) {
        if let Some(row) = store.try_row::<Canvas>(node) {
            if let Some(dl) = store.resource::<DrawList>(row.draw_list()) {
                paint_canvas_dom(el, store, node, dl);
            } else {
                clear_canvas_dom(el);
            }
        }
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

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
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
        let dl = store.resource::<DrawList>(row.draw_list());
        let count = dl.map_or(0, |d| d.len());
        if count > 0 {
            ctx.set_attr(node, "data-nve-draw-ops", &count.to_string());
        } else {
            ctx.remove_attr(node, "data-nve-draw-ops");
        }

        #[cfg(target_arch = "wasm32")]
        if let Some(el) = ctx.live_element {
            self.repaint_live(store, node, el);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn sync_retagged_element(&self, store: &NodeStore, node: NodeId, el: &web_sys::Element) {
        self.repaint_live(store, node, el);
    }
}

#[cfg(target_arch = "wasm32")]
fn clear_canvas_dom(el: &web_sys::Element) {
    use wasm_bindgen::JsCast;
    let Some(canvas) = el.dyn_ref::<web_sys::HtmlCanvasElement>() else {
        return;
    };
    let Ok(Some(ctx_obj)) = canvas.get_context("2d") else {
        return;
    };
    let Ok(ctx2d) = ctx_obj.dyn_into::<web_sys::CanvasRenderingContext2d>() else {
        return;
    };
    let _ = ctx2d.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    ctx2d.clear_rect(0.0, 0.0, f64::from(canvas.width()), f64::from(canvas.height()));
}

#[cfg(target_arch = "wasm32")]
fn format_css_color(c: nve_abi::ColorRgba, buf: &mut String) -> &str {
    use core::fmt::Write as _;
    buf.clear();
    if c.a == 255 {
        let _ = write!(buf, "rgb({},{},{})", c.r, c.g, c.b);
    } else {
        let _ = write!(
            buf,
            "rgba({},{},{},{:.4})",
            c.r,
            c.g,
            c.b,
            f32::from(c.a) / 255.0
        );
    }
    buf.as_str()
}

#[cfg(any(target_arch = "wasm32", test))]
trait WebCanvasSink {
    fn clear(&mut self, color: nve_abi::ColorRgba);
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: nve_abi::ColorRgba);
    fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: nve_abi::ColorRgba,
    );
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: nve_abi::ColorRgba);
    fn stroke_line(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width: f32,
        color: nve_abi::ColorRgba,
    );
    fn stroke_polyline(
        &mut self,
        points: nve_canvas::PolylinePoints<'_>,
        width: f32,
        color: nve_abi::ColorRgba,
    );
    fn save_and_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32);
    fn restore_clip(&mut self);
}

#[cfg(any(target_arch = "wasm32", test))]
struct BalancedWebSink<'a, S: WebCanvasSink> {
    sink: &'a mut S,
    clips: nve_canvas::ClipDepthTracker,
}

#[cfg(any(target_arch = "wasm32", test))]
impl<S: WebCanvasSink> Drop for BalancedWebSink<'_, S> {
    fn drop(&mut self) {
        for _ in 0..self.clips.drain_open() {
            self.sink.restore_clip();
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn execute_draw_list<S: WebCanvasSink>(dl: nve_canvas::DrawListView<'_>, sink: &mut S) {
    use nve_canvas::CanvasOpRef;

    let mut balanced = BalancedWebSink {
        sink,
        clips: nve_canvas::ClipDepthTracker::new(),
    };
    for op in dl.ops() {
        match op {
            CanvasOpRef::Clear { color } => balanced.sink.clear(color),
            CanvasOpRef::FillRect { x, y, w, h, color } => {
                balanced.sink.fill_rect(x, y, w, h, color);
            }
            CanvasOpRef::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius,
                color,
            } => balanced
                .sink
                .fill_rounded_rect(x, y, w, h, radius, color),
            CanvasOpRef::FillCircle {
                cx,
                cy,
                radius,
                color,
            } => balanced.sink.fill_circle(cx, cy, radius, color),
            CanvasOpRef::StrokeLine {
                x0,
                y0,
                x1,
                y1,
                width,
                color,
            } => balanced.sink.stroke_line(x0, y0, x1, y1, width, color),
            CanvasOpRef::StrokePolyline {
                points,
                width,
                color,
            } => balanced.sink.stroke_polyline(points, width, color),
            CanvasOpRef::ClipRect { x, y, w, h } => {
                if balanced.clips.push() {
                    balanced.sink.save_and_clip_rect(x, y, w, h);
                }
            }
            CanvasOpRef::RestoreClip => {
                if balanced.clips.pop() {
                    balanced.sink.restore_clip();
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
struct DomCanvas2dSink<'a> {
    ctx2d: &'a web_sys::CanvasRenderingContext2d,
    logical_w: f64,
    logical_h: f64,
    color_buf: String,
}

#[cfg(target_arch = "wasm32")]
impl WebCanvasSink for DomCanvas2dSink<'_> {
    fn clear(&mut self, color: nve_abi::ColorRgba) {
        self.ctx2d
            .clear_rect(0.0, 0.0, self.logical_w, self.logical_h);
        if color.a > 0 {
            self.ctx2d
                .set_fill_style_str(format_css_color(color, &mut self.color_buf));
            self.ctx2d
                .fill_rect(0.0, 0.0, self.logical_w, self.logical_h);
        }
    }

    fn fill_rect(&mut self, x: f32, y: f32, rw: f32, rh: f32, color: nve_abi::ColorRgba) {
        if !x.is_finite()
            || !y.is_finite()
            || !rw.is_finite()
            || !rh.is_finite()
            || rw <= 0.0
            || rh <= 0.0
        {
            return;
        }
        self.ctx2d
            .set_fill_style_str(format_css_color(color, &mut self.color_buf));
        self.ctx2d
            .fill_rect(f64::from(x), f64::from(y), f64::from(rw), f64::from(rh));
    }

    fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        rw: f32,
        rh: f32,
        radius: f32,
        color: nve_abi::ColorRgba,
    ) {
        if !x.is_finite()
            || !y.is_finite()
            || !rw.is_finite()
            || !rh.is_finite()
            || rw <= 0.0
            || rh <= 0.0
        {
            return;
        }
        let x = f64::from(x);
        let y = f64::from(y);
        let rw = f64::from(rw);
        let rh = f64::from(rh);
        let raw_r = if radius.is_finite() {
            f64::from(radius)
        } else {
            0.0
        };
        let r = raw_r.clamp(0.0, (rw * 0.5).min(rh * 0.5));
        self.ctx2d.begin_path();
        self.ctx2d.move_to(x + r, y);
        self.ctx2d.line_to(x + rw - r, y);
        self.ctx2d.quadratic_curve_to(x + rw, y, x + rw, y + r);
        self.ctx2d.line_to(x + rw, y + rh - r);
        self.ctx2d
            .quadratic_curve_to(x + rw, y + rh, x + rw - r, y + rh);
        self.ctx2d.line_to(x + r, y + rh);
        self.ctx2d.quadratic_curve_to(x, y + rh, x, y + rh - r);
        self.ctx2d.line_to(x, y + r);
        self.ctx2d.quadratic_curve_to(x, y, x + r, y);
        self.ctx2d.close_path();
        self.ctx2d
            .set_fill_style_str(format_css_color(color, &mut self.color_buf));
        self.ctx2d.fill();
    }

    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: nve_abi::ColorRgba) {
        if !cx.is_finite() || !cy.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return;
        }
        self.ctx2d.begin_path();
        let _ = self.ctx2d.arc(
            f64::from(cx),
            f64::from(cy),
            f64::from(radius),
            0.0,
            core::f64::consts::TAU,
        );
        self.ctx2d
            .set_fill_style_str(format_css_color(color, &mut self.color_buf));
        self.ctx2d.fill();
    }

    fn stroke_line(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width: f32,
        color: nve_abi::ColorRgba,
    ) {
        if !x0.is_finite()
            || !y0.is_finite()
            || !x1.is_finite()
            || !y1.is_finite()
            || !width.is_finite()
            || width <= 0.0
        {
            return;
        }
        self.ctx2d.begin_path();
        self.ctx2d.move_to(f64::from(x0), f64::from(y0));
        self.ctx2d.line_to(f64::from(x1), f64::from(y1));
        self.ctx2d
            .set_stroke_style_str(format_css_color(color, &mut self.color_buf));
        self.ctx2d.set_line_width(f64::from(width));
        self.ctx2d.stroke();
    }

    fn stroke_polyline(
        &mut self,
        points: nve_canvas::PolylinePoints<'_>,
        width: f32,
        color: nve_abi::ColorRgba,
    ) {
        if !width.is_finite() || width <= 0.0 {
            return;
        }
        let mut iter = points
            .iter()
            .filter(|(px, py)| px.is_finite() && py.is_finite());
        if let Some((x0, y0)) = iter.next() {
            self.ctx2d.begin_path();
            self.ctx2d.move_to(f64::from(x0), f64::from(y0));
            for (px, py) in iter {
                self.ctx2d.line_to(f64::from(px), f64::from(py));
            }
            self.ctx2d
                .set_stroke_style_str(format_css_color(color, &mut self.color_buf));
            self.ctx2d.set_line_width(f64::from(width));
            self.ctx2d.stroke();
        }
    }

    fn save_and_clip_rect(&mut self, x: f32, y: f32, rw: f32, rh: f32) {
        self.ctx2d.save();
        self.ctx2d.begin_path();
        if x.is_finite() && y.is_finite() && rw.is_finite() && rh.is_finite() {
            self.ctx2d.rect(
                f64::from(x),
                f64::from(y),
                f64::from(rw.max(0.0)),
                f64::from(rh.max(0.0)),
            );
        } else {
            self.ctx2d.rect(0.0, 0.0, 0.0, 0.0);
        }
        self.ctx2d.clip();
    }

    fn restore_clip(&mut self) {
        self.ctx2d.restore();
    }
}

#[cfg(target_arch = "wasm32")]
fn paint_canvas_dom(
    el: &web_sys::Element,
    store: &NodeStore,
    node: NodeId,
    dl: nve_canvas::DrawListView<'_>,
) {
    use nve_tables::tables::Layout;
    use wasm_bindgen::JsCast;

    let Some(canvas) = el.dyn_ref::<web_sys::HtmlCanvasElement>() else {
        return;
    };
    let dpr = web_sys::window()
        .map(|win| win.device_pixel_ratio())
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(1.0)
        .clamp(1.0, 4.0);

    let (mut logical_w, mut logical_h) = (0.0f64, 0.0f64);
    if let Some(l) = store.try_row::<Layout>(node) {
        if l.width().is_points() && l.width().value().is_finite() && l.width().value() > 0.0 {
            logical_w = f64::from(l.width().value());
        }
        if l.height().is_points() && l.height().value().is_finite() && l.height().value() > 0.0 {
            logical_h = f64::from(l.height().value());
        }
    }
    if logical_w <= 0.0 || logical_h <= 0.0 {
        let rect = el.get_bounding_client_rect();
        if logical_w <= 0.0 && rect.width().is_finite() && rect.width() > 0.0 {
            logical_w = rect.width();
        }
        if logical_h <= 0.0 && rect.height().is_finite() && rect.height() > 0.0 {
            logical_h = rect.height();
        }
    }
    if logical_w <= 0.0 {
        logical_w = f64::from(canvas.width().max(300)) / dpr;
    }
    if logical_h <= 0.0 {
        logical_h = f64::from(canvas.height().max(150)) / dpr;
    }

    let backing_w = ((logical_w * dpr).round() as u32).clamp(1, 8192);
    let backing_h = ((logical_h * dpr).round() as u32).clamp(1, 8192);
    if canvas.width() != backing_w {
        canvas.set_width(backing_w);
    }
    if canvas.height() != backing_h {
        canvas.set_height(backing_h);
    }

    let Ok(Some(ctx_obj)) = canvas.get_context("2d") else {
        return;
    };
    let Ok(ctx2d) = ctx_obj.dyn_into::<web_sys::CanvasRenderingContext2d>() else {
        return;
    };

    let _ = ctx2d.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
    ctx2d.clear_rect(0.0, 0.0, logical_w, logical_h);

    let mut sink = DomCanvas2dSink {
        ctx2d: &ctx2d,
        logical_w,
        logical_h,
        color_buf: String::with_capacity(32),
    };
    execute_draw_list(dl, &mut sink);
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::ColorRgba;
    use nve_canvas::{DrawListRecorder, MAX_CLIP_DEPTH};
    use nve_store::{FlatResource, ValidatedWords};

    #[derive(Default)]
    struct RecordingSink {
        log: Vec<&'static str>,
        saves: usize,
        restores: usize,
        live_depth: usize,
        max_live_depth: usize,
    }

    impl WebCanvasSink for RecordingSink {
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
        fn stroke_polyline(
            &mut self,
            _points: nve_canvas::PolylinePoints<'_>,
            _width: f32,
            _color: ColorRgba,
        ) {
            self.log.push("stroke_polyline");
        }
        fn save_and_clip_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {
            self.saves += 1;
            self.live_depth += 1;
            self.max_live_depth = self.max_live_depth.max(self.live_depth);
            self.log.push("clip");
        }
        fn restore_clip(&mut self) {
            assert!(self.live_depth > 0, "underflow restore_clip");
            self.live_depth -= 1;
            self.restores += 1;
            self.log.push("restore");
        }
    }

    #[test]
    fn web_draw_list_enforces_max_clip_depth_and_overflow_balance_66_clips_2_restores() {
        let mut rec = DrawListRecorder::new();
        for i in 0..66 {
            rec.clip_rect(i as f32, 0.0, 100.0, 100.0);
        }
        rec.restore_clip();
        rec.restore_clip();
        rec.fill_rect(0.0, 0.0, 10.0, 10.0, ColorRgba::WHITE);

        let mut words = Vec::new();
        assert!(rec.finish().write_words(&mut words));
        let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid draw list");

        let mut sink = RecordingSink::default();
        execute_draw_list(DrawList::view(vw), &mut sink);

        assert_eq!(sink.saves, MAX_CLIP_DEPTH);
        assert_eq!(sink.max_live_depth, MAX_CLIP_DEPTH);
        assert_eq!(sink.restores, MAX_CLIP_DEPTH);
        assert_eq!(sink.live_depth, 0);
        // The 2 explicit restores only absorbed the 2 overflowed clips, so `fill_rect` executes
        // BEFORE any `restore` callback and all 64 unwind restores happen after `fill_rect`.
        let fill_idx = sink
            .log
            .iter()
            .position(|&ev| ev == "fill_rect")
            .expect("fill_rect logged");
        assert_eq!(fill_idx, MAX_CLIP_DEPTH);
        assert!(sink.log[..fill_idx].iter().all(|&ev| ev == "clip"));
        assert!(sink.log[fill_idx + 1..].iter().all(|&ev| ev == "restore"));
        assert_eq!(sink.log.len(), MAX_CLIP_DEPTH + 1 + MAX_CLIP_DEPTH);
    }

    #[test]
    fn web_draw_list_ignores_unmatched_restore_and_pops_live_clip_after_overflow() {
        let mut rec = DrawListRecorder::new();
        rec.restore_clip();
        for _ in 0..66 {
            rec.clip_rect(0.0, 0.0, 50.0, 50.0);
        }
        // 2 restores cancel overflow, 3rd pops a live clip before fill_rect.
        rec.restore_clip().restore_clip().restore_clip();
        rec.fill_rect(1.0, 1.0, 4.0, 4.0, ColorRgba::BLACK);

        let mut words = Vec::new();
        assert!(rec.finish().write_words(&mut words));
        let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid draw list");

        let mut sink = RecordingSink::default();
        execute_draw_list(DrawList::view(vw), &mut sink);

        assert_eq!(sink.saves, MAX_CLIP_DEPTH);
        assert_eq!(sink.restores, MAX_CLIP_DEPTH);
        assert_eq!(sink.live_depth, 0);
        let fill_idx = sink
            .log
            .iter()
            .position(|&ev| ev == "fill_rect")
            .expect("fill_rect logged");
        assert_eq!(fill_idx, MAX_CLIP_DEPTH + 1);
        assert_eq!(sink.log[MAX_CLIP_DEPTH], "restore");
        assert_eq!(sink.log.len() - fill_idx - 1, MAX_CLIP_DEPTH - 1);
    }

    #[test]
    fn balanced_web_sink_unwinds_exact_64_restores_on_panic_while_overflowed() {
        struct PanickingSink {
            inner: RecordingSink,
        }
        impl WebCanvasSink for PanickingSink {
            fn clear(&mut self, color: ColorRgba) { self.inner.clear(color); }
            fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
                self.inner.fill_rect(x, y, w, h, color);
                panic!("simulated web canvas panic");
            }
            fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: ColorRgba) {
                self.inner.fill_rounded_rect(x, y, w, h, r, c);
            }
            fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, c: ColorRgba) {
                self.inner.fill_circle(cx, cy, r, c);
            }
            fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, c: ColorRgba) {
                self.inner.stroke_line(x0, y0, x1, y1, w, c);
            }
            fn stroke_polyline(&mut self, pts: nve_canvas::PolylinePoints<'_>, w: f32, c: ColorRgba) {
                self.inner.stroke_polyline(pts, w, c);
            }
            fn save_and_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
                self.inner.save_and_clip_rect(x, y, w, h);
            }
            fn restore_clip(&mut self) { self.inner.restore_clip(); }
        }

        let mut rec = DrawListRecorder::new();
        for _ in 0..(MAX_CLIP_DEPTH + 2) {
            rec.clip_rect(0.0, 0.0, 50.0, 50.0);
        }
        rec.fill_rect(1.0, 1.0, 4.0, 4.0, ColorRgba::WHITE);
        let mut words = Vec::new();
        assert!(rec.finish().write_words(&mut words));
        let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid draw list");

        let mut sink = PanickingSink { inner: RecordingSink::default() };
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            execute_draw_list(DrawList::view(vw), &mut sink);
        }));
        assert!(res.is_err(), "panic must propagate");
        assert_eq!(sink.inner.saves, MAX_CLIP_DEPTH);
        assert_eq!(sink.inner.restores, MAX_CLIP_DEPTH);
        assert_eq!(sink.inner.live_depth, 0);
        assert_eq!(sink.inner.log[MAX_CLIP_DEPTH], "fill_rect");
        assert_eq!(sink.inner.log.len(), MAX_CLIP_DEPTH + 1 + MAX_CLIP_DEPTH);
    }
}
