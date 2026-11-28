//! Platform-first [`I18nBackend`] abstraction with zero-vtable static dispatch.
//!
//! - On `wasm32`, [`NativeI18nBackend`] delegates directly to the browser's
//!   built-in `Intl.PluralRules`, `Intl.NumberFormat`, `Intl.DateTimeFormat`,
//!   `Intl.RelativeTimeFormat`, and cached `Intl.Locale.weekInfo` APIs (zero
//!   bytes of CLDR tables when `default-features = false`).
//! - On Apple targets (`macos` / `ios`), [`NativeI18nBackend`] delegates
//!   directly to zero-dependency `CoreFoundation.framework` (`CFLocale`,
//!   `CFNumberFormatter`, `CFDateFormatter`, `CFCalendar`) with thread-local
//!   bounded formatter caching, falling back to [`BuiltinI18nBackend`] strictly
//!   for capabilities `CoreFoundation` does not expose in its public C API
//!   (`plural_category` and `format_relative_time`).
//! - On other native targets (Linux, Windows, Android, SSR), [`NativeI18nBackend`]
//!   falls back cleanly to [`BuiltinI18nBackend`] (when `builtin-fallback` is
//!   enabled) or a minimal invariant fallback.

use crate::format::{
    CalendarDate, CalendarTime, DateStyle, LocaleMetadata, NumberFormatOptions, RelativeUnit,
    TimeStyle, Weekday,
};
use crate::locale::LocaleId;
use crate::plural::{PluralCategory, PluralKind, PluralOperands};

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple_cf;
mod fallback;
#[cfg(target_arch = "wasm32")]
mod wasm_intl;

/// Queries the active OS user locale when supported by the native platform
/// (`CFLocaleCopyCurrent` on macOS/iOS), returning `None` on platforms without
/// a native query implementation.
pub fn system_locale() -> Option<LocaleId> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        apple_cf::system_locale()
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        None
    }
}

/// Typed platform backend for CLDR pluralization, number/currency/date/time/relative
/// formatting, and locale calendar/regional metadata.
///
/// Designed for direct generic dispatch (`Translator<B = NativeI18nBackend>`)
/// with zero dynamic dispatch overhead by default.
pub trait I18nBackend {
    /// Evaluates the CLDR plural category for `locale`, `kind`, and `operands`.
    fn plural_category(
        &self,
        locale: &LocaleId,
        kind: PluralKind,
        operands: PluralOperands,
    ) -> PluralCategory;

    /// Formats a floating-point number with locale-specific grouping and decimal rules.
    fn format_number(
        &self,
        locale: &LocaleId,
        value: f64,
        opts: NumberFormatOptions,
    ) -> String;

    /// Formats a floating-point number directly into `out`.
    fn format_number_into(
        &self,
        locale: &LocaleId,
        value: f64,
        opts: NumberFormatOptions,
        out: &mut String,
    ) {
        out.push_str(&self.format_number(locale, value, opts));
    }

    /// Formats a 64-bit signed integer with locale-specific grouping.
    ///
    /// Backends may override this to preserve full 64-bit precision beyond
    /// `f64`'s 53-bit significand (`i64::MIN` / `i64::MAX`).
    fn format_integer(&self, locale: &LocaleId, value: i64) -> String {
        self.format_number(
            locale,
            value as f64,
            NumberFormatOptions {
                min_fraction_digits: Some(0),
                max_fraction_digits: Some(0),
                ..NumberFormatOptions::default()
            },
        )
    }

    /// Formats a 64-bit signed integer directly into `out`.
    fn format_integer_into(&self, locale: &LocaleId, value: i64, out: &mut String) {
        out.push_str(&self.format_integer(locale, value));
    }

    /// Formats a monetary amount using an ISO-4217 currency code (e.g. `"USD"`, `"EUR"`, `"JPY"`).
    fn format_currency(&self, locale: &LocaleId, amount: f64, currency: &str) -> String;

    /// Formats a monetary amount directly into `out`.
    fn format_currency_into(
        &self,
        locale: &LocaleId,
        amount: f64,
        currency: &str,
        out: &mut String,
    ) {
        out.push_str(&self.format_currency(locale, amount, currency));
    }

    /// Formats a Gregorian [`CalendarDate`] using `style` (`Short`, `Medium`, `Long`).
    fn format_date(&self, locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String;

    /// Formats a Gregorian [`CalendarDate`] directly into `out`.
    fn format_date_into(
        &self,
        locale: &LocaleId,
        date: CalendarDate,
        style: DateStyle,
        out: &mut String,
    ) {
        out.push_str(&self.format_date(locale, date, style));
    }

    /// Formats a wall-clock [`CalendarTime`] using `style` (`Short`, `Medium`),
    /// honoring `-u-hc-*` hour-cycle overrides.
    fn format_time(&self, locale: &LocaleId, time: CalendarTime, style: TimeStyle) -> String {
        fallback::format_time(locale, time, style)
    }

    /// Formats a wall-clock [`CalendarTime`] directly into `out`.
    fn format_time_into(
        &self,
        locale: &LocaleId,
        time: CalendarTime,
        style: TimeStyle,
        out: &mut String,
    ) {
        out.push_str(&self.format_time(locale, time, style));
    }

    /// Formats a relative time offset (negative = past, positive = future).
    fn format_relative_time(
        &self,
        locale: &LocaleId,
        value: i64,
        unit: RelativeUnit,
    ) -> String;

    /// Returns the locale's conventional first day of the week (`Sunday`, `Monday`, `Saturday`, ...).
    fn first_day_of_week(&self, locale: &LocaleId) -> Weekday;

    /// Returns resolved decimal/grouping separators, currency hints, and measurement system.
    fn locale_metadata(&self, locale: &LocaleId) -> LocaleMetadata {
        fallback::locale_metadata(locale)
    }
}

impl<B: I18nBackend + ?Sized> I18nBackend for &B {
    #[inline]
    fn plural_category(
        &self,
        locale: &LocaleId,
        kind: PluralKind,
        operands: PluralOperands,
    ) -> PluralCategory {
        (**self).plural_category(locale, kind, operands)
    }

    #[inline]
    fn format_number(
        &self,
        locale: &LocaleId,
        value: f64,
        opts: NumberFormatOptions,
    ) -> String {
        (**self).format_number(locale, value, opts)
    }

    #[inline]
    fn format_number_into(
        &self,
        locale: &LocaleId,
        value: f64,
        opts: NumberFormatOptions,
        out: &mut String,
    ) {
        (**self).format_number_into(locale, value, opts, out);
    }

    #[inline]
    fn format_integer(&self, locale: &LocaleId, value: i64) -> String {
        (**self).format_integer(locale, value)
    }

    #[inline]
    fn format_integer_into(&self, locale: &LocaleId, value: i64, out: &mut String) {
        (**self).format_integer_into(locale, value, out);
    }

    #[inline]
    fn format_currency(&self, locale: &LocaleId, amount: f64, currency: &str) -> String {
        (**self).format_currency(locale, amount, currency)
    }

    #[inline]
    fn format_currency_into(
        &self,
        locale: &LocaleId,
        amount: f64,
        currency: &str,
        out: &mut String,
    ) {
        (**self).format_currency_into(locale, amount, currency, out);
    }

    #[inline]
    fn format_date(&self, locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String {
        (**self).format_date(locale, date, style)
    }

    #[inline]
    fn format_date_into(
        &self,
        locale: &LocaleId,
        date: CalendarDate,
        style: DateStyle,
        out: &mut String,
    ) {
        (**self).format_date_into(locale, date, style, out);
    }

    #[inline]
    fn format_time(&self, locale: &LocaleId, time: CalendarTime, style: TimeStyle) -> String {
        (**self).format_time(locale, time, style)
    }

    #[inline]
    fn format_time_into(
        &self,
        locale: &LocaleId,
        time: CalendarTime,
        style: TimeStyle,
        out: &mut String,
    ) {
        (**self).format_time_into(locale, time, style, out);
    }

    #[inline]
    fn format_relative_time(
        &self,
        locale: &LocaleId,
        value: i64,
        unit: RelativeUnit,
    ) -> String {
        (**self).format_relative_time(locale, value, unit)
    }

    #[inline]
    fn first_day_of_week(&self, locale: &LocaleId) -> Weekday {
        (**self).first_day_of_week(locale)
    }

    #[inline]
    fn locale_metadata(&self, locale: &LocaleId) -> LocaleMetadata {
        (**self).locale_metadata(locale)
    }
}

/// Pure-Rust CLDR backend compiled when the `builtin-fallback` feature is active
/// (enabled by default).
///
/// Ideal for SSR, headless testing, deterministic golden snapshots, Linux, and
/// fallback when a host `Intl` API is unavailable.
#[cfg(feature = "builtin-fallback")]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinI18nBackend;

#[cfg(feature = "builtin-fallback")]
impl I18nBackend for BuiltinI18nBackend {
    #[inline]
    fn plural_category(
        &self,
        locale: &LocaleId,
        kind: PluralKind,
        operands: PluralOperands,
    ) -> PluralCategory {
        crate::plural::builtin_plural_category(locale, kind, operands)
    }

    #[inline]
    fn format_number(
        &self,
        locale: &LocaleId,
        value: f64,
        opts: NumberFormatOptions,
    ) -> String {
        crate::format::number::builtin_format_number(locale, value, opts)
    }

    #[inline]
    fn format_number_into(
        &self,
        locale: &LocaleId,
        value: f64,
        opts: NumberFormatOptions,
        out: &mut String,
    ) {
        crate::format::number::builtin_format_number_into(locale, value, opts, out);
    }

    #[inline]
    fn format_integer(&self, locale: &LocaleId, value: i64) -> String {
        crate::format::number::builtin_format_integer(locale, value)
    }

    #[inline]
    fn format_integer_into(&self, locale: &LocaleId, value: i64, out: &mut String) {
        crate::format::number::builtin_format_integer_into(locale, value, out);
    }

    #[inline]
    fn format_currency(&self, locale: &LocaleId, amount: f64, currency: &str) -> String {
        crate::format::number::builtin_format_currency_code(locale, amount, currency)
    }

    #[inline]
    fn format_date(&self, locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String {
        crate::format::calendar::builtin_format_date(locale, date, style)
    }

    #[inline]
    fn format_time(&self, locale: &LocaleId, time: CalendarTime, style: TimeStyle) -> String {
        crate::format::calendar::builtin_format_time(locale, time, style)
    }

    #[inline]
    fn format_relative_time(
        &self,
        locale: &LocaleId,
        value: i64,
        unit: RelativeUnit,
    ) -> String {
        crate::format::relative::builtin_format_relative_time(locale, value, unit)
    }

    #[inline]
    fn first_day_of_week(&self, locale: &LocaleId) -> Weekday {
        crate::format::calendar::builtin_first_day_of_week(locale)
    }

    #[inline]
    fn locale_metadata(&self, locale: &LocaleId) -> LocaleMetadata {
        crate::format::number::builtin_locale_metadata(locale)
    }
}

/// Platform-first default backend.
///
/// - On `wasm32`: delegates to browser `Intl.*` APIs via `wasm-bindgen`,
///   falling back automatically to [`BuiltinI18nBackend`] (if enabled) or an
///   invariant fallback on unsupported APIs or runtime errors.
/// - On Apple (`macos` / `ios`): delegates directly to zero-dependency
///   `CoreFoundation.framework` (`CFLocale`, `CFNumberFormatter`,
///   `CFDateFormatter`, `CFCalendar`) with bounded thread-local caching;
///   `plural_category` and `format_relative_time` fall back cleanly to
///   [`BuiltinI18nBackend`] because `CoreFoundation` does not expose
///   `CFPluralRules` or `CFRelativeDateTimeFormatter` in its public C API.
/// - On other native targets: delegates to [`BuiltinI18nBackend`] (if enabled)
///   or an invariant fallback.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NativeI18nBackend;

impl I18nBackend for NativeI18nBackend {
    fn plural_category(
        &self,
        locale: &LocaleId,
        kind: PluralKind,
        operands: PluralOperands,
    ) -> PluralCategory {
        #[cfg(target_arch = "wasm32")]
        if let Some(cat) = wasm_intl::plural_category(locale, kind, operands) {
            return cat;
        }
        // CoreFoundation does not expose `CFPluralRules`; use pure-Rust CLDR v45+ rules.
        fallback::plural_category(locale, kind, operands)
    }

    fn format_number(
        &self,
        locale: &LocaleId,
        value: f64,
        opts: NumberFormatOptions,
    ) -> String {
        let mut out = String::with_capacity(28);
        self.format_number_into(locale, value, opts, &mut out);
        out
    }

    fn format_number_into(
        &self,
        locale: &LocaleId,
        value: f64,
        opts: NumberFormatOptions,
        out: &mut String,
    ) {
        #[cfg(target_arch = "wasm32")]
        if let Some(formatted) = wasm_intl::format_number(locale, value, opts) {
            out.push_str(&formatted);
            return;
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if apple_cf::format_number_into(locale, value, opts, out) {
            return;
        }
        fallback::format_number_into(locale, value, opts, out);
    }

    fn format_integer(&self, locale: &LocaleId, value: i64) -> String {
        let mut out = String::with_capacity(28);
        self.format_integer_into(locale, value, &mut out);
        out
    }

    fn format_integer_into(&self, locale: &LocaleId, value: i64, out: &mut String) {
        #[cfg(target_arch = "wasm32")]
        if let Some(formatted) = wasm_intl::format_integer(locale, value) {
            out.push_str(&formatted);
            return;
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if apple_cf::format_integer_into(locale, value, out) {
            return;
        }
        fallback::format_integer_into(locale, value, out);
    }

    fn format_currency(&self, locale: &LocaleId, amount: f64, currency: &str) -> String {
        let mut out = String::with_capacity(32);
        self.format_currency_into(locale, amount, currency, &mut out);
        out
    }

    fn format_currency_into(
        &self,
        locale: &LocaleId,
        amount: f64,
        currency: &str,
        out: &mut String,
    ) {
        #[cfg(target_arch = "wasm32")]
        if let Some(formatted) = wasm_intl::format_currency(locale, amount, currency) {
            out.push_str(&formatted);
            return;
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if apple_cf::format_currency_into(locale, amount, currency, out) {
            return;
        }
        out.push_str(&fallback::format_currency(locale, amount, currency));
    }

    fn format_date(&self, locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String {
        let mut out = String::with_capacity(32);
        self.format_date_into(locale, date, style, &mut out);
        out
    }

    fn format_date_into(
        &self,
        locale: &LocaleId,
        date: CalendarDate,
        style: DateStyle,
        out: &mut String,
    ) {
        #[cfg(target_arch = "wasm32")]
        if let Some(formatted) = wasm_intl::format_date(locale, date, style) {
            out.push_str(&formatted);
            return;
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if apple_cf::format_date_into(locale, date, style, out) {
            return;
        }
        out.push_str(&fallback::format_date(locale, date, style));
    }

    fn format_time(&self, locale: &LocaleId, time: CalendarTime, style: TimeStyle) -> String {
        let mut out = String::with_capacity(16);
        self.format_time_into(locale, time, style, &mut out);
        out
    }

    fn format_time_into(
        &self,
        locale: &LocaleId,
        time: CalendarTime,
        style: TimeStyle,
        out: &mut String,
    ) {
        #[cfg(target_arch = "wasm32")]
        if let Some(formatted) = wasm_intl::format_time(locale, time, style) {
            out.push_str(&formatted);
            return;
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if apple_cf::format_time_into(locale, time, style, out) {
            return;
        }
        out.push_str(&fallback::format_time(locale, time, style));
    }

    fn format_relative_time(
        &self,
        locale: &LocaleId,
        value: i64,
        unit: RelativeUnit,
    ) -> String {
        #[cfg(target_arch = "wasm32")]
        if let Some(formatted) = wasm_intl::format_relative_time(locale, value, unit) {
            return formatted;
        }
        // CoreFoundation does not expose `CFRelativeDateTimeFormatter`; fall back cleanly.
        fallback::format_relative_time(locale, value, unit)
    }

    fn first_day_of_week(&self, locale: &LocaleId) -> Weekday {
        if let Some(fw) = locale.first_day_of_week_override() {
            return fw;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(day) = wasm_intl::first_day_of_week(locale) {
            return day;
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if let Some(day) = apple_cf::first_day_of_week(locale) {
            return day;
        }
        fallback::first_day_of_week(locale)
    }

    fn locale_metadata(&self, locale: &LocaleId) -> LocaleMetadata {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if let Some(meta) = apple_cf::locale_metadata(locale) {
            return meta;
        }
        fallback::locale_metadata(locale)
    }
}
