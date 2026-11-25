//! Locale-aware number, currency, calendar date, first-day-of-week, and
//! relative-time formatting.
//!
//! Top-level helper functions delegate to [`NativeI18nBackend`] (browser
//! `Intl.*` on `wasm32`, [`crate::BuiltinI18nBackend`] on native/SSR/fallback).

pub(crate) mod calendar;
#[cfg(feature = "builtin-fallback")]
pub(crate) mod calendar_data;
pub(crate) mod number;
pub(crate) mod relative;

use crate::backend::{I18nBackend, NativeI18nBackend};
use crate::locale::LocaleId;

pub use calendar::{CalendarDate, DateStyle, NameWidth, Weekday, ordered_weekdays};
#[cfg(feature = "builtin-fallback")]
pub use calendar::{month_name, weekday_name};
#[cfg(feature = "builtin-fallback")]
pub use number::{NumberSymbols, format_currency_minor};
pub use number::{Currency, NumberFormatOptions};
pub use relative::RelativeUnit;
#[cfg(feature = "builtin-fallback")]
pub use relative::format_relative_time_auto;

/// Formats a signed integer with locale grouping via [`NativeI18nBackend`].
#[inline]
pub fn format_integer(locale: &LocaleId, value: i64) -> String {
    NativeI18nBackend.format_integer(locale, value)
}

/// Formats a decimal number with default locale grouping via [`NativeI18nBackend`].
#[inline]
pub fn format_decimal(locale: &LocaleId, value: f64) -> String {
    NativeI18nBackend.format_number(locale, value, NumberFormatOptions::default())
}

/// Formats a number with explicit [`NumberFormatOptions`] via [`NativeI18nBackend`].
#[inline]
pub fn format_number(locale: &LocaleId, value: f64, opts: NumberFormatOptions) -> String {
    NativeI18nBackend.format_number(locale, value, opts)
}

/// Formats a monetary amount using an ISO-4217 [`Currency`] via [`NativeI18nBackend`].
#[inline]
pub fn format_currency(locale: &LocaleId, amount: f64, currency: Currency) -> String {
    NativeI18nBackend.format_currency(locale, amount, currency.as_code())
}

/// Formats a Gregorian [`CalendarDate`] using `style` via [`NativeI18nBackend`].
#[inline]
pub fn format_date(locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String {
    NativeI18nBackend.format_date(locale, date, style)
}

/// Returns the locale's conventional first day of the week via [`NativeI18nBackend`].
#[inline]
pub fn first_day_of_week(locale: &LocaleId) -> Weekday {
    NativeI18nBackend.first_day_of_week(locale)
}

/// Formats a relative time offset (negative = past, positive = future) via [`NativeI18nBackend`].
#[inline]
pub fn format_relative_time(locale: &LocaleId, value: i64, unit: RelativeUnit) -> String {
    NativeI18nBackend.format_relative_time(locale, value, unit)
}
