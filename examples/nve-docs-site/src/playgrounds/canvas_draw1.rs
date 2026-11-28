//! Interactive `"canvas-draw1"` demo driving one shared `CanvasPainter` via `write_canvas` (direct
//! in-process or flat `DrawList` `nve.res.draw_list` streams) across Skia, HTML5 Canvas2D,
//! CoreGraphics, and Flutter/Compose.

use std::sync::atomic::{AtomicU32, Ordering};

use nve_abi::ColorRgba;
use nve_canvas::{
    write_canvas, Canvas, CanvasPaintSize, CanvasPainter, DirectCanvasRegistry, DrawList,
};
use nve_dioxus::prelude::*;
use nve_dioxus::table_attr_fn;
use nve_ui::{
    Accent, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Theme, use_theme, with_alpha,
};

static DEMO_RECORDED_GEN: AtomicU32 = AtomicU32::new(1);

pub const CANVAS_DRAW1_SOURCE: &str = r#"use nve_canvas::{write_canvas, CanvasPaintSize, CanvasPainter, DirectCanvasRegistry};
use nve_dioxus::prelude::*;
use nve_dioxus::table_attr_fn;
use nve_ui::{Theme, use_theme, with_alpha};

fn paint_scene(p: &mut (impl CanvasPainter + ?Sized), size: CanvasPaintSize, theme: &Theme) {
    p.clear(theme.surface.sunken);
    p.fill_rounded_rect(18.0, 20.0, (size.width - 36.0).max(40.0), 74.0, 12.0, with_alpha(theme.status.accent.solid, 44));
    p.stroke_polyline(&[(24.0, 118.0), (110.0, 76.0), (210.0, 96.0), (320.0, 42.0)], 2.5, theme.status.accent.text);
    p.fill_circle(320.0, 42.0, 7.0, theme.status.info.text);
}

let theme = use_theme();
let direct = try_consume_context::<DirectCanvasRegistry>();
rsx! {
    view {
        nve_table: table_attr_fn(move |tx, node| {
            write_canvas(tx, node, direct.as_ref(), CanvasPaintSize::new(536.0, 152.0, 1.0), move |p, size| {
                paint_scene(p, size, &theme);
            });
        }),
        nve_style: TypedStyle::new().width_pct(100.0).height_px(152.0).radius(8.0),
    }
}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SceneMode {
    Sparkline,
    Bars,
    Geometry,
}

impl SceneMode {
    /// Exact draw-op count emitted by [`paint_demo_scene`] (1 clear + 3 grid lines + scene ops).
    const fn op_count(self) -> u32 {
        match self {
            Self::Sparkline => 16,
            Self::Bars => 22,
            Self::Geometry => 9,
        }
    }
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

/// Shared scene painter driving both in-process direct painting (`write_canvas`) and recorded
/// `DrawList::record` output from one single implementation.
fn paint_demo_scene(
    p: &mut (impl CanvasPainter + ?Sized),
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
    let is_direct = direct.is_some();
    let recorded_list = (!is_direct).then(|| {
        DrawList::record(|p| {
            paint_demo_scene(p, fallback, &theme, cur_mode, cur_pal, cur_stroke);
        })
    });
    let (op_count, byte_len) = match &recorded_list {
        Some(list) => (list.op_count(), list.byte_len()),
        None => (cur_mode.op_count(), 0usize),
    };
    let ops_badge = if is_direct {
        format!("Direct Ops: {op_count}")
    } else {
        format!("DrawList Ops: {op_count}")
    };
    let ir_badge = if is_direct {
        "Direct In-Process (0 B IR)".to_string()
    } else {
        format!("Binary IR: {byte_len} B")
    };
    let stream_caption = if is_direct {
        "Direct CanvasPainter (zero IR)"
    } else {
        "Binary vector stream"
    };
    let palette_name = cur_pal.label();
    let theme_for_draw = theme;
    let direct_for_write = direct;

    rsx! {
        view {
            test_id: "pg-canvas-root",
            nve_style: TypedStyle::col().width_pct(100.0).gap(12.0),

            // Telemetry bar
            view {
                nve_style: TypedStyle::row()
                    .width_pct(100.0)
                    .justify(FlexJustify::SpaceBetween)
                    .align(FlexAlign::Center)
                    .flex_wrap(FlexWrap::Wrap)
                    .gap(8.0),
                view {
                    nve_style: TypedStyle::row()
                        .flex_wrap(FlexWrap::Wrap)
                        .gap(6.0)
                        .align(FlexAlign::Center),
                    Badge {
                        test_id: Some("pg-canvas-ops".to_string()),
                        label: ops_badge,
                        variant: BadgeVariant::Accent,
                    }
                    Badge {
                        test_id: Some("pg-canvas-bytes".to_string()),
                        label: ir_badge,
                        variant: BadgeVariant::Success,
                    }
                    Badge {
                        test_id: Some("pg-canvas-palette".to_string()),
                        label: palette_name.to_string(),
                        variant: BadgeVariant::Neutral,
                    }
                }
                text {
                    nve_style: TypedStyle::new().font_size(11.0).color(theme.text.tertiary),
                    "{stream_caption}"
                }
            }

            // Live SkiaCanvas / HTML5 Canvas2D surface
            view {
                test_id: "pg-canvas-surface",
                accessibility_label: "Interactive DrawList vector canvas",
                nve_table: table_attr_fn(move |tx, node| {
                    if let Some(list) = recorded_list.as_ref() {
                        // Reuse the already-recorded `DrawList` (O(1) `Arc::clone`, zero second pass).
                        let next_gen = DEMO_RECORDED_GEN
                            .fetch_add(1, Ordering::Relaxed)
                            .wrapping_add(1)
                            .max(1);
                        tx.edit::<Canvas>(node)
                            .draw_list_ephemeral(list.clone())
                            .generation(next_gen);
                    } else {
                        write_canvas(
                            tx,
                            node,
                            direct_for_write.as_ref(),
                            fallback,
                            move |p, size| {
                                paint_demo_scene(
                                    p,
                                    size,
                                    &theme_for_draw,
                                    cur_mode,
                                    cur_pal,
                                    cur_stroke,
                                );
                            },
                        );
                    }
                }),
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .height_px(152.0)
                    .radius(8.0)
                    .border(1.0, theme.border.default)
                    .bg(theme.surface.sunken),
            }

            // Controls row: Shape mode | Palette | Stroke width
            view {
                nve_style: TypedStyle::row()
                    .width_pct(100.0)
                    .justify(FlexJustify::SpaceBetween)
                    .flex_wrap(FlexWrap::Wrap)
                    .gap(8.0),
                view {
                    nve_style: TypedStyle::row().flex_wrap(FlexWrap::Wrap).gap(6.0),
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
                    nve_style: TypedStyle::row().flex_wrap(FlexWrap::Wrap).gap(6.0),
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
