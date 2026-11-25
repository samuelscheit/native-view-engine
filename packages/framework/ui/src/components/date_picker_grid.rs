//! Shared calendar grid (`CalendarMonthGrid`, `CalendarCell`), date bounds/predicate
//! [`DateConstraints`] / [`DisabledDatePredicate`], and keyboard grid navigation with
//! disabled-date skipping.

use std::fmt;
use std::sync::Arc;

use nve_i18n::{month_name, ordered_weekdays, weekday_name, CalendarDate, LocaleId, NameWidth, Weekday};

use crate::components::date_picker_input::{
    add_days, add_months_clamped, add_years_clamped, end_of_week, start_of_week, DateRange,
};

const MAX_DISABLED_SCAN_DAYS: i64 = 366;

/// Shared, `PartialEq`-compatible custom disabled-date predicate for calendar pickers.
#[derive(Clone)]
pub struct DisabledDatePredicate(Arc<dyn Fn(CalendarDate) -> bool + Send + Sync>);

impl DisabledDatePredicate {
    pub fn new(pred: impl Fn(CalendarDate) -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(pred))
    }

    #[inline]
    pub fn is_disabled(&self, date: CalendarDate) -> bool {
        (self.0)(date)
    }
}

impl fmt::Debug for DisabledDatePredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DisabledDatePredicate(..)")
    }
}

impl PartialEq for DisabledDatePredicate {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Date bounds (`min_date` / `max_date`) and optional custom disabled-date predicate.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DateConstraints {
    pub min_date: Option<CalendarDate>,
    pub max_date: Option<CalendarDate>,
    pub disabled_predicate: Option<DisabledDatePredicate>,
}

impl DateConstraints {
    #[inline]
    pub fn is_out_of_bounds(&self, date: CalendarDate) -> bool {
        self.min_date.is_some_and(|min| date < min) || self.max_date.is_some_and(|max| date > max)
    }

    #[inline]
    pub fn is_disabled(&self, date: CalendarDate) -> bool {
        self.is_out_of_bounds(date)
            || self
                .disabled_predicate
                .as_ref()
                .is_some_and(|pred| pred.is_disabled(date))
    }

    #[inline]
    pub fn clamp_bounds(&self, mut date: CalendarDate) -> CalendarDate {
        if let Some(min) = self.min_date
            && date < min
        {
            date = min;
        }
        if let Some(max) = self.max_date
            && date > max
        {
            date = max;
        }
        date
    }

    /// Scans from `start` in `step` increments (`+1` or `-1`) for the first enabled date
    /// within `[min_date, max_date]` up to `limit` steps.
    pub(crate) fn find_enabled_step(
        &self,
        start: CalendarDate,
        step: i64,
        limit: i64,
    ) -> Option<CalendarDate> {
        let mut cur = start;
        for _ in 0..=limit {
            if let Some(min) = self.min_date
                && cur < min
            {
                if step < 0 {
                    return None;
                }
                cur = min;
                continue;
            }
            if let Some(max) = self.max_date
                && cur > max
            {
                if step > 0 {
                    return None;
                }
                cur = max;
                continue;
            }
            if !self.is_disabled(cur) {
                return Some(cur);
            }
            cur = add_days(cur, step);
        }
        None
    }

    /// Finds the nearest enabled date starting from `target`, preferring `preferred_dir`
    /// (`+1` or `-1`) before checking the opposite direction within bounds.
    pub(crate) fn find_nearest_enabled(
        &self,
        target: CalendarDate,
        preferred_dir: i64,
    ) -> Option<CalendarDate> {
        let clamped = self.clamp_bounds(target);
        if !self.is_disabled(clamped) {
            return Some(clamped);
        }
        let step = if preferred_dir >= 0 { 1 } else { -1 };
        self.find_enabled_step(add_days(clamped, step), step, MAX_DISABLED_SCAN_DAYS)
            .or_else(|| {
                self.find_enabled_step(add_days(clamped, -step), -step, MAX_DISABLED_SCAN_DAYS)
            })
    }
}

/// One day cell inside a 6×7 [`CalendarMonthGrid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarCell {
    pub date: CalendarDate,
    pub in_current_month: bool,
    pub is_today: bool,
    pub is_selected: bool,
    pub is_in_range: bool,
    pub is_range_start: bool,
    pub is_range_end: bool,
    pub is_focused: bool,
    pub is_disabled: bool,
}

/// Complete 6×7 (42-cell) calendar grid ordered by `locale`'s CLDR `first_day_of_week`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarMonthGrid {
    pub year: i32,
    pub month: u8,
    pub locale: LocaleId,
    pub first_day_of_week: Weekday,
    pub weekdays: [Weekday; 7],
    pub cells: [CalendarCell; 42],
}

impl CalendarMonthGrid {
    /// Builds a 6×7 calendar month grid with localized weekday order and full cell metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        year: i32,
        month: u8,
        locale: &LocaleId,
        today: Option<CalendarDate>,
        focused: CalendarDate,
        selected: Option<CalendarDate>,
        range: Option<DateRange>,
        constraints: &DateConstraints,
    ) -> Self {
        let month = month.clamp(1, 12);
        let weekdays = ordered_weekdays(locale);
        let first_day_of_week = weekdays[0];
        let first_of_month = CalendarDate {
            year,
            month,
            day: 1,
        };
        let leading = (first_of_month.weekday().index_from_sunday() + 7
            - first_day_of_week.index_from_sunday())
            % 7;
        let grid_start = add_days(first_of_month, -(leading as i64));
        let norm_range = range.map(DateRange::normalized);

        let cells = std::array::from_fn(|i| {
            let date = add_days(grid_start, i as i64);
            let is_range_start = norm_range.is_some_and(|r| r.start == date);
            let is_range_end = norm_range.is_some_and(|r| r.end == date);
            let is_in_range = norm_range.is_some_and(|r| r.contains(date));
            let is_selected = selected == Some(date) || is_range_start || is_range_end;
            CalendarCell {
                date,
                in_current_month: date.year == year && date.month == month,
                is_today: today == Some(date),
                is_selected,
                is_in_range,
                is_range_start,
                is_range_end,
                is_focused: focused == date,
                is_disabled: constraints.is_disabled(date),
            }
        });

        Self {
            year,
            month,
            locale: locale.clone(),
            first_day_of_week,
            weekdays,
            cells,
        }
    }

    /// Iterates over the 6 calendar rows of 7 cells each.
    #[inline]
    pub fn weeks(&self) -> impl Iterator<Item = &[CalendarCell; 7]> {
        self.cells.as_chunks::<7>().0.iter()
    }

    /// Returns the localized month + year heading (e.g. `"February 2024"`, `"Februar 2024"`,
    /// `"2024年 2月"`).
    pub fn month_title(&self, width: NameWidth) -> String {
        let m = month_name(&self.locale, self.month, width);
        match self.locale.language() {
            "ja" | "zh" => format!("{}年 {m}", self.year),
            "ko" => format!("{}년 {m}", self.year),
            _ => format!("{m} {}", self.year),
        }
    }

    /// Returns the 7 localized weekday column headers ordered by `first_day_of_week`.
    pub fn weekday_labels(&self, width: NameWidth) -> [&'static str; 7] {
        std::array::from_fn(|i| weekday_name(&self.locale, self.weekdays[i], width))
    }
}

/// Shared keyboard grid navigation helper returning the next enabled focused date.
pub(crate) fn navigate_grid_date(
    focused: CalendarDate,
    first_dow: Weekday,
    constraints: &DateConstraints,
    key: &str,
    shift: bool,
) -> Option<CalendarDate> {
    match key {
        "ArrowLeft" => {
            constraints.find_enabled_step(add_days(focused, -1), -1, MAX_DISABLED_SCAN_DAYS)
        }
        "ArrowRight" => {
            constraints.find_enabled_step(add_days(focused, 1), 1, MAX_DISABLED_SCAN_DAYS)
        }
        "ArrowUp" => {
            let target = add_days(focused, -7);
            if let Some(min) = constraints.min_date
                && target < min
            {
                constraints.find_enabled_step(min, 1, 7)
            } else {
                constraints.find_enabled_step(target, -1, MAX_DISABLED_SCAN_DAYS)
            }
        }
        "ArrowDown" => {
            let target = add_days(focused, 7);
            if let Some(max) = constraints.max_date
                && target > max
            {
                constraints.find_enabled_step(max, -1, 7)
            } else {
                constraints.find_enabled_step(target, 1, MAX_DISABLED_SCAN_DAYS)
            }
        }
        "Home" => {
            let start = constraints.clamp_bounds(start_of_week(focused, first_dow));
            constraints.find_enabled_step(start, 1, 6)
        }
        "End" => {
            let end = constraints.clamp_bounds(end_of_week(focused, first_dow));
            constraints.find_enabled_step(end, -1, 6)
        }
        "PageUp" => {
            let target = if shift {
                add_years_clamped(focused, -1)
            } else {
                add_months_clamped(focused, -1)
            };
            constraints.find_nearest_enabled(target, -1)
        }
        "PageDown" => {
            let target = if shift {
                add_years_clamped(focused, 1)
            } else {
                add_months_clamped(focused, 1)
            };
            constraints.find_nearest_enabled(target, 1)
        }
        _ => None,
    }
}
