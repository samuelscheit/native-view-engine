//! Native Skia GPU/CPU gradient shaders (`gradient::shaders::linear_gradient` / `radial_gradient`)
//! for `--features skia`.

use nve_abi::ColorRgba;
use nve_tables::paint::{normalize_stops, stops_are_normalized, BackgroundFill, GradientStop};
use nve_engine::LayoutRect;
use skia_safe::gradient::{
    self, interpolation::InPremul, Colors as GradientColors, Gradient, Interpolation,
};
use skia_safe::{Canvas, Color4f, Paint, PaintStyle, Point, RRect, Rect, Shader, TileMode};

use super::skia_renderer::to_skia_color;

pub(crate) fn draw_rounded_rect_gradient(
    canvas: &Canvas,
    rect: LayoutRect,
    radius: f32,
    fill: &BackgroundFill,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let sk_rect = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
    let r = radius
        .max(0.0)
        .min(rect.width * 0.5)
        .min(rect.height * 0.5);
    let rrect = RRect::new_rect_xy(sk_rect, r, r);

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);

    match fill {
        BackgroundFill::Solid(c) => {
            if c.a > 0 {
                paint.set_color(to_skia_color(*c));
                canvas.draw_rrect(rrect, &paint);
            }
        }
        BackgroundFill::Linear(lg) => {
            if lg.stops.is_empty() {
                return;
            }
            if lg.stops.len() == 1 {
                let c = lg.stops[0].color;
                if c.a > 0 {
                    paint.set_color(to_skia_color(c));
                    canvas.draw_rrect(rrect, &paint);
                }
                return;
            }
            let (p0, p1) = lg.endpoints(rect.x, rect.y, rect.width, rect.height);
            if let Some(shader) = build_linear_shader(p0, p1, &lg.stops) {
                paint.set_shader(shader);
                canvas.draw_rrect(rrect, &paint);
            }
        }
        BackgroundFill::Radial(rg) => {
            if rg.stops.is_empty() {
                return;
            }
            if rg.stops.len() == 1 {
                let c = rg.stops[0].color;
                if c.a > 0 {
                    paint.set_color(to_skia_color(c));
                    canvas.draw_rrect(rrect, &paint);
                }
                return;
            }
            let (cx, cy, rx, ry) = rg.geometry(rect.x, rect.y, rect.width, rect.height);
            if rx <= 1e-6 || ry <= 1e-6 {
                let c = rg.stops.last().map_or(ColorRgba::TRANSPARENT, |s| s.color);
                if c.a > 0 {
                    paint.set_color(to_skia_color(c));
                    canvas.draw_rrect(rrect, &paint);
                }
                return;
            }
            if let Some(shader) = build_radial_shader(cx, cy, rx, ry, &rg.stops) {
                paint.set_shader(shader);
                canvas.draw_rrect(rrect, &paint);
            }
        }
    }
}

fn prepare_skia_stops(stops: &[GradientStop]) -> (Vec<Color4f>, Vec<f32>) {
    let owned;
    let slice: &[GradientStop] = if stops_are_normalized(stops) {
        stops
    } else {
        owned = normalize_stops(stops.to_vec());
        &owned
    };
    let mut colors = Vec::with_capacity(slice.len());
    let mut positions = Vec::with_capacity(slice.len());
    for s in slice {
        colors.push(Color4f::from(to_skia_color(s.color)));
        positions.push(s.offset.clamp(0.0, 1.0));
    }
    (colors, positions)
}

fn build_linear_shader(p0: (f32, f32), p1: (f32, f32), stops: &[GradientStop]) -> Option<Shader> {
    let (colors, positions) = prepare_skia_stops(stops);
    let grad = Gradient::new(
        GradientColors::new(&colors, Some(&positions), TileMode::Clamp, None),
        Interpolation {
            in_premul: InPremul::Yes,
            ..Default::default()
        },
    );
    gradient::shaders::linear_gradient((Point::new(p0.0, p0.1), Point::new(p1.0, p1.1)), &grad, None)
}

fn build_radial_shader(
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    stops: &[GradientStop],
) -> Option<Shader> {
    let (colors, positions) = prepare_skia_stops(stops);
    let grad = Gradient::new(
        GradientColors::new(&colors, Some(&positions), TileMode::Clamp, None),
        Interpolation {
            in_premul: InPremul::Yes,
            ..Default::default()
        },
    );
    let r = rx.max(ry);
    let local_matrix = if (rx - ry).abs() > 1e-4 && r > 0.0 {
        let mut m = skia_safe::Matrix::new_identity();
        m.pre_translate((cx, cy));
        m.pre_scale((rx / r, ry / r), None);
        m.pre_translate((-cx, -cy));
        Some(m)
    } else {
        None
    };
    gradient::shaders::radial_gradient((Point::new(cx, cy), r), &grad, local_matrix.as_ref())
}
