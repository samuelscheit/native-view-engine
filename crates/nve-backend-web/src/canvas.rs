//! `NVE_DRAW1` vector draw-list decoding and `CanvasRenderingContext2d` rendering for `<canvas>`
//! (`ComponentKind::SkiaCanvas`) nodes in `nve-backend-web`.

use nve_abi::RawPropValue;
use nve_components::canvas_ir::{CanvasOp, DrawListDecoder, DrawListError};

#[cfg(target_arch = "wasm32")]
use nve_abi::ColorRgba;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

/// Returns `true` if `key` identifies an `NVE_DRAW1` draw-list raw prop.
#[inline]
pub fn is_draw_list_prop(key: &str) -> bool {
    matches!(key, "drawList" | "draw_list")
}

/// Decodes a `drawList` / `draw_list` [`RawPropValue`] (`Bytes` or base64 `String`) into typed
/// [`CanvasOp`]s using [`DrawListDecoder`].
pub fn decode_draw_list_prop(value: &RawPropValue) -> Result<Vec<CanvasOp>, DrawListError> {
    if matches!(value, RawPropValue::Null) {
        return Ok(Vec::new());
    }
    DrawListDecoder::decode_raw_prop(value)
}

#[cfg(target_arch = "wasm32")]
#[inline]
fn rgba_css(c: ColorRgba, buf: &mut String) {
    buf.clear();
    crate::css::write_rgba(c, buf);
}

/// Paints decoded `NVE_DRAW1` [`CanvasOp`]s onto a live browser `<canvas>` element with crisp
/// `devicePixelRatio` backing-store scaling.
#[cfg(target_arch = "wasm32")]
pub fn paint_canvas_element(
    dom_el: &web_sys::Element,
    ops: &[CanvasOp],
    fallback_width: f64,
    fallback_height: f64,
) {
    let Some(canvas) = dom_el.dyn_ref::<web_sys::HtmlCanvasElement>() else {
        return;
    };

    let rect = dom_el.get_bounding_client_rect();
    let css_w = if rect.width() > 0.0 {
        rect.width()
    } else if fallback_width > 0.0 {
        fallback_width
    } else {
        320.0
    };
    let css_h = if rect.height() > 0.0 {
        rect.height()
    } else if fallback_height > 0.0 {
        fallback_height
    } else {
        72.0
    };

    let dpr = web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(1.0)
        .clamp(1.0, 4.0);

    let backing_w = (css_w * dpr).round().max(1.0) as u32;
    let backing_h = (css_h * dpr).round().max(1.0) as u32;
    if canvas.width() != backing_w {
        canvas.set_width(backing_w);
    }
    if canvas.height() != backing_h {
        canvas.set_height(backing_h);
    }

    let Ok(Some(ctx_obj)) = canvas.get_context("2d") else {
        return;
    };
    let Ok(ctx) = ctx_obj.dyn_into::<web_sys::CanvasRenderingContext2d>() else {
        return;
    };

    let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
    ctx.clear_rect(0.0, 0.0, css_w, css_h);

    let mut color_buf = String::with_capacity(32);
    let mut clip_depth = 0u32;

    for op in ops {
        match op {
            CanvasOp::Clear { color } => {
                ctx.clear_rect(0.0, 0.0, css_w, css_h);
                if color.a > 0 {
                    rgba_css(*color, &mut color_buf);
                    ctx.set_fill_style_str(&color_buf);
                    ctx.fill_rect(0.0, 0.0, css_w, css_h);
                }
            }
            CanvasOp::FillRect { x, y, w, h, color } => {
                if *w <= 0.0 || *h <= 0.0 || color.a == 0 {
                    continue;
                }
                rgba_css(*color, &mut color_buf);
                ctx.set_fill_style_str(&color_buf);
                ctx.fill_rect(*x as f64, *y as f64, *w as f64, *h as f64);
            }
            CanvasOp::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius,
                color,
            } => {
                if *w <= 0.0 || *h <= 0.0 || color.a == 0 {
                    continue;
                }
                let xf = *x as f64;
                let yf = *y as f64;
                let wf = *w as f64;
                let hf = *h as f64;
                let r = (*radius as f64).max(0.0).min(wf * 0.5).min(hf * 0.5);
                rgba_css(*color, &mut color_buf);
                ctx.set_fill_style_str(&color_buf);
                if r <= 0.0 {
                    ctx.fill_rect(xf, yf, wf, hf);
                } else {
                    ctx.begin_path();
                    ctx.move_to(xf + r, yf);
                    ctx.line_to(xf + wf - r, yf);
                    ctx.quadratic_curve_to(xf + wf, yf, xf + wf, yf + r);
                    ctx.line_to(xf + wf, yf + hf - r);
                    ctx.quadratic_curve_to(xf + wf, yf + hf, xf + wf - r, yf + hf);
                    ctx.line_to(xf + r, yf + hf);
                    ctx.quadratic_curve_to(xf, yf + hf, xf, yf + hf - r);
                    ctx.line_to(xf, yf + r);
                    ctx.quadratic_curve_to(xf, yf, xf + r, yf);
                    ctx.close_path();
                    ctx.fill();
                }
            }
            CanvasOp::FillCircle {
                cx,
                cy,
                radius,
                color,
            } => {
                if *radius <= 0.0 || color.a == 0 {
                    continue;
                }
                ctx.begin_path();
                let _ = ctx.arc(
                    *cx as f64,
                    *cy as f64,
                    *radius as f64,
                    0.0,
                    std::f64::consts::TAU,
                );
                rgba_css(*color, &mut color_buf);
                ctx.set_fill_style_str(&color_buf);
                ctx.fill();
            }
            CanvasOp::StrokeLine {
                x0,
                y0,
                x1,
                y1,
                width,
                color,
            } => {
                if *width <= 0.0 || color.a == 0 {
                    continue;
                }
                ctx.begin_path();
                ctx.move_to(*x0 as f64, *y0 as f64);
                ctx.line_to(*x1 as f64, *y1 as f64);
                rgba_css(*color, &mut color_buf);
                ctx.set_stroke_style_str(&color_buf);
                ctx.set_line_width(*width as f64);
                ctx.set_line_cap("round");
                ctx.stroke();
            }
            CanvasOp::StrokePolyline {
                points,
                width,
                color,
            } => {
                if points.len() < 2 || *width <= 0.0 || color.a == 0 {
                    continue;
                }
                ctx.begin_path();
                ctx.move_to(points[0].0 as f64, points[0].1 as f64);
                for &(px, py) in &points[1..] {
                    ctx.line_to(px as f64, py as f64);
                }
                rgba_css(*color, &mut color_buf);
                ctx.set_stroke_style_str(&color_buf);
                ctx.set_line_width(*width as f64);
                ctx.set_line_cap("round");
                ctx.set_line_join("round");
                ctx.stroke();
            }
            CanvasOp::ClipRect { x, y, w, h } => {
                ctx.save();
                ctx.begin_path();
                ctx.rect(*x as f64, *y as f64, *w as f64, *h as f64);
                ctx.clip();
                clip_depth = clip_depth.saturating_add(1);
            }
            CanvasOp::RestoreClip => {
                if clip_depth > 0 {
                    ctx.restore();
                    clip_depth -= 1;
                }
            }
        }
    }

    while clip_depth > 0 {
        ctx.restore();
        clip_depth -= 1;
    }
}
