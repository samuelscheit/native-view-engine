//! Zero-dependency Apple `CoreFoundation` native OS i18n backend (`macos` / `ios`).
//!
//! Uses system ICU data directly via `CoreFoundation.framework` with RAII
//! [`ffi::CfRef`] guards and a bounded thread-local formatter cache (zero heap
//! allocations in steady state for cached locales and `*_into` formatting):
//! - `CFLocaleCreate` / `CFLocaleCopyCurrent` / `CFLocaleGetValue`
//! - `CFNumberFormatterCreate` (`Decimal`, `Currency`, `Percent`, `Scientific`,
//!   fraction digits, grouping, rounding, exact 64-bit integers)
//! - `CFDateFormatterCreate` (date & time styles in UTC, honoring `-u-ca-*`,
//!   `-u-hc-*`, and `-u-nu-*`)
//! - `CFCalendarCreateWithIdentifier` + `CFCalendarSetLocale` + `CFCalendarGetFirstWeekday`
//!   (honoring OS ICU territory data and explicit `-u-fw-*` overrides)
//!
//! Note: `CoreFoundation` does not expose `CFRelativeDateTimeFormatter` or
//! `CFPluralRules` in its public C API, so [`crate::NativeI18nBackend`] cleanly
//! delegates `plural_category` and `format_relative_time` to
//! [`crate::BuiltinI18nBackend`].

mod cache;
mod ffi;

use std::ffi::c_void;

use crate::format::{
    CalendarDate, CalendarTime, DateStyle, LocaleMetadata, MeasurementSystem, NumberFormatOptions,
    NumberStyle, RoundingMode, TimeStyle, Weekday,
};
use crate::locale::LocaleId;

use cache::{
    CF_CACHE, DateFmtKey, IcuLocaleKey, InlineKey, NumFmtKey, create_date_formatter,
    create_number_formatter, create_uncached_locale, icu_locale_key,
};
use ffi::{
    CF_DATE_STYLE_LONG, CF_DATE_STYLE_MEDIUM, CF_DATE_STYLE_NONE, CF_DATE_STYLE_SHORT,
    CF_NUMBER_FLOAT64_TYPE, CF_NUMBER_SINT64_TYPE, CF_NUMBER_STYLE_CURRENCY,
    CF_NUMBER_STYLE_DECIMAL, CF_NUMBER_STYLE_PERCENT, CF_NUMBER_STYLE_SCIENTIFIC, CF_ROUND_CEILING,
    CF_ROUND_DOWN, CF_ROUND_FLOOR, CF_ROUND_HALF_EVEN, CF_ROUND_HALF_UP, CFBooleanGetValue,
    CFCalendarCreateWithIdentifier, CFCalendarGetFirstWeekday, CFCalendarSetLocale,
    CFDateFormatterCreateStringWithAbsoluteTime, CFLocaleCopyCurrent, CFLocaleGetIdentifier,
    CFLocaleGetValue, CFNumberFormatterCreateStringWithValue,
    CFTimeZoneCreateWithTimeIntervalFromGMT, CfAbsoluteTime, CfIndex, CfRef, append_cf_string,
    kCFAllocatorDefault, kCFGregorianCalendar, kCFLocaleCurrencyCode, kCFLocaleCurrencySymbol,
    kCFLocaleDecimalSeparator, kCFLocaleGroupingSeparator, kCFLocaleMeasurementSystem,
    kCFLocaleUsesMetricSystem,
};

/// Days between Unix epoch (`1970-01-01`) and Apple CoreFoundation reference date (`2001-01-01`).
const DAYS_UNIX_TO_APPLE_EPOCH: i64 = 11_323;
const SECONDS_PER_DAY: f64 = 86_400.0;

fn cf_rounding_mode(mode: RoundingMode) -> i32 {
    match mode {
        RoundingMode::HalfAwayFromZero => CF_ROUND_HALF_UP,
        RoundingMode::HalfEven => CF_ROUND_HALF_EVEN,
        RoundingMode::Floor => CF_ROUND_FLOOR,
        RoundingMode::Ceiling => CF_ROUND_CEILING,
        RoundingMode::Truncate => CF_ROUND_DOWN,
    }
}

fn cf_number_style(style: NumberStyle) -> CfIndex {
    match style {
        NumberStyle::Decimal => CF_NUMBER_STYLE_DECIMAL,
        NumberStyle::Percent => CF_NUMBER_STYLE_PERCENT,
        NumberStyle::Scientific => CF_NUMBER_STYLE_SCIENTIFIC,
    }
}

fn cf_date_style(style: DateStyle) -> CfIndex {
    match style {
        DateStyle::Short => CF_DATE_STYLE_SHORT,
        DateStyle::Medium => CF_DATE_STYLE_MEDIUM,
        DateStyle::Long => CF_DATE_STYLE_LONG,
    }
}

fn cf_time_style(style: TimeStyle) -> CfIndex {
    match style {
        TimeStyle::Short => CF_DATE_STYLE_SHORT,
        TimeStyle::Medium => CF_DATE_STYLE_MEDIUM,
    }
}

fn format_f64_with_formatter(nf: &CfRef, value: f64, out: &mut String) -> bool {
    let normalized = if value == 0.0 { 0.0 } else { value };
    // SAFETY: `nf` is a valid `CFNumberFormatterRef` and `&normalized` points to an `f64`
    // matching `kCFNumberFloat64Type`.
    let Some(cf_str) = CfRef::from_create(unsafe {
        CFNumberFormatterCreateStringWithValue(
            kCFAllocatorDefault,
            nf.as_ptr(),
            CF_NUMBER_FLOAT64_TYPE,
            (&raw const normalized).cast::<c_void>(),
        )
    }) else {
        return false;
    };
    append_cf_string(cf_str.as_ptr(), out)
}

/// Formats `value` into `out` via `CFNumberFormatter`, reusing cached thread-local formatters.
pub(crate) fn format_number_into(
    locale: &LocaleId,
    value: f64,
    opts: NumberFormatOptions,
    out: &mut String,
) -> bool {
    let min_frac = opts
        .min_fraction_digits
        .map(|v| v.min(20) as i8)
        .unwrap_or(-1);
    let max_frac = opts
        .max_fraction_digits
        .map(|v| v.min(20) as i8)
        .unwrap_or(-1);

    let key_base = NumFmtKey {
        locale: IcuLocaleKey::empty(),
        style: cf_number_style(opts.style),
        use_grouping: opts.use_grouping,
        min_frac,
        max_frac,
        rounding: cf_rounding_mode(opts.rounding_mode),
        currency: InlineKey::empty(),
    };

    if let Some(loc_key) = icu_locale_key(locale) {
        let key = NumFmtKey {
            locale: loc_key,
            ..key_base
        };
        return CF_CACHE.with(|cell| {
            let mut cache = cell.borrow_mut();
            let Some(nf) = cache.get_or_create_number_formatter(key) else {
                return false;
            };
            format_f64_with_formatter(&nf, value, out)
        });
    }

    let Some(cf_loc) = create_uncached_locale(locale) else {
        return false;
    };
    let Some(nf) = create_number_formatter(&cf_loc, key_base) else {
        return false;
    };
    format_f64_with_formatter(&nf, value, out)
}

/// Formats a signed 64-bit integer into `out` with full `i64` precision (`kCFNumberSInt64Type`).
pub(crate) fn format_integer_into(locale: &LocaleId, value: i64, out: &mut String) -> bool {
    let format_with = |nf: &CfRef, out: &mut String| -> bool {
        // SAFETY: `nf` is a valid `CFNumberFormatterRef` and `&value` points to an `i64`
        // matching `kCFNumberSInt64Type`.
        let Some(cf_str) = CfRef::from_create(unsafe {
            CFNumberFormatterCreateStringWithValue(
                kCFAllocatorDefault,
                nf.as_ptr(),
                CF_NUMBER_SINT64_TYPE,
                (&raw const value).cast::<c_void>(),
            )
        }) else {
            return false;
        };
        append_cf_string(cf_str.as_ptr(), out)
    };

    let key_base = NumFmtKey {
        locale: IcuLocaleKey::empty(),
        style: CF_NUMBER_STYLE_DECIMAL,
        use_grouping: true,
        min_frac: 0,
        max_frac: 0,
        rounding: CF_ROUND_HALF_UP,
        currency: InlineKey::empty(),
    };

    if let Some(loc_key) = icu_locale_key(locale) {
        let key = NumFmtKey {
            locale: loc_key,
            ..key_base
        };
        return CF_CACHE.with(|cell| {
            let mut cache = cell.borrow_mut();
            let Some(nf) = cache.get_or_create_number_formatter(key) else {
                return false;
            };
            format_with(&nf, out)
        });
    }

    let Some(cf_loc) = create_uncached_locale(locale) else {
        return false;
    };
    let Some(nf) = create_number_formatter(&cf_loc, key_base) else {
        return false;
    };
    format_with(&nf, out)
}

/// Formats a monetary amount into `out` via `CFNumberFormatter` (`kCFNumberFormatterCurrencyStyle`).
pub(crate) fn format_currency_into(
    locale: &LocaleId,
    amount: f64,
    currency: &str,
    out: &mut String,
) -> bool {
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    let Some(cur_key) = InlineKey::<8>::from_bytes_upper(currency.as_bytes()) else {
        return false;
    };
    let key_base = NumFmtKey {
        locale: IcuLocaleKey::empty(),
        style: CF_NUMBER_STYLE_CURRENCY,
        use_grouping: true,
        min_frac: -1,
        max_frac: -1,
        rounding: CF_ROUND_HALF_UP,
        currency: cur_key,
    };

    if let Some(loc_key) = icu_locale_key(locale) {
        let key = NumFmtKey {
            locale: loc_key,
            ..key_base
        };
        return CF_CACHE.with(|cell| {
            let mut cache = cell.borrow_mut();
            let Some(nf) = cache.get_or_create_number_formatter(key) else {
                return false;
            };
            format_f64_with_formatter(&nf, amount, out)
        });
    }

    let Some(cf_loc) = create_uncached_locale(locale) else {
        return false;
    };
    let Some(nf) = create_number_formatter(&cf_loc, key_base) else {
        return false;
    };
    format_f64_with_formatter(&nf, amount, out)
}

fn format_absolute_time_into(
    locale: &LocaleId,
    abs_time: CfAbsoluteTime,
    date_style: CfIndex,
    time_style: CfIndex,
    out: &mut String,
) -> bool {
    let format_with = |df: &CfRef, out: &mut String| -> bool {
        // SAFETY: `df` is a valid `CFDateFormatterRef` configured with UTC timezone.
        let Some(cf_str) = CfRef::from_create(unsafe {
            CFDateFormatterCreateStringWithAbsoluteTime(kCFAllocatorDefault, df.as_ptr(), abs_time)
        }) else {
            return false;
        };
        append_cf_string(cf_str.as_ptr(), out)
    };

    if let Some(loc_key) = icu_locale_key(locale) {
        let key = DateFmtKey {
            locale: loc_key,
            date_style,
            time_style,
        };
        return CF_CACHE.with(|cell| {
            let mut cache = cell.borrow_mut();
            let Some(df) = cache.get_or_create_date_formatter(key) else {
                return false;
            };
            format_with(&df, out)
        });
    }

    let Some(cf_loc) = create_uncached_locale(locale) else {
        return false;
    };
    // SAFETY: Creates a valid UTC timezone ref.
    let Some(utc_tz) = CfRef::from_create(unsafe {
        CFTimeZoneCreateWithTimeIntervalFromGMT(kCFAllocatorDefault, 0.0)
    }) else {
        return false;
    };
    let Some(df) = create_date_formatter(&cf_loc, &utc_tz, date_style, time_style) else {
        return false;
    };
    format_with(&df, out)
}

/// Formats `date` into `out` via `CFDateFormatter` in UTC (zero timezone/DST drift),
/// honoring `-u-ca-*` calendar and `-u-nu-*` numbering system overrides.
pub(crate) fn format_date_into(
    locale: &LocaleId,
    date: CalendarDate,
    style: DateStyle,
    out: &mut String,
) -> bool {
    let days_since_2001 = date.days_since_unix_epoch() - DAYS_UNIX_TO_APPLE_EPOCH;
    let abs_time = (days_since_2001 as f64) * SECONDS_PER_DAY;
    format_absolute_time_into(
        locale,
        abs_time,
        cf_date_style(style),
        CF_DATE_STYLE_NONE,
        out,
    )
}

/// Formats wall-clock `time` into `out` via `CFDateFormatter` in UTC,
/// honoring `-u-hc-*` hour-cycle and `-u-nu-*` numbering system overrides.
pub(crate) fn format_time_into(
    locale: &LocaleId,
    time: CalendarTime,
    style: TimeStyle,
    out: &mut String,
) -> bool {
    let secs = i32::from(time.hour) * 3600 + i32::from(time.minute) * 60 + i32::from(time.second);
    format_absolute_time_into(
        locale,
        f64::from(secs),
        CF_DATE_STYLE_NONE,
        cf_time_style(style),
        out,
    )
}

fn query_first_weekday(cf_loc: &CfRef) -> Option<Weekday> {
    // SAFETY: `kCFGregorianCalendar` is a valid CoreFoundation calendar identifier constant.
    let cal = CfRef::from_create(unsafe {
        CFCalendarCreateWithIdentifier(kCFAllocatorDefault, kCFGregorianCalendar)
    })?;
    // SAFETY: `cal` is a valid `CFCalendarRef` and `cf_loc` is a valid `CFLocaleRef`.
    unsafe {
        CFCalendarSetLocale(cal.as_ptr(), cf_loc.as_ptr());
    }
    // SAFETY: `cal` is a valid `CFCalendarRef`; returns 1 = Sunday .. 7 = Saturday.
    let idx = unsafe { CFCalendarGetFirstWeekday(cal.as_ptr()) };
    let zero_based = usize::try_from(idx.checked_sub(1)?).ok()?;
    Weekday::ALL_FROM_SUNDAY.get(zero_based).copied()
}

/// Returns `locale`'s first day of the week via `CFCalendarCreateWithIdentifier` +
/// `CFCalendarSetLocale` + `CFCalendarGetFirstWeekday`, honoring OS ICU data and `-u-fw-*`.
pub(crate) fn first_day_of_week(locale: &LocaleId) -> Option<Weekday> {
    if let Some(loc_key) = icu_locale_key(locale) {
        return CF_CACHE.with(|cell| {
            let mut cache = cell.borrow_mut();
            if let Some(&day) = cache.first_weekdays.get(&loc_key) {
                return Some(day);
            }
            let cf_loc = cache.get_or_create_locale(loc_key)?;
            let day = query_first_weekday(&cf_loc)?;
            cache.first_weekdays.insert(loc_key, day);
            Some(day)
        });
    }
    let cf_loc = create_uncached_locale(locale)?;
    query_first_weekday(&cf_loc)
}

/// Queries the active macOS/iOS user locale via `CFLocaleCopyCurrent`.
pub(crate) fn system_locale() -> Option<LocaleId> {
    // SAFETY: `CFLocaleCopyCurrent` returns an owned `+1` `CFLocaleRef` per the Copy rule.
    let cur = CfRef::from_create(unsafe { CFLocaleCopyCurrent() })?;
    // SAFETY: `cur` is a valid `CFLocaleRef`; `CFLocaleGetIdentifier` returns a borrowed `CFStringRef`.
    let id_ref = CfRef::retain_borrowed(unsafe { CFLocaleGetIdentifier(cur.as_ptr()) })?;
    let mut raw = String::with_capacity(32);
    if !append_cf_string(id_ref.as_ptr(), &mut raw) {
        return None;
    }
    if let Ok(parsed) = LocaleId::parse(&raw) {
        return Some(parsed);
    }
    let base = raw.split('@').next().unwrap_or(&raw);
    let trimmed = base.strip_suffix("_POSIX").unwrap_or(base);
    LocaleId::parse(trimmed).ok()
}

fn read_locale_string_value(cf_loc: &CfRef, key: ffi::CfStringRef) -> Option<String> {
    // SAFETY: `cf_loc` is a valid `CFLocaleRef` and `key` is a valid `CFLocaleKey` string constant.
    let borrowed = unsafe { CFLocaleGetValue(cf_loc.as_ptr(), key) };
    let val = CfRef::retain_borrowed(borrowed)?;
    let mut out = String::new();
    append_cf_string(val.as_ptr(), &mut out).then_some(out)
}

/// Queries OS locale metadata (`decimal_separator`, `grouping_separator`, `currency_symbol`,
/// `currency_code`, `measurement_system`) via `CFLocaleGetValue`.
pub(crate) fn locale_metadata(locale: &LocaleId) -> Option<LocaleMetadata> {
    let cf_loc = if let Some(loc_key) = icu_locale_key(locale) {
        CF_CACHE.with(|cell| cell.borrow_mut().get_or_create_locale(loc_key))?
    } else {
        create_uncached_locale(locale)?
    };

    // SAFETY: All `kCFLocale*` symbols are standard CoreFoundation locale key constants.
    let (decimal_separator, grouping_separator, currency_symbol, currency_code, ms_str, uses_metric) = unsafe {
        let dec = read_locale_string_value(&cf_loc, kCFLocaleDecimalSeparator)?;
        let grp = read_locale_string_value(&cf_loc, kCFLocaleGroupingSeparator).unwrap_or_default();
        let cur_sym = read_locale_string_value(&cf_loc, kCFLocaleCurrencySymbol);
        let cur_code = read_locale_string_value(&cf_loc, kCFLocaleCurrencyCode);
        let ms = read_locale_string_value(&cf_loc, kCFLocaleMeasurementSystem);
        let metric_ref = CfRef::retain_borrowed(CFLocaleGetValue(
            cf_loc.as_ptr(),
            kCFLocaleUsesMetricSystem,
        ));
        let metric = metric_ref.map(|b| CFBooleanGetValue(b.as_ptr()) != 0);
        (dec, grp, cur_sym, cur_code, ms, metric)
    };

    let measurement_system = match ms_str.as_deref() {
        Some("U.S.") => MeasurementSystem::Us,
        Some("U.K.") => MeasurementSystem::Uk,
        Some("Metric") => MeasurementSystem::Metric,
        _ if uses_metric == Some(false) => MeasurementSystem::Us,
        _ => MeasurementSystem::Metric,
    };

    Some(LocaleMetadata {
        decimal_separator,
        grouping_separator,
        currency_symbol,
        currency_code,
        measurement_system,
    })
}
