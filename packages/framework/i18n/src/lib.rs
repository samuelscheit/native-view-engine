//! Platform-first, `wasm32`-ready CLDR internationalization and localization
//! crate for Native View Engine (`nve-i18n`).
//!
//! # Architecture
//! - **Platform-first [`I18nBackend`]**: [`Translator<B = NativeI18nBackend>`]
//!   uses static generic dispatch (zero vtable by default). On `wasm32`,
//!   [`NativeI18nBackend`] delegates directly to browser `Intl.PluralRules`,
//!   `Intl.NumberFormat`, `Intl.DateTimeFormat`, `Intl.RelativeTimeFormat`, and
//!   cached `Intl.Locale.weekInfo` at zero CLDR data cost. On Apple (`macos` /
//!   `ios`), [`NativeI18nBackend`] calls zero-dependency `CoreFoundation.framework`
//!   (`CFLocale`, `CFNumberFormatter`, `CFDateFormatter`, `CFCalendar`) directly,
//!   falling back to [`BuiltinI18nBackend`] (when `builtin-fallback` is enabled)
//!   only for capabilities not exposed in `CoreFoundation`'s public C API.
//! - **Pure-Rust ICU MessageFormat & QA**: [`message`], [`catalog`], [`pseudo`],
//!   and [`locale`] run uniformly across Web, SSR, native desktop/mobile, and
//!   headless tests, wired generically over `B: I18nBackend`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod backend;
pub mod catalog;
pub mod error;
pub mod format;
pub mod locale;
pub mod message;
pub mod plural;
pub mod pseudo;

#[cfg(feature = "builtin-fallback")]
pub use backend::BuiltinI18nBackend;
pub use backend::{I18nBackend, NativeI18nBackend, system_locale};
pub use catalog::{MessageCatalog, Translator};
pub use error::{I18nError, MissingKeyDiagnostic};
#[cfg(feature = "builtin-fallback")]
pub use format::{
    NumberSymbols, format_currency_minor, format_relative_time_auto, month_name, weekday_name,
};
pub use format::{
    CalendarDate, CalendarTime, Currency, DateStyle, LocaleMetadata, MeasurementSystem, NameWidth,
    NumberFormatOptions, NumberStyle, RelativeUnit, RoundingMode, TimeStyle, Weekday,
    effective_hour_cycle, first_day_of_week, format_currency, format_date, format_decimal,
    format_integer, format_number, format_relative_time, format_time, locale_metadata,
    ordered_weekdays,
};
pub use locale::{
    IconMirrorPolicy, LocaleId, TextDirection, UnicodeExtensions, UnicodeHourCycle,
    should_mirror_icon,
};
pub use message::{
    ArgLookup, MAX_NESTING_DEPTH, MessageArgs, MessagePattern, MessageValue, format_message,
    format_message_into, format_message_into_with_backend, format_message_with_backend,
    parse_message,
};
pub use plural::{
    PluralCategory, PluralKind, PluralOperands, plural_category, plural_category_for_int,
};
pub use pseudo::{
    PseudoLocaleOptions, PseudoLocalizer, pseudo_localize, pseudo_localize_with_options,
};
