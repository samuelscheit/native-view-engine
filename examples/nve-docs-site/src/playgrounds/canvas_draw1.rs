//! Interactive `"canvas-draw1"` demo compiling real zero-JSON `NVE_DRAW1` binary draw-lists
//! rendered by `<skia_canvas>` across Skia GPU, HTML5 Canvas2D, CoreGraphics, and Flutter/Compose.

use nve_abi::ColorRgba;
use nve_dioxus::prelude::*;
use nve_ui::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, use_theme};

pub const CANVAS_DRAW1_SOURCE: &str = r#"use nve_abi::ColorRgba;
use nve_dioxus::prelude::*;

let mut dl = DrawList::builder();
dl.clear(ColorRgba::rgb(12, 14, 22))
  .fill_rounded_rect(18.0, 20.0, 140.0, 74.0, 12.0, ColorRgba::rgba(56, 189, 248, 44))
  .stroke_polyline(&[(24.0, 118.0), (110.0, 76.0), (210.0, 96.0), (320.0, 42.0)], 2.5, ColorRgba::rgb(56, 189, 248))
  .fill_circle(320.0, 42.0, 7.0, ColorRgba::rgb(129, 140, 248));

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

impl PaletteChoice {
    fn colors(self) -> (ColorRgba, ColorRgba, ColorRgba, &'static str) {
        match self {
            Self::Cyan => (
                ColorRgba::rgb(56, 189, 248),
                ColorRgba::rgb(129, 140, 248),
                ColorRgba::rgba(56, 189, 248, 48),
                "Cyber Cyan",
            ),
            Self::Violet => (
                ColorRgba::rgb(167, 139, 250),
                ColorRgba::rgb(244, 114, 182),
                ColorRgba::rgba(167, 139, 250, 48),
                "Linear Violet",
            ),
            Self::Emerald => (
                ColorRgba::rgb(52, 211, 153),
                ColorRgba::rgb(56, 189, 248),
                ColorRgba::rgba(52, 211, 153, 48),
                "Emerald",
            ),
        }
    }
}

fn build_scene_draw_list(mode: SceneMode, palette: PaletteChoice, stroke_width: f32) -> (DrawList, u32, usize) {
    let (primary, secondary, tint, _) = palette.colors();
    let bg = ColorRgba::rgb(12, 13, 18);
    let grid = ColorRgba::rgba(148, 163, 184, 42);

    let mut dl = DrawList::with_capacity(24);
    dl.clear(bg);

    // Subtle baseline grid lines
    for y in [36.0f32, 76.0, 116.0] {
        dl.stroke_line(14.0, y, 520.0, y, 1.0, grid);
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
            dl.fill_rounded_rect(18.0, 18.0, 158.0, 42.0, 8.0, tint);
            dl.stroke_polyline(&pts, stroke_width, primary);
            for &(x, y) in &pts {
                dl.fill_circle(x, y, stroke_width + 2.0, secondary);
            }
        }
        SceneMode::Bars => {
            let heights = [48.0f32, 72.0, 56.0, 96.0, 82.0, 110.0, 68.0, 102.0, 88.0];
            for (idx, h) in heights.into_iter().enumerate() {
                let x = 24.0 + (idx as f32) * 54.0;
                let y = 134.0 - h;
                let color = if idx % 2 == 0 { primary } else { secondary };
                dl.fill_rounded_rect(x, y, 36.0, h, 6.0, color);
                dl.stroke_line(x, y, x + 36.0, y, stroke_width, ColorRgba::WHITE);
            }
        }
        SceneMode::Geometry => {
            dl.fill_rounded_rect(26.0, 26.0, 144.0, 96.0, 16.0, tint);
            dl.fill_circle(236.0, 76.0, 44.0, primary);
            dl.fill_circle(272.0, 76.0, 30.0, secondary);
            dl.stroke_polyline(
                &[
                    (336.0, 118.0),
                    (388.0, 26.0),
                    (440.0, 118.0),
                    (336.0, 118.0),
                ],
                stroke_width,
                primary,
            );
            dl.fill_rounded_rect(462.0, 32.0, 44.0, 88.0, 10.0, secondary);
        }
    }

    let list = dl.finish();
    let op_count = list.op_count();
    let raw_len = list.to_nve_draw1_bytes().len();
    (list, op_count, raw_len)
}

#[component]
pub fn CanvasDraw1Demo() -> Element {
    let theme = use_theme();
    let mut mode = use_signal(|| SceneMode::Sparkline);
    let mut palette = use_signal(|| PaletteChoice::Cyan);
    let mut stroke_w = use_signal(|| 2.5f32);

    let (draw_list, op_count, byte_len) = build_scene_draw_list(mode(), palette(), stroke_w());
    let (_, _, _, palette_name) = palette().colors();

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
                        label: format!("NVE_DRAW1 Ops: {op_count}"),
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
                    style: "font-size: 11px; color: {theme.palette.text_muted};",
                    "Zero-JSON vector stream"
                }
            }

            // Live SkiaCanvas / HTML5 Canvas2D surface
            canvas {
                test_id: "pg-canvas-surface",
                accessibility_label: "Interactive NVE_DRAW1 vector canvas",
                draw_list: CanvasDrawList(draw_list),
                style: "width: 100%; height: 152px; border-radius: 8px; border-width: 1px; border-color: {theme.palette.border}; background-color: #0c0d12;",
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
