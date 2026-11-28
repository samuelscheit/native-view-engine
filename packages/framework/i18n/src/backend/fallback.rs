//! Pure-Rust / minimal-invariant fallback dispatchers used by [`super::NativeI18nBackend`]
//! when no platform-native implementation handles a capability or when a host API fails.

#[cfg(feature = "builtin-fallback")]
use super::{BuiltinI18nBackend, I18nBackend};
use crate::format::{
    CalendarDate, CalendarTime, DateStyle, LocaleMetadata, NumberFormatOptions, RelativeUnit,
    TimeStyle, Weekday,
};
#[cfg(not(feature = "builtin-fallback"))]
use crate::format::MeasurementSystem;
use crate::locale::LocaleId;
use crate::plural::{PluralCategory, PluralKind, PluralOperands};

#[cfg(feature = "builtin-fallback")]
#[inline]
pub(super) fn plural_category(
    locale: &LocaleId,
    kind: PluralKind,
    operands: PluralOperands,
) -> PluralCategory {
    BuiltinI18nBackend.plural_category(locale, kind, operands)
}

#[cfg(not(feature = "builtin-fallback"))]
#[inline]
pub(super) fn plural_category(
    _locale: &LocaleId,
    _kind: PluralKind,
    _operands: PluralOperands,
) -> PluralCategory {
    PluralCategory::Other
}

#[cfg(feature = "builtin-fallback")]
#[inline]
pub(super) fn format_number_into(
    locale: &LocaleId,
    value: f64,
    opts: NumberFormatOptions,
    out: &mut String,
) {
    BuiltinI18nBackend.format_number_into(locale, value, opts, out);
}

#[cfg(not(feature = "builtin-fallback"))]
pub(super) fn format_number_into(
    _locale: &LocaleId,
    value: f64,
    opts: NumberFormatOptions,
    out: &mut String,
) {
    use std::fmt::Write as _;
    if value.is_nan() {
        out.push_str("NaN");
        return;
    }
    if value.is_infinite() {
        out.push_str(if value.is_sign_negative() { "-∞" } else { "∞" });
        return;
    }
    let normalized = if value == 0.0 { 0.0 } else { value };
    if let Some(max_frac) = opts.max_fraction_digits {
        let p = (max_frac as usize).min(18);
        let _ = write!(out, "{normalized:.p$}");
    } else {
        let _ = write!(out, "{normalized}");
    }
}

#[cfg(feature = "builtin-fallback")]
#[inline]
pub(super) fn format_integer_into(locale: &LocaleId, value: i64, out: &mut String) {
    BuiltinI18nBackend.format_integer_into(locale, value, out);
}

#[cfg(not(feature = "builtin-fallback"))]
#[inline]
pub(super) fn format_integer_into(_locale: &LocaleId, value: i64, out: &mut String) {
    use std::fmt::Write as _;
    let _ = write!(out, "{value}");
}

#[cfg(feature = "builtin-fallback")]
#[inline]
pub(super) fn format_currency(locale: &LocaleId, amount: f64, currency: &str) -> String {
    BuiltinI18nBackend.format_currency(locale, amount, currency)
}

#[cfg(not(feature = "builtin-fallback"))]
pub(super) fn format_currency(_locale: &LocaleId, amount: f64, currency: &str) -> String {
    let normalized = if amount == 0.0 { 0.0 } else { amount };
    format!("{currency} {normalized:.2}")
}

#[cfg(feature = "builtin-fallback")]
#[inline]
pub(super) fn format_date(locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String {
    BuiltinI18nBackend.format_date(locale, date, style)
}

#[cfg(not(feature = "builtin-fallback"))]
pub(super) fn format_date(_locale: &LocaleId, date: CalendarDate, _style: DateStyle) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

#[cfg(feature = "builtin-fallback")]
#[inline]
pub(super) fn format_time(locale: &LocaleId, time: CalendarTime, style: TimeStyle) -> String {
    BuiltinI18nBackend.format_time(locale, time, style)
}

#[cfg(not(feature = "builtin-fallback"))]
pub(super) fn format_time(_locale: &LocaleId, time: CalendarTime, style: TimeStyle) -> String {
    match style {
        TimeStyle::Short => format!("{:02}:{:02}", time.hour, time.minute),
        TimeStyle::Medium => format!("{:02}:{:02}:{:02}", time.hour, time.minute, time.second),
    }
}

#[cfg(feature = "builtin-fallback")]
#[inline]
pub(super) fn format_relative_time(
    locale: &LocaleId,
    value: i64,
    unit: RelativeUnit,
) -> String {
    BuiltinI18nBackend.format_relative_time(locale, value, unit)
}

#[cfg(not(feature = "builtin-fallback"))]
pub(super) fn format_relative_time(
    _locale: &LocaleId,
    value: i64,
    unit: RelativeUnit,
) -> String {
    let abs = value.unsigned_abs();
    let unit_str = unit.as_intl_str();
    if value < 0 {
        format!("{abs} {unit_str} ago")
    } else {
        format!("in {abs} {unit_str}")
    }
}

#[cfg(feature = "builtin-fallback")]
#[inline]
pub(super) fn first_day_of_week(locale: &LocaleId) -> Weekday {
    BuiltinI18nBackend.first_day_of_week(locale)
}

#[cfg(not(feature = "builtin-fallback"))]
#[inline]
pub(super) fn first_day_of_week(locale: &LocaleId) -> Weekday {
    locale
        .first_day_of_week_override()
        .unwrap_or(Weekday::Monday)
}

#[cfg(feature = "builtin-fallback")]
#[inline]
pub(super) fn locale_metadata(locale: &LocaleId) -> LocaleMetadata {
    BuiltinI18nBackend.locale_metadata(locale)
}

#[cfg(not(feature = "builtin-fallback"))]
pub(super) fn locale_metadata(_locale: &LocaleId) -> LocaleMetadata {
    LocaleMetadata {
        decimal_separator: ".".to_owned(),
        grouping_separator: ",".to_owned(),
        currency_symbol: None,
        currency_code: None,
        measurement_system: MeasurementSystem::Metric,
    }
}
