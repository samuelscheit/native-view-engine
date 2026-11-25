//! Calendar date formatting, localized month/weekday names, and locale
//! first-day-of-week metadata for calendar and DatePicker components.

use crate::backend::{I18nBackend, NativeI18nBackend};
use crate::error::I18nError;
#[cfg(feature = "builtin-fallback")]
use crate::format::calendar_data;
use crate::locale::LocaleId;

/// Day of the week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weekday {
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

impl Weekday {
    /// All 7 weekdays starting from Sunday (`0..=6`).
    pub const ALL_FROM_SUNDAY: [Self; 7] = [
        Self::Sunday,
        Self::Monday,
        Self::Tuesday,
        Self::Wednesday,
        Self::Thursday,
        Self::Friday,
        Self::Saturday,
    ];

    /// 0-based index with `Sunday = 0` .. `Saturday = 6`.
    #[inline]
    pub const fn index_from_sunday(self) -> usize {
        match self {
            Self::Sunday => 0,
            Self::Monday => 1,
            Self::Tuesday => 2,
            Self::Wednesday => 3,
            Self::Thursday => 4,
            Self::Friday => 5,
            Self::Saturday => 6,
        }
    }

    /// Returns the weekday corresponding to `index % 7` (`0 = Sunday`).
    #[inline]
    pub const fn from_sunday_index(idx: usize) -> Self {
        Self::ALL_FROM_SUNDAY[idx % 7]
    }
}

/// Calendar date formatting style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateStyle {
    Short,
    Medium,
    Long,
}

/// Width of localized month or weekday names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameWidth {
    Wide,
    Abbreviated,
    Narrow,
}

/// Validated proleptic Gregorian calendar date (`year`, `month 1..=12`, `day 1..=31`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CalendarDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl CalendarDate {
    /// Validates and constructs a Gregorian calendar date (accounting for leap years).
    pub fn new(year: i32, month: u8, day: u8) -> Result<Self, I18nError> {
        if !(1..=12).contains(&month) {
            return Err(I18nError::InvalidDate { year, month, day });
        }
        let max_day = days_in_month(year, month);
        if day == 0 || day > max_day {
            return Err(I18nError::InvalidDate { year, month, day });
        }
        Ok(Self { year, month, day })
    }

    /// Returns `true` if `year` is a Gregorian leap year.
    #[inline]
    pub const fn is_leap_year(year: i32) -> bool {
        (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
    }

    /// Computes the exact proleptic Gregorian day of the week using Howard
    /// Hinnant's civil-from-days formula.
    pub fn weekday(self) -> Weekday {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let m = i64::from(self.month);
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days_since_epoch = era * 146_097 + doe - 719_468; // 1970-01-01 = 0 (Thursday)
        let sun_idx = (days_since_epoch + 4).rem_euclid(7) as usize;
        Weekday::from_sunday_index(sun_idx)
    }
}

/// Returns the number of days in `month` (`1..=12`) of `year`.
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

/// Returns the 7 weekdays ordered starting from the locale's first day of the week
/// (ready for rendering DatePicker / calendar headers).
pub fn ordered_weekdays(locale: &LocaleId) -> [Weekday; 7] {
    let start = NativeI18nBackend.first_day_of_week(locale).index_from_sunday();
    [
        Weekday::from_sunday_index(start),
        Weekday::from_sunday_index(start + 1),
        Weekday::from_sunday_index(start + 2),
        Weekday::from_sunday_index(start + 3),
        Weekday::from_sunday_index(start + 4),
        Weekday::from_sunday_index(start + 5),
        Weekday::from_sunday_index(start + 6),
    ]
}

/// CLDR territory `weekData` first-day-of-week lookup.
#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_first_day_of_week(locale: &LocaleId) -> Weekday {
    if let Some(region) = locale.region() {
        return match region {
            // Saturday-start territories (CLDR weekData)
            "EG" | "AE" | "AF" | "BH" | "DJ" | "DZ" | "IQ" | "IR" | "JO" | "KW" | "LY"
            | "OM" | "QA" | "SD" | "SY" | "YE" => Weekday::Saturday,
            // Sunday-start territories (CLDR weekData)
            "US" | "CA" | "JP" | "BR" | "IL" | "TW" | "HK" | "MO" | "CN" | "KR" | "MX"
            | "PH" | "SG" | "ZA" | "IN" | "ID" | "TH" | "SA" | "CO" | "PE" | "VE" | "PK"
            | "BD" => Weekday::Sunday,
            // ISO-8601 Monday-start default (Europe, AU, NZ, AR, CL, etc.)
            _ => Weekday::Monday,
        };
    }

    match locale.language() {
        "ar" | "fa" | "ps" => Weekday::Saturday,
        "he" | "iw" | "ja" | "ko" | "zh" | "hi" | "th" | "id" => Weekday::Sunday,
        _ => Weekday::Monday,
    }
}

/// Returns the localized month name (`month` in `1..=12`, falls back cleanly if out of range).
#[cfg(feature = "builtin-fallback")]
pub fn month_name(locale: &LocaleId, month: u8, width: NameWidth) -> &'static str {
    let idx = month.saturating_sub(1) as usize;
    if idx >= 12 {
        return "";
    }
    let table = calendar_data::months_for(locale.language());
    match width {
        NameWidth::Wide => table.wide[idx],
        NameWidth::Abbreviated => table.abbr[idx],
        NameWidth::Narrow => table.narrow[idx],
    }
}

/// Returns the localized weekday name.
#[cfg(feature = "builtin-fallback")]
pub fn weekday_name(locale: &LocaleId, weekday: Weekday, width: NameWidth) -> &'static str {
    let idx = weekday.index_from_sunday();
    let table = calendar_data::weekdays_for(locale.language());
    match width {
        NameWidth::Wide => table.wide[idx],
        NameWidth::Abbreviated => table.abbr[idx],
        NameWidth::Narrow => table.narrow[idx],
    }
}

/// Pure-Rust CLDR date formatter used by [`crate::BuiltinI18nBackend`].
#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_format_date(locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String {
    let lang = locale.language();
    let region = locale.region();
    let y = date.year;
    let m = date.month;
    let d = date.day;
    let wide = month_name(locale, m, NameWidth::Wide);
    let abbr = month_name(locale, m, NameWidth::Abbreviated);

    match (lang, region, style) {
        // US English: M/D/YYYY · Mon D, YYYY · Month D, YYYY
        ("en", Some("US") | None, DateStyle::Short) => format!("{m}/{d}/{y}"),
        ("en", Some("US") | None, DateStyle::Medium) => format!("{abbr} {d}, {y}"),
        ("en", Some("US") | None, DateStyle::Long) => format!("{wide} {d}, {y}"),
        // Commonwealth English (`en-GB`, `en-AU`, `en-IN`, ...)
        ("en", Some(_), DateStyle::Short) => format!("{d:02}/{m:02}/{y}"),
        ("en", Some(_), DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("en", Some(_), DateStyle::Long) => format!("{d} {wide} {y}"),

        // German (`de-DE`, `de-AT`, `de-CH`)
        ("de", _, DateStyle::Short) => format!("{d:02}.{m:02}.{y}"),
        ("de", _, DateStyle::Medium) => format!("{d}. {abbr} {y}"),
        ("de", _, DateStyle::Long) => format!("{d}. {wide} {y}"),

        // French
        ("fr", _, DateStyle::Short) => format!("{d:02}/{m:02}/{y}"),
        ("fr", _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("fr", _, DateStyle::Long) => format!("{d} {wide} {y}"),

        // Spanish
        ("es", _, DateStyle::Short) => format!("{d}/{m}/{y}"),
        ("es", _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("es", _, DateStyle::Long) => format!("{d} de {wide} de {y}"),

        // Italian
        ("it", _, DateStyle::Short) => format!("{d:02}/{m:02}/{y}"),
        ("it", _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("it", _, DateStyle::Long) => format!("{d} {wide} {y}"),

        // Portuguese
        ("pt", _, DateStyle::Short) => format!("{d:02}/{m:02}/{y}"),
        ("pt", _, DateStyle::Medium) => format!("{d} de {abbr} de {y}"),
        ("pt", _, DateStyle::Long) => format!("{d} de {wide} de {y}"),

        // Japanese
        ("ja", _, DateStyle::Short | DateStyle::Medium) => format!("{y}/{m:02}/{d:02}"),
        ("ja", _, DateStyle::Long) => format!("{y}年{m}月{d}日"),

        // Chinese
        ("zh", _, DateStyle::Short) => format!("{y}/{m}/{d}"),
        ("zh", _, DateStyle::Medium | DateStyle::Long) => format!("{y}年{m}月{d}日"),

        // Korean
        ("ko", _, DateStyle::Short | DateStyle::Medium) => format!("{y}. {m}. {d}."),
        ("ko", _, DateStyle::Long) => format!("{y}년 {m}월 {d}일"),

        // Polish / Czech / Slovak
        ("pl" | "cs" | "sk", _, DateStyle::Short) => format!("{d:02}.{m:02}.{y}"),
        ("pl" | "cs" | "sk", _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("pl" | "cs" | "sk", _, DateStyle::Long) => format!("{d} {wide} {y}"),

        // Russian / Ukrainian
        ("ru" | "uk", _, DateStyle::Short) => format!("{d:02}.{m:02}.{y}"),
        ("ru", _, DateStyle::Medium) => format!("{d} {abbr} {y} г."),
        ("ru", _, DateStyle::Long) => format!("{d} {wide} {y} г."),
        ("uk", _, DateStyle::Medium) => format!("{d} {abbr} {y} р."),
        ("uk", _, DateStyle::Long) => format!("{d} {wide} {y} р."),

        // Arabic
        ("ar", _, DateStyle::Short) => format!("{d}/{m}/{y}"),
        ("ar", _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("ar", _, DateStyle::Long) => format!("{d} {wide} {y}"),

        // Swedish (ISO short)
        ("sv", _, DateStyle::Short) => format!("{y}-{m:02}-{d:02}"),
        ("sv", _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("sv", _, DateStyle::Long) => format!("{d} {wide} {y}"),

        // Dutch / Turkish / Vietnamese / Welsh / fallback
        (_, _, DateStyle::Short) => format!("{y:04}-{m:02}-{d:02}"),
        (_, _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        (_, _, DateStyle::Long) => format!("{d} {wide} {y}"),
    }
}
