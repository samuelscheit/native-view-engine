//! Live Metrics view (`ActiveView::Metrics`) built on shared `nve-ui` components
//! (`Card`, `Button`, `Separator`) with 2x2 KPI grid, `SkiaCanvas` `NVE_DRAW1`
//! telemetry sparkline, and interactive stress row controls.

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;
use nve_ui::{Button, ButtonSize, ButtonVariant, Card, CardVariant, Separator};

use crate::assets::build_metrics_sparkline;
use crate::state::{WorkspaceSnapshot, WorkspaceState};
use crate::theme::{
    card_border, row_border, LayoutMetrics, BORDER_DEFAULT, CARD_ALT_BG, CARD_BG, CONTROL_BG,
    PANEL_BG, STRETCH_COL, TEXT_MUTED, TEXT_SECONDARY, XRAY_ROW,
};
use crate::ui::mutate_state;

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_metrics_view(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    current: &WorkspaceSnapshot,
    frame_num: u64,
    snap_idx: usize,
    hist_len: usize,
    active_tasks: usize,
    total_msgs: usize,
    accent_hex: &'static str,
    accent_tint: &'static str,
    xray: bool,
    m: LayoutMetrics,
) -> Element {
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);
    let stress_rows = current.stress_rows;
    let sparkline_attr = build_metrics_sparkline(&current.telemetry_samples_us, hist_len, accent_hex)
        .to_attr_string();

    let s_stress8 = Arc::clone(store);
    let s_stress24 = Arc::clone(store);
    let s_stress_rst = Arc::clone(store);

    let kpi_card_style = format!(
        "flex: 1; padding: 10px; gap: 4px; background-color: {CARD_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.card_radius
    );
    let spark_card_style = format!(
        "width: 100%; flex-shrink: 0; padding: 8px; gap: 4px; background-color: {PANEL_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.card_radius
    );
    let add_btn_style = format!(
        "flex: 1; height: 28px; min-height: 28px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        m.button_radius
    );
    let rst_btn_style = format!(
        "flex: 1; height: 28px; min-height: 28px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: {}px; border-width: 1px; border-color: #334155;",
        m.button_radius
    );

    rsx! {
        // 2x2 KPI Grid (`nve-ui` Card)
        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 8px;",
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 8px;",
                Card {
                    variant: CardVariant::Elevated,
                    padding: Some(10.0),
                    gap: Some(4.0),
                    style: Some(kpi_card_style.clone()),
                    text { style: "color: {TEXT_MUTED}; font-size: 10px;", "FRAMES" }
                    text { style: "color: {accent_hex}; font-size: 18px;", "{frame_num}" }
                }
                Card {
                    variant: CardVariant::Elevated,
                    padding: Some(10.0),
                    gap: Some(4.0),
                    style: Some(kpi_card_style.clone()),
                    text { style: "color: {TEXT_MUTED}; font-size: 10px;", "SNAPSHOTS" }
                    text { style: "color: #a78bfa; font-size: 18px;", "{snap_idx}/{hist_len}" }
                }
            }
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 8px;",
                Card {
                    variant: CardVariant::Elevated,
                    padding: Some(10.0),
                    gap: Some(4.0),
                    style: Some(kpi_card_style.clone()),
                    text { style: "color: {TEXT_MUTED}; font-size: 10px;", "ACTIVE TASKS" }
                    text { style: "color: #22c55e; font-size: 18px;", "{active_tasks}" }
                }
                Card {
                    variant: CardVariant::Elevated,
                    padding: Some(10.0),
                    gap: Some(4.0),
                    style: Some(kpi_card_style),
                    text { style: "color: {TEXT_MUTED}; font-size: 10px;", "MESSAGES" }
                    text { style: "color: #f59e0b; font-size: 18px;", "{total_msgs}" }
                }
            }
        }

        // Live SkiaCanvas Telemetry Sparkline emitting NVE_DRAW1 binary draw-list IR
        Card {
            variant: CardVariant::Outlined,
            padding: Some(8.0),
            gap: Some(4.0),
            style: Some(spark_card_style),
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                text { style: "color: {TEXT_MUTED}; font-size: 10px;", "SKIA SPARKLINE (NVE_DRAW1)" }
                text { style: "color: {accent_hex}; font-size: 10px;", "{current.telemetry_samples_us.len()} pts" }
            }
            canvas {
                test_id: "metrics-sparkline-canvas",
                draw_list: "{sparkline_attr}",
                accessibility_label: "Live telemetry frame timing sparkline",
                accessibility_role: "image",
                style: "width: 100%; height: 72px; flex-shrink: 0; background-color: {CARD_ALT_BG}; border-radius: {m.button_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
            }
        }

        Separator {}

        // Stress Controls Row (`nve-ui` Button)
        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px; border-width: {row_bw}px; border-color: {row_bc};",
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("stress-add-8".into()),
                style: Some(add_btn_style.clone()),
                on_press: move |_| mutate_state(&s_stress8, version, |st| st.add_stress_rows(8)),
                "+8 Rows"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("stress-add-24".into()),
                style: Some(add_btn_style),
                on_press: move |_| mutate_state(&s_stress24, version, |st| st.add_stress_rows(24)),
                "+24 Rows"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("stress-reset".into()),
                style: Some(rst_btn_style),
                on_press: move |_| mutate_state(&s_stress_rst, version, |st| st.reset_stress_rows()),
                "Reset"
            }
        }

        // Live Stress Metric Rows
        scrollview {
            style: "width: 100%; flex: 1; flex-direction: column; gap: 5px; border-width: {row_bw}px; border-color: {row_bc};",
            for row_idx in 1..=stress_rows {
                {
                    let us = 180 + ((row_idx * 17) % 140);
                    let row_bg = if row_idx % 2 == 0 { CARD_BG } else { CARD_ALT_BG };
                    let row_lbl = format!("Node Batch #{row_idx:02}");
                    let timing_lbl = format!("{us}us | OK");
                    let (rbw, rbc) = if xray { (1, XRAY_ROW) } else { (1, "#1e293b") };
                    rsx! {
                        view {
                            key: "metric-row-{row_idx}",
                            style: "width: 100%; height: 28px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; background-color: {row_bg}; border-radius: {m.pill_radius}px; border-width: {rbw}px; border-color: {rbc};",
                            text { style: "color: {TEXT_SECONDARY}; font-size: 11px;", "{row_lbl}" }
                            text { style: "color: #22c55e; font-size: 11px;", "{timing_lbl}" }
                        }
                    }
                }
            }
        }
    }
}
