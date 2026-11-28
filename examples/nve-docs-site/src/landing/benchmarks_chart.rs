//! Purple-Garden-inspired interactive Benchmark Progression Dashboard rendered 100% in NVE
//! using `NVE_DRAW1` `<canvas>` vector charts and `nve-ui` controls — zero external JS/Chart.js.

use nve_dioxus::prelude::*;
use nve_ui::{accent_indicator, ColorHex};
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
    let mobile_col = TypedStyle::new()
        .at(MediaQuery::max_width(959), RowPatch::new().flex_col());
    let bar_canvas_resp = TypedStyle::new()
        .at(MediaQuery::max_width(959), RowPatch::new().width_px(270.0));
    let line_canvas_resp = TypedStyle::new()
        .at(MediaQuery::max_width(959), RowPatch::new().width_px(260.0));

    rsx! {
        view {
            test_id: "bench-dashboard",
            style: "width: 100%; flex-direction: column; gap: 18px;",

            // Purple-Garden-style Controls Bar: Category pills | Log scale switch | Table view toggle
            view {
                test_id: "bench-controls-bar",
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 12px; padding: 14px 16px; border-radius: 12px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",

                view {
                    style: "flex-direction: row; align-items: center; flex-wrap: wrap; gap: 6px;",
                    for cat in BenchCategory::ALL {
                        {
                            let sel = active_cat == cat;
                            let bg = if sel { theme.status.accent.subtle.hex() } else { theme.surface.subtle.hex() };
                            let fg = if sel { theme.text.link.hex() } else { theme.text.secondary.hex() };
                            let bc = if sel { accent_indicator(&theme).hex() } else { theme.border.default.hex() };
                            let tid = format!("bench-filter-{}", cat.id());
                            let swatch = cat.color_idx().map(|ci| series_tone(&theme, ci).text.hex());
                            rsx! {
                                pressable {
                                    key: "cat-{cat.id()}",
                                    test_id: "{tid}",
                                    accessibility_role: "button",
                                    "aria-pressed": if sel { "true" } else { "false" },
                                    style: "flex-shrink: 0; flex-direction: row; align-items: center; gap: 6px; padding-horizontal: 11px; padding-vertical: 6px; border-radius: 7px; background-color: {bg}; border-width: 1px; border-color: {bc};",
                                    onpress: move |_| category.set(cat),
                                    if let Some(hex) = swatch {
                                        view {
                                            style: "width: 7px; height: 7px; border-radius: 999px; background-color: {hex}; flex-shrink: 0;",
                                        }
                                    }
                                    text {
                                        style: "font-size: 12px; font-weight: 600; color: {fg};",
                                        "{cat.label()}"
                                    }
                                }
                            }
                        }
                    }
                }

                view {
                    style: "flex-direction: row; align-items: center; flex-wrap: wrap; gap: 14px;",
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
                        style: "flex-shrink: 0; padding-horizontal: 12px; padding-vertical: 6px; border-radius: 7px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                        onpress: move |_| table_view.set(!is_table),
                        text {
                            style: "font-size: 12px; font-weight: 600; color: {theme.text.primary.hex()};",
                            "{view_toggle_label}"
                        }
                    }
                }
            }

            // Latest Run Comparison Panel (NVE_DRAW1 Bar Chart OR Structured Table)
            view {
                test_id: "bench-latest-panel",
                style: "width: 100%; flex-direction: column; gap: 14px; padding: 20px; border-radius: 14px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",

                view {
                    style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px;",
                    view {
                        style: "flex-direction: column; gap: 2px;",
                        text {
                            style: "font-size: 15px; font-weight: 700; color: {theme.text.primary.hex()};",
                            "Latest Verified Baseline Comparison (benchmarks/baseline.json)"
                        }
                        text {
                            style: "font-size: 12px; color: {theme.text.secondary.hex()};",
                            "{BASELINE_MACHINE} · Snapshot {BASELINE_GENERATED_AT} · {BASELINE_BENCHMARKS.len()} tracked benchmarks"
                        }
                    }
                    view {
                        style: "flex-direction: row; align-items: center; gap: 6px; flex-wrap: wrap;",
                        PillBadge { label: scale_badge.to_string(), accent: true }
                        PillBadge { label: "NVE_DRAW1 Canvas".to_string(), secondary: true }
                    }
                }

                if is_table {
                    view {
                        test_id: "bench-latest-table",
                        accessibility_role: "table",
                        style: "width: 100%; flex-direction: column; border-radius: 10px; border-width: 1px; border-color: {theme.border.default.hex()}; overflow: hidden;",
                        view {
                            accessibility_role: "row",
                            style: "width: 100%; flex-direction: row; padding: 9px 12px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                            text { style: "flex-grow: 3; flex-basis: 0%; font-size: 11px; font-weight: 700; color: {theme.text.secondary.hex()};", "BENCHMARK" }
                            text { style: "flex-grow: 1; flex-basis: 0%; font-size: 11px; font-weight: 700; color: {theme.text.secondary.hex()};", "SUITE" }
                            text { style: "flex-grow: 1; flex-basis: 0%; font-size: 11px; font-weight: 700; color: {theme.text.secondary.hex()}; text-align: right;", "MEDIAN" }
                            text { style: "flex-grow: 1; flex-basis: 0%; font-size: 11px; font-weight: 700; color: {theme.text.secondary.hex()}; text-align: right;", "MEAN" }
                            text { style: "flex-grow: 1; flex-basis: 0%; font-size: 11px; font-weight: 700; color: {theme.text.secondary.hex()}; text-align: right;", "TOL." }
                        }
                        for (idx, (spec, entry)) in filtered_bars.iter().enumerate() {
                            {
                                let row_bg = if idx % 2 == 0 { theme.surface.canvas.hex() } else { theme.surface.base.hex() };
                                let med_str = format_ns(entry.median_ns);
                                let mean_str = format_ns(entry.mean_ns);
                                let tol_str = format!("±{:.0}%", entry.tolerance_pct);
                                rsx! {
                                    view {
                                        key: "bt-row-{spec.baseline_key}",
                                        accessibility_role: "row",
                                        style: "width: 100%; flex-direction: row; align-items: center; padding: 8px 12px; background-color: {row_bg}; border-width: 1px; border-color: {theme.border.default.hex()};",
                                        text { style: "flex-grow: 3; flex-basis: 0%; font-size: 12px; font-family: monospace; color: {theme.text.primary.hex()};", "{entry.name}" }
                                        text { style: "flex-grow: 1; flex-basis: 0%; font-size: 12px; color: {theme.text.secondary.hex()};", "{entry.suite}" }
                                        text { style: "flex-grow: 1; flex-basis: 0%; font-size: 12px; font-family: monospace; font-weight: 700; color: {theme.text.link.hex()}; text-align: right;", "{med_str}" }
                                        text { style: "flex-grow: 1; flex-basis: 0%; font-size: 12px; font-family: monospace; color: {theme.text.secondary.hex()}; text-align: right;", "{mean_str}" }
                                        text { style: "flex-grow: 1; flex-basis: 0%; font-size: 12px; font-family: monospace; color: {theme.text.secondary.hex()}; text-align: right;", "{tol_str}" }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    view {
                        style: "width: 100%; flex-direction: column; gap: 10px;",
                        view {
                            style: "width: 100%; flex-direction: row; align-items: stretch; gap: 10px;",
                            view {
                                test_id: "bench-bar-yaxis",
                                style: "width: 58px; flex-shrink: 0; flex-direction: column; justify-content: space-between; padding-vertical: 10px; align-items: flex-end;",
                                text { style: "font-size: 10px; font-family: monospace; color: {theme.text.secondary.hex()};", "{format_ns(max_bar_ns)}" }
                                text { style: "font-size: 10px; font-family: monospace; color: {theme.text.secondary.hex()};", "{format_ns(mid_bar_ns)}" }
                                text { style: "font-size: 10px; font-family: monospace; color: {theme.text.secondary.hex()};", "{lo_bar_label}" }
                            }
                            view {
                                test_id: "bench-latest-canvas",
                                nve_style: bar_canvas_resp.clone(),
                                accessibility_label: "Latest verified NVE benchmark latency bar chart",
                                nve_table: nve_dioxus::table_attr_fn(move |tx, node| {
                                    tx.edit::<nve_canvas::Canvas>(node)
                                        .draw_list_ephemeral(bar_draw_list.clone());
                                }),
                                style: "width: {bar_canvas_w}px; max-width: 100%; flex-grow: 1; height: {bar_canvas_h}px; border-radius: 10px; border-width: 1px; border-color: {theme.border.default.hex()};",
                            }
                        }
                        view {
                            style: "width: 100%; flex-direction: row; align-items: center; gap: 10px;",
                            view { style: "width: 58px; flex-shrink: 0;" }
                            view {
                                style: "flex-grow: 1; flex-direction: row; justify-content: space-around; align-items: center; padding-horizontal: 14px;",
                                for (bar_idx, (spec, _)) in filtered_bars.iter().enumerate() {
                                    text {
                                        key: "bar-tick-{spec.short_label}",
                                        style: "font-size: 10px; font-family: monospace; color: {theme.text.secondary.hex()};",
                                        "#{bar_idx + 1}"
                                    }
                                }
                            }
                        }
                        view {
                            test_id: "bench-bar-legend",
                            style: "width: 100%; flex-direction: row; flex-wrap: wrap; gap: 6px;",
                            for (bar_idx, (spec, entry)) in filtered_bars.iter().enumerate() {
                                {
                                    let swatch = series_tone(&theme, spec.color_idx).text.hex();
                                    let ordinal = bar_idx + 1;
                                    rsx! {
                                        view {
                                            key: "bar-lbl-{spec.short_label}",
                                            style: "flex-shrink: 0; flex-direction: row; align-items: center; gap: 6px; padding-horizontal: 8px; padding-vertical: 4px; border-radius: 6px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                                            view {
                                                style: "width: 7px; height: 7px; border-radius: 999px; background-color: {swatch}; flex-shrink: 0;",
                                            }
                                            text { style: "font-size: 10px; font-family: monospace; color: {theme.text.secondary.hex()};", "#{ordinal}" }
                                            text { style: "font-size: 11px; color: {theme.text.secondary.hex()};", "{spec.short_label}" }
                                            text { style: "font-size: 11px; font-family: monospace; font-weight: 700; color: {theme.text.primary.hex()};", "{format_ns(entry.median_ns)}" }
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
                style: "width: 100%; flex-direction: column; gap: 16px;",
                for (ridx, row) in series_rows.into_iter().enumerate() {
                    view {
                        key: "prog-row-{ridx}",
                        nve_style: mobile_col.clone(),
                        style: "width: 100%; flex-direction: row; gap: 16px;",
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
                                        style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0; flex-direction: column; gap: 10px; padding: 18px; border-radius: 14px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",

                                        view {
                                            style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: flex-start; gap: 10px;",
                                            view {
                                                style: "flex-direction: column; gap: 3px; min-width: 0; flex-shrink: 1;",
                                                text {
                                                    style: "font-size: 14px; font-weight: 700; color: {theme.text.primary.hex()};",
                                                    "{series.title}"
                                                }
                                                text {
                                                    style: "font-size: 12px; color: {theme.text.secondary.hex()};",
                                                    "{series.subtitle}"
                                                }
                                            }
                                            view {
                                                style: "flex-shrink: 0; flex-direction: column; align-items: flex-end; gap: 4px;",
                                                PillBadge { label: series.delta_badge.to_string(), accent: true }
                                                text {
                                                    style: "font-size: 11px; font-family: monospace; font-weight: 700; color: {theme.status.success.text.hex()};",
                                                    "{start_str} → {end_str}"
                                                }
                                            }
                                        }

                                        view {
                                            style: "width: 100%; flex-direction: row; align-items: stretch; gap: 8px;",
                                            view {
                                                style: "width: 56px; flex-shrink: 0; flex-direction: column; justify-content: space-between; padding-vertical: 10px; align-items: flex-end;",
                                                text { style: "font-size: 10px; font-family: monospace; color: {theme.text.secondary.hex()};", "{format_val(max_pt, series.unit)}" }
                                                text { style: "font-size: 10px; font-family: monospace; color: {theme.text.secondary.hex()};", "{lo_label}" }
                                            }
                                            view {
                                                test_id: "bench-canvas-{series.id}",
                                                nve_style: line_canvas_resp.clone(),
                                                accessibility_label: "{series.title} progression chart",
                                                nve_table: nve_dioxus::table_attr_fn(move |tx, node| {
                                                    tx.edit::<nve_canvas::Canvas>(node)
                                                        .draw_list_ephemeral(draw_list.clone());
                                                }),
                                                style: "width: {line_canvas_w}px; max-width: 100%; flex-grow: 1; height: {line_canvas_h}px; border-radius: 10px; border-width: 1px; border-color: {theme.border.default.hex()};",
                                            }
                                        }

                                        view {
                                            style: "width: 100%; flex-direction: row; align-items: center; gap: 8px;",
                                            view { style: "width: 56px; flex-shrink: 0;" }
                                            view {
                                                style: "flex-grow: 1; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 14px;",
                                                for pt in series.points {
                                                    text {
                                                        key: "pt-{series.id}-{pt.label}",
                                                        style: "font-size: 10px; font-family: monospace; color: {theme.text.secondary.hex()};",
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
