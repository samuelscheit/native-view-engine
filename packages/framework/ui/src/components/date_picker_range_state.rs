//! Pure headless date-range picker state machine ([`DateRangePickerState`]):
//! - 1st click / `Enter` sets the selection anchor (`AnchorSet`) while keeping the dialog open,
//! - Hovering or moving keyboard focus previews the normalized range `[min, max]`,
//! - 2nd click / `Enter` commits the normalized [`DateRange`] (`start <= end`, even if reversed).

use nve_i18n::{format_date, ordered_weekdays, CalendarDate, DateStyle, LocaleId};

use crate::components::date_picker_grid::{
    navigate_grid_date, CalendarMonthGrid, DateConstraints, DisabledDatePredicate,
};
use crate::components::date_picker_input::{
    add_months_clamped, DateRange, DateSegmentInput, SegmentKeyOutcome,
};

/// Active endpoint in a [`DateRangePickerState`] segmented input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeEndpoint {
    Start,
    End,
}

/// Outcome produced by [`DateRangePickerState`] interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateRangeAction {
    None,
    Opened,
    Closed,
    Focused(CalendarDate),
    AnchorSet(CalendarDate),
    Hovered(Option<CalendarDate>),
    Selected(DateRange),
    Cleared,
    SegmentUpdated,
}

/// Headless date-range picker state machine with live normalized hover preview and
/// reversed-range selection support.
#[derive(Debug, Clone, PartialEq)]
pub struct DateRangePickerState {
    locale: LocaleId,
    selected: Option<DateRange>,
    anchor: Option<CalendarDate>,
    hovered: Option<CalendarDate>,
    focused: CalendarDate,
    view_year: i32,
    view_month: u8,
    today: Option<CalendarDate>,
    open: bool,
    constraints: DateConstraints,
    start_segments: DateSegmentInput,
    end_segments: DateSegmentInput,
}

impl DateRangePickerState {
    pub fn new(locale: LocaleId, initial_focus: CalendarDate) -> Self {
        let start_segments = DateSegmentInput::new(locale.clone());
        let end_segments = DateSegmentInput::new(locale.clone());
        Self {
            locale,
            selected: None,
            anchor: None,
            hovered: None,
            focused: initial_focus,
            view_year: initial_focus.year,
            view_month: initial_focus.month,
            today: Some(initial_focus),
            open: false,
            constraints: DateConstraints::default(),
            start_segments,
            end_segments,
        }
    }

    pub fn with_today(mut self, today: Option<CalendarDate>) -> Self {
        self.today = today;
        self
    }

    pub fn with_selected(mut self, range: Option<DateRange>) -> Self {
        let valid = range.map(DateRange::normalized).filter(|r| {
            !self.constraints.is_disabled(r.start) && !self.constraints.is_disabled(r.end)
        });
        self.selected = valid;
        if let Some(r) = valid {
            self.focused = r.start;
            self.view_year = r.start.year;
            self.view_month = r.start.month;
            self.start_segments.set_date(Some(r.start));
            self.end_segments.set_date(Some(r.end));
        } else {
            self.start_segments.set_date(None);
            self.end_segments.set_date(None);
        }
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

    pub fn with_open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    fn reconcile_focus(&mut self) {
        if let Some(r) = self.selected
            && (self.constraints.is_disabled(r.start) || self.constraints.is_disabled(r.end))
        {
            self.selected = None;
            self.start_segments.set_date(None);
            self.end_segments.set_date(None);
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
        self.start_segments.set_locale(locale.clone());
        self.end_segments.set_locale(locale);
    }

    #[inline]
    pub fn selected(&self) -> Option<DateRange> {
        self.selected
    }

    #[inline]
    pub fn anchor(&self) -> Option<CalendarDate> {
        self.anchor
    }

    #[inline]
    pub fn hovered(&self) -> Option<CalendarDate> {
        self.hovered
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
    pub fn start_segments(&self) -> &DateSegmentInput {
        &self.start_segments
    }

    #[inline]
    pub fn end_segments(&self) -> &DateSegmentInput {
        &self.end_segments
    }

    /// Returns the live normalized preview range while selecting (`anchor..=hovered`), or the
    /// committed range when idle.
    pub fn preview_range(&self) -> Option<DateRange> {
        if let Some(anchor) = self.anchor {
            Some(DateRange::new(anchor, self.hovered.unwrap_or(self.focused)))
        } else {
            self.selected
        }
    }

    pub fn open(&mut self) -> DateRangeAction {
        if !self.open {
            self.open = true;
            if let Some(r) = self.selected {
                self.focused = r.start;
                self.view_year = r.start.year;
                self.view_month = r.start.month;
            }
            DateRangeAction::Opened
        } else {
            DateRangeAction::None
        }
    }

    pub fn close(&mut self) -> DateRangeAction {
        if self.open {
            self.open = false;
            self.anchor = None;
            self.hovered = None;
            DateRangeAction::Closed
        } else {
            DateRangeAction::None
        }
    }

    pub fn toggle(&mut self) -> DateRangeAction {
        if self.open { self.close() } else { self.open() }
    }

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
            if self.anchor.is_some() {
                self.hovered = Some(enabled);
            }
        }
    }

    pub fn prev_month(&mut self) {
        self.shift_view_months(-1);
    }

    pub fn next_month(&mut self) {
        self.shift_view_months(1);
    }

    /// Updates hover preview while an anchor date is active.
    pub fn hover_date(&mut self, date: Option<CalendarDate>) -> DateRangeAction {
        if self.anchor.is_none() {
            return DateRangeAction::None;
        }
        let valid = date.filter(|&d| !self.constraints.is_disabled(d));
        self.hovered = valid;
        DateRangeAction::Hovered(valid)
    }

    /// First click sets the anchor (staying open); second click commits normalized `[start, end]`.
    pub fn select_date(&mut self, date: CalendarDate) -> DateRangeAction {
        if self.constraints.is_disabled(date) {
            return DateRangeAction::None;
        }
        self.focused = date;
        self.view_year = date.year;
        self.view_month = date.month;
        if let Some(anchor) = self.anchor.take() {
            let range = DateRange::new(anchor, date);
            self.selected = Some(range);
            self.hovered = None;
            self.start_segments.set_date(Some(range.start));
            self.end_segments.set_date(Some(range.end));
            self.open = false;
            DateRangeAction::Selected(range)
        } else {
            self.anchor = Some(date);
            self.hovered = Some(date);
            DateRangeAction::AnchorSet(date)
        }
    }

    pub fn clear(&mut self) -> DateRangeAction {
        self.selected = None;
        self.anchor = None;
        self.hovered = None;
        self.start_segments.set_date(None);
        self.end_segments.set_date(None);
        DateRangeAction::Cleared
    }

    /// Builds the 6×7 localized calendar grid with live range preview highlighting.
    pub fn grid(&self) -> CalendarMonthGrid {
        CalendarMonthGrid::build(
            self.view_year,
            self.view_month,
            &self.locale,
            self.today,
            self.focused,
            None,
            self.preview_range(),
            &self.constraints,
        )
    }

    /// Formats the selected range using `nve-i18n` (`"<start> – <end>"`).
    pub fn formatted_selected(&self, style: DateStyle) -> Option<String> {
        self.selected.map(|r| {
            format!(
                "{} – {}",
                format_date(&self.locale, r.start, style),
                format_date(&self.locale, r.end, style)
            )
        })
    }

    /// Handles keyboard navigation inside the open calendar grid.
    pub fn handle_grid_key(&mut self, key: &str, shift: bool) -> DateRangeAction {
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
                    if self.anchor.is_some() {
                        self.hovered = Some(next);
                    }
                    DateRangeAction::Focused(next)
                } else {
                    DateRangeAction::None
                }
            }
        }
    }

    /// Handles keyboard typing on either the start or end segmented input, auto-normalizing
    /// once both endpoints hold valid enabled dates.
    pub fn handle_segment_key(
        &mut self,
        endpoint: RangeEndpoint,
        key: &str,
        shift: bool,
    ) -> DateRangeAction {
        let outcome = match endpoint {
            RangeEndpoint::Start => self.start_segments.handle_key(key, shift),
            RangeEndpoint::End => self.end_segments.handle_key(key, shift),
        };
        match outcome {
            SegmentKeyOutcome::ValueUpdated(_) => {
                if let (Some(s), Some(e)) = (self.start_segments.date(), self.end_segments.date())
                    && !self.constraints.is_disabled(s)
                    && !self.constraints.is_disabled(e)
                {
                    let range = DateRange::new(s, e);
                    self.selected = Some(range);
                    self.focused = range.start;
                    self.view_year = range.start.year;
                    self.view_month = range.start.month;
                    return DateRangeAction::Selected(range);
                }
                DateRangeAction::SegmentUpdated
            }
            SegmentKeyOutcome::SegmentMoved(_) => DateRangeAction::SegmentUpdated,
            SegmentKeyOutcome::Ignored
            | SegmentKeyOutcome::FocusExitPrev
            | SegmentKeyOutcome::FocusExitNext => DateRangeAction::None,
        }
    }
}
