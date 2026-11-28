//! Locale-aware decimal, integer, percentage, scientific, and currency formatting.

#[cfg(feature = "builtin-fallback")]
use std::fmt::Write as _;

#[cfg(feature = "builtin-fallback")]
use crate::format::number_types::{numbering_system_digits, push_digits_mapped};
pub use crate::format::number_types::{
    Currency, LocaleMetadata, MeasurementSystem, NumberFormatOptions, NumberStyle, RoundingMode,
};
#[cfg(feature = "builtin-fallback")]
use crate::locale::LocaleId;

/// Locale-specific decimal and grouping symbols used by [`crate::BuiltinI18nBackend`].
#[cfg(feature = "builtin-fallback")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberSymbols {
    pub decimal_sep: &'static str,
    pub group_sep: &'static str,
    pub primary_group: usize,
    pub secondary_group: usize,
}

#[cfg(feature = "builtin-fallback")]
impl NumberSymbols {
    /// Resolves CLDR decimal and grouping conventions for `locale` (honoring `-u-nu-*`).
    pub fn for_locale(locale: &LocaleId) -> Self {
        if matches!(locale.numbering_system(), Some("arab" | "arabext")) {
            return Self {
                decimal_sep: "\u{066B}",
                group_sep: "\u{066C}",
                primary_group: 3,
                secondary_group: 3,
            };
        }

        let lang = locale.language();
        let region = locale.region();

        // Indian numbering system (`1,23,456.78`): primary 3, secondary 2
        if region == Some("IN") && matches!(lang, "en" | "hi" | "bn" | "ta" | "te" | "mr" | "gu")
            || matches!(lang, "hi" | "bn")
        {
            return Self {
                decimal_sep: ".",
                group_sep: ",",
                primary_group: 3,
                secondary_group: 2,
            };
        }

        // Swiss locales (`1'234.56`)
        if region == Some("CH") && matches!(lang, "de" | "fr" | "it") {
            return Self {
                decimal_sep: ".",
                group_sep: "'",
                primary_group: 3,
                secondary_group: 3,
            };
        }

        // French (`1\u{202F}234,56` per CLDR 45 NNBSP)
        if lang == "fr" {
            return Self {
                decimal_sep: ",",
                group_sep: "\u{202F}",
                primary_group: 3,
                secondary_group: 3,
            };
        }

        // Space-grouped comma-decimal locales (Slavic, Nordic, Baltic)
        if matches!(
            lang,
            "pl" | "ru" | "uk" | "cs" | "sk" | "sv" | "fi" | "nb" | "nn" | "no" | "et"
        ) {
            return Self {
                decimal_sep: ",",
                group_sep: "\u{00A0}",
                primary_group: 3,
                secondary_group: 3,
            };
        }

        // Dot-grouped comma-decimal locales (Germanic/Romance/Turkish/Vietnamese/Greek)
        if matches!(
            lang,
            "de" | "nl" | "es" | "it" | "pt" | "tr" | "vi" | "el" | "da" | "id"
        ) {
            return Self {
                decimal_sep: ",",
                group_sep: ".",
                primary_group: 3,
                secondary_group: 3,
            };
        }

        // Default Anglo / East Asian / Arabic Western digits (`1,234.56`)
        Self {
            decimal_sep: ".",
            group_sep: ",",
            primary_group: 3,
            secondary_group: 3,
        }
    }
}

/// Resolves builtin regional metadata (`decimal_separator`, `grouping_separator`,
/// currency hints, and `MeasurementSystem`).
#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_locale_metadata(locale: &LocaleId) -> LocaleMetadata {
    let syms = NumberSymbols::for_locale(locale);
    let measurement_system = match locale.region() {
        Some("US" | "LR" | "MM") => MeasurementSystem::Us,
        Some("GB") => MeasurementSystem::Uk,
        _ => MeasurementSystem::Metric,
    };
    let currency_code = match locale.region() {
        Some("US") => Some("USD"),
        Some("GB") => Some("GBP"),
        Some("JP") => Some("JPY"),
        Some("CH") => Some("CHF"),
        Some("IN") => Some("INR"),
        Some("CN") => Some("CNY"),
        Some("KR") => Some("KRW"),
        Some("SE") => Some("SEK"),
        Some("CZ") => Some("CZK"),
        Some("PL") => Some("PLN"),
        Some("DE" | "FR" | "ES" | "IT" | "NL" | "AT" | "BE" | "FI" | "PT" | "IE") => Some("EUR"),
        _ => None,
    };
    let currency_symbol = currency_code
        .and_then(Currency::from_code)
        .map(|c| c.symbol().to_owned());

    LocaleMetadata {
        decimal_separator: syms.decimal_sep.to_owned(),
        grouping_separator: syms.group_sep.to_owned(),
        currency_symbol,
        currency_code: currency_code.map(str::to_owned),
        measurement_system,
    }
}

/// Formats `value` directly into `out` with zero heap allocations (when `out` has capacity).
#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_format_integer_into(locale: &LocaleId, value: i64, out: &mut String) {
    let syms = NumberSymbols::for_locale(locale);
    let digits_map = numbering_system_digits(locale.numbering_system());
    if value < 0 {
        out.push('-');
    }
    let mut buf = [0u8; 20];
    let digits = write_u64_ascii(value.unsigned_abs(), &mut buf);
    write_grouped_digits_into(digits, syms, digits_map, out);
}

#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_format_integer(locale: &LocaleId, value: i64) -> String {
    let mut out = String::with_capacity(28);
    builtin_format_integer_into(locale, value, &mut out);
    out
}

#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_format_number_into(
    locale: &LocaleId,
    value: f64,
    opts: NumberFormatOptions,
    out: &mut String,
) {
    if value.is_nan() {
        out.push_str("NaN");
        return;
    }
    if value.is_infinite() {
        if value.is_sign_negative() {
            out.push_str("-∞");
        } else {
            out.push('∞');
        }
        return;
    }

    let syms = NumberSymbols::for_locale(locale);
    let digits_map = numbering_system_digits(locale.numbering_system());
    let normalized = if value == 0.0 { 0.0 } else { value };

    match opts.style {
        NumberStyle::Scientific => {
            format_scientific_into(normalized, syms, digits_map, opts, out);
            return;
        }
        NumberStyle::Percent => {
            let scaled = normalized * 100.0;
            let eff_opts = NumberFormatOptions {
                style: NumberStyle::Decimal,
                max_fraction_digits: opts.max_fraction_digits.or(Some(0)),
                ..opts
            };
            format_decimal_into(scaled, syms, digits_map, eff_opts, out);
            if matches!(locale.language(), "de" | "fr" | "sv" | "fi" | "cs" | "sk") {
                out.push('\u{00A0}');
            }
            out.push('%');
            return;
        }
        NumberStyle::Decimal => {}
    }

    format_decimal_into(normalized, syms, digits_map, opts, out);
}

#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_format_number(
    locale: &LocaleId,
    value: f64,
    opts: NumberFormatOptions,
) -> String {
    let mut out = String::with_capacity(28);
    builtin_format_number_into(locale, value, opts, &mut out);
    out
}

#[cfg(feature = "builtin-fallback")]
fn format_decimal_into(
    normalized: f64,
    syms: NumberSymbols,
    digits_map: Option<[char; 10]>,
    opts: NumberFormatOptions,
    out: &mut String,
) {
    let negative = normalized.is_sign_negative();
    let abs = normalized.abs();
    let (int_digits, frac_digits) = format_abs_decimal_parts(abs, negative, opts);
    let is_zero = int_digits.bytes().all(|b| b == b'0')
        && frac_digits
            .as_deref()
            .is_none_or(|f| f.bytes().all(|b| b == b'0'));

    if negative && !is_zero {
        out.push('-');
    }
    if opts.use_grouping {
        write_grouped_digits_into(&int_digits, syms, digits_map, out);
    } else {
        push_digits_mapped(&int_digits, digits_map, out);
    }
    if let Some(frac) = frac_digits
        && !frac.is_empty()
    {
        out.push_str(syms.decimal_sep);
        push_digits_mapped(&frac, digits_map, out);
    }
}

#[cfg(feature = "builtin-fallback")]
fn format_scientific_into(
    normalized: f64,
    syms: NumberSymbols,
    digits_map: Option<[char; 10]>,
    opts: NumberFormatOptions,
    out: &mut String,
) {
    let negative = normalized.is_sign_negative();
    let abs = normalized.abs();
    let (mantissa, exp) = if abs == 0.0 {
        (0.0, 0i32)
    } else {
        let e = abs.log10().floor() as i32;
        (abs / 10_f64.powi(e), e)
    };
    let mant_opts = NumberFormatOptions {
        use_grouping: false,
        style: NumberStyle::Decimal,
        ..opts
    };
    let signed_mant = if negative && abs != 0.0 {
        -mantissa
    } else {
        mantissa
    };
    format_decimal_into(signed_mant, syms, digits_map, mant_opts, out);
    out.push('E');
    if exp < 0 {
        out.push('-');
    }
    let mut buf = [0u8; 20];
    let exp_str = write_u64_ascii(exp.unsigned_abs() as u64, &mut buf);
    push_digits_mapped(exp_str, digits_map, out);
}

#[cfg(feature = "builtin-fallback")]
fn format_abs_decimal_parts(
    abs: f64,
    negative: bool,
    opts: NumberFormatOptions,
) -> (String, Option<String>) {
    let raw = match opts.max_fraction_digits {
        Some(max_f) => {
            let p = (max_f as usize).min(18);
            round_decimal_digits(abs, negative, p, opts.rounding_mode)
        }
        None => {
            let s = format!("{abs}");
            if s.contains(['e', 'E']) {
                format!("{abs:.12}")
            } else {
                s
            }
        }
    };

    let (int_part, frac_opt) = match raw.split_once('.') {
        Some((i, f)) => (i.to_owned(), Some(f.to_owned())),
        None => (raw, None),
    };

    let min_f = opts.min_fraction_digits.unwrap_or(0) as usize;
    let frac_final = match frac_opt {
        Some(mut f) => {
            if opts.max_fraction_digits.is_none() || f.len() > min_f {
                while f.len() > min_f && f.ends_with('0') {
                    f.pop();
                }
            }
            while f.len() < min_f {
                f.push('0');
            }
            if f.is_empty() { None } else { Some(f) }
        }
        None => {
            if min_f > 0 {
                Some("0".repeat(min_f))
            } else {
                None
            }
        }
    };

    (int_part, frac_final)
}

/// Formats `abs` to `prec` fractional digits using `mode` with exact decimal
/// carry propagation (e.g. `999.995` -> `1000.00`).
#[cfg(feature = "builtin-fallback")]
fn round_decimal_digits(abs: f64, negative: bool, prec: usize, mode: RoundingMode) -> String {
    let work_prec = (prec + 4).min(20);
    let expanded = format!("{abs:.work_prec$}");
    let (int_str, frac_str) = expanded.split_once('.').unwrap_or((&expanded, ""));

    let mut digits: Vec<u8> = Vec::with_capacity(int_str.len() + prec + 1);
    digits.extend(int_str.bytes().map(|b| b - b'0'));
    let keep_frac = frac_str.len().min(prec);
    digits.extend(frac_str.as_bytes()[..keep_frac].iter().map(|b| b - b'0'));

    let rem = frac_str.as_bytes().get(prec..).unwrap_or(&[]);
    let first_rem = rem.first().copied().unwrap_or(b'0');
    let any_rem_nonzero = rem.iter().any(|&b| b != b'0');
    let tail_after_first = rem.get(1..).is_some_and(|s| s.iter().any(|&b| b != b'0'));
    let last_kept_odd = digits.last().is_some_and(|&d| d % 2 == 1);

    let round_up = match mode {
        RoundingMode::HalfAwayFromZero => first_rem >= b'5',
        RoundingMode::HalfEven => {
            first_rem > b'5' || (first_rem == b'5' && (tail_after_first || last_kept_odd))
        }
        RoundingMode::Floor => negative && any_rem_nonzero,
        RoundingMode::Ceiling => !negative && any_rem_nonzero,
        RoundingMode::Truncate => false,
    };

    if round_up {
        let mut carry = 1u8;
        for d in digits.iter_mut().rev() {
            let sum = *d + carry;
            if sum >= 10 {
                *d = sum - 10;
                carry = 1;
            } else {
                *d = sum;
                carry = 0;
                break;
            }
        }
        if carry > 0 {
            digits.insert(0, carry);
        }
    }

    let split_at = digits.len().saturating_sub(prec);
    let mut out = String::with_capacity(digits.len() + 2);
    for &d in &digits[..split_at] {
        out.push((b'0' + d) as char);
    }
    if prec > 0 {
        out.push('.');
        for &d in &digits[split_at..] {
            out.push((b'0' + d) as char);
        }
    }
    out
}

#[cfg(feature = "builtin-fallback")]
fn write_u64_ascii(mut v: u64, buf: &mut [u8; 20]) -> &str {
    if v == 0 {
        buf[19] = b'0';
        return std::str::from_utf8(&buf[19..]).unwrap_or("0");
    }
    let mut pos = 20usize;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    std::str::from_utf8(&buf[pos..]).unwrap_or("0")
}

#[cfg(feature = "builtin-fallback")]
fn write_grouped_digits_into(
    digits: &str,
    syms: NumberSymbols,
    digits_map: Option<[char; 10]>,
    out: &mut String,
) {
    let len = digits.len();
    if len <= syms.primary_group || syms.group_sep.is_empty() {
        push_digits_mapped(digits, digits_map, out);
        return;
    }

    let step = syms.secondary_group.max(1);
    let prefix_len = len - syms.primary_group;
    let first_group = {
        let r = prefix_len % step;
        if r == 0 { step } else { r }
    };

    push_digits_mapped(&digits[..first_group], digits_map, out);
    let mut pos = first_group;
    while pos < prefix_len {
        out.push_str(syms.group_sep);
        push_digits_mapped(&digits[pos..pos + step], digits_map, out);
        pos += step;
    }
    out.push_str(syms.group_sep);
    push_digits_mapped(&digits[prefix_len..], digits_map, out);
}

#[cfg(feature = "builtin-fallback")]
#[derive(Debug, Clone, Copy)]
enum CurrencyPlacement {
    PrefixAdjacent,
    PrefixWithSpace,
    SuffixWithSpace,
}

#[cfg(feature = "builtin-fallback")]
fn currency_placement(locale: &LocaleId, symbol: &str) -> CurrencyPlacement {
    let lang = locale.language();
    let region = locale.region();
    let is_alpha_code = symbol.bytes().all(|b| b.is_ascii_alphabetic());

    if region == Some("CH") && matches!(lang, "de" | "fr" | "it") {
        return CurrencyPlacement::PrefixWithSpace;
    }
    if matches!(
        lang,
        "de" | "fr" | "es" | "it" | "pl" | "ru" | "uk" | "cs" | "sk" | "sv" | "fi" | "vi"
    ) {
        return CurrencyPlacement::SuffixWithSpace;
    }
    if matches!(lang, "nl" | "pt") || is_alpha_code {
        return CurrencyPlacement::PrefixWithSpace;
    }
    CurrencyPlacement::PrefixAdjacent
}

#[cfg(feature = "builtin-fallback")]
fn resolve_currency_symbol<'a>(locale: &LocaleId, currency_code: &'a str) -> (&'a str, u32) {
    if let Some(known) = Currency::from_code(currency_code) {
        let sym = match (known, locale.region()) {
            (Currency::Cad, Some("CA")) | (Currency::Aud, Some("AU")) => "$",
            _ => known.symbol(),
        };
        (sym, known.minor_units())
    } else {
        (currency_code, 2)
    }
}

#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_format_currency_code(
    locale: &LocaleId,
    amount: f64,
    currency_code: &str,
) -> String {
    let (symbol, minor_units) = resolve_currency_symbol(locale, currency_code);
    let normalized = if amount == 0.0 { 0.0 } else { amount };
    let negative = normalized.is_sign_negative();
    let opts = NumberFormatOptions {
        use_grouping: true,
        min_fraction_digits: Some(minor_units),
        max_fraction_digits: Some(minor_units),
        ..NumberFormatOptions::default()
    };
    let mag_str = builtin_format_number(locale, normalized.abs(), opts);
    assemble_currency(locale, negative, &mag_str, symbol)
}

/// Formats an exact integer amount in currency minor units (e.g. cents for USD,
/// yen for JPY, fils for KWD) with zero floating-point drift.
#[cfg(feature = "builtin-fallback")]
pub fn format_currency_minor(locale: &LocaleId, minor_amount: i64, currency: Currency) -> String {
    let syms = NumberSymbols::for_locale(locale);
    let digits_map = numbering_system_digits(locale.numbering_system());
    let (symbol, minor_units) = resolve_currency_symbol(locale, currency.as_code());
    let negative = minor_amount < 0;
    let abs = minor_amount.unsigned_abs();

    let mut mag_str = String::with_capacity(28);
    let mut buf = [0u8; 20];
    if minor_units == 0 {
        let d = write_u64_ascii(abs, &mut buf);
        write_grouped_digits_into(d, syms, digits_map, &mut mag_str);
    } else {
        let scale = 10_u64.pow(minor_units);
        let whole = abs / scale;
        let frac = abs % scale;
        let d = write_u64_ascii(whole, &mut buf);
        write_grouped_digits_into(d, syms, digits_map, &mut mag_str);
        mag_str.push_str(syms.decimal_sep);
        let width = minor_units as usize;
        let mut frac_buf = String::with_capacity(width);
        let _ = write!(&mut frac_buf, "{frac:0width$}");
        push_digits_mapped(&frac_buf, digits_map, &mut mag_str);
    }

    assemble_currency(locale, negative, &mag_str, symbol)
}

#[cfg(feature = "builtin-fallback")]
fn assemble_currency(locale: &LocaleId, negative: bool, magnitude: &str, symbol: &str) -> String {
    let sign = if negative { "-" } else { "" };
    match currency_placement(locale, symbol) {
        CurrencyPlacement::PrefixAdjacent => format!("{sign}{symbol}{magnitude}"),
        CurrencyPlacement::PrefixWithSpace => format!("{sign}{symbol}\u{00A0}{magnitude}"),
        CurrencyPlacement::SuffixWithSpace => format!("{sign}{magnitude}\u{00A0}{symbol}"),
    }
}
