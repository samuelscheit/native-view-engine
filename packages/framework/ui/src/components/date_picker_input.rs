//! Pure proleptic Gregorian civil date arithmetic, normalized [`DateRange`],
//! locale-derived [`DateSegmentOrder`] / [`DateSegmentInput`] keyboard segment typing,
//! and locale-aware [`TimeOfDay`] (`12h` / `24h`) helpers.
//!
//! Built directly on [`nve_i18n::CalendarDate`] with zero external date/time crates and
//! zero timestamp/UTC/DST conversions, making DST and timezone off-by-one-day bugs
//! impossible by construction.

use nve_i18n::{CalendarDate, LocaleId, Weekday};

/// Number of days in Gregorian `month` (`1..=12`) of `year`, accounting for leap years
/// (including century rules: 2000 is a leap year, 1900 is not).
#[inline]
pub const fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if CalendarDate::is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Howard Hinnant's exact civil-to-days algorithm (`1970-01-01 == 0`).
#[inline]
pub const fn days_from_civil(date: CalendarDate) -> i64 {
    let y = (date.year as i64) - if date.month <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = date.month as i64;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + (date.day as i64) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Howard Hinnant's exact days-to-civil inverse (`0 == 1970-01-01`).
#[inline]
pub const fn civil_from_days(days_since_epoch: i64) -> CalendarDate {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = (y + if m <= 2 { 1 } else { 0 }) as i32;
    CalendarDate {
        year,
        month: m as u8,
        day: d as u8,
    }
}

/// Adds `delta_days` (positive or negative) to `date` in pure civil calendar arithmetic.
#[inline]
pub const fn add_days(date: CalendarDate, delta_days: i64) -> CalendarDate {
    civil_from_days(days_from_civil(date) + delta_days)
}

/// Adds `delta_months` to `date`, clamping `day` to the target month's length
/// (e.g., `2024-01-31 + 1 month -> 2024-02-29`, `2025-01-31 + 1 month -> 2025-02-28`).
#[inline]
pub const fn add_months_clamped(date: CalendarDate, delta_months: i32) -> CalendarDate {
    let total = (date.year as i64) * 12 + ((date.month - 1) as i64) + (delta_months as i64);
    let year = total.div_euclid(12) as i32;
    let month = (total.rem_euclid(12) + 1) as u8;
    let max_day = days_in_month(year, month);
    let day = if date.day > max_day { max_day } else { date.day };
    CalendarDate { year, month, day }
}

/// Adds `delta_years` to `date`, clamping Feb 29 to Feb 28 on non-leap years.
#[inline]
pub const fn add_years_clamped(date: CalendarDate, delta_years: i32) -> CalendarDate {
    let year = date.year.saturating_add(delta_years);
    let max_day = days_in_month(year, date.month);
    let day = if date.day > max_day { max_day } else { date.day };
    CalendarDate {
        year,
        month: date.month,
        day,
    }
}

/// Returns the first day of `date`'s week according to `first_day_of_week`.
#[inline]
pub fn start_of_week(date: CalendarDate, first_day_of_week: Weekday) -> CalendarDate {
    let offset = (date.weekday().index_from_sunday() + 7 - first_day_of_week.index_from_sunday()) % 7;
    add_days(date, -(offset as i64))
}

/// Returns the last day of `date`'s week according to `first_day_of_week`.
#[inline]
pub fn end_of_week(date: CalendarDate, first_day_of_week: Weekday) -> CalendarDate {
    add_days(start_of_week(date, first_day_of_week), 6)
}

/// Inclusive civil date range normalized so `start <= end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DateRange {
    pub start: CalendarDate,
    pub end: CalendarDate,
}

impl DateRange {
    /// Constructs a normalized [`DateRange`] guaranteeing `start <= end`.
    #[inline]
    pub fn new(a: CalendarDate, b: CalendarDate) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }

    /// Constructs a single-day range `[date, date]`.
    #[inline]
    pub const fn single(date: CalendarDate) -> Self {
        Self {
            start: date,
            end: date,
        }
    }

    /// Returns a normalized copy with `start <= end` even if fields were mutated directly.
    #[inline]
    pub fn normalized(self) -> Self {
        Self::new(self.start, self.end)
    }

    /// Returns `true` if `date` lies inclusively within `[start, end]`.
    #[inline]
    pub fn contains(&self, date: CalendarDate) -> bool {
        let norm = self.normalized();
        date >= norm.start && date <= norm.end
    }

    /// Inclusive number of civil days in the range (`>= 1`).
    #[inline]
    pub fn len_days(&self) -> u64 {
        let norm = self.normalized();
        (days_from_civil(norm.end) - days_from_civil(norm.start) + 1) as u64
    }
}

/// Editable date segment in a localized segmented date input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateSegment {
    Year,
    Month,
    Day,
}

/// Locale-derived order of date input segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateSegmentOrder {
    /// Month / Day / Year (e.g., `en-US`: `MM/DD/YYYY`).
    Mdy,
    /// Day / Month / Year (e.g., `de-DE`: `DD.MM.YYYY`, `fr-FR`/`en-GB`: `DD/MM/YYYY`).
    Dmy,
    /// Year / Month / Day (e.g., `ja-JP`/`zh-CN`/`ko-KR`/`sv-SE`: `YYYY-MM-DD`).
    Ymd,
}

impl DateSegmentOrder {
    /// Derives the conventional numeric date segment order from a BCP-47 [`LocaleId`].
    pub fn from_locale(locale: &LocaleId) -> Self {
        let lang = locale.language();
        let region = locale.region();
        if matches!(lang, "ja" | "zh" | "ko" | "sv" | "hu" | "lt" | "eu" | "mn") {
            return Self::Ymd;
        }
        if matches!((lang, region), ("en", Some("US") | None))
            || matches!(region, Some("US" | "PH" | "FM" | "MH" | "PW"))
        {
            return Self::Mdy;
        }
        Self::Dmy
    }

    /// Returns the 3 editable segments in visual left-to-right input order.
    #[inline]
    pub const fn segments(self) -> [DateSegment; 3] {
        match self {
            Self::Mdy => [DateSegment::Month, DateSegment::Day, DateSegment::Year],
            Self::Dmy => [DateSegment::Day, DateSegment::Month, DateSegment::Year],
            Self::Ymd => [DateSegment::Year, DateSegment::Month, DateSegment::Day],
        }
    }

    /// Returns the conventional numeric segment separator for `locale`.
    pub fn separator_for(locale: &LocaleId) -> &'static str {
        let order = Self::from_locale(locale);
        if order == Self::Ymd {
            return "-";
        }
        match locale.language() {
            "de" | "pl" | "cs" | "sk" | "ru" | "uk" | "fi" | "nb" | "nn" | "no" | "tr" => ".",
            _ => "/",
        }
    }

    /// Returns the localized numeric input placeholder (e.g. `"MM/DD/YYYY"`, `"DD.MM.YYYY"`,
    /// `"YYYY-MM-DD"`).
    pub fn placeholder_for(locale: &LocaleId) -> String {
        let order = Self::from_locale(locale);
        let sep = Self::separator_for(locale);
        let segs = order.segments();
        let tok = |s: DateSegment| match s {
            DateSegment::Year => "YYYY",
            DateSegment::Month => "MM",
            DateSegment::Day => "DD",
        };
        format!("{}{sep}{}{sep}{}", tok(segs[0]), tok(segs[1]), tok(segs[2]))
    }
}

/// Outcome returned by [`DateSegmentInput::handle_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKeyOutcome {
    Ignored,
    SegmentMoved(DateSegment),
    ValueUpdated(Option<CalendarDate>),
    FocusExitPrev,
    FocusExitNext,
}

/// Pure headless keyboard segment input (`Tab` / `ArrowLeft` / `ArrowRight` between
/// Year/Month/Day segments, multi-digit accumulation, `ArrowUp` / `ArrowDown` with
/// leap-year day clamping).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateSegmentInput {
    locale: LocaleId,
    order: DateSegmentOrder,
    active_index: usize,
    year: Option<i32>,
    month: Option<u8>,
    day: Option<u8>,
    digit_buffer: String,
}

impl DateSegmentInput {
    pub fn new(locale: LocaleId) -> Self {
        let order = DateSegmentOrder::from_locale(&locale);
        Self {
            locale,
            order,
            active_index: 0,
            year: None,
            month: None,
            day: None,
            digit_buffer: String::new(),
        }
    }

    pub fn with_date(mut self, date: Option<CalendarDate>) -> Self {
        self.set_date(date);
        self
    }

    #[inline]
    pub fn locale(&self) -> &LocaleId {
        &self.locale
    }

    #[inline]
    pub fn order(&self) -> DateSegmentOrder {
        self.order
    }

    #[inline]
    pub fn active_index(&self) -> usize {
        self.active_index
    }

    #[inline]
    pub fn active_segment(&self) -> DateSegment {
        self.order.segments()[self.active_index]
    }

    pub fn set_active_segment(&mut self, segment: DateSegment) {
        if let Some(idx) = self.order.segments().iter().position(|&s| s == segment) {
            self.active_index = idx;
            self.digit_buffer.clear();
        }
    }

    pub fn set_locale(&mut self, locale: LocaleId) {
        let prev_seg = self.active_segment();
        self.order = DateSegmentOrder::from_locale(&locale);
        self.locale = locale;
        self.set_active_segment(prev_seg);
    }

    pub fn set_date(&mut self, date: Option<CalendarDate>) {
        self.digit_buffer.clear();
        match date {
            Some(d) => {
                self.year = Some(d.year);
                self.month = Some(d.month);
                self.day = Some(d.day);
            }
            None => {
                self.year = None;
                self.month = None;
                self.day = None;
            }
        }
    }

    /// Returns the complete validated [`CalendarDate`] when all three segments are populated.
    pub fn date(&self) -> Option<CalendarDate> {
        let (y, m, d) = (self.year?, self.month?, self.day?);
        CalendarDate::new(y, m, d).ok()
    }

    #[inline]
    pub fn year(&self) -> Option<i32> {
        self.year
    }

    #[inline]
    pub fn month(&self) -> Option<u8> {
        self.month
    }

    #[inline]
    pub fn day(&self) -> Option<u8> {
        self.day
    }

    /// Formats the segmented value in locale order (e.g. `"02/29/2024"`, `"29.02.2024"`,
    /// `"2024-02-29"`), using `"YYYY"`/`"MM"`/`"DD"` for unset segments.
    pub fn formatted_value(&self) -> String {
        let sep = DateSegmentOrder::separator_for(&self.locale);
        let segs = self.order.segments();
        let fmt_seg = |seg: DateSegment| match seg {
            DateSegment::Year => self
                .year
                .map(|y| format!("{y:04}"))
                .unwrap_or_else(|| "YYYY".to_owned()),
            DateSegment::Month => self
                .month
                .map(|m| format!("{m:02}"))
                .unwrap_or_else(|| "MM".to_owned()),
            DateSegment::Day => self
                .day
                .map(|d| format!("{d:02}"))
                .unwrap_or_else(|| "DD".to_owned()),
        };
        format!(
            "{}{sep}{}{sep}{}",
            fmt_seg(segs[0]),
            fmt_seg(segs[1]),
            fmt_seg(segs[2])
        )
    }

    /// Handles a keyboard event on the segmented date input.
    pub fn handle_key(&mut self, key: &str, shift: bool) -> SegmentKeyOutcome {
        match key {
            "ArrowRight" => {
                self.digit_buffer.clear();
                self.active_index = (self.active_index + 1).min(2);
                SegmentKeyOutcome::SegmentMoved(self.active_segment())
            }
            "ArrowLeft" => {
                self.digit_buffer.clear();
                self.active_index = self.active_index.saturating_sub(1);
                SegmentKeyOutcome::SegmentMoved(self.active_segment())
            }
            "Tab" if !shift => {
                self.digit_buffer.clear();
                if self.active_index < 2 {
                    self.active_index += 1;
                    SegmentKeyOutcome::SegmentMoved(self.active_segment())
                } else {
                    SegmentKeyOutcome::FocusExitNext
                }
            }
            "Tab" if shift => {
                self.digit_buffer.clear();
                if self.active_index > 0 {
                    self.active_index -= 1;
                    SegmentKeyOutcome::SegmentMoved(self.active_segment())
                } else {
                    SegmentKeyOutcome::FocusExitPrev
                }
            }
            "/" | "." | "-" => {
                self.digit_buffer.clear();
                self.active_index = (self.active_index + 1).min(2);
                SegmentKeyOutcome::SegmentMoved(self.active_segment())
            }
            "ArrowUp" => {
                self.digit_buffer.clear();
                self.step_active(1);
                SegmentKeyOutcome::ValueUpdated(self.date())
            }
            "ArrowDown" => {
                self.digit_buffer.clear();
                self.step_active(-1);
                SegmentKeyOutcome::ValueUpdated(self.date())
            }
            "Backspace" | "Delete" => {
                self.digit_buffer.clear();
                match self.active_segment() {
                    DateSegment::Year => self.year = None,
                    DateSegment::Month => self.month = None,
                    DateSegment::Day => self.day = None,
                }
                SegmentKeyOutcome::ValueUpdated(self.date())
            }
            _ => {
                let mut chars = key.chars();
                if let (Some(ch), None) = (chars.next(), chars.next())
                    && ch.is_ascii_digit()
                {
                    self.push_digit(ch);
                    return SegmentKeyOutcome::ValueUpdated(self.date());
                }
                SegmentKeyOutcome::Ignored
            }
        }
    }

    fn effective_max_day(&self) -> u8 {
        match self.month {
            // While `year` is unset (e.g. typing `02/29/2024` in Mdy order), allow leap day 29
            // and clamp once the 4-digit year is committed.
            Some(m) => days_in_month(self.year.unwrap_or(2000), m),
            None => 31,
        }
    }

    fn clamp_day_for_year_month(&mut self) {
        if let Some(d) = self.day {
            let max_d = self.effective_max_day();
            if d > max_d {
                self.day = Some(max_d);
            }
        }
    }

    fn step_active(&mut self, delta: i32) {
        match self.active_segment() {
            DateSegment::Year => {
                let cur = self.year.unwrap_or(2026);
                self.year = Some((cur + delta).clamp(1, 9999));
                self.clamp_day_for_year_month();
            }
            DateSegment::Month => {
                let cur = self.month.unwrap_or(if delta > 0 { 12 } else { 1 }) as i32;
                let next = ((cur - 1 + delta).rem_euclid(12) + 1) as u8;
                self.month = Some(next);
                self.clamp_day_for_year_month();
            }
            DateSegment::Day => {
                let max_d = self.effective_max_day() as i32;
                let cur = self.day.unwrap_or(if delta > 0 { max_d as u8 } else { 1 }) as i32;
                let next = ((cur - 1 + delta).rem_euclid(max_d) + 1) as u8;
                self.day = Some(next);
            }
        }
    }

    fn advance_segment(&mut self) {
        self.digit_buffer.clear();
        if self.active_index < 2 {
            self.active_index += 1;
        }
    }

    fn push_digit(&mut self, ch: char) {
        self.digit_buffer.push(ch);
        let seg = self.active_segment();
        match seg {
            DateSegment::Month => {
                let val: u8 = self.digit_buffer.parse().unwrap_or(1);
                if self.digit_buffer.len() == 1 {
                    if val >= 2 {
                        self.month = Some(val.clamp(1, 12));
                        self.clamp_day_for_year_month();
                        self.advance_segment();
                    } else if val == 1 {
                        self.month = Some(1);
                        self.clamp_day_for_year_month();
                    }
                } else {
                    self.month = Some(val.clamp(1, 12));
                    self.clamp_day_for_year_month();
                    self.advance_segment();
                }
            }
            DateSegment::Day => {
                let max_d = self.effective_max_day();
                let val: u8 = self.digit_buffer.parse().unwrap_or(1);
                if self.digit_buffer.len() == 1 {
                    if val > max_d / 10 {
                        self.day = Some(val.clamp(1, max_d));
                        self.advance_segment();
                    } else if val >= 1 {
                        self.day = Some(val);
                    }
                } else {
                    self.day = Some(val.clamp(1, max_d));
                    self.advance_segment();
                }
            }
            DateSegment::Year => {
                let val: i32 = self.digit_buffer.parse().unwrap_or(1);
                if self.digit_buffer.len() >= 4 {
                    self.year = Some(val.clamp(1, 9999));
                    self.clamp_day_for_year_month();
                    self.advance_segment();
                } else if val > 0 {
                    self.year = Some(val);
                }
            }
        }
    }
}

/// Locale clock convention (`12h` AM/PM vs `24h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HourCycle {
    H12,
    H24,
}

impl HourCycle {
    /// Derives the conventional clock hour cycle for `locale`.
    pub fn from_locale(locale: &LocaleId) -> Self {
        let lang = locale.language();
        let region = locale.region();
        if matches!(
            region,
            Some("US" | "CA" | "AU" | "NZ" | "IN" | "PH" | "EG" | "SA" | "CO" | "MX")
        ) || (lang == "en" && region.is_none())
        {
            Self::H12
        } else {
            Self::H24
        }
    }
}

/// Validated wall-clock time of day (`0..=23`, `0..=59`, `0..=59`) without timezone offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl TimeOfDay {
    /// Validates and constructs a [`TimeOfDay`], returning `None` if any field is out of range.
    #[inline]
    pub const fn new(hour: u8, minute: u8, second: u8) -> Option<Self> {
        if hour < 24 && minute < 60 && second < 60 {
            Some(Self {
                hour,
                minute,
                second,
            })
        } else {
            None
        }
    }

    /// Converts 24-hour `hour` to `(1..=12, is_pm)`.
    #[inline]
    pub const fn hour_12(self) -> (u8, bool) {
        let is_pm = self.hour >= 12;
        let h = match self.hour % 12 {
            0 => 12,
            n => n,
        };
        (h, is_pm)
    }

    /// Returns a new [`TimeOfDay`] shifted by `delta_minutes` modulo 24 hours.
    #[inline]
    pub const fn add_minutes(self, delta_minutes: i32) -> Self {
        let total = ((self.hour as i32) * 60 + (self.minute as i32) + delta_minutes).rem_euclid(1440);
        Self {
            hour: (total / 60) as u8,
            minute: (total % 60) as u8,
            second: self.second,
        }
    }

    /// Toggles AM/PM by shifting 12 hours modulo 24.
    #[inline]
    pub fn toggle_meridiem(&mut self) {
        self.hour = (self.hour + 12) % 24;
    }

    /// Formats the time using `locale`'s `12h` vs `24h` convention.
    pub fn format_for_locale(self, locale: &LocaleId, include_seconds: bool) -> String {
        match HourCycle::from_locale(locale) {
            HourCycle::H24 => {
                if include_seconds {
                    format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
                } else {
                    format!("{:02}:{:02}", self.hour, self.minute)
                }
            }
            HourCycle::H12 => {
                let (h12, is_pm) = self.hour_12();
                let suffix = if is_pm { "PM" } else { "AM" };
                if include_seconds {
                    format!("{h12}:{:02}:{:02} {suffix}", self.minute, self.second)
                } else {
                    format!("{h12}:{:02} {suffix}", self.minute)
                }
            }
        }
    }
}
