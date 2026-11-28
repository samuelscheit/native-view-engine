//! Live Metrics view (`ActiveView::Metrics`) built on shared `nve-ui` components
//! (`Card`, `Button`, `Separator`) with a 2x2 KPI grid, `SkiaCanvas` `NVE_DRAW1`
//! telemetry sparkline, and interactive stress row controls.
//!
//! Subscribes to the `metrics` and `inspector` domains.

use nve_abi::ColorRgba;
use nve_dioxus::prelude::*;
use nve_ui::{Button, ButtonSize, ButtonVariant, Card, CardVariant, Separator};

use crate::assets::build_metrics_sparkline;
use crate::ui::store::use_studio;
use crate::ui::style::{StudioStyleExt, flex_dims, row_between, text_style, use_studio_style};

#[component]
fn KpiCard(label: &'static str, value: String, color: ColorRgba) -> Element {
    nve_dioxus::render_probe::record_render("KpiCard");
    let s = use_studio_style();
    rsx! {
        Card {
            variant: CardVariant::Elevated,
            padding: Some(10.0),
            gap: Some(4.0),
            nve_style: Some(
                TypedStyle::new()
                    .flex_1()
                    .padding(10.0)
                    .gap(4.0)
                    .bg(s.t.surface.raised)
                    .radius(s.m.card_radius as f32)
                    .outline(s.card()),
            ),
            text { nve_style: text_style(s.t.text.tertiary, 10.0), "{label}" }
            text { nve_style: text_style(color, 18.0), "{value}" }
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
    let add_btn = s.compact_control(flex_dims(28.0), 2, s.m.button_radius, Some(true));
    let row = s.row();
    let row_border = s.list_row_border();
    let ok_ink = t.status.success.text;

    rsx! {
        view {
            nve_style: TypedStyle::new().width_pct(100.0).flex_shrink(0.0).flex_col().gap(8.0),
            view {
                nve_style: TypedStyle::new().width_pct(100.0).flex_shrink(0.0).flex_row().gap(8.0),
                KpiCard { label: "FRAMES", value: frame_count.to_string(), color: t.status.accent.text }
                KpiCard { label: "SNAPSHOTS", value: format!("{snapshot_index}/{history_len}"), color: t.status.info.text }
            }
            view {
                nve_style: TypedStyle::new().width_pct(100.0).flex_shrink(0.0).flex_row().gap(8.0),
                KpiCard { label: "ACTIVE TASKS", value: metrics.active_tasks.to_string(), color: t.status.success.text }
                KpiCard { label: "MESSAGES", value: metrics.total_messages.to_string(), color: t.status.warning.text }
            }
        }

        Card {
            variant: CardVariant::Outlined,
            padding: Some(8.0),
            gap: Some(4.0),
            nve_style: Some(s.card_surface(8, 4, t.surface.base, s.m.card_radius)),
            view {
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_row()
                    .justify_content(FlexJustify::SpaceBetween)
                    .align_items(FlexAlign::Center),
                text { nve_style: text_style(t.text.tertiary, 10.0), "SKIA SPARKLINE (NVE_DRAW1)" }
                text { nve_style: text_style(t.text.link, 10.0), "{metrics.telemetry_samples_us.len()} pts" }
            }
            view {
                test_id: "metrics-sparkline-canvas",
                nve_table: nve_dioxus::table_attr_fn(move |tx, node| {
                    tx.edit::<nve_canvas::Canvas>(node).draw_list_ephemeral(sparkline.clone());
                }),
                accessibility_label: "Live telemetry frame timing sparkline",
                accessibility_role: "image",
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .height_px(72.0)
                    .flex_shrink(0.0)
                    .bg(t.surface.sunken)
                    .radius(s.m.button_radius as f32)
                    .border(1.0, t.border.default),
            }
        }

        Separator {}

        view {
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_shrink(0.0)
                .flex_row()
                .gap(6.0)
                .outline(row),
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("stress-add-8".into()),
                nve_style: Some(add_btn.clone()),
                on_press: move |_| store.update(|st| st.add_stress_rows(8)),
                "+8 Rows"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("stress-add-24".into()),
                nve_style: Some(add_btn),
                on_press: move |_| store.update(|st| st.add_stress_rows(24)),
                "+24 Rows"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("stress-reset".into()),
                nve_style: Some(s.compact_control(flex_dims(28.0), 2, s.m.button_radius, Some(false))),
                on_press: move |_| store.update(|st| st.reset_stress_rows()),
                "Reset"
            }
        }

        scrollview {
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_1()
                .flex_col()
                .gap(5.0)
                .outline(row),
            for row_idx in 1..=metrics.stress_rows {
                view {
                    key: "metric-row-{row_idx}",
                    nve_style: row_between()
                        .height_px(28.0)
                        .padding_x(10.0)
                        .bg(s.stripe(row_idx))
                        .radius(s.m.pill_radius as f32)
                        .border(1.0, row_border),
                    text { nve_style: text_style(t.text.secondary, 11.0), "Node Batch #{row_idx:02}" }
                    text { nve_style: text_style(ok_ink, 11.0), "{180 + ((row_idx * 17) % 140)}us | OK" }
                }
            }
        }
    }
}
