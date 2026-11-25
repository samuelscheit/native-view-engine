//! Native Skia GPU/CPU gradient shaders (`gradient::shaders::linear_gradient` / `radial_gradient`)
//! and `skia_safe::Picture` cached vector SVG rendering (`--features skia`).

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use nve_abi::ColorRgba;
use nve_components::paint::{normalize_stops, stops_are_normalized, BackgroundFill, GradientStop};
use nve_components::svg::{FillRule, SvgPicture, VectorPathCmd};
use nve_engine::LayoutRect;
use skia_safe::gradient::{self, interpolation::InPremul, Colors as GradientColors, Gradient, Interpolation};
use skia_safe::{
    Canvas, Color4f, Matrix, Paint, PaintCap, PaintJoin, PaintStyle, PathBuilder, PathFillType,
    Picture, PictureRecorder, Point, RRect, Rect, Shader, TileMode,
};

use super::skia_renderer::to_skia_color;

const MAX_SKIA_SVG_PICTURES: usize = 512;

/// Key for cached immutable `skia_safe::Picture` recordings of [`SvgPicture`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SkiaSvgPictureKey {
    content_hash: u64,
    tint: Option<[u8; 4]>,
}

struct PictureCacheEntry {
    picture: Picture,
    last_used: u64,
}

#[derive(Default)]
struct PictureLruInner {
    tick: u64,
    hits: u64,
    misses: u64,
    entries: HashMap<SkiaSvgPictureKey, PictureCacheEntry>,
    recency: BTreeMap<u64, SkiaSvgPictureKey>,
}

/// Thread-safe bounded LRU cache of recorded [`skia_safe::Picture`] objects keyed by
/// `(content_hash, tint)` for zero-rebuild GPU vector SVG replay.
pub(crate) struct SkiaSvgPictureCache {
    inner: Mutex<PictureLruInner>,
}

impl Default for SkiaSvgPictureCache {
    fn default() -> Self {
        Self {
            inner: Mutex::new(PictureLruInner::default()),
        }
    }
}

impl SkiaSvgPictureCache {
    pub(crate) fn get_or_record(&self, svg: &SvgPicture, tint: Option<ColorRgba>) -> Option<Picture> {
        let key = SkiaSvgPictureKey {
            content_hash: svg.content_hash,
            tint: tint.map(|c| [c.r, c.g, c.b, c.a]),
        };
        {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let new_tick = guard.tick.wrapping_add(1);
            guard.tick = new_tick;
            if let Some(entry) = guard.entries.get_mut(&key) {
                let old_tick = entry.last_used;
                entry.last_used = new_tick;
                let pic = entry.picture.clone();
                if let Some(k) = guard.recency.remove(&old_tick) {
                    guard.recency.insert(new_tick, k);
                }
                guard.hits = guard.hits.saturating_add(1);
                return Some(pic);
            }
            guard.misses = guard.misses.saturating_add(1);
        }

        let picture = record_svg_picture(svg, tint)?;
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        while guard.entries.len() >= MAX_SKIA_SVG_PICTURES {
            let Some((_, evict_key)) = guard.recency.pop_first() else {
                break;
            };
            guard.entries.remove(&evict_key);
        }
        let tick = guard.tick.wrapping_add(1);
        guard.tick = tick;
        guard.recency.insert(tick, key);
        guard.entries.insert(
            key,
            PictureCacheEntry {
                picture: picture.clone(),
                last_used: tick,
            },
        );
        Some(picture)
    }

    #[cfg(test)]
    pub(crate) fn stats(&self) -> (u64, u64, usize) {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        (guard.hits, guard.misses, guard.entries.len())
    }
}

/// Paints a [`BackgroundFill`] rounded rectangle directly onto a Skia [`Canvas`] using native
/// `gradient::shaders::linear_gradient` / `radial_gradient` on `Paint::set_shader`.
pub(crate) fn draw_rounded_rect_gradient(
    canvas: &Canvas,
    rect: LayoutRect,
    radius: f32,
    gradient: &BackgroundFill,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let sk_rect = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
    let clamped_r = radius
        .max(0.0)
        .min(rect.width * 0.5)
        .min(rect.height * 0.5);
    let rrect = RRect::new_rect_xy(sk_rect, clamped_r, clamped_r);

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);

    match gradient {
        BackgroundFill::Solid(c) => {
            if c.a == 0 {
                return;
            }
            paint.set_color(to_skia_color(*c));
            canvas.draw_rrect(rrect, &paint);
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

fn premul_interpolation() -> Interpolation {
    Interpolation {
        in_premul: InPremul::Yes,
        ..Default::default()
    }
}

fn build_linear_shader(p0: (f32, f32), p1: (f32, f32), stops: &[GradientStop]) -> Option<Shader> {
    let (colors, positions) = prepare_skia_stops(stops);
    let grad_colors = GradientColors::new(&colors, Some(&positions), TileMode::Clamp, None);
    let grad = Gradient::new(grad_colors, premul_interpolation());
    gradient::shaders::linear_gradient(
        (Point::new(p0.0, p0.1), Point::new(p1.0, p1.1)),
        &grad,
        None,
    )
}

fn build_radial_shader(
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    stops: &[GradientStop],
) -> Option<Shader> {
    let (colors, positions) = prepare_skia_stops(stops);
    let grad_colors = GradientColors::new(&colors, Some(&positions), TileMode::Clamp, None);
    let grad = Gradient::new(grad_colors, premul_interpolation());
    if (rx - ry).abs() <= 1e-4 {
        gradient::shaders::radial_gradient((Point::new(cx, cy), rx), &grad, None)
    } else {
        // Unit radial gradient at origin mapped via scale(rx, ry) + translate(cx, cy) matches
        // CSS `radial-gradient(<rx>% <ry>% at <cx>% <cy>%, ...)` and the software rasterizer.
        let mut local = Matrix::scale((rx, ry));
        local.post_translate((cx, cy));
        gradient::shaders::radial_gradient((Point::new(0.0, 0.0), 1.0), &grad, &local)
    }
}

/// Records an [`SvgPicture`] into an immutable resolution-independent [`skia_safe::Picture`] in
/// its `viewBox` coordinate space using native `PathBuilder` commands.
fn record_svg_picture(svg: &SvgPicture, tint: Option<ColorRgba>) -> Option<Picture> {
    let (vx, vy, vw, vh) = svg.view_box;
    let bounds = Rect::from_xywh(vx, vy, vw.max(1.0), vh.max(1.0));
    let mut recorder = PictureRecorder::new();
    let rec_canvas = recorder.begin_recording(bounds, false);

    let mut fill_paint = Paint::default();
    fill_paint.set_anti_alias(true);
    fill_paint.set_style(PaintStyle::Fill);

    let mut stroke_paint = Paint::default();
    stroke_paint.set_anti_alias(true);
    stroke_paint.set_style(PaintStyle::Stroke);
    stroke_paint.set_stroke_cap(PaintCap::Round);
    stroke_paint.set_stroke_join(PaintJoin::Round);

    for shape in svg.shapes.iter() {
        if shape.path.is_empty() {
            continue;
        }
        let mut pb = PathBuilder::new();
        pb.set_fill_type(match shape.fill_rule {
            FillRule::NonZero => PathFillType::Winding,
            FillRule::EvenOdd => PathFillType::EvenOdd,
        });
        for &cmd in shape.path.iter() {
            match cmd {
                VectorPathCmd::MoveTo(x, y) => {
                    pb.move_to((x, y));
                }
                VectorPathCmd::LineTo(x, y) => {
                    pb.line_to((x, y));
                }
                VectorPathCmd::QuadTo(cx, cy, x, y) => {
                    pb.quad_to((cx, cy), (x, y));
                }
                VectorPathCmd::CubicTo(x1, y1, x2, y2, x, y) => {
                    pb.cubic_to((x1, y1), (x2, y2), (x, y));
                }
                VectorPathCmd::Close => {
                    pb.close();
                }
            }
        }
        let path = pb.detach();
        let (fill, stroke) = shape.effective_colors(tint);
        if let Some(fc) = fill
            && fc.a > 0
        {
            fill_paint.set_color(to_skia_color(fc));
            rec_canvas.draw_path(&path, &fill_paint);
        }
        if let Some(sc) = stroke
            && sc.a > 0
            && shape.stroke_width.is_finite()
            && shape.stroke_width > 0.0
        {
            stroke_paint.set_color(to_skia_color(sc));
            stroke_paint.set_stroke_width(shape.stroke_width);
            rec_canvas.draw_path(&path, &stroke_paint);
        }
    }

    recorder.finish_recording_as_picture(Some(&bounds))
}

/// Draws a cached [`SvgPicture`] onto `canvas` inside `rect` with `xMidYMid meet` scaling.
pub(crate) fn draw_cached_svg(
    cache: &SkiaSvgPictureCache,
    canvas: &Canvas,
    rect: LayoutRect,
    svg: &SvgPicture,
    tint: Option<ColorRgba>,
) {
    if svg.shapes.is_empty() {
        return;
    }
    let Some((scale, ox, oy)) = svg.view_box_transform(rect.x, rect.y, rect.width, rect.height)
    else {
        return;
    };
    let Some(picture) = cache.get_or_record(svg, tint) else {
        return;
    };
    let saved = canvas.save();
    canvas.clip_rect(
        Rect::from_xywh(rect.x, rect.y, rect.width, rect.height),
        skia_safe::ClipOp::Intersect,
        true,
    );
    canvas.translate((ox, oy));
    canvas.scale((scale, scale));
    canvas.draw_picture(&picture, None, None);
    canvas.restore_to_count(saved);
}
