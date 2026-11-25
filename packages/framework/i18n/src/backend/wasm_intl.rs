//! Browser `Intl.*` bindings for `wasm32` targets via a tiny `wasm_bindgen(inline_js = ...)` shim.
//!
//! All calls pass typed scalars/strings with direct binary FFI and catch
//! runtime errors gracefully so [`crate::backend::NativeI18nBackend`] can fall
//! back cleanly when an `Intl` API is absent or rejects an input.

use wasm_bindgen::prelude::*;

use crate::format::{CalendarDate, DateStyle, NumberFormatOptions, RelativeUnit, Weekday};
use crate::locale::LocaleId;
use crate::plural::{PluralCategory, PluralKind, PluralOperands};

#[wasm_bindgen(inline_js = r#"
const _cache = new Map();
function _cached(key, create) {
  let v = _cache.get(key);
  if (v !== undefined) return v;
  if (_cache.size >= 64) {
    const first = _cache.keys().next().value;
    _cache.delete(first);
  }
  v = create();
  _cache.set(key, v);
  return v;
}

export function nve_intl_plural(locale, isOrdinal, value, fracDigits) {
  try {
    if (typeof Intl === "undefined" || typeof Intl.PluralRules !== "function") return 255;
    const type = isOrdinal ? "ordinal" : "cardinal";
    const key = "p|" + locale + "|" + type + "|" + fracDigits;
    const pr = _cached(key, () => {
      const opts = { type };
      if (fracDigits >= 0) {
        opts.minimumFractionDigits = fracDigits;
        opts.maximumFractionDigits = fracDigits;
      }
      return new Intl.PluralRules(locale, opts);
    });
    const cat = pr.select(Object.is(value, -0) ? 0 : value);
    switch (cat) {
      case "zero": return 0;
      case "one": return 1;
      case "two": return 2;
      case "few": return 3;
      case "many": return 4;
      case "other": return 5;
      default: return 255;
    }
  } catch {
    return 255;
  }
}

export function nve_intl_format_number(locale, value, useGrouping, minFrac, maxFrac) {
  try {
    if (typeof Intl === "undefined" || typeof Intl.NumberFormat !== "function") return undefined;
    const key = "n|" + locale + "|" + (useGrouping ? 1 : 0) + "|" + minFrac + "|" + maxFrac;
    const nf = _cached(key, () => {
      const opts = { useGrouping: !!useGrouping };
      if (minFrac >= 0) opts.minimumFractionDigits = minFrac;
      if (maxFrac >= 0) opts.maximumFractionDigits = maxFrac;
      return new Intl.NumberFormat(locale, opts);
    });
    return nf.format(Object.is(value, -0) ? 0 : value);
  } catch {
    return undefined;
  }
}

export function nve_intl_format_integer(locale, value) {
  try {
    if (typeof Intl === "undefined" || typeof Intl.NumberFormat !== "function") return undefined;
    const key = "i|" + locale;
    const nf = _cached(key, () => new Intl.NumberFormat(locale, { maximumFractionDigits: 0 }));
    return nf.format(value);
  } catch {
    return undefined;
  }
}

export function nve_intl_format_currency(locale, amount, currency) {
  try {
    if (typeof Intl === "undefined" || typeof Intl.NumberFormat !== "function") return undefined;
    const key = "c|" + locale + "|" + currency;
    const nf = _cached(key, () => new Intl.NumberFormat(locale, { style: "currency", currency }));
    return nf.format(Object.is(amount, -0) ? 0 : amount);
  } catch {
    return undefined;
  }
}

export function nve_intl_format_date(locale, year, month, day, styleCode) {
  try {
    if (typeof Intl === "undefined" || typeof Intl.DateTimeFormat !== "function") return undefined;
    const style = styleCode === 0 ? "short" : styleCode === 1 ? "medium" : "long";
    const key = "d|" + locale + "|" + style;
    const dtf = _cached(key, () => new Intl.DateTimeFormat(locale, { dateStyle: style, timeZone: "UTC" }));
    const dt = new Date(Date.UTC(year, month - 1, day));
    dt.setUTCFullYear(year);
    return dtf.format(dt);
  } catch {
    return undefined;
  }
}

export function nve_intl_format_relative(locale, value, unit) {
  try {
    if (typeof Intl === "undefined" || typeof Intl.RelativeTimeFormat !== "function") return undefined;
    const key = "r|" + locale;
    const rtf = _cached(key, () => new Intl.RelativeTimeFormat(locale, { numeric: "always", style: "long" }));
    return rtf.format(Number(value), unit);
  } catch {
    return undefined;
  }
}

export function nve_intl_first_day_of_week(locale) {
  try {
    if (typeof Intl === "undefined" || typeof Intl.Locale !== "function") return 0;
    const loc = new Intl.Locale(locale);
    const info = typeof loc.getWeekInfo === "function" ? loc.getWeekInfo() : loc.weekInfo;
    if (!info || typeof info.firstDay !== "number") return 0;
    return info.firstDay;
  } catch {
    return 0;
  }
}
"#)]
extern "C" {
    fn nve_intl_plural(locale: &str, is_ordinal: bool, value: f64, frac_digits: i32) -> u8;
    fn nve_intl_format_number(
        locale: &str,
        value: f64,
        use_grouping: bool,
        min_frac: i32,
        max_frac: i32,
    ) -> Option<String>;
    fn nve_intl_format_integer(locale: &str, value: i64) -> Option<String>;
    fn nve_intl_format_currency(locale: &str, amount: f64, currency: &str) -> Option<String>;
    fn nve_intl_format_date(
        locale: &str,
        year: i32,
        month: u8,
        day: u8,
        style_code: u8,
    ) -> Option<String>;
    fn nve_intl_format_relative(locale: &str, value: i64, unit: &str) -> Option<String>;
    fn nve_intl_first_day_of_week(locale: &str) -> u8;
}

pub fn plural_category(
    locale: &LocaleId,
    kind: PluralKind,
    operands: PluralOperands,
) -> Option<PluralCategory> {
    let tag = locale.canonical_tag();
    let is_ordinal = matches!(kind, PluralKind::Ordinal);
    let frac_digits = i32::try_from(operands.v.min(20)).unwrap_or(20);
    match nve_intl_plural(&tag, is_ordinal, operands.n, frac_digits) {
        0 => Some(PluralCategory::Zero),
        1 => Some(PluralCategory::One),
        2 => Some(PluralCategory::Two),
        3 => Some(PluralCategory::Few),
        4 => Some(PluralCategory::Many),
        5 => Some(PluralCategory::Other),
        _ => None,
    }
}

pub fn format_number(locale: &LocaleId, value: f64, opts: NumberFormatOptions) -> Option<String> {
    let tag = locale.canonical_tag();
    let min_frac = opts
        .min_fraction_digits
        .map(|v| i32::try_from(v.min(20)).unwrap_or(20))
        .unwrap_or(-1);
    let max_frac = opts
        .max_fraction_digits
        .map(|v| i32::try_from(v.min(20)).unwrap_or(20))
        .unwrap_or(-1);
    nve_intl_format_number(&tag, value, opts.use_grouping, min_frac, max_frac)
}

pub fn format_integer(locale: &LocaleId, value: i64) -> Option<String> {
    let tag = locale.canonical_tag();
    nve_intl_format_integer(&tag, value)
}

pub fn format_currency(locale: &LocaleId, amount: f64, currency: &str) -> Option<String> {
    let tag = locale.canonical_tag();
    nve_intl_format_currency(&tag, amount, currency)
}

pub fn format_date(locale: &LocaleId, date: CalendarDate, style: DateStyle) -> Option<String> {
    let tag = locale.canonical_tag();
    let style_code = match style {
        DateStyle::Short => 0,
        DateStyle::Medium => 1,
        DateStyle::Long => 2,
    };
    nve_intl_format_date(&tag, date.year, date.month, date.day, style_code)
}

pub fn format_relative_time(
    locale: &LocaleId,
    value: i64,
    unit: RelativeUnit,
) -> Option<String> {
    let tag = locale.canonical_tag();
    nve_intl_format_relative(&tag, value, unit.as_intl_str())
}

pub fn first_day_of_week(locale: &LocaleId) -> Option<Weekday> {
    let tag = locale.canonical_tag();
    // ISO 8601 weekday numbering returned by `Intl.Locale.weekInfo.firstDay`:
    // 1 = Monday .. 6 = Saturday, 7 = Sunday.
    match nve_intl_first_day_of_week(&tag) {
        1 => Some(Weekday::Monday),
        2 => Some(Weekday::Tuesday),
        3 => Some(Weekday::Wednesday),
        4 => Some(Weekday::Thursday),
        5 => Some(Weekday::Friday),
        6 => Some(Weekday::Saturday),
        7 => Some(Weekday::Sunday),
        _ => None,
    }
}
