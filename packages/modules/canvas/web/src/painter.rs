//! Stack-allocated CSS color formatter, [`WebCanvas2dSink`] (`CanvasSink`), and
//! [`WebCanvas2dPainter`] (`GuardedPainter`) for HTML5 `<canvas>` 2D contexts.

#[cfg(any(target_arch = "wasm32", test))]
use nve_abi::ColorRgba;

/// Zero-heap-allocation stack formatter producing CSS `rgb(r,g,b)` / `rgba(r,g,b,0.xxxx)` strings.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) struct CssColorBuf {
    buf: [u8; 32],
    len: usize,
}

#[cfg(any(target_arch = "wasm32", test))]
impl CssColorBuf {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            buf: [0; 32],
            len: 0,
        }
    }

    #[inline]
    fn push_bytes(&mut self, bytes: &[u8]) {
        let end = self.len + bytes.len();
        if end <= self.buf.len() {
            self.buf[self.len..end].copy_from_slice(bytes);
            self.len = end;
        }
    }

    #[inline]
    fn push_byte(&mut self, b: u8) {
        if self.len < self.buf.len() {
            self.buf[self.len] = b;
            self.len += 1;
        }
    }

    #[inline]
    fn push_u8(&mut self, v: u8) {
        if v >= 100 {
            self.push_byte(b'0' + (v / 100));
            self.push_byte(b'0' + ((v / 10) % 10));
            self.push_byte(b'0' + (v % 10));
        } else if v >= 10 {
            self.push_byte(b'0' + (v / 10));
            self.push_byte(b'0' + (v % 10));
        } else {
            self.push_byte(b'0' + v);
        }
    }

    #[inline]
    pub(crate) fn format(&mut self, c: ColorRgba) -> &str {
        self.len = 0;
        if c.a == 255 {
            self.push_bytes(b"rgb(");
            self.push_u8(c.r);
            self.push_byte(b',');
            self.push_u8(c.g);
            self.push_byte(b',');
            self.push_u8(c.b);
            self.push_byte(b')');
        } else {
            self.push_bytes(b"rgba(");
            self.push_u8(c.r);
            self.push_byte(b',');
            self.push_u8(c.g);
            self.push_byte(b',');
            self.push_u8(c.b);
            self.push_bytes(b",0.");
            let scaled = (u32::from(c.a) * 10_000 + 127) / 255;
            self.push_byte(b'0' + ((scaled / 1000) % 10) as u8);
            self.push_byte(b'0' + ((scaled / 100) % 10) as u8);
            self.push_byte(b'0' + ((scaled / 10) % 10) as u8);
            self.push_byte(b'0' + (scaled % 10) as u8);
            self.push_byte(b')');
        }
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("transparent")
    }
}

#[cfg(target_arch = "wasm32")]
mod dom {
    use nve_abi::ColorRgba;
    use nve_canvas::{
        Canvas, CanvasPaintSize, CanvasPainter, CanvasSink, DirectCanvasRegistry, DrawList,
        GuardedPainter, PolylinePoints,
    };
    use nve_store::{NodeId, NodeStore};
    use nve_tables::tables::Layout;
    use wasm_bindgen::JsCast;

    use super::CssColorBuf;

    /// Post-sanitization HTML5 Canvas2D [`CanvasSink`] with zero heap allocations per draw op.
    pub struct WebCanvas2dSink<'a> {
        ctx2d: &'a web_sys::CanvasRenderingContext2d,
        logical_w: f64,
        logical_h: f64,
    }

    impl<'a> WebCanvas2dSink<'a> {
        #[inline]
        pub const fn new(
            ctx2d: &'a web_sys::CanvasRenderingContext2d,
            logical_w: f64,
            logical_h: f64,
        ) -> Self {
            Self {
                ctx2d,
                logical_w,
                logical_h,
            }
        }
    }

    impl CanvasSink for WebCanvas2dSink<'_> {
        fn clear(&mut self, color: ColorRgba) {
            self.ctx2d
                .clear_rect(0.0, 0.0, self.logical_w, self.logical_h);
            if color.a > 0 {
                let mut css = CssColorBuf::new();
                self.ctx2d.set_fill_style_str(css.format(color));
                self.ctx2d
                    .fill_rect(0.0, 0.0, self.logical_w, self.logical_h);
            }
        }

        fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
            if color.a == 0 {
                return;
            }
            let mut css = CssColorBuf::new();
            self.ctx2d.set_fill_style_str(css.format(color));
            self.ctx2d
                .fill_rect(f64::from(x), f64::from(y), f64::from(w), f64::from(h));
        }

        fn fill_rounded_rect(
            &mut self,
            x: f32,
            y: f32,
            w: f32,
            h: f32,
            radius: f32,
            color: ColorRgba,
        ) {
            if color.a == 0 {
                return;
            }
            let x = f64::from(x);
            let y = f64::from(y);
            let rw = f64::from(w);
            let rh = f64::from(h);
            let r = f64::from(radius);
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
            let mut css = CssColorBuf::new();
            self.ctx2d.set_fill_style_str(css.format(color));
            self.ctx2d.fill();
        }

        fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) {
            if color.a == 0 {
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
            let mut css = CssColorBuf::new();
            self.ctx2d.set_fill_style_str(css.format(color));
            self.ctx2d.fill();
        }

        fn stroke_line(
            &mut self,
            x0: f32,
            y0: f32,
            x1: f32,
            y1: f32,
            width: f32,
            color: ColorRgba,
        ) {
            if color.a == 0 {
                return;
            }
            let mut css = CssColorBuf::new();
            self.ctx2d.begin_path();
            self.ctx2d.move_to(f64::from(x0), f64::from(y0));
            self.ctx2d.line_to(f64::from(x1), f64::from(y1));
            self.ctx2d.set_stroke_style_str(css.format(color));
            self.ctx2d.set_line_width(f64::from(width));
            self.ctx2d.stroke();
        }

        fn stroke_polyline(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba) {
            if color.a == 0 {
                return;
            }
            if points.len() == 1 {
                // 1-point polyline dot parity with `nve-canvas-skia`.
                if let Some((px, py)) = points.get(0) {
                    let radius = (f64::from(width) * 0.5).max(0.5);
                    let mut css = CssColorBuf::new();
                    self.ctx2d.begin_path();
                    let _ = self.ctx2d.arc(
                        f64::from(px),
                        f64::from(py),
                        radius,
                        0.0,
                        core::f64::consts::TAU,
                    );
                    self.ctx2d.set_fill_style_str(css.format(color));
                    self.ctx2d.fill();
                }
                return;
            }
            let mut iter = points.iter();
            if let Some((x0, y0)) = iter.next() {
                let mut css = CssColorBuf::new();
                self.ctx2d.begin_path();
                self.ctx2d.move_to(f64::from(x0), f64::from(y0));
                for (px, py) in iter {
                    self.ctx2d.line_to(f64::from(px), f64::from(py));
                }
                self.ctx2d.set_stroke_style_str(css.format(color));
                self.ctx2d.set_line_width(f64::from(width));
                self.ctx2d.stroke();
            }
        }

        fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
            self.ctx2d.save();
            self.ctx2d.begin_path();
            self.ctx2d
                .rect(f64::from(x), f64::from(y), f64::from(w), f64::from(h));
            self.ctx2d.clip();
        }

        fn pop_clip(&mut self) {
            self.ctx2d.restore();
        }
    }

    /// Guarded [`CanvasPainter`] targeting an HTML5 [`web_sys::CanvasRenderingContext2d`].
    pub struct WebCanvas2dPainter<'a> {
        inner: GuardedPainter<WebCanvas2dSink<'a>>,
    }

    impl<'a> WebCanvas2dPainter<'a> {
        #[inline]
        pub const fn new(
            ctx2d: &'a web_sys::CanvasRenderingContext2d,
            logical_w: f64,
            logical_h: f64,
        ) -> Self {
            Self {
                inner: GuardedPainter::new(WebCanvas2dSink::new(ctx2d, logical_w, logical_h)),
            }
        }
    }

    impl CanvasPainter for WebCanvas2dPainter<'_> {
        #[inline]
        fn clear(&mut self, color: ColorRgba) {
            self.inner.clear(color);
        }

        #[inline]
        fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
            self.inner.fill_rect(x, y, w, h, color);
        }

        #[inline]
        fn fill_rounded_rect(
            &mut self,
            x: f32,
            y: f32,
            w: f32,
            h: f32,
            radius: f32,
            color: ColorRgba,
        ) {
            self.inner.fill_rounded_rect(x, y, w, h, radius, color);
        }

        #[inline]
        fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) {
            self.inner.fill_circle(cx, cy, radius, color);
        }

        #[inline]
        fn stroke_line(
            &mut self,
            x0: f32,
            y0: f32,
            x1: f32,
            y1: f32,
            width: f32,
            color: ColorRgba,
        ) {
            self.inner.stroke_line(x0, y0, x1, y1, width, color);
        }

        #[inline]
        fn stroke_polyline_points(
            &mut self,
            points: PolylinePoints<'_>,
            width: f32,
            color: ColorRgba,
        ) {
            self.inner.stroke_polyline_points(points, width, color);
        }

        #[inline]
        fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
            self.inner.clip_rect(x, y, w, h);
        }

        #[inline]
        fn restore_clip(&mut self) {
            self.inner.restore_clip();
        }
    }

    pub(crate) fn paint_canvas_dom(
        el: &web_sys::Element,
        store: &NodeStore,
        node: NodeId,
        direct: Option<&DirectCanvasRegistry>,
    ) {
        let Some(row) = store.try_row::<Canvas>(node) else {
            return;
        };
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
            if l.height().is_points() && l.height().value().is_finite() && l.height().value() > 0.0
            {
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
            logical_w = 300.0;
        }
        if logical_h <= 0.0 {
            logical_h = 150.0;
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

        // Derive transform from actual clamped backing size so `8192px` clamping scales smoothly
        // instead of cropping the right/bottom edges.
        let scale_x = f64::from(backing_w) / logical_w;
        let scale_y = f64::from(backing_h) / logical_h;
        let _ = ctx2d.set_transform(scale_x, 0.0, 0.0, scale_y, 0.0, 0.0);
        ctx2d.clear_rect(0.0, 0.0, logical_w, logical_h);
        ctx2d.set_line_cap("round");
        ctx2d.set_line_join("round");

        let eff_scale = scale_x.min(scale_y) as f32;
        let size = CanvasPaintSize::new(logical_w as f32, logical_h as f32, eff_scale);
        let mut painter = WebCanvas2dPainter::new(&ctx2d, logical_w, logical_h);
        if let Some(reg) = direct
            && reg.paint(node, &mut painter, size)
        {
            return;
        }
        if let Some(dl) = store.resource::<DrawList>(row.draw_list()) {
            dl.replay(&mut painter);
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use dom::{WebCanvas2dPainter, WebCanvas2dSink};
#[cfg(target_arch = "wasm32")]
pub(crate) use dom::paint_canvas_dom;
