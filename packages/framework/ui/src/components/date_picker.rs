//! Accessible, locale-aware [`DatePicker`] and [`DateRangePicker`] RSX components built
//! on `nve-i18n`, `nve_ui::Theme` tokens, WAI-ARIA dialog/grid semantics (`role="dialog"`,
//! `role="grid"`, `role="gridcell"`, `aria-selected`, `aria-disabled`), and `positioning.rs`
//! collision-flipping geometry.

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;
use nve_i18n::{CalendarDate, DateStyle, LocaleId, NameWidth};

use crate::components::date_picker_calendar::{
    build_trigger_styles, render_calendar_dialog, CalendarCallbacks, DEFAULT_FALLBACK_DATE,
};
pub use crate::components::date_picker_state::{
    add_days, add_months_clamped, add_years_clamped, civil_from_days, days_from_civil,
    days_in_month, end_of_week, start_of_week, CalendarCell, CalendarMonthGrid, DateConstraints,
    DatePickerAction, DatePickerState, DateRange, DateRangeAction, DateRangePickerState,
    DateSegment, DateSegmentInput, DateSegmentOrder, DisabledDatePredicate, HourCycle,
    RangeEndpoint, SegmentKeyOutcome, TimeOfDay,
};
use crate::components::overlays::resolve_anchored_position;
use crate::positioning::{Placement, PositionOptions, Rect, Size, Viewport};
use crate::theme::use_theme;

/// Accessible locale-aware single-date picker with segmented keyboard typing, localized
/// 6×7 calendar grid (`role="dialog"`, `role="grid"`, `role="gridcell"`, `aria-selected`,
/// `aria-disabled`), and optional `compute_overlay_position` collision-flipping geometry.
#[component]
pub fn DatePicker(
    #[props(default)] value: Option<CalendarDate>,
    #[props(default)] locale: LocaleId,
    #[props(default)] today: Option<CalendarDate>,
    #[props(default)] min_date: Option<CalendarDate>,
    #[props(default)] max_date: Option<CalendarDate>,
    #[props(default)] disabled_dates: Option<DisabledDatePredicate>,
    #[props(default)] open: Option<bool>,
    #[props(default)] default_open: bool,
    #[props(default)] disabled: bool,
    #[props(default)] label: Option<String>,
    #[props(default = DateStyle::Medium)] date_style: DateStyle,
    #[props(default = NameWidth::Abbreviated)] weekday_width: NameWidth,
    #[props(default)] anchor_rect: Option<Rect>,
    #[props(default)] overlay_size: Option<Size>,
    #[props(default)] viewport: Option<Viewport>,
    #[props(default)] position_options: Option<PositionOptions>,
    #[props(default)] on_change: Option<EventHandler<Option<CalendarDate>>>,
    #[props(default)] on_open_change: Option<EventHandler<bool>>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
) -> Element {
    let theme = use_theme();
    let mut trigger_focused = use_signal(|| false);
    let constraints = DateConstraints {
        min_date,
        max_date,
        disabled_predicate: disabled_dates,
    };
    let anchor_date = value.or(today).unwrap_or(DEFAULT_FALLBACK_DATE);
    let mut state = use_signal(|| {
        DatePickerState::new(locale.clone(), anchor_date)
            .with_today(today)
            .with_constraints(constraints.clone())
            .with_selected(value)
            .with_open(open.unwrap_or(default_open))
    });

    {
        let mut s = state.write_unchecked();
        if s.locale() != &locale {
            s.set_locale(locale.clone());
        }
        if let Some(ctrl_open) = open {
            if ctrl_open && !s.is_open() {
                s.open();
            } else if !ctrl_open && s.is_open() {
                s.close();
            }
        }
    }

    let base_id = test_id.as_deref().unwrap_or("nve-date-picker");
    let trigger_id = format!("{base_id}-trigger");
    let dialog_id = format!("{base_id}-dialog");
    let label_id = label.as_ref().map(|_| format!("{base_id}-label"));

    let (is_open, grid, display_text, has_value, seg_value) = {
        let s = state.read();
        let formatted = s.formatted_selected(date_style);
        let segs = s.segments().formatted_value();
        let has_val = s.selected().is_some();
        (s.is_open(), s.grid(), formatted.unwrap_or_else(|| segs.clone()), has_val, segs)
    };

    let default_opts =
        position_options.unwrap_or_else(|| PositionOptions::new(Placement::BottomStart));
    let computed = resolve_anchored_position(
        anchor_rect,
        overlay_size,
        viewport,
        Some(default_opts),
        Size::new(300.0, 320.0),
    );
    let (wrap_style, label_style, trigger_style, trigger_text_style) =
        build_trigger_styles(&theme, trigger_focused(), disabled, has_value);

    let aria_expanded = if is_open { "true" } else { "false" };
    let aria_dis = if disabled { "true" } else { "false" };
    let cbs = CalendarCallbacks {
        on_prev: EventHandler::new(move |_| state.write().prev_month()),
        on_next: EventHandler::new(move |_| state.write().next_month()),
        on_select: EventHandler::new(move |d: CalendarDate| {
            if let DatePickerAction::Selected(sel) = state.write().select_date(d) {
                if let Some(h) = on_change {
                    h.call(Some(sel));
                }
                if let Some(h) = on_open_change {
                    h.call(false);
                }
            }
        }),
        on_grid_key: EventHandler::new(move |(key, shift): (String, bool)| {
            match state.write().handle_grid_key(&key, shift) {
                DatePickerAction::Selected(sel) => {
                    if let Some(h) = on_change {
                        h.call(Some(sel));
                    }
                    if let Some(h) = on_open_change {
                        h.call(false);
                    }
                }
                DatePickerAction::Closed => {
                    if let Some(h) = on_open_change {
                        h.call(false);
                    }
                }
                _ => {}
            }
        }),
    };

    rsx! {
        view {
            test_id: "{base_id}",
            nve_style: wrap_style,
            style: style,
            if let Some(ref lbl) = label {
                text { test_id: label_id.clone(), nve_style: label_style, "{lbl}" }
            }
            pressable {
                test_id: "{trigger_id}",
                accessibility_role: "button",
                aria_expanded: "{aria_expanded}",
                aria_controls: "{dialog_id}",
                aria_labelledby: label_id,
                aria_disabled: "{aria_dis}",
                disabled: disabled,
                tab_index: if disabled { -1 } else { 0 },
                nve_table: super::html_tables::html_data_attrs(vec![("segments".to_owned(), seg_value.clone())]),
                nve_style: trigger_style,
                onfocus: move |_| trigger_focused.set(true),
                onblur: move |_| trigger_focused.set(false),
                onpress: move |_| {
                    if !disabled {
                        let act = state.write().toggle();
                        if let Some(h) = on_open_change {
                            h.call(matches!(act, DatePickerAction::Opened));
                        }
                    }
                },
                onkeydown: move |ev: KeyEvent| {
                    if disabled {
                        return;
                    }
                    let d = ev.data();
                    if matches!(d.key.as_str(), "Enter" | " " | "Space") {
                        let act = state.write().toggle();
                        if let Some(h) = on_open_change {
                            h.call(matches!(act, DatePickerAction::Opened));
                        }
                    } else if matches!(d.key.as_str(), "Escape" | "Esc") {
                        if state.write().close() == DatePickerAction::Closed
                            && let Some(h) = on_open_change
                        {
                            h.call(false);
                        }
                    } else if let DatePickerAction::Selected(sel) =
                        state.write().handle_segment_key(&d.key, d.shift)
                        && let Some(h) = on_change
                    {
                        h.call(Some(sel));
                    }
                },
                text { test_id: format!("{base_id}-value"), nve_style: trigger_text_style, "{display_text}" }
            }
            if is_open {
                {render_calendar_dialog(base_id, &grid, weekday_width, computed, &theme, disabled, cbs)}
            }
        }
    }
}

/// Accessible locale-aware date-range picker with anchor/hover/normalized-range selection,
/// localized 6×7 calendar grid (`role="dialog"`, `role="grid"`, `role="gridcell"`,
/// `aria-selected`, `aria-disabled`), and optional `compute_overlay_position` geometry.
#[component]
pub fn DateRangePicker(
    #[props(default)] value: Option<DateRange>,
    #[props(default)] locale: LocaleId,
    #[props(default)] today: Option<CalendarDate>,
    #[props(default)] min_date: Option<CalendarDate>,
    #[props(default)] max_date: Option<CalendarDate>,
    #[props(default)] disabled_dates: Option<DisabledDatePredicate>,
    #[props(default)] open: Option<bool>,
    #[props(default)] default_open: bool,
    #[props(default)] disabled: bool,
    #[props(default)] label: Option<String>,
    #[props(default = DateStyle::Medium)] date_style: DateStyle,
    #[props(default = NameWidth::Abbreviated)] weekday_width: NameWidth,
    #[props(default)] anchor_rect: Option<Rect>,
    #[props(default)] overlay_size: Option<Size>,
    #[props(default)] viewport: Option<Viewport>,
    #[props(default)] position_options: Option<PositionOptions>,
    #[props(default)] on_change: Option<EventHandler<Option<DateRange>>>,
    #[props(default)] on_open_change: Option<EventHandler<bool>>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
) -> Element {
    let theme = use_theme();
    let mut trigger_focused = use_signal(|| false);
    let constraints = DateConstraints {
        min_date,
        max_date,
        disabled_predicate: disabled_dates,
    };
    let anchor_date = value
        .map(|r| r.normalized().start)
        .or(today)
        .unwrap_or(DEFAULT_FALLBACK_DATE);
    let mut state = use_signal(|| {
        DateRangePickerState::new(locale.clone(), anchor_date)
            .with_today(today)
            .with_constraints(constraints.clone())
            .with_selected(value)
            .with_open(open.unwrap_or(default_open))
    });

    {
        let mut s = state.write_unchecked();
        if s.locale() != &locale {
            s.set_locale(locale.clone());
        }
        if let Some(ctrl_open) = open {
            if ctrl_open && !s.is_open() {
                s.open();
            } else if !ctrl_open && s.is_open() {
                s.close();
            }
        }
    }

    let base_id = test_id.as_deref().unwrap_or("nve-date-range-picker");
    let trigger_id = format!("{base_id}-trigger");
    let dialog_id = format!("{base_id}-dialog");
    let label_id = label.as_ref().map(|_| format!("{base_id}-label"));

    let (is_open, grid, display_text, has_value) = {
        let s = state.read();
        let placeholder = format!(
            "{} – {}",
            DateSegmentOrder::placeholder_for(s.locale()),
            DateSegmentOrder::placeholder_for(s.locale())
        );
        let formatted = s.formatted_selected(date_style);
        let has_val = s.selected().is_some();
        (s.is_open(), s.grid(), formatted.unwrap_or(placeholder), has_val)
    };

    let default_opts =
        position_options.unwrap_or_else(|| PositionOptions::new(Placement::BottomStart));
    let computed = resolve_anchored_position(
        anchor_rect,
        overlay_size,
        viewport,
        Some(default_opts),
        Size::new(300.0, 320.0),
    );
    let (wrap_style, label_style, trigger_style, trigger_text_style) =
        build_trigger_styles(&theme, trigger_focused(), disabled, has_value);

    let aria_expanded = if is_open { "true" } else { "false" };
    let aria_dis = if disabled { "true" } else { "false" };
    let cbs = CalendarCallbacks {
        on_prev: EventHandler::new(move |_| state.write().prev_month()),
        on_next: EventHandler::new(move |_| state.write().next_month()),
        on_select: EventHandler::new(move |d: CalendarDate| {
            if let DateRangeAction::Selected(range) = state.write().select_date(d) {
                if let Some(h) = on_change {
                    h.call(Some(range));
                }
                if let Some(h) = on_open_change {
                    h.call(false);
                }
            }
        }),
        on_grid_key: EventHandler::new(move |(key, shift): (String, bool)| {
            match state.write().handle_grid_key(&key, shift) {
                DateRangeAction::Selected(range) => {
                    if let Some(h) = on_change {
                        h.call(Some(range));
                    }
                    if let Some(h) = on_open_change {
                        h.call(false);
                    }
                }
                DateRangeAction::Closed => {
                    if let Some(h) = on_open_change {
                        h.call(false);
                    }
                }
                _ => {}
            }
        }),
    };

    rsx! {
        view {
            test_id: "{base_id}",
            nve_style: wrap_style,
            style: style,
            if let Some(ref lbl) = label {
                text { test_id: label_id.clone(), nve_style: label_style, "{lbl}" }
            }
            pressable {
                test_id: "{trigger_id}",
                accessibility_role: "button",
                aria_expanded: "{aria_expanded}",
                aria_controls: "{dialog_id}",
                aria_labelledby: label_id,
                aria_disabled: "{aria_dis}",
                disabled: disabled,
                tab_index: if disabled { -1 } else { 0 },
                nve_style: trigger_style,
                onfocus: move |_| trigger_focused.set(true),
                onblur: move |_| trigger_focused.set(false),
                onpress: move |_| {
                    if !disabled {
                        let act = state.write().toggle();
                        if let Some(h) = on_open_change {
                            h.call(matches!(act, DateRangeAction::Opened));
                        }
                    }
                },
                onkeydown: move |ev: KeyEvent| {
                    if disabled {
                        return;
                    }
                    let d = ev.data();
                    if matches!(d.key.as_str(), "Enter" | " " | "Space") {
                        let act = state.write().toggle();
                        if let Some(h) = on_open_change {
                            h.call(matches!(act, DateRangeAction::Opened));
                        }
                    } else if matches!(d.key.as_str(), "Escape" | "Esc")
                        && state.write().close() == DateRangeAction::Closed
                        && let Some(h) = on_open_change
                    {
                        h.call(false);
                    }
                },
                text { test_id: format!("{base_id}-value"), nve_style: trigger_text_style, "{display_text}" }
            }
            if is_open {
                {render_calendar_dialog(base_id, &grid, weekday_width, computed, &theme, disabled, cbs)}
            }
        }
    }
}
