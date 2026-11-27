//! Live Metrics view (`ActiveView::Metrics`) built on shared `nve-ui` components
//! (`Card`, `Button`, `Separator`) with a 2x2 KPI grid, `SkiaCanvas` `NVE_DRAW1`
//! telemetry sparkline, and interactive stress row controls.
//!
//! Subscribes to the `metrics` and `inspector` domains.

use nve_abi::ColorRgba;
use nve_dioxus::prelude::*;
use nve_ui::{Button, ButtonSize, ButtonVariant, Card, CardVariant, ColorHex, Separator};

use crate::assets::build_metrics_sparkline;
use crate::ui::store::use_studio;
use crate::ui::style::use_studio_style;

#[component]
fn KpiCard(label: &'static str, value: String, color: ColorRgba) -> Element {
    nve_dioxus::render_probe::record_render("KpiCard");
    let s = use_studio_style();
    rsx! {
        Card {
            variant: CardVariant::Elevated,
            padding: Some(10.0),
            gap: Some(4.0),
            style: Some(format!(
                "flex: 1; padding: 10px; gap: 4px; background-color: {}; border-radius: {}px; {}",
                s.t.surface.raised.hex(),
                s.m.card_radius,
                s.card()
            )),
            text { style: "color: {s.t.text.tertiary.hex()}; font-size: 10px;", "{label}" }
            text { style: "color: {color.hex()}; font-size: 18px;", "{value}" }
        }
    }
}

#[component]
pub(crate) fn MetricsView() -> Element {
    nve_dioxus::render_probe::record_render("MetricsView");
    let store = use_studio();
    let s = use_studio_style();
    let t = s.t;
    let metrics = store.metrics.read().clone();
    let (frame_count, snapshot_index, history_len) = {
        let i = store.inspector.read();
        (i.frame_count, i.snapshot_index, i.history_len)
    };
    let sparkline = build_metrics_sparkline(
        &metrics.telemetry_samples_us,
        history_len,
        t.status.accent.text,
    );
    let add_btn = s.compact_control(
        "flex: 1; height: 28px; min-height: 28px;",
        2,
        s.m.button_radius,
        Some(true),
    );
    let row = s.row();
    let row_border = s.list_row_border();
    let ok_ink = t.status.success.text.hex();

    rsx! {
        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 8px;",
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 8px;",
                KpiCard { label: "FRAMES", value: frame_count.to_string(), color: t.status.accent.text }
                KpiCard { label: "SNAPSHOTS", value: format!("{snapshot_index}/{history_len}"), color: t.status.info.text }
            }
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 8px;",
                KpiCard { label: "ACTIVE TASKS", value: metrics.active_tasks.to_string(), color: t.status.success.text }
                KpiCard { label: "MESSAGES", value: metrics.total_messages.to_string(), color: t.status.warning.text }
            }
        }

        Card {
            variant: CardVariant::Outlined,
            padding: Some(8.0),
            gap: Some(4.0),
            style: Some(format!(
                "width: 100%; flex-shrink: 0; padding: 8px; gap: 4px; background-color: {}; border-radius: {}px; {}",
                t.surface.base.hex(),
                s.m.card_radius,
                s.card()
            )),
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                text { style: "color: {t.text.tertiary.hex()}; font-size: 10px;", "SKIA SPARKLINE (NVE_DRAW1)" }
                text { style: "color: {t.text.link.hex()}; font-size: 10px;", "{metrics.telemetry_samples_us.len()} pts" }
            }
            view {
                test_id: "metrics-sparkline-canvas",
                nve_table: nve_dioxus::table_attr_fn(move |tx, node| {
                    tx.edit::<nve_canvas::Canvas>(node).draw_list_ephemeral(sparkline.clone());
                }),
                accessibility_label: "Live telemetry frame timing sparkline",
                accessibility_role: "image",
                style: "width: 100%; height: 72px; flex-shrink: 0; background-color: {t.surface.sunken.hex()}; border-radius: {s.m.button_radius}px; border-width: 1px; border-color: {t.border.default.hex()};",
            }
        }

        Separator {}

        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px; {row}",
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("stress-add-8".into()),
                style: Some(add_btn.clone()),
                on_press: move |_| store.update(|st| st.add_stress_rows(8)),
                "+8 Rows"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("stress-add-24".into()),
                style: Some(add_btn),
                on_press: move |_| store.update(|st| st.add_stress_rows(24)),
                "+24 Rows"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("stress-reset".into()),
                style: Some(s.compact_control("flex: 1; height: 28px; min-height: 28px;", 2, s.m.button_radius, Some(false))),
                on_press: move |_| store.update(|st| st.reset_stress_rows()),
                "Reset"
            }
        }

        scrollview {
            style: "width: 100%; flex: 1; flex-direction: column; gap: 5px; {row}",
            for row_idx in 1..=metrics.stress_rows {
                view {
                    key: "metric-row-{row_idx}",
                    style: "width: 100%; height: 28px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; background-color: {s.stripe(row_idx).hex()}; border-radius: {s.m.pill_radius}px; border-width: 1px; border-color: {row_border.hex()};",
                    text { style: "color: {t.text.secondary.hex()}; font-size: 11px;", "Node Batch #{row_idx:02}" }
                    text { style: "color: {ok_ink}; font-size: 11px;", "{180 + ((row_idx * 17) % 140)}us | OK" }
                }
            }
        }
    }
}
