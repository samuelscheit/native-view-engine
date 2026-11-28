//! Zero-dependency Apple `CoreFoundation` native OS i18n backend (`macos` / `ios`).
//!
//! Uses system ICU data directly via `CoreFoundation.framework` with RAII
//! [`ffi::CfRef`] guards and a bounded thread-local formatter cache (zero **Rust**
//! heap allocations in steady state for cached locales and `*_into` formatting;
//! CoreFoundation itself may allocate transient CF objects internally via
//! `kCFAllocatorDefault`):
//! - `CFLocaleCreate` / `CFLocaleCopyCurrent` / `CFLocaleGetValue`
//! - `CFNumberFormatterCreate` (`Decimal`, `Currency`, `Percent`, `Scientific`,
//!   fraction digits, grouping, rounding, exact 64-bit integers)
//! - `CFDateFormatterCreate` (date & time styles in UTC, honoring `-u-ca-*`,
//!   `-u-hc-*`, and `-u-nu-*`; configures `kCFDateFormatterGregorianStartDate`
//!   to distant past when no non-Gregorian `-u-ca-*` is active so pre-`1582-10-15`
//!   dates use proleptic Gregorian arithmetic instead of ICU's historical
//!   Julian/Gregorian cutover)
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
use std::fmt::Write as _;

use crate::format::{
    CalendarDate, CalendarTime, DateStyle, LocaleMetadata, MeasurementSystem, NumberFormatOptions,
    NumberStyle, RoundingMode, TimeStyle, Weekday,
};
use crate::locale::{LocaleId, UnicodeHourCycle};

use cache::{
    IcuLocaleKey, InlineKey, NumFmtKey, get_or_insert_first_weekday, with_date_formatter,
    with_locale, with_number_formatter, with_time_formatter,
};
use ffi::{
    CF_DATE_STYLE_LONG, CF_DATE_STYLE_MEDIUM, CF_DATE_STYLE_SHORT, CF_NUMBER_FLOAT64_TYPE,
    CF_NUMBER_SINT64_TYPE, CF_NUMBER_STYLE_CURRENCY, CF_NUMBER_STYLE_DECIMAL,
    CF_NUMBER_STYLE_PERCENT, CF_NUMBER_STYLE_SCIENTIFIC, CF_ROUND_CEILING, CF_ROUND_DOWN,
    CF_ROUND_FLOOR, CF_ROUND_HALF_EVEN, CF_ROUND_HALF_UP, CFBooleanGetValue,
    CFCalendarCreateWithIdentifier, CFCalendarGetFirstWeekday, CFCalendarSetLocale,
    CFDateFormatterCreateStringWithAbsoluteTime, CFLocaleCopyCurrent, CFLocaleGetIdentifier,
    CFLocaleGetValue, CFNumberFormatterCreateStringWithValue, CfAbsoluteTime, CfIndex, CfRef,
    append_cf_string, kCFAllocatorDefault, kCFGregorianCalendar, kCFLocaleCurrencyCode,
    kCFLocaleCurrencySymbol, kCFLocaleDecimalSeparator, kCFLocaleGroupingSeparator,
    kCFLocaleMeasurementSystem, kCFLocaleUsesMetricSystem,
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
    // matching `kCFNumberFloat64Type`; `CreateStringWithValue` returns an owned `+1` ref.
    let Some(cf_str) = (unsafe {
        CfRef::from_create(CFNumberFormatterCreateStringWithValue(
            kCFAllocatorDefault,
            nf.as_ptr(),
            CF_NUMBER_FLOAT64_TYPE,
            (&raw const normalized).cast::<c_void>(),
        ))
    }) else {
        return false;
    };
    append_cf_string(&cf_str, out)
}

fn format_abs_time_with_formatter(df: &CfRef, abs_time: CfAbsoluteTime, out: &mut String) -> bool {
    // SAFETY: `df` is a valid `CFDateFormatterRef` configured with UTC timezone;
    // `CreateStringWithAbsoluteTime` returns an owned `+1` `CFStringRef`.
    let Some(cf_str) = (unsafe {
        CfRef::from_create(CFDateFormatterCreateStringWithAbsoluteTime(
            kCFAllocatorDefault,
            df.as_ptr(),
            abs_time,
        ))
    }) else {
        return false;
    };
    append_cf_string(&cf_str, out)
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
        .map(|v| {
            let clamped = v.min(20) as i8;
            if min_frac >= 0 {
                clamped.max(min_frac)
            } else {
                clamped
            }
        })
        .unwrap_or(-1);

    let key_base = NumFmtKey {
        style: cf_number_style(opts.style),
        rounding: cf_rounding_mode(opts.rounding_mode),
        min_frac,
        max_frac,
        use_grouping: opts.use_grouping,
        currency: InlineKey::empty(),
        locale: IcuLocaleKey::empty(),
    };

    with_number_formatter(locale, key_base, |nf| {
        format_f64_with_formatter(nf, value, out)
    })
}

/// Formats a signed 64-bit integer into `out` with full `i64` precision (`kCFNumberSInt64Type`).
pub(crate) fn format_integer_into(locale: &LocaleId, value: i64, out: &mut String) -> bool {
    let key_base = NumFmtKey {
        style: CF_NUMBER_STYLE_DECIMAL,
        rounding: CF_ROUND_HALF_UP,
        min_frac: 0,
        max_frac: 0,
        use_grouping: true,
        currency: InlineKey::empty(),
        locale: IcuLocaleKey::empty(),
    };

    with_number_formatter(locale, key_base, |nf| {
        // SAFETY: `nf` is a valid `CFNumberFormatterRef` and `&value` points to an `i64`
        // matching `kCFNumberSInt64Type`; `CreateStringWithValue` returns an owned `+1` ref.
        let Some(cf_str) = (unsafe {
            CfRef::from_create(CFNumberFormatterCreateStringWithValue(
                kCFAllocatorDefault,
                nf.as_ptr(),
                CF_NUMBER_SINT64_TYPE,
                (&raw const value).cast::<c_void>(),
            ))
        }) else {
            return false;
        };
        append_cf_string(&cf_str, out)
    })
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
        style: CF_NUMBER_STYLE_CURRENCY,
        rounding: CF_ROUND_HALF_UP,
        min_frac: -1,
        max_frac: -1,
        use_grouping: true,
        currency: cur_key,
        locale: IcuLocaleKey::empty(),
    };

    with_number_formatter(locale, key_base, |nf| {
        format_f64_with_formatter(nf, amount, out)
    })
}

/// Formats `date` into `out` via `CFDateFormatter` in UTC (zero timezone/DST drift),
/// honoring `-u-ca-*` calendar and `-u-nu-*` numbering system overrides.
///
/// When no non-Gregorian `-u-ca-*` override is active, `kCFDateFormatterGregorianStartDate`
/// is set to distant past so dates before `1582-10-15` format in the proleptic Gregorian
/// calendar matching [`CalendarDate`] and ECMAScript `Intl.DateTimeFormat`.
pub(crate) fn format_date_into(
    locale: &LocaleId,
    date: CalendarDate,
    style: DateStyle,
    out: &mut String,
) -> bool {
    let days_since_2001 = date.days_since_unix_epoch() - DAYS_UNIX_TO_APPLE_EPOCH;
    let abs_time = (days_since_2001 as f64) * SECONDS_PER_DAY;
    with_date_formatter(locale, cf_date_style(style), |df| {
        format_abs_time_with_formatter(df, abs_time, out)
    })
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
    with_time_formatter(locale, cf_time_style(style), |df| {
        format_abs_time_with_formatter(df, f64::from(secs), out)
    })
}

fn query_first_weekday(cf_loc: &CfRef) -> Option<Weekday> {
    // SAFETY: `kCFGregorianCalendar` is a valid CoreFoundation calendar identifier constant;
    // `CFCalendarCreateWithIdentifier` returns an owned `+1` `CFCalendarRef`.
    let cal = unsafe {
        CfRef::from_create(CFCalendarCreateWithIdentifier(
            kCFAllocatorDefault,
            kCFGregorianCalendar,
        ))
    }?;
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
    get_or_insert_first_weekday(locale, query_first_weekday)
}

fn is_valid_bcp47_u_segment(seg: &str) -> bool {
    let bytes = seg.as_bytes();
    (3..=8).contains(&bytes.len())
        && bytes[0].is_ascii_alphabetic()
        && bytes.iter().all(|b| b.is_ascii_alphanumeric())
}

fn normalize_icu_calendar_value(raw: &str) -> Option<&str> {
    if raw.eq_ignore_ascii_case("gregorian") {
        return Some("gregory");
    }
    if raw.eq_ignore_ascii_case("ethiopic-amete-alem") {
        return Some("ethioaa");
    }
    if raw.len() > 24 {
        return None;
    }
    let mut seg_count = 0usize;
    for seg in raw.split('-') {
        seg_count += 1;
        if seg_count > 3 || !is_valid_bcp47_u_segment(seg) {
            return None;
        }
    }
    (seg_count > 0).then_some(raw)
}

/// Parses a macOS/iOS `CFLocaleGetIdentifier` string (e.g.
/// `"en_US@calendar=japanese;fw=mon;hours=h23;numbers=latn;rg=uszzzz"` or
/// `"en_US_POSIX"`) into a canonical [`LocaleId`], preserving known ICU `@`
/// keywords (`calendar`, `fw`, `hours`, `numbers`) as BCP-47 `-u-` extensions
/// while ignoring unsupported ICU keywords (such as `rg` or `currency`).
pub(crate) fn parse_icu_locale_identifier(raw: &str) -> Option<LocaleId> {
    if !raw.contains('@') {
        let trimmed = raw
            .strip_suffix("_POSIX")
            .or_else(|| raw.strip_suffix("-POSIX"))
            .unwrap_or(raw);
        return LocaleId::parse(trimmed).ok();
    }

    let (base_raw, kw_str) = raw.split_once('@')?;
    let base_trimmed = base_raw
        .strip_suffix("_POSIX")
        .or_else(|| base_raw.strip_suffix("-POSIX"))
        .unwrap_or(base_raw);
    let base_loc = LocaleId::parse(base_trimmed).ok()?;

    let mut calendar: Option<&str> = None;
    let mut first_weekday: Option<Weekday> = None;
    let mut hour_cycle: Option<UnicodeHourCycle> = None;
    let mut numbering_system: Option<&str> = None;

    for entry in kw_str.split(';') {
        let Some((k, v)) = entry.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        if k.eq_ignore_ascii_case("calendar") || k.eq_ignore_ascii_case("ca") {
            if calendar.is_none() {
                calendar = normalize_icu_calendar_value(v);
            }
        } else if k.eq_ignore_ascii_case("fw") {
            if first_weekday.is_none() {
                first_weekday = Weekday::parse_bcp47(v);
            }
        } else if k.eq_ignore_ascii_case("hours") || k.eq_ignore_ascii_case("hc") {
            if hour_cycle.is_none() {
                hour_cycle = UnicodeHourCycle::parse_bcp47(v);
            }
        } else if (k.eq_ignore_ascii_case("numbers") || k.eq_ignore_ascii_case("nu"))
            && numbering_system.is_none()
            && is_valid_bcp47_u_segment(v)
        {
            numbering_system = Some(v);
        }
    }

    if calendar.is_none()
        && first_weekday.is_none()
        && hour_cycle.is_none()
        && numbering_system.is_none()
    {
        return Some(base_loc);
    }

    let mut tag = String::with_capacity(base_trimmed.len() + 32);
    let _ = base_loc.write_canonical_tag(&mut tag);
    // Avoid duplicating `-u-` if `base_loc` somehow already carried extensions.
    if base_loc.has_extensions() {
        return Some(base_loc);
    }
    tag.push_str("-u");
    if let Some(ca) = calendar {
        let _ = write!(&mut tag, "-ca-{ca}");
    }
    if let Some(fw) = first_weekday {
        let _ = write!(&mut tag, "-fw-{}", fw.as_bcp47_str());
    }
    if let Some(hc) = hour_cycle {
        let _ = write!(&mut tag, "-hc-{}", hc.as_bcp47_str());
    }
    if let Some(nu) = numbering_system {
        let _ = write!(&mut tag, "-nu-{nu}");
    }
    LocaleId::parse(&tag).ok().or(Some(base_loc))
}

/// Queries the active macOS/iOS user locale via `CFLocaleCopyCurrent`.
pub(crate) fn system_locale() -> Option<LocaleId> {
    // SAFETY: `CFLocaleCopyCurrent` returns an owned `+1` `CFLocaleRef` per the Copy rule.
    let cur = unsafe { CfRef::from_create(CFLocaleCopyCurrent()) }?;
    // SAFETY: `cur` is a valid `CFLocaleRef`; `CFLocaleGetIdentifier` returns a borrowed `CFStringRef`.
    let id_ref = unsafe { CfRef::retain_borrowed(CFLocaleGetIdentifier(cur.as_ptr())) }?;
    let mut raw = String::with_capacity(32);
    if !append_cf_string(&id_ref, &mut raw) {
        return None;
    }
    parse_icu_locale_identifier(&raw)
}

fn read_locale_string_value(cf_loc: &CfRef, key: ffi::CfStringRef) -> Option<String> {
    // SAFETY: `cf_loc` is a valid `CFLocaleRef` and `key` is a valid `CFLocaleKey` string constant;
    // `CFLocaleGetValue` returns a borrowed CFTypeRef (Get rule).
    let val = unsafe { CfRef::retain_borrowed(CFLocaleGetValue(cf_loc.as_ptr(), key)) }?;
    let mut out = String::new();
    append_cf_string(&val, &mut out).then_some(out)
}

/// Queries OS locale metadata (`decimal_separator`, `grouping_separator`, `currency_symbol`,
/// `currency_code`, `measurement_system`) via `CFLocaleGetValue`.
pub(crate) fn locale_metadata(locale: &LocaleId) -> Option<LocaleMetadata> {
    with_locale(locale, |cf_loc| {
        // SAFETY: All `kCFLocale*` symbols are standard CoreFoundation locale key constants.
        let (
            decimal_separator,
            grouping_separator,
            currency_symbol,
            currency_code,
            ms_str,
            uses_metric,
        ) = unsafe {
            let dec = read_locale_string_value(cf_loc, kCFLocaleDecimalSeparator)?;
            let grp =
                read_locale_string_value(cf_loc, kCFLocaleGroupingSeparator).unwrap_or_default();
            let cur_sym = read_locale_string_value(cf_loc, kCFLocaleCurrencySymbol);
            let cur_code = read_locale_string_value(cf_loc, kCFLocaleCurrencyCode);
            let ms = read_locale_string_value(cf_loc, kCFLocaleMeasurementSystem);
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
    })
}

#[cfg(test)]
mod tests {
    use super::parse_icu_locale_identifier;
    use crate::format::Weekday;
    use crate::locale::UnicodeHourCycle;

    #[test]
    fn parse_icu_identifier_extracts_known_keywords_and_ignores_unknown() {
        let loc = parse_icu_locale_identifier(
            "en_US@calendar=japanese;fw=mon;hours=h23;numbers=latn;rg=uszzzz",
        )
        .expect("parses ICU identifier with mixed keywords");
        assert_eq!(loc.canonical_tag(), "en-US-u-ca-japanese-fw-mon-hc-h23-nu-latn");
        assert_eq!(loc.calendar(), Some("japanese"));
        assert_eq!(loc.first_day_of_week_override(), Some(Weekday::Monday));
        assert_eq!(loc.hour_cycle(), Some(UnicodeHourCycle::H23));
        assert_eq!(loc.numbering_system(), Some("latn"));

        let greg = parse_icu_locale_identifier("de_DE@calendar=gregorian;currency=EUR;hours=h12")
            .expect("maps ICU gregorian -> BCP-47 gregory");
        assert_eq!(greg.canonical_tag(), "de-DE-u-ca-gregory-hc-h12");

        let posix = parse_icu_locale_identifier("en_US_POSIX@fw=sun;numbers=arab")
            .expect("strips _POSIX and keeps ICU keywords");
        assert_eq!(posix.canonical_tag(), "en-US-u-fw-sun-nu-arab");

        let only_unknown = parse_icu_locale_identifier("en_GB@rg=gbzzzz;measure=uksystem")
            .expect("keeps base locale when only unsupported ICU keywords exist");
        assert_eq!(only_unknown.canonical_tag(), "en-GB");

        let compound_cal =
            parse_icu_locale_identifier("ar_SA@calendar=islamic-umalqura;numbers=arab")
                .expect("supports hyphenated ICU calendars");
        assert_eq!(compound_cal.canonical_tag(), "ar-SA-u-ca-islamic-umalqura-nu-arab");

        let partial_invalid =
            parse_icu_locale_identifier("fr_FR@calendar=bad_cal_value;hours=invalid;fw=fri;fw=mon")
                .expect("skips invalid keyword values and keeps first valid value");
        assert_eq!(partial_invalid.canonical_tag(), "fr-FR-u-fw-fri");

        assert_eq!(
            parse_icu_locale_identifier("en_US_POSIX")
                .expect("plain POSIX locale")
                .canonical_tag(),
            "en-US"
        );
        assert!(parse_icu_locale_identifier("").is_none());
        assert!(parse_icu_locale_identifier("12345@hours=h23").is_none());
    }
}
