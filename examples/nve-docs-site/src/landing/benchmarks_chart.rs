//! Purple-Garden-inspired interactive Benchmark Progression Dashboard rendered 100% in NVE
//! using `NVE_DRAW1` `<canvas>` vector charts and `nve-ui` controls — zero external JS/Chart.js.

use nve_dioxus::prelude::*;
use nve_ui::accent_indicator;
use nve_docs::use_docs_state;
use nve_ui::Switch;

use crate::landing::PillBadge;
use crate::landing::benchmarks_data::{
    BASELINE_BENCHMARKS, BASELINE_GENERATED_AT, BASELINE_MACHINE, BenchCategory,
    EmbeddedBaselineEntry, LATEST_BARS, LatestBarSpec, PROGRESSION_SERIES, ProgressionSeries,
    bar_chart_lo_bound, build_bar_chart_draw_list, build_line_chart_draw_list, series_tone,
    format_ns, format_val, line_chart_lo_bound, lookup_baseline,
};

/// Interactive Purple-Garden-style Benchmark Progression Dashboard powered 100% by `NVE_DRAW1`.
#[component]
pub fn BenchmarkProgressionDashboard() -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let is_mobile = snap.viewport.is_mobile();
    drop(snap);

    let mut category = use_signal(|| BenchCategory::All);
    let mut log_scale = use_signal(|| true);
    let mut table_view = use_signal(|| false);

    let active_cat = category();
    let is_log = log_scale();
    let is_table = table_view();

    let filtered_bars: Vec<(&LatestBarSpec, &'static EmbeddedBaselineEntry)> = LATEST_BARS
        .iter()
        .filter(|b| active_cat == BenchCategory::All || b.category == active_cat)
        .filter_map(|b| lookup_baseline(b.baseline_key).map(|entry| (b, entry)))
        .collect();

    let filtered_series: Vec<&ProgressionSeries> = PROGRESSION_SERIES
        .iter()
        .filter(|s| active_cat == BenchCategory::All || s.category == active_cat)
        .collect();

    let bar_canvas_w: f32 = if is_mobile { 270.0 } else { 1040.0 };
    let bar_canvas_h: f32 = 176.0;
    let bar_values: Vec<(f64, u8)> = filtered_bars
        .iter()
        .map(|(spec, entry)| (entry.median_ns, spec.color_idx))
        .collect();
    let bar_draw_list =
        build_bar_chart_draw_list(&bar_values, is_log, bar_canvas_w, bar_canvas_h, &theme);

    let max_bar_ns = filtered_bars
        .iter()
        .map(|(_, e)| e.median_ns)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let min_bar_ns = filtered_bars
        .iter()
        .map(|(_, e)| e.median_ns)
        .fold(f64::INFINITY, f64::min);
    let lo_bar_ns = bar_chart_lo_bound(min_bar_ns, is_log);
    let mid_bar_ns = if is_log {
        (max_bar_ns * lo_bar_ns.max(0.5)).sqrt()
    } else {
        max_bar_ns * 0.5
    };
    let lo_bar_label = if is_log {
        format_ns(lo_bar_ns)
    } else {
        "0 ns".to_string()
    };

    let scale_badge = if is_log { "Log10 Y-Axis" } else { "Linear Y-Axis" };
    let view_toggle_label = if is_table { "Chart view" } else { "Table view" };
    let line_canvas_w: f32 = if is_mobile { 260.0 } else { 446.0 };
    let line_canvas_h: f32 = 148.0;
    let series_rows: Vec<Vec<&ProgressionSeries>> = filtered_series
        .chunks(2)
        .map(|c| c.to_vec())
        .collect();
    let mobile_col_row = TypedStyle::new()
        .width_pct(100.0)
        .flex_row()
        .gap(16.0)
        .at(MediaQuery::max_width(959), RowPatch::new().flex_col());
    let bar_canvas_style = TypedStyle::new()
        .width_px(bar_canvas_w)
        .max_width_pct(100.0)
        .flex_grow(1.0)
        .height_px(bar_canvas_h)
        .radius(10.0)
        .border(1.0, theme.border.default)
        .at(MediaQuery::max_width(959), RowPatch::new().width_px(270.0));
    let line_canvas_style = TypedStyle::new()
        .width_px(line_canvas_w)
        .max_width_pct(100.0)
        .flex_grow(1.0)
        .height_px(line_canvas_h)
        .radius(10.0)
        .border(1.0, theme.border.default)
        .at(MediaQuery::max_width(959), RowPatch::new().width_px(260.0));

    let th_cell = |grow: f32, right: bool| {
        let s = TypedStyle::new().flex_grow(grow).flex_basis_pct(0.0).font_size(11.0).font_weight(700).color(theme.text.secondary);
        if right { s.text_align(TextAlign::Right) } else { s }
    };
    let mono_muted = TypedStyle::new().font_size(10.0).font_family("monospace").color(theme.text.secondary);

    rsx! {
        view {
            test_id: "bench-dashboard",
            nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(18.0),

            // Purple-Garden-style Controls Bar: Category pills | Log scale switch | Table view toggle
            view {
                test_id: "bench-controls-bar",
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_row()
                    .justify_content(FlexJustify::SpaceBetween)
                    .align_items(FlexAlign::Center)
                    .flex_wrap(FlexWrap::Wrap)
                    .gap(12.0)
                    .padding_xy(16.0, 14.0)
                    .radius(12.0)
                    .bg(theme.surface.base)
                    .border(1.0, theme.border.default),

                view {
                    nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).flex_wrap(FlexWrap::Wrap).gap(6.0),
                    for cat in BenchCategory::ALL {
                        {
                            let sel = active_cat == cat;
                            let bg = if sel { theme.status.accent.subtle } else { theme.surface.subtle };
                            let fg = if sel { theme.text.link } else { theme.text.secondary };
                            let bc = if sel { accent_indicator(&theme) } else { theme.border.default };
                            let tid = format!("bench-filter-{}", cat.id());
                            let swatch = cat.color_idx().map(|ci| series_tone(&theme, ci).text);
                            rsx! {
                                pressable {
                                    key: "cat-{cat.id()}",
                                    test_id: "{tid}",
                                    accessibility_role: "button",
                                    "aria-pressed": if sel { "true" } else { "false" },
                                    nve_style: TypedStyle::new()
                                        .flex_shrink(0.0)
                                        .flex_row()
                                        .align_items(FlexAlign::Center)
                                        .gap(6.0)
                                        .padding_xy(11.0, 6.0)
                                        .radius(7.0)
                                        .bg(bg)
                                        .border(1.0, bc),
                                    onpress: move |_| category.set(cat),
                                    if let Some(col) = swatch {
                                        view {
                                            nve_style: TypedStyle::new().width_px(7.0).height_px(7.0).radius(999.0).bg(col).flex_shrink(0.0),
                                        }
                                    }
                                    text {
                                        nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(fg),
                                        "{cat.label()}"
                                    }
                                }
                            }
                        }
                    }
                }

                view {
                    nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).flex_wrap(FlexWrap::Wrap).gap(14.0),
                    Switch {
                        test_id: Some("bench-log-scale".to_string()),
                        checked: is_log,
                        label: Some("Log scale".to_string()),
                        on_change: move |v| log_scale.set(v),
                    }
                    pressable {
                        test_id: "bench-table-toggle",
                        accessibility_role: "button",
                        "aria-pressed": if is_table { "true" } else { "false" },
                        nve_style: TypedStyle::new()
                            .flex_shrink(0.0)
                            .padding_xy(12.0, 6.0)
                            .radius(7.0)
                            .bg(theme.surface.subtle)
                            .border(1.0, theme.border.default),
                        onpress: move |_| table_view.set(!is_table),
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(theme.text.primary),
                            "{view_toggle_label}"
                        }
                    }
                }
            }

            // Latest Run Comparison Panel (NVE_DRAW1 Bar Chart OR Structured Table)
            view {
                test_id: "bench-latest-panel",
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_col()
                    .gap(14.0)
                    .padding(20.0)
                    .radius(14.0)
                    .bg(theme.surface.base)
                    .border(1.0, theme.border.default),

                view {
                    nve_style: TypedStyle::new()
                        .width_pct(100.0)
                        .flex_row()
                        .justify_content(FlexJustify::SpaceBetween)
                        .align_items(FlexAlign::Center)
                        .flex_wrap(FlexWrap::Wrap)
                        .gap(8.0),
                    view {
                        nve_style: TypedStyle::new().flex_col().gap(2.0),
                        text {
                            nve_style: TypedStyle::new().font_size(15.0).font_weight(700).color(theme.text.primary),
                            "Latest Verified Baseline Comparison (benchmarks/baseline.json)"
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                            "{BASELINE_MACHINE} · Snapshot {BASELINE_GENERATED_AT} · {BASELINE_BENCHMARKS.len()} tracked benchmarks"
                        }
                    }
                    view {
                        nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(6.0).flex_wrap(FlexWrap::Wrap),
                        PillBadge { label: scale_badge.to_string(), accent: true }
                        PillBadge { label: "NVE_DRAW1 Canvas".to_string(), secondary: true }
                    }
                }

                if is_table {
                    view {
                        test_id: "bench-latest-table",
                        accessibility_role: "table",
                        nve_style: TypedStyle::new()
                            .width_pct(100.0)
                            .flex_col()
                            .radius(10.0)
                            .border(1.0, theme.border.default)
                            .overflow_mode(Overflow::Hidden),
                        view {
                            accessibility_role: "row",
                            nve_style: TypedStyle::new()
                                .width_pct(100.0)
                                .flex_row()
                                .padding_xy(12.0, 9.0)
                                .bg(theme.surface.subtle)
                                .border(1.0, theme.border.default),
                            text { nve_style: th_cell(3.0, false), "BENCHMARK" }
                            text { nve_style: th_cell(1.0, false), "SUITE" }
                            text { nve_style: th_cell(1.0, true), "MEDIAN" }
                            text { nve_style: th_cell(1.0, true), "MEAN" }
                            text { nve_style: th_cell(1.0, true), "TOL." }
                        }
                        for (idx, (spec, entry)) in filtered_bars.iter().enumerate() {
                            {
                                let row_bg = if idx % 2 == 0 { theme.surface.canvas } else { theme.surface.base };
                                let med_str = format_ns(entry.median_ns);
                                let mean_str = format_ns(entry.mean_ns);
                                let tol_str = format!("±{:.0}%", entry.tolerance_pct);
                                let td_mono_r = TypedStyle::new().flex_grow(1.0).flex_basis_pct(0.0).font_size(12.0).font_family("monospace").color(theme.text.secondary).text_align(TextAlign::Right);
                                rsx! {
                                    view {
                                        key: "bt-row-{spec.baseline_key}",
                                        accessibility_role: "row",
                                        nve_style: TypedStyle::new()
                                            .width_pct(100.0)
                                            .flex_row()
                                            .align_items(FlexAlign::Center)
                                            .padding_xy(12.0, 8.0)
                                            .bg(row_bg)
                                            .border(1.0, theme.border.default),
                                        text { nve_style: TypedStyle::new().flex_grow(3.0).flex_basis_pct(0.0).font_size(12.0).font_family("monospace").color(theme.text.primary), "{entry.name}" }
                                        text { nve_style: TypedStyle::new().flex_grow(1.0).flex_basis_pct(0.0).font_size(12.0).color(theme.text.secondary), "{entry.suite}" }
                                        text { nve_style: td_mono_r.clone().font_weight(700).color(theme.text.link), "{med_str}" }
                                        text { nve_style: td_mono_r.clone(), "{mean_str}" }
                                        text { nve_style: td_mono_r, "{tol_str}" }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    view {
                        nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(10.0),
                        view {
                            nve_style: TypedStyle::new().width_pct(100.0).flex_row().align_items(FlexAlign::Stretch).gap(10.0),
                            view {
                                test_id: "bench-bar-yaxis",
                                nve_style: TypedStyle::new()
                                    .width_px(58.0)
                                    .flex_shrink(0.0)
                                    .flex_col()
                                    .justify_content(FlexJustify::SpaceBetween)
                                    .padding_y(10.0)
                                    .align_items(FlexAlign::FlexEnd),
                                text { nve_style: mono_muted.clone(), "{format_ns(max_bar_ns)}" }
                                text { nve_style: mono_muted.clone(), "{format_ns(mid_bar_ns)}" }
                                text { nve_style: mono_muted.clone(), "{lo_bar_label}" }
                            }
                            view {
                                test_id: "bench-latest-canvas",
                                nve_style: bar_canvas_style,
                                accessibility_label: "Latest verified NVE benchmark latency bar chart",
                                nve_table: nve_dioxus::table_attr_fn(move |tx, node| {
                                    tx.edit::<nve_canvas::Canvas>(node)
                                        .draw_list_ephemeral(bar_draw_list.clone());
                                }),
                            }
                        }
                        view {
                            nve_style: TypedStyle::new().width_pct(100.0).flex_row().align_items(FlexAlign::Center).gap(10.0),
                            view { nve_style: TypedStyle::new().width_px(58.0).flex_shrink(0.0) }
                            view {
                                nve_style: TypedStyle::new()
                                    .flex_grow(1.0)
                                    .flex_row()
                                    .justify_content(FlexJustify::SpaceAround)
                                    .align_items(FlexAlign::Center)
                                    .padding_x(14.0),
                                for (bar_idx, (spec, _)) in filtered_bars.iter().enumerate() {
                                    text {
                                        key: "bar-tick-{spec.short_label}",
                                        nve_style: mono_muted.clone(),
                                        "#{bar_idx + 1}"
                                    }
                                }
                            }
                        }
                        view {
                            test_id: "bench-bar-legend",
                            nve_style: TypedStyle::new().width_pct(100.0).flex_row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                            for (bar_idx, (spec, entry)) in filtered_bars.iter().enumerate() {
                                {
                                    let swatch = series_tone(&theme, spec.color_idx).text;
                                    let ordinal = bar_idx + 1;
                                    rsx! {
                                        view {
                                            key: "bar-lbl-{spec.short_label}",
                                            nve_style: TypedStyle::new()
                                                .flex_shrink(0.0)
                                                .flex_row()
                                                .align_items(FlexAlign::Center)
                                                .gap(6.0)
                                                .padding_xy(8.0, 4.0)
                                                .radius(6.0)
                                                .bg(theme.surface.subtle)
                                                .border(1.0, theme.border.default),
                                            view {
                                                nve_style: TypedStyle::new().width_px(7.0).height_px(7.0).radius(999.0).bg(swatch).flex_shrink(0.0),
                                            }
                                            text { nve_style: mono_muted.clone(), "#{ordinal}" }
                                            text { nve_style: TypedStyle::new().font_size(11.0).color(theme.text.secondary), "{spec.short_label}" }
                                            text { nve_style: TypedStyle::new().font_size(11.0).font_family("monospace").font_weight(700).color(theme.text.primary), "{format_ns(entry.median_ns)}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Benchmark Progression History Grid (explicit 2-column rows on desktop, 1-column on mobile)
            view {
                test_id: "bench-history-grid",
                nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(16.0),
                for (ridx, row) in series_rows.into_iter().enumerate() {
                    view {
                        key: "prog-row-{ridx}",
                        nve_style: mobile_col_row.clone(),
                        for series in row {
                            {
                                let draw_list = build_line_chart_draw_list(
                                    series.points,
                                    is_log,
                                    line_canvas_w,
                                    line_canvas_h,
                                    &theme,
                                    series.color_idx,
                                );
                                let first_pt = series.points.first().map(|p| p.value).unwrap_or(0.0);
                                let last_pt = series.points.last().map(|p| p.value).unwrap_or(0.0);
                                let max_pt = series.points.iter().map(|p| p.value).fold(0.0_f64, f64::max);
                                let min_pt = series.points.iter().map(|p| p.value).fold(f64::INFINITY, f64::min);
                                let lo_pt = line_chart_lo_bound(min_pt, is_log);
                                let lo_label = if is_log {
                                    format_val(lo_pt, series.unit)
                                } else {
                                    format!("0 {}", series.unit)
                                };
                                let start_str = format_val(first_pt, series.unit);
                                let end_str = format_val(last_pt, series.unit);
                                rsx! {
                                    view {
                                        key: "prog-{series.id}",
                                        test_id: "bench-card-{series.id}",
                                        nve_style: TypedStyle::new()
                                            .flex_1()
                                            .min_width_px(0.0)
                                            .flex_col()
                                            .gap(10.0)
                                            .padding(18.0)
                                            .radius(14.0)
                                            .bg(theme.surface.base)
                                            .border(1.0, theme.border.default),

                                        view {
                                            nve_style: TypedStyle::new()
                                                .width_pct(100.0)
                                                .flex_row()
                                                .justify_content(FlexJustify::SpaceBetween)
                                                .align_items(FlexAlign::FlexStart)
                                                .gap(10.0),
                                            view {
                                                nve_style: TypedStyle::new().flex_col().gap(3.0).min_width_px(0.0).flex_shrink(1.0),
                                                text {
                                                    nve_style: TypedStyle::new().font_size(14.0).font_weight(700).color(theme.text.primary),
                                                    "{series.title}"
                                                }
                                                text {
                                                    nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                                                    "{series.subtitle}"
                                                }
                                            }
                                            view {
                                                nve_style: TypedStyle::new().flex_shrink(0.0).flex_col().align_items(FlexAlign::FlexEnd).gap(4.0),
                                                PillBadge { label: series.delta_badge.to_string(), accent: true }
                                                text {
                                                    nve_style: TypedStyle::new().font_size(11.0).font_family("monospace").font_weight(700).color(theme.status.success.text),
                                                    "{start_str} → {end_str}"
                                                }
                                            }
                                        }

                                        view {
                                            nve_style: TypedStyle::new().width_pct(100.0).flex_row().align_items(FlexAlign::Stretch).gap(8.0),
                                            view {
                                                nve_style: TypedStyle::new()
                                                    .width_px(56.0)
                                                    .flex_shrink(0.0)
                                                    .flex_col()
                                                    .justify_content(FlexJustify::SpaceBetween)
                                                    .padding_y(10.0)
                                                    .align_items(FlexAlign::FlexEnd),
                                                text { nve_style: mono_muted.clone(), "{format_val(max_pt, series.unit)}" }
                                                text { nve_style: mono_muted.clone(), "{lo_label}" }
                                            }
                                            view {
                                                test_id: "bench-canvas-{series.id}",
                                                nve_style: line_canvas_style.clone(),
                                                accessibility_label: "{series.title} progression chart",
                                                nve_table: nve_dioxus::table_attr_fn(move |tx, node| {
                                                    tx.edit::<nve_canvas::Canvas>(node)
                                                        .draw_list_ephemeral(draw_list.clone());
                                                }),
                                            }
                                        }

                                        view {
                                            nve_style: TypedStyle::new().width_pct(100.0).flex_row().align_items(FlexAlign::Center).gap(8.0),
                                            view { nve_style: TypedStyle::new().width_px(56.0).flex_shrink(0.0) }
                                            view {
                                                nve_style: TypedStyle::new()
                                                    .flex_grow(1.0)
                                                    .flex_row()
                                                    .justify_content(FlexJustify::SpaceBetween)
                                                    .align_items(FlexAlign::Center)
                                                    .padding_x(14.0),
                                                for pt in series.points {
                                                    text {
                                                        key: "pt-{series.id}-{pt.label}",
                                                        nve_style: mono_muted.clone(),
                                                        "{pt.label}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
