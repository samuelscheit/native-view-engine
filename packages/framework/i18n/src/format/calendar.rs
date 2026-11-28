//! Calendar date/time formatting, localized month/weekday names, and locale
//! first-day-of-week metadata for calendar and DatePicker components.

use crate::backend::{I18nBackend, NativeI18nBackend};
use crate::error::I18nError;
#[cfg(feature = "builtin-fallback")]
use crate::format::calendar_data;
#[cfg(feature = "builtin-fallback")]
use crate::format::number_types::{numbering_system_digits, push_digits_mapped};
use crate::locale::{LocaleId, UnicodeHourCycle};

/// Day of the week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    /// Canonical lowercase BCP-47 `-u-fw-*` subtag (`"sun"` .. `"sat"`).
    #[inline]
    pub const fn as_bcp47_str(self) -> &'static str {
        match self {
            Self::Sunday => "sun",
            Self::Monday => "mon",
            Self::Tuesday => "tue",
            Self::Wednesday => "wed",
            Self::Thursday => "thu",
            Self::Friday => "fri",
            Self::Saturday => "sat",
        }
    }

    /// Parses a BCP-47 `-u-fw-*` weekday subtag (case-insensitive, zero-alloc).
    #[inline]
    pub fn parse_bcp47(subtag: &str) -> Option<Self> {
        if subtag.eq_ignore_ascii_case("sun") {
            Some(Self::Sunday)
        } else if subtag.eq_ignore_ascii_case("mon") {
            Some(Self::Monday)
        } else if subtag.eq_ignore_ascii_case("tue") {
            Some(Self::Tuesday)
        } else if subtag.eq_ignore_ascii_case("wed") {
            Some(Self::Wednesday)
        } else if subtag.eq_ignore_ascii_case("thu") {
            Some(Self::Thursday)
        } else if subtag.eq_ignore_ascii_case("fri") {
            Some(Self::Friday)
        } else if subtag.eq_ignore_ascii_case("sat") {
            Some(Self::Saturday)
        } else {
            None
        }
    }
}

/// Calendar date formatting style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateStyle {
    Short,
    Medium,
    Long,
}

/// Wall-clock time formatting style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeStyle {
    Short,
    Medium,
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

    /// Days since Unix epoch (`1970-01-01 == 0`) via Howard Hinnant's civil formula.
    #[inline]
    pub const fn days_since_unix_epoch(self) -> i64 {
        let y = (self.year as i64) - if self.month <= 2 { 1 } else { 0 };
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let m = self.month as i64;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + (self.day as i64) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// Computes the exact proleptic Gregorian day of the week.
    #[inline]
    pub fn weekday(self) -> Weekday {
        let sun_idx = (self.days_since_unix_epoch() + 4).rem_euclid(7) as usize;
        Weekday::from_sunday_index(sun_idx)
    }
}

/// Validated wall-clock time (`hour 0..=23`, `minute 0..=59`, `second 0..=59`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CalendarTime {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl CalendarTime {
    /// Validates and constructs a wall-clock time.
    pub fn new(hour: u8, minute: u8, second: u8) -> Result<Self, I18nError> {
        if hour >= 24 || minute >= 60 || second >= 60 {
            return Err(I18nError::InvalidTime {
                hour,
                minute,
                second,
            });
        }
        Ok(Self {
            hour,
            minute,
            second,
        })
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

/// Resolves the effective clock hour cycle for `locale`, honoring any explicit
/// `-u-hc-*` Unicode extension override before falling back to territory defaults.
pub fn effective_hour_cycle(locale: &LocaleId) -> UnicodeHourCycle {
    if let Some(hc) = locale.hour_cycle() {
        return hc;
    }
    let lang = locale.language();
    let region = locale.region();
    if matches!(
        region,
        Some("US" | "CA" | "AU" | "NZ" | "IN" | "PH" | "EG" | "SA" | "CO" | "MX")
    ) || (lang == "en" && region.is_none())
    {
        UnicodeHourCycle::H12
    } else {
        UnicodeHourCycle::H23
    }
}

/// CLDR territory `weekData` first-day-of-week lookup, honoring explicit `-u-fw-*`.
#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_first_day_of_week(locale: &LocaleId) -> Weekday {
    if let Some(fw) = locale.first_day_of_week_override() {
        return fw;
    }
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

/// Pure-Rust CLDR date formatter used by [`crate::BuiltinI18nBackend`],
/// honoring `-u-ca-*` (`iso8601`, `buddhist`, `roc`, `japanese`, `gregory`) and `-u-nu-*`.
#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_format_date(locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String {
    let raw = format_date_raw(locale, date, style);
    let Some(digits_map) = numbering_system_digits(locale.numbering_system()) else {
        return raw;
    };
    let mut out = String::with_capacity(raw.len() * 2);
    push_digits_mapped(&raw, Some(digits_map), &mut out);
    out
}

#[cfg(feature = "builtin-fallback")]
fn japanese_era(date: CalendarDate) -> Option<(&'static str, &'static str, i32)> {
    let ymd = (date.year, date.month, date.day);
    if ymd >= (2019, 5, 1) {
        Some(("R", "令和", date.year - 2018))
    } else if ymd >= (1989, 1, 8) {
        Some(("H", "平成", date.year - 1988))
    } else if ymd >= (1926, 12, 25) {
        Some(("S", "昭和", date.year - 1925))
    } else {
        None
    }
}

#[cfg(feature = "builtin-fallback")]
fn format_date_raw(locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String {
    let lang = locale.language();
    let region = locale.region();
    let m = date.month;
    let d = date.day;
    let wide = month_name(locale, m, NameWidth::Wide);
    let abbr = month_name(locale, m, NameWidth::Abbreviated);

    match locale.calendar() {
        Some("iso8601") if style == DateStyle::Short => {
            return format!("{:04}-{m:02}-{d:02}", date.year);
        }
        Some("japanese") => {
            if let Some((short_era, ja_era, era_year)) = japanese_era(date) {
                return match (lang, style) {
                    ("ja", DateStyle::Short | DateStyle::Medium) => {
                        format!("{short_era}{era_year}/{m:02}/{d:02}")
                    }
                    ("ja", DateStyle::Long) => format!("{ja_era}{era_year}年{m}月{d}日"),
                    (_, DateStyle::Short) => format!("{m}/{d}/{era_year} {short_era}"),
                    (_, DateStyle::Medium) => format!("{abbr} {d}, {era_year} {ja_era}"),
                    (_, DateStyle::Long) => format!("{wide} {d}, {era_year} {ja_era}"),
                };
            }
        }
        Some("buddhist") => {
            let be_year = date.year + 543;
            return match style {
                DateStyle::Short => format!("{d}/{m}/{be_year} BE"),
                DateStyle::Medium => format!("{abbr} {d}, {be_year} BE"),
                DateStyle::Long => format!("{wide} {d}, {be_year} BE"),
            };
        }
        Some("roc") if date.year >= 1912 => {
            let roc_year = date.year - 1911;
            return match style {
                DateStyle::Short => format!("ROC {roc_year}/{m:02}/{d:02}"),
                DateStyle::Medium => format!("{abbr} {d}, {roc_year} Minguo"),
                DateStyle::Long => format!("{wide} {d}, {roc_year} Minguo"),
            };
        }
        _ => {}
    }

    let y = date.year;
    match (lang, region, style) {
        ("en", Some("US") | None, DateStyle::Short) => format!("{m}/{d}/{y}"),
        ("en", Some("US") | None, DateStyle::Medium) => format!("{abbr} {d}, {y}"),
        ("en", Some("US") | None, DateStyle::Long) => format!("{wide} {d}, {y}"),
        ("en", Some(_), DateStyle::Short) => format!("{d:02}/{m:02}/{y}"),
        ("en", Some(_), DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("en", Some(_), DateStyle::Long) => format!("{d} {wide} {y}"),
        ("de", _, DateStyle::Short) => format!("{d:02}.{m:02}.{y}"),
        ("de", _, DateStyle::Medium) => format!("{d}. {abbr} {y}"),
        ("de", _, DateStyle::Long) => format!("{d}. {wide} {y}"),
        ("fr" | "it", _, DateStyle::Short) => format!("{d:02}/{m:02}/{y}"),
        ("fr" | "it", _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("fr" | "it", _, DateStyle::Long) => format!("{d} {wide} {y}"),
        ("es", _, DateStyle::Short) => format!("{d}/{m}/{y}"),
        ("es", _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("es", _, DateStyle::Long) => format!("{d} de {wide} de {y}"),
        ("pt", _, DateStyle::Short) => format!("{d:02}/{m:02}/{y}"),
        ("pt", _, DateStyle::Medium) => format!("{d} de {abbr} de {y}"),
        ("pt", _, DateStyle::Long) => format!("{d} de {wide} de {y}"),
        ("ja", _, DateStyle::Short | DateStyle::Medium) => format!("{y}/{m:02}/{d:02}"),
        ("ja", _, DateStyle::Long) => format!("{y}年{m}月{d}日"),
        ("zh", _, DateStyle::Short) => format!("{y}/{m}/{d}"),
        ("zh", _, DateStyle::Medium | DateStyle::Long) => format!("{y}年{m}月{d}日"),
        ("ko", _, DateStyle::Short | DateStyle::Medium) => format!("{y}. {m}. {d}."),
        ("ko", _, DateStyle::Long) => format!("{y}년 {m}월 {d}일"),
        ("pl" | "cs" | "sk", _, DateStyle::Short) => format!("{d:02}.{m:02}.{y}"),
        ("pl" | "cs" | "sk", _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("pl" | "cs" | "sk", _, DateStyle::Long) => format!("{d} {wide} {y}"),
        ("ru" | "uk", _, DateStyle::Short) => format!("{d:02}.{m:02}.{y}"),
        ("ru", _, DateStyle::Medium) => format!("{d} {abbr} {y} г."),
        ("ru", _, DateStyle::Long) => format!("{d} {wide} {y} г."),
        ("uk", _, DateStyle::Medium) => format!("{d} {abbr} {y} р."),
        ("uk", _, DateStyle::Long) => format!("{d} {wide} {y} р."),
        ("ar", _, DateStyle::Short) => format!("{d}/{m}/{y}"),
        ("ar", _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("ar", _, DateStyle::Long) => format!("{d} {wide} {y}"),
        ("sv", _, DateStyle::Short) => format!("{y}-{m:02}-{d:02}"),
        ("sv", _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        ("sv", _, DateStyle::Long) => format!("{d} {wide} {y}"),
        (_, _, DateStyle::Short) => format!("{y:04}-{m:02}-{d:02}"),
        (_, _, DateStyle::Medium) => format!("{d} {abbr} {y}"),
        (_, _, DateStyle::Long) => format!("{d} {wide} {y}"),
    }
}

/// Pure-Rust wall-clock time formatter honoring `-u-hc-*` (`h11`/`h12`/`h23`/`h24`)
/// and `-u-nu-*` digits.
#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_format_time(locale: &LocaleId, time: CalendarTime, style: TimeStyle) -> String {
    let hc = effective_hour_cycle(locale);
    let m = time.minute;
    let s = time.second;
    let raw = match hc {
        UnicodeHourCycle::H23 => match style {
            TimeStyle::Short => format!("{:02}:{m:02}", time.hour),
            TimeStyle::Medium => format!("{:02}:{m:02}:{s:02}", time.hour),
        },
        UnicodeHourCycle::H24 => {
            let h24 = if time.hour == 0 { 24 } else { time.hour };
            match style {
                TimeStyle::Short => format!("{h24:02}:{m:02}"),
                TimeStyle::Medium => format!("{h24:02}:{m:02}:{s:02}"),
            }
        }
        UnicodeHourCycle::H11 => {
            let h11 = time.hour % 12;
            let suffix = if time.hour >= 12 { "PM" } else { "AM" };
            match style {
                TimeStyle::Short => format!("{h11}:{m:02} {suffix}"),
                TimeStyle::Medium => format!("{h11}:{m:02}:{s:02} {suffix}"),
            }
        }
        UnicodeHourCycle::H12 => {
            let h12 = match time.hour % 12 {
                0 => 12,
                n => n,
            };
            let suffix = if time.hour >= 12 { "PM" } else { "AM" };
            match style {
                TimeStyle::Short => format!("{h12}:{m:02} {suffix}"),
                TimeStyle::Medium => format!("{h12}:{m:02}:{s:02} {suffix}"),
            }
        }
    };
    let Some(digits_map) = numbering_system_digits(locale.numbering_system()) else {
        return raw;
    };
    let mut out = String::with_capacity(raw.len() * 2);
    push_digits_mapped(&raw, Some(digits_map), &mut out);
    out
}
