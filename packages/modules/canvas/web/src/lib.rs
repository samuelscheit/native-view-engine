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

#[cfg(target_arch = "wasm32")]
fn paint_canvas_dom(
    el: &web_sys::Element,
    store: &NodeStore,
    node: NodeId,
    dl: nve_canvas::DrawListView<'_>,
) {
    use nve_canvas::CanvasOpRef;
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

    let mut color_buf = String::with_capacity(32);
    let mut clip_depth = 0u32;
    for op in dl.ops() {
        match op {
            CanvasOpRef::Clear { color } => {
                ctx2d.clear_rect(0.0, 0.0, logical_w, logical_h);
                if color.a > 0 {
                    ctx2d.set_fill_style_str(format_css_color(color, &mut color_buf));
                    ctx2d.fill_rect(0.0, 0.0, logical_w, logical_h);
                }
            }
            CanvasOpRef::FillRect { x, y, w: rw, h: rh, color } => {
                if !x.is_finite() || !y.is_finite() || !rw.is_finite() || !rh.is_finite() || rw <= 0.0 || rh <= 0.0 {
                    continue;
                }
                ctx2d.set_fill_style_str(format_css_color(color, &mut color_buf));
                ctx2d.fill_rect(f64::from(x), f64::from(y), f64::from(rw), f64::from(rh));
            }
            CanvasOpRef::FillRoundedRect { x, y, w: rw, h: rh, radius, color } => {
                if !x.is_finite() || !y.is_finite() || !rw.is_finite() || !rh.is_finite() || rw <= 0.0 || rh <= 0.0 {
                    continue;
                }
                let x = f64::from(x);
                let y = f64::from(y);
                let rw = f64::from(rw);
                let rh = f64::from(rh);
                let raw_r = if radius.is_finite() { f64::from(radius) } else { 0.0 };
                let r = raw_r.clamp(0.0, (rw * 0.5).min(rh * 0.5));
                ctx2d.begin_path();
                ctx2d.move_to(x + r, y);
                ctx2d.line_to(x + rw - r, y);
                ctx2d.quadratic_curve_to(x + rw, y, x + rw, y + r);
                ctx2d.line_to(x + rw, y + rh - r);
                ctx2d.quadratic_curve_to(x + rw, y + rh, x + rw - r, y + rh);
                ctx2d.line_to(x + r, y + rh);
                ctx2d.quadratic_curve_to(x, y + rh, x, y + rh - r);
                ctx2d.line_to(x, y + r);
                ctx2d.quadratic_curve_to(x, y, x + r, y);
                ctx2d.close_path();
                ctx2d.set_fill_style_str(format_css_color(color, &mut color_buf));
                ctx2d.fill();
            }
            CanvasOpRef::FillCircle { cx, cy, radius, color } => {
                if !cx.is_finite() || !cy.is_finite() || !radius.is_finite() || radius <= 0.0 {
                    continue;
                }
                ctx2d.begin_path();
                let _ = ctx2d.arc(
                    f64::from(cx),
                    f64::from(cy),
                    f64::from(radius),
                    0.0,
                    core::f64::consts::TAU,
                );
                ctx2d.set_fill_style_str(format_css_color(color, &mut color_buf));
                ctx2d.fill();
            }
            CanvasOpRef::StrokeLine { x0, y0, x1, y1, width, color } => {
                if !x0.is_finite() || !y0.is_finite() || !x1.is_finite() || !y1.is_finite() || !width.is_finite() || width <= 0.0 {
                    continue;
                }
                ctx2d.begin_path();
                ctx2d.move_to(f64::from(x0), f64::from(y0));
                ctx2d.line_to(f64::from(x1), f64::from(y1));
                ctx2d.set_stroke_style_str(format_css_color(color, &mut color_buf));
                ctx2d.set_line_width(f64::from(width));
                ctx2d.stroke();
            }
            CanvasOpRef::StrokePolyline { points, width, color } => {
                if !width.is_finite() || width <= 0.0 {
                    continue;
                }
                let mut iter = points.iter().filter(|(px, py)| px.is_finite() && py.is_finite());
                if let Some((x0, y0)) = iter.next() {
                    ctx2d.begin_path();
                    ctx2d.move_to(f64::from(x0), f64::from(y0));
                    for (px, py) in iter {
                        ctx2d.line_to(f64::from(px), f64::from(py));
                    }
                    ctx2d.set_stroke_style_str(format_css_color(color, &mut color_buf));
                    ctx2d.set_line_width(f64::from(width));
                    ctx2d.stroke();
                }
            }
            CanvasOpRef::ClipRect { x, y, w: rw, h: rh } => {
                ctx2d.save();
                clip_depth += 1;
                ctx2d.begin_path();
                if x.is_finite() && y.is_finite() && rw.is_finite() && rh.is_finite() {
                    ctx2d.rect(
                        f64::from(x),
                        f64::from(y),
                        f64::from(rw.max(0.0)),
                        f64::from(rh.max(0.0)),
                    );
                } else {
                    // Push an empty clip rect so subsequent `RestoreClip` stays balanced.
                    ctx2d.rect(0.0, 0.0, 0.0, 0.0);
                }
                ctx2d.clip();
            }
            CanvasOpRef::RestoreClip => {
                if clip_depth > 0 {
                    ctx2d.restore();
                    clip_depth -= 1;
                }
            }
        }
    }
    while clip_depth > 0 {
        ctx2d.restore();
        clip_depth -= 1;
    }
}
