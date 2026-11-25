//! Platform-first [`I18nBackend`] abstraction with zero-vtable static dispatch.
//!
//! - On `wasm32`, [`NativeI18nBackend`] delegates directly to the browser's
//!   built-in `Intl.PluralRules`, `Intl.NumberFormat`, `Intl.DateTimeFormat`,
//!   `Intl.RelativeTimeFormat`, and `Intl.Locale.weekInfo` APIs (zero bytes of
//!   CLDR tables when `default-features = false`). If any browser API is
//!   missing or throws, it falls back seamlessly to [`BuiltinI18nBackend`]
//!   (when `builtin-fallback` is enabled) or a minimal invariant fallback.
//! - On native targets (`not(target_arch = "wasm32")`), [`NativeI18nBackend`]
//!   uses [`BuiltinI18nBackend`] by default, keeping a clean seam for optional
//!   platform adapters (Apple `Foundation` `NSNumberFormatter`/`NSDateFormatter`,
//!   Android `android.icu`, Windows `icu.dll`).

use crate::format::{CalendarDate, DateStyle, NumberFormatOptions, RelativeUnit, Weekday};
use crate::locale::LocaleId;
use crate::plural::{PluralCategory, PluralKind, PluralOperands};

#[cfg(target_arch = "wasm32")]
mod wasm_intl;

/// Typed platform backend for CLDR pluralization, number/currency/date/relative
/// formatting, and locale calendar metadata.
///
/// Designed for direct generic dispatch (`Translator<B = NativeI18nBackend>`)
/// with zero JSON serialization and zero dynamic dispatch overhead by default.
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

    /// Formats a monetary amount using an ISO-4217 currency code (e.g. `"USD"`, `"EUR"`, `"JPY"`).
    fn format_currency(&self, locale: &LocaleId, amount: f64, currency: &str) -> String;

    /// Formats a Gregorian [`CalendarDate`] using `style` (`Short`, `Medium`, `Long`).
    fn format_date(&self, locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String;

    /// Formats a relative time offset (negative = past, positive = future).
    fn format_relative_time(
        &self,
        locale: &LocaleId,
        value: i64,
        unit: RelativeUnit,
    ) -> String;

    /// Returns the locale's conventional first day of the week (`Sunday`, `Monday`, `Saturday`).
    fn first_day_of_week(&self, locale: &LocaleId) -> Weekday;
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
    fn format_integer(&self, locale: &LocaleId, value: i64) -> String {
        (**self).format_integer(locale, value)
    }

    #[inline]
    fn format_currency(&self, locale: &LocaleId, amount: f64, currency: &str) -> String {
        (**self).format_currency(locale, amount, currency)
    }

    #[inline]
    fn format_date(&self, locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String {
        (**self).format_date(locale, date, style)
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
    fn format_integer(&self, locale: &LocaleId, value: i64) -> String {
        crate::format::number::builtin_format_integer(locale, value)
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
}

/// Platform-first default backend.
///
/// - On `wasm32`: delegates to browser `Intl.*` APIs via `wasm-bindgen`,
///   falling back automatically to [`BuiltinI18nBackend`] (if enabled) or an
///   invariant fallback on unsupported APIs or runtime errors.
/// - On native targets: delegates to [`BuiltinI18nBackend`] (if enabled), with
///   a clean extension seam for future OS-native ICU adapters (Apple
///   `Foundation`, Android `android.icu`, Windows `icu.dll`).
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
        fallback_plural_category(locale, kind, operands)
    }

    fn format_number(
        &self,
        locale: &LocaleId,
        value: f64,
        opts: NumberFormatOptions,
    ) -> String {
        #[cfg(target_arch = "wasm32")]
        if let Some(formatted) = wasm_intl::format_number(locale, value, opts) {
            return formatted;
        }
        fallback_format_number(locale, value, opts)
    }

    fn format_integer(&self, locale: &LocaleId, value: i64) -> String {
        #[cfg(target_arch = "wasm32")]
        if let Some(formatted) = wasm_intl::format_integer(locale, value) {
            return formatted;
        }
        fallback_format_integer(locale, value)
    }

    fn format_currency(&self, locale: &LocaleId, amount: f64, currency: &str) -> String {
        #[cfg(target_arch = "wasm32")]
        if let Some(formatted) = wasm_intl::format_currency(locale, amount, currency) {
            return formatted;
        }
        fallback_format_currency(locale, amount, currency)
    }

    fn format_date(&self, locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String {
        #[cfg(target_arch = "wasm32")]
        if let Some(formatted) = wasm_intl::format_date(locale, date, style) {
            return formatted;
        }
        fallback_format_date(locale, date, style)
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
        fallback_format_relative_time(locale, value, unit)
    }

    fn first_day_of_week(&self, locale: &LocaleId) -> Weekday {
        #[cfg(target_arch = "wasm32")]
        if let Some(day) = wasm_intl::first_day_of_week(locale) {
            return day;
        }
        fallback_first_day_of_week(locale)
    }
}

#[cfg(feature = "builtin-fallback")]
#[inline]
fn fallback_plural_category(
    locale: &LocaleId,
    kind: PluralKind,
    operands: PluralOperands,
) -> PluralCategory {
    BuiltinI18nBackend.plural_category(locale, kind, operands)
}

#[cfg(not(feature = "builtin-fallback"))]
#[inline]
fn fallback_plural_category(
    _locale: &LocaleId,
    _kind: PluralKind,
    _operands: PluralOperands,
) -> PluralCategory {
    PluralCategory::Other
}

#[cfg(feature = "builtin-fallback")]
#[inline]
fn fallback_format_number(
    locale: &LocaleId,
    value: f64,
    opts: NumberFormatOptions,
) -> String {
    BuiltinI18nBackend.format_number(locale, value, opts)
}

#[cfg(not(feature = "builtin-fallback"))]
fn fallback_format_number(
    _locale: &LocaleId,
    value: f64,
    opts: NumberFormatOptions,
) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-∞".to_owned()
        } else {
            "∞".to_owned()
        };
    }
    let normalized = if value == 0.0 { 0.0 } else { value };
    if let Some(max_frac) = opts.max_fraction_digits {
        let p = (max_frac as usize).min(18);
        format!("{normalized:.p$}")
    } else {
        format!("{normalized}")
    }
}

#[cfg(feature = "builtin-fallback")]
#[inline]
fn fallback_format_integer(locale: &LocaleId, value: i64) -> String {
    BuiltinI18nBackend.format_integer(locale, value)
}

#[cfg(not(feature = "builtin-fallback"))]
#[inline]
fn fallback_format_integer(_locale: &LocaleId, value: i64) -> String {
    value.to_string()
}

#[cfg(feature = "builtin-fallback")]
#[inline]
fn fallback_format_currency(locale: &LocaleId, amount: f64, currency: &str) -> String {
    BuiltinI18nBackend.format_currency(locale, amount, currency)
}

#[cfg(not(feature = "builtin-fallback"))]
fn fallback_format_currency(_locale: &LocaleId, amount: f64, currency: &str) -> String {
    let normalized = if amount == 0.0 { 0.0 } else { amount };
    format!("{currency} {normalized:.2}")
}

#[cfg(feature = "builtin-fallback")]
#[inline]
fn fallback_format_date(locale: &LocaleId, date: CalendarDate, style: DateStyle) -> String {
    BuiltinI18nBackend.format_date(locale, date, style)
}

#[cfg(not(feature = "builtin-fallback"))]
fn fallback_format_date(_locale: &LocaleId, date: CalendarDate, _style: DateStyle) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

#[cfg(feature = "builtin-fallback")]
#[inline]
fn fallback_format_relative_time(
    locale: &LocaleId,
    value: i64,
    unit: RelativeUnit,
) -> String {
    BuiltinI18nBackend.format_relative_time(locale, value, unit)
}

#[cfg(not(feature = "builtin-fallback"))]
fn fallback_format_relative_time(
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
fn fallback_first_day_of_week(locale: &LocaleId) -> Weekday {
    BuiltinI18nBackend.first_day_of_week(locale)
}

#[cfg(not(feature = "builtin-fallback"))]
#[inline]
fn fallback_first_day_of_week(_locale: &LocaleId) -> Weekday {
    Weekday::Monday
}
