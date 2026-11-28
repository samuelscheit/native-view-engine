//! Deep edge-case state machine and `HeadlessBackend` E2E tests for locale-aware
//! `DatePicker`, `DateRangePicker`, `CalendarMonthGrid`, `DateSegmentInput`, and `TimeOfDay`.

mod common;

use common::UiHarness;
use nve_dioxus::prelude::*;
use nve_i18n::{CalendarDate, DateStyle, LocaleId, NameWidth, Weekday};
use nve_ui::prelude::*;

fn d(year: i32, month: u8, day: u8) -> CalendarDate {
    CalendarDate::new(year, month, day).expect("valid test date")
}

fn loc(tag: &str) -> LocaleId {
    LocaleId::parse(tag).expect("valid locale")
}

// ---------------------------------------------------------------------------
// 1. Civil Date Math, Leap Years & Century Rules (Zero DST / UTC Drift)
// ---------------------------------------------------------------------------

#[test]
fn civil_date_leap_years_century_rules_and_clamping() {
    // 2024 leap, 2025 non-leap, 2000 century leap (div 400), 1900 century non-leap (div 100).
    assert!(CalendarDate::new(2024, 2, 29).is_ok());
    assert!(CalendarDate::new(2025, 2, 29).is_err());
    assert!(CalendarDate::new(2000, 2, 29).is_ok());
    assert!(CalendarDate::new(1900, 2, 29).is_err());

    assert_eq!(days_in_month(2024, 2), 29);
    assert_eq!(days_in_month(2025, 2), 28);
    assert_eq!(days_in_month(2000, 2), 29);
    assert_eq!(days_in_month(1900, 2), 28);

    // Month & year clamping across leap days and 31-day months.
    assert_eq!(add_months_clamped(d(2024, 1, 31), 1), d(2024, 2, 29));
    assert_eq!(add_months_clamped(d(2025, 1, 31), 1), d(2025, 2, 28));
    assert_eq!(add_years_clamped(d(2024, 2, 29), 1), d(2025, 2, 28));
    assert_eq!(add_years_clamped(d(2024, 2, 29), 4), d(2028, 2, 29));
    assert_eq!(add_years_clamped(d(2000, 2, 29), -100), d(1900, 2, 28));
    assert_eq!(add_months_clamped(d(2024, 1, 15), -1), d(2023, 12, 15));
    assert_eq!(add_months_clamped(d(2024, 12, 31), 1), d(2025, 1, 31));

    // Exact day-by-day monotonicity across leap boundaries & DST transition dates.
    let mut cur = d(1900, 2, 27);
    let expected_1900 = [d(1900, 2, 27), d(1900, 2, 28), d(1900, 3, 1)];
    for exp in expected_1900 {
        assert_eq!(cur, exp);
        assert_eq!(civil_from_days(days_from_civil(cur)), cur);
        cur = add_days(cur, 1);
    }
    assert_eq!(add_days(d(2024, 2, 28), 1), d(2024, 2, 29));
    assert_eq!(add_days(d(2024, 2, 29), 1), d(2024, 3, 1));
    // Typical US/EU DST dates: purely civil arithmetic never skips or duplicates a day.
    assert_eq!(add_days(d(2024, 3, 10), 1), d(2024, 3, 11));
    assert_eq!(add_days(d(2024, 11, 3), 1), d(2024, 11, 4));
}

// ---------------------------------------------------------------------------
// 2. Localized Week Starts & 6x7 Calendar Grids
// ---------------------------------------------------------------------------

#[test]
fn calendar_grid_respects_cldr_first_day_of_week_and_localized_labels() {
    let today = d(2024, 2, 15);
    let constraints = DateConstraints::default();

    // en-US: Sunday start. Feb 1, 2024 is Thursday -> 4 leading Jan days (Jan 28..31).
    let us = CalendarMonthGrid::build(2024, 2, &loc("en-US"), Some(today), today, Some(today), None, &constraints);
    assert_eq!(us.first_day_of_week, Weekday::Sunday);
    assert_eq!(us.cells.len(), 42);
    assert_eq!(us.cells[0].date, d(2024, 1, 28));
    assert!(!us.cells[0].in_current_month);
    assert_eq!(us.cells[4].date, d(2024, 2, 1));
    assert!(us.cells[4].in_current_month);
    assert_eq!(us.cells[4 + 28].date, d(2024, 2, 29)); // leap day!
    assert_eq!(us.cells[4 + 29].date, d(2024, 3, 1));
    assert!(!us.cells[4 + 29].in_current_month);
    assert_eq!(us.month_title(NameWidth::Wide), "February 2024");
    assert_eq!(us.weekday_labels(NameWidth::Abbreviated)[0], "Sun");
    assert_eq!(us.weekday_labels(NameWidth::Abbreviated)[6], "Sat");

    // de-DE: Monday start. Feb 1, 2024 is Thursday -> 3 leading Jan days (Jan 29..31).
    let de = CalendarMonthGrid::build(2024, 2, &loc("de-DE"), Some(today), today, None, None, &constraints);
    assert_eq!(de.first_day_of_week, Weekday::Monday);
    assert_eq!(de.cells[0].date, d(2024, 1, 29));
    assert_eq!(de.cells[3].date, d(2024, 2, 1));
    assert_eq!(de.month_title(NameWidth::Wide), "Februar 2024");
    assert_eq!(de.weekday_labels(NameWidth::Abbreviated)[0], "Mo.");
    assert_eq!(de.weekday_labels(NameWidth::Abbreviated)[6], "So.");

    // ar-EG: Saturday start. Feb 1, 2024 is Thursday -> 5 leading Jan days (Jan 27..31).
    let ar = CalendarMonthGrid::build(2024, 2, &loc("ar-EG"), Some(today), today, None, None, &constraints);
    assert_eq!(ar.first_day_of_week, Weekday::Saturday);
    assert_eq!(ar.cells[0].date, d(2024, 1, 27));
    assert_eq!(ar.cells[5].date, d(2024, 2, 1));
    assert_eq!(ar.month_title(NameWidth::Wide), "فبراير 2024");
    assert_eq!(ar.weekday_labels(NameWidth::Abbreviated)[0], "السبت");

    // ja-JP: Sunday start, localized CJK header.
    let ja = CalendarMonthGrid::build(2024, 2, &loc("ja-JP"), Some(today), today, None, None, &constraints);
    assert_eq!(ja.first_day_of_week, Weekday::Sunday);
    assert_eq!(ja.month_title(NameWidth::Wide), "2024年 2月");
}

// ---------------------------------------------------------------------------
// 3. Localized Segment Order & Keyboard Segment Typing
// ---------------------------------------------------------------------------

#[test]
fn segment_order_and_keyboard_typing_with_leap_year_clamping() {
    assert_eq!(DateSegmentOrder::from_locale(&loc("en-US")), DateSegmentOrder::Mdy);
    assert_eq!(DateSegmentOrder::placeholder_for(&loc("en-US")), "MM/DD/YYYY");

    for tag in ["de-DE", "fr-FR", "en-GB", "es", "it", "pt", "pl", "ru"] {
        assert_eq!(
            DateSegmentOrder::from_locale(&loc(tag)),
            DateSegmentOrder::Dmy,
            "expected Dmy for {tag}"
        );
    }
    assert_eq!(DateSegmentOrder::placeholder_for(&loc("de-DE")), "DD.MM.YYYY");
    assert_eq!(DateSegmentOrder::placeholder_for(&loc("fr-FR")), "DD/MM/YYYY");

    for tag in ["ja-JP", "zh-CN", "ko-KR", "sv-SE"] {
        assert_eq!(
            DateSegmentOrder::from_locale(&loc(tag)),
            DateSegmentOrder::Ymd,
            "expected Ymd for {tag}"
        );
    }
    assert_eq!(DateSegmentOrder::placeholder_for(&loc("ja-JP")), "YYYY-MM-DD");
    assert_eq!(DateSegmentOrder::placeholder_for(&loc("sv-SE")), "YYYY-MM-DD");

    // en-US typing 02/29/2024 commits valid leap day; ArrowUp on Year clamps 2025-02-29 -> 2025-02-28.
    let mut us = DateSegmentInput::new(loc("en-US"));
    assert_eq!(us.active_segment(), DateSegment::Month);
    for ch in ["0", "2", "2", "9", "2", "0", "2", "4"] {
        us.handle_key(ch, false);
    }
    assert_eq!(us.date(), Some(d(2024, 2, 29)));
    assert_eq!(us.formatted_value(), "02/29/2024");
    assert_eq!(us.active_segment(), DateSegment::Year);
    us.handle_key("ArrowUp", false);
    assert_eq!(us.date(), Some(d(2025, 2, 28)));
    assert_eq!(us.formatted_value(), "02/28/2025");

    // de-DE typing 29.02.1900 clamps century non-leap year to 28.02.1900; 29.02.2000 stays 29!
    let mut de_1900 = DateSegmentInput::new(loc("de-DE"));
    for ch in ["2", "9", "0", "2", "1", "9", "0", "0"] {
        de_1900.handle_key(ch, false);
    }
    assert_eq!(de_1900.date(), Some(d(1900, 2, 28)));
    assert_eq!(de_1900.formatted_value(), "28.02.1900");

    let mut de_2000 = DateSegmentInput::new(loc("de-DE"));
    for ch in ["2", "9", "0", "2", "2", "0", "0", "0"] {
        de_2000.handle_key(ch, false);
    }
    assert_eq!(de_2000.date(), Some(d(2000, 2, 29)));
    assert_eq!(de_2000.formatted_value(), "29.02.2000");

    // ja-JP Ymd typing 2025-02-29 clamps day to 28 immediately since year is known!
    let mut ja = DateSegmentInput::new(loc("ja-JP"));
    for ch in ["2", "0", "2", "5", "0", "2", "2", "9"] {
        ja.handle_key(ch, false);
    }
    assert_eq!(ja.date(), Some(d(2025, 2, 28)));
    assert_eq!(ja.formatted_value(), "2025-02-28");

    // Tab / Shift+Tab navigation and FocusExit boundaries + Backspace clearing.
    let mut nav = DateSegmentInput::new(loc("en-US")).with_date(Some(d(2024, 3, 31)));
    assert_eq!(nav.handle_key("Tab", true), SegmentKeyOutcome::FocusExitPrev);
    assert_eq!(nav.handle_key("ArrowDown", false), SegmentKeyOutcome::ValueUpdated(Some(d(2024, 2, 29))));
    assert_eq!(nav.handle_key("Tab", false), SegmentKeyOutcome::SegmentMoved(DateSegment::Day));
    assert_eq!(nav.handle_key("Tab", false), SegmentKeyOutcome::SegmentMoved(DateSegment::Year));
    assert_eq!(nav.handle_key("Tab", false), SegmentKeyOutcome::FocusExitNext);
    assert_eq!(nav.handle_key("Backspace", false), SegmentKeyOutcome::ValueUpdated(None));
    assert_eq!(nav.formatted_value(), "02/29/YYYY");
}

// ---------------------------------------------------------------------------
// 4. Keyboard Grid Navigation, Disabled Skipping & Min/Max Clamping
// ---------------------------------------------------------------------------

#[test]
fn keyboard_grid_navigation_crosses_boundaries_skips_disabled_and_clamps_bounds() {
    // Disable weekends (Saturday + Sunday) with bounds [2024-02-01, 2025-03-10].
    let mut state = DatePickerState::new(loc("de-DE"), d(2024, 2, 29))
        .with_min_date(d(2024, 2, 1))
        .with_max_date(d(2025, 3, 10))
        .with_disabled_predicate(|date| {
            matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday)
        })
        .with_open(true);

    // 2024-02-29 is Thursday -> ArrowRight -> 2024-03-01 (Friday, crosses leap-month boundary!).
    assert_eq!(state.handle_grid_key("ArrowRight", false), DatePickerAction::Focused(d(2024, 3, 1)));
    assert_eq!((state.view_year(), state.view_month()), (2024, 3));

    // ArrowRight from Friday 2024-03-01 skips disabled Sat Mar 2 & Sun Mar 3 -> Mon Mar 4!
    assert_eq!(state.handle_grid_key("ArrowRight", false), DatePickerAction::Focused(d(2024, 3, 4)));
    // ArrowLeft from Monday skips Sun & Sat back to Friday Mar 1!
    assert_eq!(state.handle_grid_key("ArrowLeft", false), DatePickerAction::Focused(d(2024, 3, 1)));

    // Home in de-DE (Monday start) moves from Friday Mar 1 to Monday Feb 26!
    assert_eq!(state.handle_grid_key("Home", false), DatePickerAction::Focused(d(2024, 2, 26)));
    // End in de-DE targets Sunday Mar 3 (disabled) -> scans inward to Friday Mar 1!
    assert_eq!(state.handle_grid_key("End", false), DatePickerAction::Focused(d(2024, 3, 1)));

    // Shift+PageDown (+1 year from 2024-02-29): first go back to Feb 29, then +1 year -> 2025-02-28 (Fri)!
    assert_eq!(state.handle_grid_key("ArrowLeft", false), DatePickerAction::Focused(d(2024, 2, 29)));
    assert_eq!(state.handle_grid_key("PageDown", true), DatePickerAction::Focused(d(2025, 2, 28)));

    // PageDown past max_date (2025-03-10) clamps cleanly to max_date (Monday 2025-03-10).
    assert_eq!(state.handle_grid_key("PageDown", false), DatePickerAction::Focused(d(2025, 3, 10)));
    // ArrowRight at max_date stays at max_date (never escapes bounds).
    assert_eq!(state.handle_grid_key("ArrowRight", false), DatePickerAction::None);
    assert_eq!(state.focused(), d(2025, 3, 10));

    // Disabled weekend cannot be selected directly.
    assert_eq!(state.select_date(d(2024, 3, 2)), DatePickerAction::None);
    assert_eq!(state.selected(), None);

    // Enter selects focused enabled date and closes dialog; Escape closes when open.
    assert_eq!(state.handle_grid_key("Enter", false), DatePickerAction::Selected(d(2025, 3, 10)));
    assert!(!state.is_open());
    state.open();
    assert_eq!(state.handle_grid_key("Escape", false), DatePickerAction::Closed);
    assert!(!state.is_open());
}

// ---------------------------------------------------------------------------
// 5. DateRange Normalization, Hover Preview & Reversed Selection
// ---------------------------------------------------------------------------

#[test]
fn date_range_reversed_normalization_hover_preview_and_keyboard_selection() {
    // Direct DateRange normalization even when end < start.
    let norm = DateRange::new(d(2024, 3, 10), d(2024, 3, 2));
    assert_eq!(norm.start, d(2024, 3, 2));
    assert_eq!(norm.end, d(2024, 3, 10));
    assert_eq!(norm.len_days(), 9);
    assert!(norm.contains(d(2024, 3, 5)));
    assert!(!norm.contains(d(2024, 3, 11)));

    let mut range = DateRangePickerState::new(loc("en-US"), d(2024, 3, 15)).with_open(true);

    // 1st click sets anchor and stays open.
    assert_eq!(range.select_date(d(2024, 3, 15)), DateRangeAction::AnchorSet(d(2024, 3, 15)));
    assert!(range.is_open());
    assert_eq!(range.selected(), None);

    // Hovering an EARLIER date (Mar 10) previews normalized [Mar 10, Mar 15] in grid!
    assert_eq!(range.hover_date(Some(d(2024, 3, 10))), DateRangeAction::Hovered(Some(d(2024, 3, 10))));
    assert_eq!(range.preview_range(), Some(DateRange::new(d(2024, 3, 10), d(2024, 3, 15))));
    let grid = range.grid();
    let cell_10 = grid.cells.iter().find(|c| c.date == d(2024, 3, 10)).unwrap();
    let cell_12 = grid.cells.iter().find(|c| c.date == d(2024, 3, 12)).unwrap();
    let cell_15 = grid.cells.iter().find(|c| c.date == d(2024, 3, 15)).unwrap();
    assert!(cell_10.is_range_start && cell_10.is_in_range && cell_10.is_selected);
    assert!(cell_12.is_in_range && !cell_12.is_range_start && !cell_12.is_range_end);
    assert!(cell_15.is_range_end && cell_15.is_in_range && cell_15.is_selected);

    // 2nd click on EARLIER date commits normalized [Mar 10, Mar 15] and closes dialog!
    assert_eq!(
        range.select_date(d(2024, 3, 10)),
        DateRangeAction::Selected(DateRange {
            start: d(2024, 3, 10),
            end: d(2024, 3, 15),
        })
    );
    assert!(!range.is_open());
    assert_eq!(range.anchor(), None);
    let expected_short = format!(
        "{} – {}",
        nve_i18n::format_date(&loc("en-US"), d(2024, 3, 10), DateStyle::Short),
        nve_i18n::format_date(&loc("en-US"), d(2024, 3, 15), DateStyle::Short),
    );
    assert_eq!(
        range.formatted_selected(DateStyle::Short).as_deref(),
        Some(expected_short.as_str())
    );
    assert_eq!(
        range.formatted_selected(DateStyle::Medium).as_deref(),
        Some("Mar 10, 2024 – Mar 15, 2024")
    );

    // Keyboard range selection + Escape cancels mid-selection anchor cleanly.
    range.open();
    assert_eq!(range.handle_grid_key("Enter", false), DateRangeAction::AnchorSet(d(2024, 3, 10)));
    assert_eq!(range.handle_grid_key("ArrowRight", false), DateRangeAction::Focused(d(2024, 3, 11)));
    assert_eq!(range.preview_range(), Some(DateRange::new(d(2024, 3, 10), d(2024, 3, 11))));
    assert_eq!(range.handle_grid_key("Escape", false), DateRangeAction::Closed);
    assert_eq!(range.anchor(), None);
    // Original committed range preserved on Escape!
    assert_eq!(range.selected(), Some(DateRange::new(d(2024, 3, 10), d(2024, 3, 15))));

    // Segmented typing with reversed endpoints normalizes cleanly.
    let mut seg_range = DateRangePickerState::new(loc("sv-SE"), d(2024, 6, 1));
    for ch in ["2", "0", "2", "4", "0", "6", "2", "0"] {
        seg_range.handle_segment_key(RangeEndpoint::Start, ch, false);
    }
    let mut last = DateRangeAction::None;
    for ch in ["2", "0", "2", "4", "0", "6", "0", "5"] {
        last = seg_range.handle_segment_key(RangeEndpoint::End, ch, false);
    }
    assert_eq!(last, DateRangeAction::Selected(DateRange::new(d(2024, 6, 5), d(2024, 6, 20))));
}

// ---------------------------------------------------------------------------
// 6. TimeOfDay (12h vs 24h by Locale)
// ---------------------------------------------------------------------------

#[test]
fn time_of_day_locale_12h_vs_24h_and_wrapping() {
    assert!(TimeOfDay::new(24, 0, 0).is_none());
    assert!(TimeOfDay::new(12, 60, 0).is_none());
    assert!(TimeOfDay::new(12, 0, 60).is_none());

    let midnight = TimeOfDay::new(0, 5, 9).unwrap();
    let afternoon = TimeOfDay::new(14, 30, 5).unwrap();
    let noon = TimeOfDay::new(12, 0, 0).unwrap();

    assert_eq!(HourCycle::from_locale(&loc("en-US")), HourCycle::H12);
    assert_eq!(HourCycle::from_locale(&loc("de-DE")), HourCycle::H24);
    assert_eq!(HourCycle::from_locale(&loc("ja-JP")), HourCycle::H24);

    assert_eq!(midnight.format_for_locale(&loc("en-US"), false), "12:05 AM");
    assert_eq!(noon.format_for_locale(&loc("en-US"), false), "12:00 PM");
    assert_eq!(afternoon.format_for_locale(&loc("en-US"), true), "2:30:05 PM");
    assert_eq!(afternoon.format_for_locale(&loc("de-DE"), false), "14:30");
    assert_eq!(afternoon.format_for_locale(&loc("ja-JP"), true), "14:30:05");

    assert_eq!(midnight.add_minutes(-10), TimeOfDay::new(23, 55, 9).unwrap());
    let mut toggled = afternoon;
    toggled.toggle_meridiem();
    assert_eq!(toggled, TimeOfDay::new(2, 30, 5).unwrap());
}

// ---------------------------------------------------------------------------
// 7. HeadlessBackend E2E: DatePicker + DateRangePicker ARIA & Collision Flip
// ---------------------------------------------------------------------------

#[component]
fn DatePickerE2eApp() -> Element {
    let mut single_val = use_signal(|| Option::<CalendarDate>::None);
    let mut range_val = use_signal(|| Option::<DateRange>::None);
    let no_weekends = DisabledDatePredicate::new(|date| {
        matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday)
    });
    let vp = Viewport::new(800.0, 600.0);
    let bottom_right_anchor = Rect::new(620.0, 520.0, 160.0, 38.0);

    let single_str = single_val()
        .map(|dt| format!("{:04}-{:02}-{:02}", dt.year, dt.month, dt.day))
        .unwrap_or_default();
    let range_str = range_val()
        .map(|r| {
            format!(
                "{:04}-{:02}-{:02}..{:04}-{:02}-{:02}",
                r.start.year, r.start.month, r.start.day, r.end.year, r.end.month, r.end.day
            )
        })
        .unwrap_or_default();

    rsx! {
        ThemeProvider {
            theme: Theme::LIGHT,
            Col {
                padding: 12.0,
                gap: 12.0,
                DatePicker {
                    test_id: "dp",
                    label: "Termin",
                    locale: loc("de-DE"),
                    today: Some(d(2024, 2, 29)),
                    min_date: Some(d(2024, 2, 1)),
                    max_date: Some(d(2024, 3, 31)),
                    disabled_dates: Some(no_weekends),
                    anchor_rect: Some(bottom_right_anchor),
                    overlay_size: Some(Size::new(300.0, 320.0)),
                    viewport: Some(vp),
                    on_change: move |v| single_val.set(v),
                }
                DateRangePicker {
                    test_id: "drp",
                    label: "الفترة",
                    locale: loc("ar-EG"),
                    today: Some(d(2024, 2, 15)),
                    on_change: move |v| range_val.set(v),
                }
                text { test_id: "out-single", "{single_str}" }
                text { test_id: "out-range", "{range_str}" }
            }
        }
    }
}

#[test]
fn headless_e2e_date_picker_aria_locale_collision_flip_and_range_selection() {
    let mut h = UiHarness::new(DatePickerE2eApp);

    // Closed initially; trigger shows German DD.MM.YYYY placeholder and aria-expanded="false".
    let trigger = h.find("dp-trigger");
    assert_eq!(h.raw_str(trigger, "ariaExpanded"), Some("false"));
    assert_eq!(h.raw_str(trigger, "ariaControls"), Some("dp-dialog"));
    assert_eq!(h.text_of("dp-value"), "DD.MM.YYYY");
    assert!(h.maybe_find("dp-dialog").is_none());

    // Pressing trigger opens dialog with role="dialog", role="grid", localized German header & Monday start.
    h.press("dp-trigger");
    let dialog = h.find("dp-dialog");
    assert_eq!(h.raw_str(dialog, "accessibilityRole"), Some("dialog"));
    assert_eq!(h.raw_str(dialog, "data-placement"), Some("top-end"));
    assert_eq!(h.raw_str(dialog, "data-flipped"), Some("true"));
    assert_eq!(h.text_of("dp-month-label"), "Februar 2024");
    assert_eq!(h.text_of("dp-weekday-0"), "Mo.");
    assert_eq!(h.text_of("dp-weekday-6"), "So.");

    let grid = h.find("dp-grid");
    assert_eq!(h.raw_str(grid, "accessibilityRole"), Some("grid"));
    assert_eq!(h.raw_str(grid, "ariaLabelledBy"), Some("dp-month-label"));

    // Leap day Feb 29 cell exists, is today + focused, enabled; Saturday Mar 2 is aria-disabled="true".
    let leap = h.find("dp-cell-2024-02-29");
    assert_eq!(h.raw_str(leap, "accessibilityRole"), Some("gridcell"));
    assert_eq!(h.raw_str(leap, "ariaDisabled"), Some("false"));
    assert_eq!(h.raw_str(leap, "data-today"), Some("true"));
    assert_eq!(h.raw_str(leap, "data-focused"), Some("true"));

    let sat = h.find("dp-cell-2024-03-02");
    assert_eq!(h.raw_str(sat, "ariaDisabled"), Some("true"));
    assert_eq!(h.raw_bool(sat, "disabled"), Some(true));

    // Clicking disabled Saturday does not select or close.
    h.press("dp-cell-2024-03-02");
    assert_eq!(h.text_of("out-single"), "");
    assert!(h.maybe_find("dp-dialog").is_some());

    // Keyboard ArrowRight twice from Thu Feb 29 -> Fri Mar 1 -> skips Sat/Sun -> Mon Mar 4, Enter commits!
    h.keydown("dp-grid", "ArrowRight", false);
    h.keydown("dp-grid", "ArrowRight", false);
    assert_eq!(h.text_of("dp-month-label"), "März 2024");
    assert_eq!(
        h.raw_str(h.find("dp-cell-2024-03-04"), "data-focused"),
        Some("true")
    );
    h.keydown("dp-grid", "Enter", false);
    assert_eq!(h.text_of("out-single"), "2024-03-04");
    assert!(h.maybe_find("dp-dialog").is_none());

    // Reopen and verify aria-selected="true" on selected cell + prev-month button switches back to Februar 2024.
    h.press("dp-trigger");
    assert_eq!(
        h.raw_str(h.find("dp-cell-2024-03-04"), "ariaSelected"),
        Some("true")
    );
    h.press("dp-prev-month");
    assert_eq!(h.text_of("dp-month-label"), "Februar 2024");
    h.keydown("dp-grid", "Escape", false);
    assert!(h.maybe_find("dp-dialog").is_none());

    // DateRangePicker in ar-EG: Saturday-first weekday header + reversed-order click normalization!
    h.press("drp-trigger");
    assert_eq!(h.text_of("drp-month-label"), "فبراير 2024");
    assert_eq!(h.text_of("drp-weekday-0"), "السبت");

    // 1st click on Feb 20 sets anchor and keeps dialog open.
    h.press("drp-cell-2024-02-20");
    assert!(h.maybe_find("drp-dialog").is_some());
    assert_eq!(
        h.raw_str(h.find("drp-cell-2024-02-20"), "data-range-start"),
        Some("true")
    );

    // 2nd click on EARLIER Feb 12 commits normalized [2024-02-12, 2024-02-20] and closes dialog!
    h.press("drp-cell-2024-02-12");
    assert_eq!(h.text_of("out-range"), "2024-02-12..2024-02-20");
    assert!(h.maybe_find("drp-dialog").is_none());

    // Reopen and verify range cells carry data-range-start / data-in-range / data-range-end + ariaSelected.
    h.press("drp-trigger");
    assert_eq!(h.raw_str(h.find("drp-cell-2024-02-12"), "ariaSelected"), Some("true"));
    assert_eq!(h.raw_str(h.find("drp-cell-2024-02-12"), "data-range-start"), Some("true"));
    assert_eq!(h.raw_str(h.find("drp-cell-2024-02-16"), "data-in-range"), Some("true"));
    assert_eq!(h.raw_str(h.find("drp-cell-2024-02-20"), "ariaSelected"), Some("true"));
    assert_eq!(h.raw_str(h.find("drp-cell-2024-02-20"), "data-range-end"), Some("true"));
}
