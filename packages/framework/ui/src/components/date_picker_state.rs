//! Pure headless state machine for [`DatePickerState`] (single date), and re-exports of
//! [`DateRangePickerState`], [`CalendarMonthGrid`], [`DateConstraints`], [`DateRange`],
//! [`DateSegmentInput`], and [`TimeOfDay`] for ergonomic single-module imports.

use nve_i18n::{format_date, ordered_weekdays, CalendarDate, DateStyle, LocaleId};

use crate::components::date_picker_grid::navigate_grid_date;
pub use crate::components::date_picker_grid::{
    CalendarCell, CalendarMonthGrid, DateConstraints, DisabledDatePredicate,
};
pub use crate::components::date_picker_input::{
    add_days, add_months_clamped, add_years_clamped, civil_from_days, days_from_civil,
    days_in_month, end_of_week, start_of_week, DateRange, DateSegment, DateSegmentInput,
    DateSegmentOrder, HourCycle, SegmentKeyOutcome, TimeOfDay,
};
pub use crate::components::date_picker_range_state::{
    DateRangeAction, DateRangePickerState, RangeEndpoint,
};

/// Outcome produced by [`DatePickerState`] interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePickerAction {
    None,
    Opened,
    Closed,
    Focused(CalendarDate),
    Selected(CalendarDate),
    Cleared,
    SegmentMoved(DateSegment),
    SegmentUpdated,
}

/// Headless single-date picker state machine combining a 6×7 localized calendar grid,
/// keyboard navigation with disabled-date skipping, segmented keyboard input, and optional
/// [`TimeOfDay`].
#[derive(Debug, Clone, PartialEq)]
pub struct DatePickerState {
    locale: LocaleId,
    selected: Option<CalendarDate>,
    focused: CalendarDate,
    view_year: i32,
    view_month: u8,
    today: Option<CalendarDate>,
    open: bool,
    constraints: DateConstraints,
    segments: DateSegmentInput,
    time: Option<TimeOfDay>,
}

impl DatePickerState {
    pub fn new(locale: LocaleId, initial_focus: CalendarDate) -> Self {
        let segments = DateSegmentInput::new(locale.clone());
        Self {
            locale,
            selected: None,
            focused: initial_focus,
            view_year: initial_focus.year,
            view_month: initial_focus.month,
            today: Some(initial_focus),
            open: false,
            constraints: DateConstraints::default(),
            segments,
            time: None,
        }
    }

    pub fn with_today(mut self, today: Option<CalendarDate>) -> Self {
        self.today = today;
        self
    }

    pub fn with_selected(mut self, selected: Option<CalendarDate>) -> Self {
        self.selected = selected.filter(|&d| !self.constraints.is_disabled(d));
        if let Some(d) = self.selected {
            self.focused = d;
            self.view_year = d.year;
            self.view_month = d.month;
        }
        self.segments.set_date(self.selected);
        self
    }

    pub fn with_min_date(mut self, min_date: CalendarDate) -> Self {
        self.constraints.min_date = Some(min_date);
        self.reconcile_focus();
        self
    }

    pub fn with_max_date(mut self, max_date: CalendarDate) -> Self {
        self.constraints.max_date = Some(max_date);
        self.reconcile_focus();
        self
    }

    pub fn with_disabled_predicate(
        mut self,
        pred: impl Fn(CalendarDate) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.constraints.disabled_predicate = Some(DisabledDatePredicate::new(pred));
        self.reconcile_focus();
        self
    }

    pub fn with_constraints(mut self, constraints: DateConstraints) -> Self {
        self.constraints = constraints;
        self.reconcile_focus();
        self
    }

    pub fn with_time(mut self, time: Option<TimeOfDay>) -> Self {
        self.time = time;
        self
    }

    pub fn with_open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    fn reconcile_focus(&mut self) {
        if let Some(sel) = self.selected
            && self.constraints.is_disabled(sel)
        {
            self.selected = None;
            self.segments.set_date(None);
        }
        if let Some(enabled) = self.constraints.find_nearest_enabled(self.focused, 1) {
            self.focused = enabled;
            self.view_year = enabled.year;
            self.view_month = enabled.month;
        }
    }

    #[inline]
    pub fn locale(&self) -> &LocaleId {
        &self.locale
    }

    pub fn set_locale(&mut self, locale: LocaleId) {
        self.locale = locale.clone();
        self.segments.set_locale(locale);
    }

    #[inline]
    pub fn selected(&self) -> Option<CalendarDate> {
        self.selected
    }

    #[inline]
    pub fn focused(&self) -> CalendarDate {
        self.focused
    }

    #[inline]
    pub fn view_year(&self) -> i32 {
        self.view_year
    }

    #[inline]
    pub fn view_month(&self) -> u8 {
        self.view_month
    }

    #[inline]
    pub fn is_open(&self) -> bool {
        self.open
    }

    #[inline]
    pub fn constraints(&self) -> &DateConstraints {
        &self.constraints
    }

    #[inline]
    pub fn segments(&self) -> &DateSegmentInput {
        &self.segments
    }

    #[inline]
    pub fn time(&self) -> Option<TimeOfDay> {
        self.time
    }

    #[inline]
    pub fn set_time(&mut self, time: Option<TimeOfDay>) {
        self.time = time;
    }

    pub fn open(&mut self) -> DatePickerAction {
        if !self.open {
            self.open = true;
            if let Some(d) = self.selected {
                self.focused = d;
                self.view_year = d.year;
                self.view_month = d.month;
            }
            DatePickerAction::Opened
        } else {
            DatePickerAction::None
        }
    }

    pub fn close(&mut self) -> DatePickerAction {
        if self.open {
            self.open = false;
            DatePickerAction::Closed
        } else {
            DatePickerAction::None
        }
    }

    pub fn toggle(&mut self) -> DatePickerAction {
        if self.open { self.close() } else { self.open() }
    }

    /// Navigates the visible calendar month by `delta_months` (`-1` prev, `+1` next).
    pub fn shift_view_months(&mut self, delta_months: i32) {
        let shifted = add_months_clamped(
            CalendarDate {
                year: self.view_year,
                month: self.view_month,
                day: self.focused.day,
            },
            delta_months,
        );
        self.view_year = shifted.year;
        self.view_month = shifted.month;
        if let Some(enabled) = self
            .constraints
            .find_nearest_enabled(shifted, if delta_months >= 0 { 1 } else { -1 })
        {
            self.focused = enabled;
        }
    }

    pub fn prev_month(&mut self) {
        self.shift_view_months(-1);
    }

    pub fn next_month(&mut self) {
        self.shift_view_months(1);
    }

    /// Selects `date` if not disabled, syncs focus/view/segments, and closes the calendar dialog.
    pub fn select_date(&mut self, date: CalendarDate) -> DatePickerAction {
        if self.constraints.is_disabled(date) {
            return DatePickerAction::None;
        }
        self.selected = Some(date);
        self.focused = date;
        self.view_year = date.year;
        self.view_month = date.month;
        self.segments.set_date(Some(date));
        self.open = false;
        DatePickerAction::Selected(date)
    }

    pub fn clear(&mut self) -> DatePickerAction {
        self.selected = None;
        self.segments.set_date(None);
        DatePickerAction::Cleared
    }

    /// Builds the current 6×7 localized calendar grid.
    pub fn grid(&self) -> CalendarMonthGrid {
        CalendarMonthGrid::build(
            self.view_year,
            self.view_month,
            &self.locale,
            self.today,
            self.focused,
            self.selected,
            None,
            &self.constraints,
        )
    }

    /// Formats the selected date via `nve-i18n` (`DateStyle`), if any.
    pub fn formatted_selected(&self, style: DateStyle) -> Option<String> {
        self.selected.map(|d| format_date(&self.locale, d, style))
    }

    /// Handles keyboard navigation inside the open calendar grid.
    pub fn handle_grid_key(&mut self, key: &str, shift: bool) -> DatePickerAction {
        match key {
            "Escape" | "Esc" => self.close(),
            "Enter" | " " | "Space" => self.select_date(self.focused),
            _ => {
                let first_dow = ordered_weekdays(&self.locale)[0];
                if let Some(next) =
                    navigate_grid_date(self.focused, first_dow, &self.constraints, key, shift)
                {
                    self.focused = next;
                    self.view_year = next.year;
                    self.view_month = next.month;
                    DatePickerAction::Focused(next)
                } else {
                    DatePickerAction::None
                }
            }
        }
    }

    /// Handles keyboard input on the segmented date input field.
    pub fn handle_segment_key(&mut self, key: &str, shift: bool) -> DatePickerAction {
        match self.segments.handle_key(key, shift) {
            SegmentKeyOutcome::SegmentMoved(seg) => DatePickerAction::SegmentMoved(seg),
            SegmentKeyOutcome::ValueUpdated(Some(date)) => {
                if !self.constraints.is_disabled(date) {
                    self.selected = Some(date);
                    self.focused = date;
                    self.view_year = date.year;
                    self.view_month = date.month;
                    DatePickerAction::Selected(date)
                } else {
                    DatePickerAction::SegmentUpdated
                }
            }
            SegmentKeyOutcome::ValueUpdated(None) => {
                self.selected = None;
                DatePickerAction::SegmentUpdated
            }
            SegmentKeyOutcome::Ignored
            | SegmentKeyOutcome::FocusExitPrev
            | SegmentKeyOutcome::FocusExitNext => DatePickerAction::None,
        }
    }
}
