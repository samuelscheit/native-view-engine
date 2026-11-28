//! Interactive `"canvas-draw1"` demo compiling flat `DrawList` (`nve.res.draw_list`) streams
//! rendered by `<skia_canvas>` across Skia GPU, HTML5 Canvas2D, CoreGraphics, and Flutter/Compose.

use nve_abi::ColorRgba;
use nve_canvas::{
    write_canvas, CanvasPaintSize, CanvasPainter, CanvasSink, DirectCanvasRegistry, DrawList,
    GuardedPainter, PolylinePoints,
};
use nve_dioxus::prelude::*;
use nve_dioxus::table_attr_fn;
use nve_ui::{
    Accent, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, ColorHex, Theme, use_theme,
    with_alpha,
};

pub const CANVAS_DRAW1_SOURCE: &str = r#"use nve_dioxus::prelude::*;
use nve_ui::{use_theme, with_alpha};

let theme = use_theme();
let mut dl = DrawList::builder();
dl.clear(theme.surface.sunken)
  .fill_rounded_rect(18.0, 20.0, 140.0, 74.0, 12.0, with_alpha(theme.status.accent.solid, 44))
  .stroke_polyline(&[(24.0, 118.0), (110.0, 76.0), (210.0, 96.0), (320.0, 42.0)], 2.5, theme.status.accent.text)
  .fill_circle(320.0, 42.0, 7.0, theme.status.info.text);

let list = dl.finish();
rsx! {
    skia_canvas {
        draw_list: CanvasDrawList(list),
        style: "width: 100%; height: 156px; border-radius: 8px;",
    }
}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SceneMode {
    Sparkline,
    Bars,
    Geometry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteChoice {
    Cyan,
    Violet,
    Emerald,
}

/// Pink partner hue for the violet palette.
const PINK: Accent = Accent::new(350.0, 0.17, 0.76);

impl PaletteChoice {
    const fn label(self) -> &'static str {
        match self {
            Self::Cyan => "Cyber Cyan",
            Self::Violet => "Linear Violet",
            Self::Emerald => "Emerald",
        }
    }

    /// `(primary, secondary, area fill)` derived from `theme` with each palette's accent pair, so
    /// the strokes stay contrast-safe on the chart well in every polarity.
    fn colors(self, theme: &Theme) -> (ColorRgba, ColorRgba, ColorRgba) {
        let (primary, secondary) = match self {
            Self::Cyan => (Accent::CYAN, Accent::INDIGO),
            Self::Violet => (Accent::VIOLET, PINK),
            Self::Emerald => (Accent::GREEN, Accent::CYAN),
        };
        let primary = theme.with_accent(primary).status.accent;
        let secondary = theme.with_accent(secondary).status.accent.text;
        let area_fill = with_alpha(primary.solid, 48);
        (primary.text, secondary, area_fill)
    }
}

/// Zero-allocation op counter used for direct-mode telemetry without constructing a `DrawList`.
#[derive(Default)]
struct OpCounter(u32);

impl CanvasSink for OpCounter {
    fn clear(&mut self, _color: ColorRgba) {
        self.0 += 1;
    }
    fn fill_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _color: ColorRgba) {
        self.0 += 1;
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
        self.0 += 1;
    }
    fn fill_circle(&mut self, _cx: f32, _cy: f32, _radius: f32, _color: ColorRgba) {
        self.0 += 1;
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
        self.0 += 1;
    }
    fn stroke_polyline(&mut self, _points: PolylinePoints<'_>, _width: f32, _color: ColorRgba) {
        self.0 += 1;
    }
    fn push_clip_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {
        self.0 += 1;
    }
    fn pop_clip(&mut self) {
        self.0 += 1;
    }
}

/// Shared scene painter driving both in-process direct painting (`write_canvas`) and recorded
/// `DrawList::record` output from one single implementation.
fn paint_demo_scene(
    p: &mut impl CanvasPainter,
    size: CanvasPaintSize,
    theme: &Theme,
    mode: SceneMode,
    palette: PaletteChoice,
    stroke_width: f32,
) {
    let (primary, secondary, tint) = palette.colors(theme);
    let bg = theme.surface.sunken;
    let grid = theme.border.subtle;

    p.clear(bg);

    // Subtle baseline grid lines adapting to live logical width.
    let right = (size.width - 14.0).max(14.0);
    for y in [36.0f32, 76.0, 116.0] {
        p.stroke_line(14.0, y, right, y, 1.0, grid);
    }

    match mode {
        SceneMode::Sparkline => {
            let pts = [
                (20.0, 118.0),
                (76.0, 96.0),
                (132.0, 108.0),
                (188.0, 62.0),
                (244.0, 74.0),
                (300.0, 42.0),
                (356.0, 56.0),
                (412.0, 30.0),
                (468.0, 38.0),
                (510.0, 22.0),
            ];
            p.fill_rounded_rect(18.0, 18.0, 158.0, 42.0, 8.0, tint);
            p.stroke_polyline(&pts, stroke_width, primary);
            for &(x, y) in &pts {
                p.fill_circle(x, y, stroke_width + 2.0, secondary);
            }
        }
        SceneMode::Bars => {
            let heights = [48.0f32, 72.0, 56.0, 96.0, 82.0, 110.0, 68.0, 102.0, 88.0];
            for (idx, h) in heights.into_iter().enumerate() {
                let x = 24.0 + (idx as f32) * 54.0;
                let y = 134.0 - h;
                let color = if idx % 2 == 0 { primary } else { secondary };
                p.fill_rounded_rect(x, y, 36.0, h, 6.0, color);
                p.stroke_line(x, y, x + 36.0, y, stroke_width, theme.surface.base);
            }
        }
        SceneMode::Geometry => {
            p.fill_rounded_rect(26.0, 26.0, 144.0, 96.0, 16.0, tint);
            p.fill_circle(236.0, 76.0, 44.0, primary);
            p.fill_circle(272.0, 76.0, 30.0, secondary);
            p.stroke_polyline(
                &[
                    (336.0, 118.0),
                    (388.0, 26.0),
                    (440.0, 118.0),
                    (336.0, 118.0),
                ],
                stroke_width,
                primary,
            );
            p.fill_rounded_rect(462.0, 32.0, 44.0, 88.0, 10.0, secondary);
        }
    }
}

#[component]
pub fn CanvasDraw1Demo() -> Element {
    let theme = use_theme();
    let direct = try_consume_context::<DirectCanvasRegistry>();
    let mut mode = use_signal(|| SceneMode::Sparkline);
    let mut palette = use_signal(|| PaletteChoice::Cyan);
    let mut stroke_w = use_signal(|| 2.5f32);

    let cur_mode = mode();
    let cur_pal = palette();
    let cur_stroke = stroke_w();
    let fallback = CanvasPaintSize::new(536.0, 152.0, 1.0);
    let (op_count, byte_len) = if direct.is_some() {
        let mut counter = GuardedPainter::new(OpCounter::default());
        paint_demo_scene(&mut counter, fallback, &theme, cur_mode, cur_pal, cur_stroke);
        (counter.sink().0, 0usize)
    } else {
        let list = DrawList::record(|p| {
            paint_demo_scene(p, fallback, &theme, cur_mode, cur_pal, cur_stroke);
        });
        (list.op_count(), list.byte_len())
    };
    let palette_name = cur_pal.label();
    let theme_for_draw = theme;
    let direct_for_write = direct;

    rsx! {
        view {
            test_id: "pg-canvas-root",
            style: "width: 100%; flex-direction: column; gap: 12px;",

            // Telemetry bar
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px;",
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 6px; align-items: center;",
                    Badge {
                        test_id: Some("pg-canvas-ops".to_string()),
                        label: format!("DrawList Ops: {op_count}"),
                        variant: BadgeVariant::Accent,
                    }
                    Badge {
                        test_id: Some("pg-canvas-bytes".to_string()),
                        label: format!("Binary IR: {byte_len} B"),
                        variant: BadgeVariant::Success,
                    }
                    Badge {
                        test_id: Some("pg-canvas-palette".to_string()),
                        label: palette_name.to_string(),
                        variant: BadgeVariant::Neutral,
                    }
                }
                text {
                    style: "font-size: 11px; color: {theme.text.tertiary.hex()};",
                    "Binary vector stream"
                }
            }

            // Live SkiaCanvas / HTML5 Canvas2D surface
            view {
                test_id: "pg-canvas-surface",
                accessibility_label: "Interactive DrawList vector canvas",
                nve_table: table_attr_fn(move |tx, node| {
                    write_canvas(
                        tx,
                        node,
                        direct_for_write.as_ref(),
                        fallback,
                        move |mut p, size| {
                            paint_demo_scene(
                                &mut p,
                                size,
                                &theme_for_draw,
                                cur_mode,
                                cur_pal,
                                cur_stroke,
                            );
                        },
                    );
                }),
                style: "width: 100%; height: 152px; border-radius: 8px; border-width: 1px; border-color: {theme.border.default.hex()}; background-color: {theme.surface.sunken.hex()};",
            }

            // Controls row: Shape mode | Palette | Stroke width
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; flex-wrap: wrap; gap: 8px;",
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                    for (id, lbl, m) in [
                        ("sparkline", "Sparkline", SceneMode::Sparkline),
                        ("bars", "Bars", SceneMode::Bars),
                        ("geometry", "Geometry", SceneMode::Geometry),
                    ] {
                        Button {
                            key: "mode-{id}",
                            test_id: Some(format!("pg-canvas-mode-{id}")),
                            variant: if mode() == m { ButtonVariant::Primary } else { ButtonVariant::Secondary },
                            size: ButtonSize::Sm,
                            on_press: move |_| mode.set(m),
                            "{lbl}"
                        }
                    }
                }
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                    for (id, lbl, p) in [
                        ("cyan", "Cyan", PaletteChoice::Cyan),
                        ("violet", "Violet", PaletteChoice::Violet),
                        ("emerald", "Emerald", PaletteChoice::Emerald),
                    ] {
                        Button {
                            key: "pal-{id}",
                            test_id: Some(format!("pg-canvas-pal-{id}")),
                            variant: if palette() == p { ButtonVariant::Primary } else { ButtonVariant::Outline },
                            size: ButtonSize::Sm,
                            on_press: move |_| palette.set(p),
                            "{lbl}"
                        }
                    }
                    Button {
                        test_id: Some("pg-canvas-stroke".to_string()),
                        variant: ButtonVariant::Ghost,
                        size: ButtonSize::Sm,
                        on_press: move |_| {
                            let next = if stroke_w() < 2.0 {
                                2.5
                            } else if stroke_w() < 3.5 {
                                4.0
                            } else {
                                1.5
                            };
                            stroke_w.set(next);
                        },
                        "Stroke {stroke_w:.1}px"
                    }
                }
            }
        }
    }
}
