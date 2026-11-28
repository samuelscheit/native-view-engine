//! Browser `Intl.*` bindings for `wasm32` targets via a tiny `wasm_bindgen(inline_js = ...)` shim.
//!
//! All calls pass typed scalars/strings with direct binary FFI, reuse a single
//! bounded 64-entry FIFO cache (`_cached`) across all `Intl.*` formatters and
//! `Intl.Locale` week-info lookups so untrusted locale tags cannot cause
//! unbounded memory growth, and catch runtime errors gracefully so
//! [`crate::backend::NativeI18nBackend`] can fall back cleanly.

use wasm_bindgen::prelude::*;

use crate::format::{
    CalendarDate, CalendarTime, DateStyle, NumberFormatOptions, RelativeUnit, TimeStyle, Weekday,
};
use crate::locale::LocaleId;
use crate::plural::{PluralCategory, PluralKind, PluralOperands};

#[wasm_bindgen(inline_js = r#"
const _cache = new Map();
function _cached(key, create) {
  let v = _cache.get(key);
  if (v !== undefined) return v;
  v = create();
  if (_cache.size >= 64) {
    const first = _cache.keys().next().value;
    _cache.delete(first);
  }
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

export function nve_intl_format_number(locale, value, useGrouping, minFrac, maxFrac, styleCode, roundCode) {
  try {
    if (typeof Intl === "undefined" || typeof Intl.NumberFormat !== "function") return undefined;
    if (minFrac >= 0 && maxFrac >= 0) maxFrac = Math.max(maxFrac, minFrac);
    const key = "n|" + locale + "|" + (useGrouping ? 1 : 0) + "|" + minFrac + "|" + maxFrac + "|" + styleCode + "|" + roundCode;
    const nf = _cached(key, () => {
      const opts = { useGrouping: !!useGrouping };
      if (styleCode === 1) opts.style = "percent";
      else if (styleCode === 2) opts.notation = "scientific";
      if (minFrac >= 0) opts.minimumFractionDigits = minFrac;
      if (maxFrac >= 0) opts.maximumFractionDigits = maxFrac;
      if (roundCode === 1) opts.roundingMode = "halfEven";
      else if (roundCode === 2) opts.roundingMode = "floor";
      else if (roundCode === 3) opts.roundingMode = "ceil";
      else if (roundCode === 4) opts.roundingMode = "trunc";
      else opts.roundingMode = "halfExpand";
      try {
        return new Intl.NumberFormat(locale, opts);
      } catch {
        delete opts.roundingMode;
        return new Intl.NumberFormat(locale, opts);
      }
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
    const dt = new Date(0);
    dt.setUTCFullYear(year, month - 1, day);
    return dtf.format(dt);
  } catch {
    return undefined;
  }
}

export function nve_intl_format_time(locale, hour, minute, second, styleCode) {
  try {
    if (typeof Intl === "undefined" || typeof Intl.DateTimeFormat !== "function") return undefined;
    const style = styleCode === 0 ? "short" : "medium";
    const key = "t|" + locale + "|" + style;
    const dtf = _cached(key, () => new Intl.DateTimeFormat(locale, { timeStyle: style, timeZone: "UTC" }));
    const dt = new Date(Date.UTC(2026, 0, 1, hour, minute, second));
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
    return _cached("w|" + locale, () => {
      const loc = new Intl.Locale(locale);
      const info = typeof loc.getWeekInfo === "function" ? loc.getWeekInfo() : loc.weekInfo;
      return info && typeof info.firstDay === "number" ? info.firstDay : 0;
    });
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
        style_code: u8,
        round_code: u8,
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
    fn nve_intl_format_time(
        locale: &str,
        hour: u8,
        minute: u8,
        second: u8,
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
        .map(|v| {
            let clamped = i32::try_from(v.min(20)).unwrap_or(20);
            if min_frac >= 0 {
                clamped.max(min_frac)
            } else {
                clamped
            }
        })
        .unwrap_or(-1);
    let style_code = opts.style as u8;
    let round_code = opts.rounding_mode as u8;
    nve_intl_format_number(
        &tag,
        value,
        opts.use_grouping,
        min_frac,
        max_frac,
        style_code,
        round_code,
    )
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

pub fn format_time(locale: &LocaleId, time: CalendarTime, style: TimeStyle) -> Option<String> {
    let tag = locale.canonical_tag();
    let style_code = match style {
        TimeStyle::Short => 0,
        TimeStyle::Medium => 1,
    };
    nve_intl_format_time(&tag, time.hour, time.minute, time.second, style_code)
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
    // 1 = Monday .. 6 = Saturday, 7 = Sunday (`idx % 7` maps directly onto `ALL_FROM_SUNDAY`).
    let iso_day = nve_intl_first_day_of_week(&tag);
    (1..=7)
        .contains(&iso_day)
        .then(|| Weekday::from_sunday_index(usize::from(iso_day)))
}
