//! Section 6: Motion, Graphics & Resilience — interactive `SpringSpec` (`snappy`/`gentle`/
//! `bouncy`/`stiff`) + `TransitionSpec` respecting `ReducedMotion`, live `Canvas` (`NVE_DRAW1`)
//! sparkline & bar chart, and live `UiErrorBoundary` subtree error catch + Retry recovery.

use nve_abi::ColorRgba;
use nve_canvas::{Canvas, DrawList};
use nve_motion::{
    write_transitions, EasingCurve, Motion, MotionClock, MotionKey, MotionPreference,
    TransitionSpec,
};
use nve_dioxus::prelude::*;
use nve_dioxus::table_attr_fn;
use nve_ui::{ColorHex, 
    ui_error, use_theme, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card,
    CardVariant, ErrorBoundary as UiErrorBoundary, ErrorBoundaryReset, ErrorFallback,
    UiErrorReport,
};

use crate::i18n_copy::copy;
use crate::sections::{responsive_two_col, responsive_two_col_cell, SectionHeader};
use crate::shell::AppHandle;
use crate::state::SpringPreset;

const SPARKLINE_A: [f32; 16] = [
    18.0, 24.0, 21.0, 34.0, 42.0, 38.0, 56.0, 61.0, 52.0, 68.0, 74.0, 65.0, 79.0, 86.0, 81.0, 94.0,
];
const SPARKLINE_B: [f32; 16] = [
    64.0, 58.0, 62.0, 49.0, 55.0, 72.0, 68.0, 77.0, 84.0, 71.0, 66.0, 80.0, 88.0, 74.0, 91.0, 89.0,
];

fn sample_spring_curve(preset: SpringPreset, reduced_motion: bool) -> (f32, f32, f32) {
    let pref = MotionPreference::from_reduced_motion(reduced_motion);
    let mut clock = MotionClock::with_preference(pref);
    const DEMO: MotionKey = MotionKey::named("demo");
    clock.animate_spring(DEMO, preset.spec(), 0.0, 100.0);
    clock.tick(0.08);
    let t80 = clock.value(&DEMO).unwrap_or(100.0);
    clock.tick(0.12);
    let t200 = clock.value(&DEMO).unwrap_or(100.0);
    clock.tick(0.30);
    let t500 = clock.value(&DEMO).unwrap_or(100.0);
    (t80, t200, t500)
}

fn build_canvas_draw_list(
    bar_mode: bool,
    alt_series: bool,
    accent: ColorRgba,
    success: ColorRgba,
    bg: ColorRgba,
    grid: ColorRgba,
) -> DrawList {
    let samples = if alt_series { &SPARKLINE_B } else { &SPARKLINE_A };
    let mut dl = DrawList::with_capacity(24);
    if bar_mode {
        dl.clear(bg);
        for line in 1..4 {
            let y = line as f32 * 24.0;
            dl.stroke_line(8.0, y, 312.0, y, 1.0, grid);
        }
        let bar_w = 13.0;
        let gap = 5.5;
        for (i, &val) in samples.iter().enumerate() {
            let x = 10.0 + i as f32 * (bar_w + gap);
            let h = (val / 100.0 * 78.0).max(4.0);
            let y = 92.0 - h;
            let color = if i % 2 == 0 { accent } else { success };
            dl.fill_rounded_rect(x, y, bar_w, h, 3.0, color);
        }
    } else {
        dl.sparkline(samples, 320.0, 100.0, accent, bg);
        dl.fill_circle(308.0, 100.0 - (samples[15] / 100.0 * 88.0), 4.0, success);
    }
    dl.finish()
}

#[component]
pub fn MotionSection(handle: AppHandle) -> Element {
    let theme = use_theme();
    let snap = handle.read_state();
    let pseudo = snap.pseudo_locale;

    let mut bar_mode = use_signal(|| false);
    let mut alt_series = use_signal(|| false);
    let armed_error = use_signal(|| false);
    let mut recovery_count = use_signal(|| 0u32);

    let text_primary = theme.text.primary.hex();
    let text_muted = theme.text.tertiary.hex();
    let surface_subtle = theme.surface.subtle.hex();
    let border = theme.border.default.hex();

    let (t80, t200, t500) = sample_spring_curve(snap.spring_preset, snap.reduced_motion);
    let spec = snap.spring_preset.spec();

    let transition_ms = if snap.reduced_motion {
        0
    } else {
        match snap.spring_preset {
            SpringPreset::Snappy => 180,
            SpringPreset::Gentle => 380,
            SpringPreset::Bouncy => 280,
            SpringPreset::Stiff => 120,
        }
    };

    let motion_pill_style = TypedStyle::new()
        .width_pct(if snap.motion_toggled { 100.0 } else { 42.0 })
        .padding_xy(14.0, 10.0)
        .radius(if snap.motion_toggled { 14.0 } else { 8.0 })
        .bg(if snap.motion_toggled {
            theme.status.accent.solid
        } else {
            theme.surface.subtle
        })
        .border(
            1.0,
            if snap.motion_toggled {
                theme.status.accent.text
            } else {
                theme.border.default
            },
        );
    let motion_pill_transitions = [TransitionSpec::all(transition_ms)
        .easing(EasingCurve::EaseOut)
        .reduced_motion_instant(true)];

    let canvas_dl = build_canvas_draw_list(
        bar_mode(),
        alt_series(),
        theme.status.accent.text,
        theme.status.success.text,
        theme.surface.subtle,
        theme.border.default,
    );
    let canvas_table = table_attr_fn(move |tx, node| {
        tx.edit::<Canvas>(node).draw_list_ephemeral(canvas_dl.clone());
    });

    let custom_fallback = ErrorFallback::from(move |report: UiErrorReport, reset: ErrorBoundaryReset| {
        let loc_text = report
            .source_location
            .clone()
            .unwrap_or_else(|| "unknown location".to_owned());
        rsx! {
            ErrorFallbackCard {
                report_msg: report.message,
                loc_text,
                reset_count: report.reset_count,
                armed_error,
                reset,
            }
        }
    });

    rsx! {
        view {
            test_id: "section-motion",
            style: "width: 100%; flex-direction: column; gap: 16px;",

            SectionHeader {
                title: copy("Motion, Graphics & Resilience", pseudo),
                subtitle: copy("SpringSpec physics presets + TransitionSpec respecting ReducedMotion, Canvas draw-list charts, and isolated UiErrorBoundary recovery.", pseudo),
                tag: "4 Engine Systems".to_owned(),
            }

            view {
                nve_style: responsive_two_col(),

                // SpringSpec & TransitionSpec Card
                view {
                    nve_style: responsive_two_col_cell(),
                    Card {
                        test_id: Some("ks-motion-card".to_owned()),
                        variant: CardVariant::Outlined,
                        padding: Some(16.0),
                        gap: Some(12.0),
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 6px;",
                            text {
                                style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                                "{copy(\"SpringSpec & TransitionSpec\", pseudo)}"
                            }
                            Badge {
                                label: if snap.reduced_motion { "Reduced Motion (0ms)".to_owned() } else { format!("{transition_ms}ms") },
                                variant: if snap.reduced_motion { BadgeVariant::Warning } else { BadgeVariant::Accent },
                            }
                        }

                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                            for preset in SpringPreset::ALL {
                                Button {
                                    key: "{preset.id()}",
                                    test_id: Some(format!("ks-spring-{}", preset.id())),
                                    variant: if snap.spring_preset == preset {
                                        ButtonVariant::Primary
                                    } else {
                                        ButtonVariant::Outline
                                    },
                                    size: ButtonSize::Sm,
                                    on_press: move |_| {
                                        handle.update(|st| st.spring_preset = preset);
                                    },
                                    text { "{preset.label()}" }
                                }
                            }
                        }

                        text {
                            test_id: "ks-spring-metrics",
                            style: "font-size: 12px; color: {text_muted};",
                            "k={spec.stiffness:.0}, c={spec.damping:.0}, m={spec.mass:.1} · 80ms={t80:.1}% · 200ms={t200:.1}% · 500ms={t500:.1}%"
                        }

                        view {
                            style: "width: 100%; padding: 8px; border-radius: 10px; background-color: {surface_subtle}; border-width: 1px; border-color: {border};",
                            view {
                                test_id: "ks-motion-pill",
                                nve_style: motion_pill_style,
                                nve_table: nve_dioxus::table_attr::<Motion, _>(
                                    motion_pill_transitions,
                                    |tx, node, specs| write_transitions(tx, node, specs),
                                ),
                                text {
                                    style: "font-size: 12px; font-weight: 700; color: {text_primary};",
                                    if snap.motion_toggled { "Expanded Target State" } else { "Compact Rest State" }
                                }
                            }
                        }

                        Button {
                            test_id: Some("ks-motion-toggle".to_owned()),
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Sm,
                            on_press: move |_| {
                                handle.update(|st| st.motion_toggled = !st.motion_toggled);
                            },
                            text { "{copy(\"Animate State Toggle\", pseudo)}" }
                        }
                    }
                }

                // NVE_DRAW1 Canvas Card
                view {
                    nve_style: responsive_two_col_cell(),
                    Card {
                        test_id: Some("ks-canvas-card".to_owned()),
                        variant: CardVariant::Outlined,
                        padding: Some(16.0),
                        gap: Some(12.0),
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 6px;",
                            text {
                                style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                                "{copy(\"Canvas (NVE_DRAW1)\", pseudo)}"
                            }
                            Badge {
                                label: if bar_mode() { "Bar Chart".to_owned() } else { "Sparkline".to_owned() },
                                variant: BadgeVariant::Success,
                            }
                        }

                        view {
                            test_id: "ks-draw1-canvas",
                            accessibility_label: "Live NVE_DRAW1 telemetry visualization",
                            accessibility_role: "image",
                            nve_table: canvas_table,
                            style: "width: 100%; height: 100px; border-radius: 8px;",
                        }

                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 8px;",
                            Button {
                                test_id: Some("ks-canvas-mode".to_owned()),
                                variant: ButtonVariant::Primary,
                                size: ButtonSize::Sm,
                                on_press: move |_| bar_mode.set(!bar_mode()),
                                text {
                                    if bar_mode() { "Switch to Sparkline" } else { "Switch to Bar Chart" }
                                }
                            }
                            Button {
                                test_id: Some("ks-canvas-dataset".to_owned()),
                                variant: ButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                on_press: move |_| alt_series.set(!alt_series()),
                                text { "{copy(\"Swap Dataset\", pseudo)}" }
                            }
                        }
                    }
                }
            }

            // Live UiErrorBoundary Resilience Demo
            Card {
                test_id: Some("ks-error-boundary-card".to_owned()),
                variant: CardVariant::Elevated,
                padding: Some(16.0),
                gap: Some(12.0),
                view {
                    style: "flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px;",
                    text {
                        style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                        "{copy(\"UiErrorBoundary Subtree Isolation\", pseudo)}"
                    }
                    Badge {
                        label: format!("Recoveries: {}", recovery_count()),
                        variant: BadgeVariant::Neutral,
                    }
                }
                text {
                    style: "font-size: 12px; color: {text_muted};",
                    "Triggering a render error inside the isolated subtree catches the UiErrorReport with source location while keeping the rest of the gallery live."
                }

                UiErrorBoundary {
                    test_id: Some("ks-error-boundary".to_owned()),
                    fallback: custom_fallback,
                    on_reset: move |count: u32| {
                        recovery_count.set(count);
                    },
                    IsolatedWidgetSubtree { armed_error }
                }
            }
        }
    }
}

#[component]
fn IsolatedWidgetSubtree(mut armed_error: Signal<bool>) -> Element {
    if armed_error() {
        return Err(ui_error("Simulated render failure inside isolated Kitchen Sink subtree").into());
    }
    let theme = use_theme();
    let text_primary = theme.text.primary.hex();
    let surface_subtle = theme.surface.subtle.hex();
    let border = theme.border.default.hex();

    rsx! {
        view {
            test_id: "ks-error-subtree-healthy",
            style: "width: 100%; flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px; padding: 12px; border-radius: 8px; background-color: {surface_subtle}; border-width: 1px; border-color: {border};",
            view {
                style: "flex-direction: row; align-items: center; gap: 8px;",
                Badge { label: "Healthy".to_owned(), variant: BadgeVariant::Success }
                text {
                    style: "font-size: 13px; color: {text_primary};",
                    "Protected subtree is mounted and operational."
                }
            }
            Button {
                test_id: Some("ks-trigger-error".to_owned()),
                variant: ButtonVariant::Destructive,
                size: ButtonSize::Sm,
                on_press: move |_| armed_error.set(true),
                text { "Trigger Subtree Error" }
            }
        }
    }
}

#[component]
fn ErrorFallbackCard(
    report_msg: String,
    loc_text: String,
    reset_count: u32,
    mut armed_error: Signal<bool>,
    reset: ErrorBoundaryReset,
) -> Element {
    let theme = use_theme();
    let danger = theme.status.danger.text.hex();
    let text_primary = theme.text.primary.hex();
    let text_muted = theme.text.tertiary.hex();
    let surface_subtle = theme.surface.subtle.hex();

    rsx! {
        view {
            test_id: "ks-error-boundary-fallback",
            accessibility_role: "alert",
            aria_live: "assertive",
            style: "width: 100%; flex-direction: column; gap: 8px; padding: 14px; border-radius: 10px; background-color: {surface_subtle}; border-width: 1px; border-color: {danger};",
            view {
                style: "flex-direction: row; flex-wrap: wrap; align-items: center; gap: 8px;",
                Badge { label: "Subtree Error Caught".to_owned(), variant: BadgeVariant::Danger }
                text {
                    style: "font-size: 13px; font-weight: 700; color: {danger};",
                    "{report_msg}"
                }
            }
            text {
                test_id: "ks-error-source-location",
                style: "font-size: 12px; color: {text_muted};",
                "Origin: {loc_text} (previous resets: {reset_count})"
            }
            text {
                style: "font-size: 12px; color: {text_primary};",
                "Sibling controls and the rest of Kitchen Sink remain completely unaffected."
            }
            view {
                style: "flex-direction: row; gap: 8px;",
                Button {
                    test_id: Some("ks-error-boundary-retry".to_owned()),
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    on_press: move |_| {
                        armed_error.set(false);
                        reset.reset();
                    },
                    text { "Retry" }
                }
            }
        }
    }
}
