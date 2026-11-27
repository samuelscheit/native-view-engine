//! Shared calendar dialog and trigger styling helpers for [`crate::DatePicker`] and
//! [`crate::DateRangePicker`].

use dioxus_core::EventHandler;
use nve_abi::{FlexAlign, FlexJustify};
use nve_dioxus::prelude::*;
use nve_i18n::{CalendarDate, NameWidth};

use crate::components::date_picker_grid::CalendarMonthGrid;
use crate::positioning::ComputedPosition;
use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle, TypedTextStyle};
use crate::components::roles::{calendar_day_colors, ControlColors};
use crate::theme::{Layer, Theme};

pub(crate) const DEFAULT_FALLBACK_DATE: CalendarDate = CalendarDate {
    year: 2026,
    month: 1,
    day: 1,
};

#[inline]
fn iso_date(d: CalendarDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

#[derive(Clone)]
pub(crate) struct CalendarCallbacks {
    pub on_prev: EventHandler<()>,
    pub on_next: EventHandler<()>,
    pub on_select: EventHandler<CalendarDate>,
    pub on_grid_key: EventHandler<(String, bool)>,
}

/// Shared accessible calendar dialog renderer (`role="dialog"`, localized header, prev/next
/// controls, localized weekday headers ordered by `first_day_of_week`, 6×7 `role="grid"` /
/// `role="gridcell"`, and optional `compute_overlay_position` collision-flipping geometry).
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_calendar_dialog(
    base_id: &str,
    grid: &CalendarMonthGrid,
    weekday_width: NameWidth,
    computed: Option<ComputedPosition>,
    theme: &Theme,
    disabled: bool,
    cb: CalendarCallbacks,
) -> Element {
    let dialog_id = format!("{base_id}-dialog");
    let month_label_id = format!("{base_id}-month-label");
    let grid_id = format!("{base_id}-grid");
    let month_title = grid.month_title(NameWidth::Wide);
    let weekday_labels = grid.weekday_labels(weekday_width);

    let mut dialog_builder = TypedStyleBuilder::new()
        .flex_col()
        .padding(theme.spacing.md)
        .gap(theme.spacing.sm)
        .min_width_px(280.0)
        .z_index(Layer::Popover.z())
        .surface(TypedSurfaceStyle {
            background: theme.surface.overlay,
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border_color: theme.border.default,
            border_radius: theme.radius.lg,
            opacity: 1.0,
            box_shadow: theme.elevation.overlay,
        });
    if let Some(pos) = computed {
        if let Some(mw) = pos.max_width {
            dialog_builder = dialog_builder.max_width_px(mw);
        }
        if let Some(mh) = pos.max_height {
            dialog_builder = dialog_builder.max_height_px(mh);
        }
    }
    let dialog_style = dialog_builder.finish();

    let header_style = TypedStyleBuilder::new()
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Center)
        .gap(theme.spacing.sm)
        .finish();
    let nav_btn_style = TypedStyleBuilder::new()
        .width_px(30.0)
        .height_px(30.0)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .surface(TypedSurfaceStyle {
            background: theme.surface.base,
            border_width: 1.0,
            border_color: theme.border.default,
            border_radius: theme.radius.sm,
            opacity: if disabled { 0.45 } else { 1.0 },
            box_shadow: "none",
        })
        .finish();
    let nav_text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: theme.text.primary,
            font_size: theme.typography.ui.size,
            font_weight: "600",
            text_align: "center",
            opacity: 1.0,
        })
        .finish();
    let title_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: theme.text.primary,
            font_size: theme.typography.body.size,
            font_weight: "600",
            text_align: "center",
            opacity: 1.0,
        })
        .finish();
    let grid_style = TypedStyleBuilder::new().flex_col().gap(theme.spacing.xs).finish();
    let row_style = TypedStyleBuilder::new()
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .gap(2.0)
        .finish();
    let weekday_cell_style = TypedStyleBuilder::new()
        .width_px(34.0)
        .height_px(24.0)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .finish();
    let weekday_text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: theme.text.tertiary,
            font_size: theme.typography.caption.size,
            font_weight: "600",
            text_align: "center",
            opacity: 1.0,
        })
        .finish();

    let rendered_weekdays: Vec<Element> = weekday_labels
        .iter()
        .enumerate()
        .map(|(idx, &lbl)| {
            let tid = format!("{base_id}-weekday-{idx}");
            rsx! {
                view {
                    key: "{idx}",
                    test_id: "{tid}",
                    accessibility_role: "columnheader",
                    nve_style: weekday_cell_style.clone(),
                    text { nve_style: weekday_text_style.clone(), "{lbl}" }
                }
            }
        })
        .collect();

    let rendered_weeks: Vec<Element> = grid
        .weeks()
        .enumerate()
        .map(|(week_idx, week)| {
            let week_tid = format!("{base_id}-week-{week_idx}");
            let rendered_cells: Vec<Element> = week
                .iter()
                .map(|cell| {
                    let iso = iso_date(cell.date);
                    let cell_tid = format!("{base_id}-cell-{iso}");
                    let day_num = cell.date.day;
                    let cell_date = cell.date;
                    let cell_disabled = disabled || cell.is_disabled;
                    let is_endpoint = cell.is_selected || cell.is_range_start || cell.is_range_end;
                    let ControlColors { bg, fg, border_width: border_w, border: border_col } = calendar_day_colors(
                        theme,
                        is_endpoint,
                        cell.is_in_range,
                        cell.is_today,
                        cell.is_focused,
                        cell_disabled || !cell.in_current_month,
                    );
                    let cell_style = TypedStyleBuilder::new()
                        .width_px(34.0)
                        .height_px(34.0)
                        .justify_content(FlexJustify::Center)
                        .align_items(FlexAlign::Center)
                        .surface(TypedSurfaceStyle {
                            background: bg,
                            border_width: border_w,
                            border_color: border_col,
                            border_radius: theme.radius.sm,
                            opacity: if cell_disabled {
                                0.35
                            } else if !cell.in_current_month {
                                0.55
                            } else {
                                1.0
                            },
                            box_shadow: "none",
                        })
                        .finish();
                    let cell_text_style = TypedStyleBuilder::new()
                        .text_style(TypedTextStyle {
                            color: fg,
                            font_size: theme.typography.ui.size,
                            font_weight: if is_endpoint || cell.is_today { "600" } else { "400" },
                            text_align: "center",
                            opacity: 1.0,
                        })
                        .finish();

                    let aria_sel = if is_endpoint { "true" } else { "false" };
                    let aria_dis = if cell_disabled { "true" } else { "false" };
                    let in_month_str = if cell.in_current_month { "true" } else { "false" };
                    let today_str = if cell.is_today { "true" } else { "false" };
                    let in_range_str = if cell.is_in_range { "true" } else { "false" };
                    let range_start_str = if cell.is_range_start { "true" } else { "false" };
                    let range_end_str = if cell.is_range_end { "true" } else { "false" };
                    let focused_str = if cell.is_focused { "true" } else { "false" };
                    let on_select = cb.on_select;
                    let on_grid_key = cb.on_grid_key;
                    let cell_data = super::html_tables::html_data_attrs(vec![
                        ("date".to_owned(), iso.clone()),
                        ("in-month".to_owned(), in_month_str.to_owned()),
                        ("today".to_owned(), today_str.to_owned()),
                        ("in-range".to_owned(), in_range_str.to_owned()),
                        ("range-start".to_owned(), range_start_str.to_owned()),
                        ("range-end".to_owned(), range_end_str.to_owned()),
                        ("focused".to_owned(), focused_str.to_owned()),
                    ]);

                    rsx! {
                        pressable {
                            key: "{iso}",
                            test_id: "{cell_tid}",
                            accessibility_role: "gridcell",
                            accessibility_label: "{iso}",
                            aria_selected: "{aria_sel}",
                            aria_disabled: "{aria_dis}",
                            disabled: cell_disabled,
                            tab_index: if cell.is_focused && !cell_disabled { 0 } else { -1 },
                            nve_table: cell_data,
                            nve_style: cell_style,
                            onpress: move |_| {
                                if !cell_disabled {
                                    on_select.call(cell_date);
                                }
                            },
                            onkeydown: move |ev: KeyEvent| {
                                if !disabled {
                                    let d = ev.data();
                                    on_grid_key.call((d.key.clone(), d.shift));
                                }
                            },
                            text { nve_style: cell_text_style, "{day_num}" }
                        }
                    }
                })
                .collect();

            rsx! {
                view {
                    key: "{week_idx}",
                    test_id: "{week_tid}",
                    accessibility_role: "row",
                    nve_style: row_style.clone(),
                    {rendered_cells.into_iter()}
                }
            }
        })
        .collect();

    let pos_placement = computed.map(|p| p.placement.as_str().to_owned());
    let pos_x = computed.map(|p| format!("{:.2}", p.x));
    let pos_y = computed.map(|p| format!("{:.2}", p.y));
    let pos_flipped = computed.map(|p| if p.flipped { "true" } else { "false" }.to_owned());
    let pos_shifted = computed.map(|p| if p.shifted { "true" } else { "false" }.to_owned());
    let pos_arrow_x = computed.and_then(|p| p.arrow_x).map(|v| format!("{v:.2}"));
    let pos_arrow_y = computed.and_then(|p| p.arrow_y).map(|v| format!("{v:.2}"));
    let on_prev = cb.on_prev;
    let on_next = cb.on_next;
    let on_grid_key = cb.on_grid_key;
    let mut cal_data = Vec::new();
    if let Some(ref p) = pos_placement { cal_data.push(("placement".to_owned(), p.clone())); }
    if let Some(ref x) = pos_x { cal_data.push(("x".to_owned(), x.clone())); }
    if let Some(ref y) = pos_y { cal_data.push(("y".to_owned(), y.clone())); }
    if let Some(ref f) = pos_flipped { cal_data.push(("flipped".to_owned(), f.clone())); }
    if let Some(ref s) = pos_shifted { cal_data.push(("shifted".to_owned(), s.clone())); }
    if let Some(ref ax) = pos_arrow_x { cal_data.push(("arrow-x".to_owned(), ax.clone())); }
    if let Some(ref ay) = pos_arrow_y { cal_data.push(("arrow-y".to_owned(), ay.clone())); }
    let cal_table = super::html_tables::html_data_attrs(cal_data);

    rsx! {
        view {
            test_id: "{dialog_id}",
            accessibility_role: "dialog",
            aria_labelledby: "{month_label_id}",
            tab_index: -1,
            nve_table: cal_table,
            nve_style: dialog_style,
            view {
                test_id: format!("{base_id}-header"),
                nve_style: header_style,
                pressable {
                    test_id: format!("{base_id}-prev-month"),
                    accessibility_role: "button",
                    accessibility_label: "Previous month",
                    disabled: disabled,
                    tab_index: if disabled { -1 } else { 0 },
                    nve_style: nav_btn_style.clone(),
                    onpress: move |_| if !disabled { on_prev.call(()); },
                    text { nve_style: nav_text_style.clone(), "<" }
                }
                text {
                    test_id: "{month_label_id}",
                    aria_live: "polite",
                    nve_style: title_style,
                    "{month_title}"
                }
                pressable {
                    test_id: format!("{base_id}-next-month"),
                    accessibility_role: "button",
                    accessibility_label: "Next month",
                    disabled: disabled,
                    tab_index: if disabled { -1 } else { 0 },
                    nve_style: nav_btn_style,
                    onpress: move |_| if !disabled { on_next.call(()); },
                    text { nve_style: nav_text_style, ">" }
                }
            }
            view {
                test_id: "{grid_id}",
                accessibility_role: "grid",
                aria_labelledby: "{month_label_id}",
                tab_index: 0,
                nve_style: grid_style,
                onkeydown: move |ev: KeyEvent| {
                    if !disabled {
                        let d = ev.data();
                        on_grid_key.call((d.key.clone(), d.shift));
                    }
                },
                view {
                    test_id: format!("{base_id}-weekdays"),
                    accessibility_role: "row",
                    nve_style: row_style,
                    {rendered_weekdays.into_iter()}
                }
                {rendered_weeks.into_iter()}
            }
        }
    }
}

pub(crate) fn build_trigger_styles(
    theme: &Theme,
    focused: bool,
    disabled: bool,
    has_value: bool,
) -> (TypedStyle, TypedStyle, TypedStyle, TypedStyle) {
    let wrap_style = TypedStyleBuilder::new().flex_col().gap(theme.spacing.xs).finish();
    let label_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: theme.text.primary,
            font_size: theme.typography.ui.size,
            font_weight: "600",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();
    let trigger_style = TypedStyleBuilder::new()
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Center)
        .min_height_px(38.0)
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .surface(TypedSurfaceStyle {
            background: if disabled { theme.surface.subtle } else { theme.surface.base },
            border_width: if focused && !disabled { 2.0 } else { 1.0 },
            border_color: if focused && !disabled { theme.border.focus } else { theme.border.strong },
            border_radius: theme.radius.md,
            opacity: if disabled { 0.55 } else { 1.0 },
            box_shadow: "none",
        })
        .finish();
    let trigger_text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: if has_value { theme.text.primary } else { theme.text.tertiary },
            font_size: theme.typography.body.size,
            font_weight: "400",
            text_align: "left",
            opacity: if disabled { 0.55 } else { 1.0 },
        })
        .finish();
    (wrap_style, label_style, trigger_style, trigger_text_style)
}
