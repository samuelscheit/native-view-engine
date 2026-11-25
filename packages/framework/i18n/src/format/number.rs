//! Locale-aware decimal, integer, and currency formatting.

#[cfg(feature = "builtin-fallback")]
use crate::locale::LocaleId;

/// Formatting options for decimal numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberFormatOptions {
    /// Whether to insert locale thousands/lakh grouping separators (default `true`).
    pub use_grouping: bool,
    /// Minimum fraction digits to display (pads with trailing zeros).
    pub min_fraction_digits: Option<u32>,
    /// Maximum fraction digits to display (rounds half-away-from-zero).
    pub max_fraction_digits: Option<u32>,
}

impl Default for NumberFormatOptions {
    fn default() -> Self {
        Self {
            use_grouping: true,
            min_fraction_digits: None,
            max_fraction_digits: None,
        }
    }
}

/// Well-known ISO-4217 currencies with built-in symbol and minor-unit metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Currency {
    Usd,
    Eur,
    Gbp,
    Jpy,
    Chf,
    Inr,
    Cny,
    Krw,
    Cad,
    Aud,
    Brl,
    Pln,
    Rub,
    Sek,
    Czk,
    Try,
    Vnd,
    Aed,
    Sar,
    Kwd,
    Bhd,
}

impl Currency {
    /// Returns the uppercase 3-letter ISO-4217 code.
    pub const fn as_code(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Eur => "EUR",
            Self::Gbp => "GBP",
            Self::Jpy => "JPY",
            Self::Chf => "CHF",
            Self::Inr => "INR",
            Self::Cny => "CNY",
            Self::Krw => "KRW",
            Self::Cad => "CAD",
            Self::Aud => "AUD",
            Self::Brl => "BRL",
            Self::Pln => "PLN",
            Self::Rub => "RUB",
            Self::Sek => "SEK",
            Self::Czk => "CZK",
            Self::Try => "TRY",
            Self::Vnd => "VND",
            Self::Aed => "AED",
            Self::Sar => "SAR",
            Self::Kwd => "KWD",
            Self::Bhd => "BHD",
        }
    }

    /// Parses an ISO-4217 3-letter currency code (case-insensitive).
    pub fn from_code(code: &str) -> Option<Self> {
        let up = code.to_ascii_uppercase();
        match up.as_str() {
            "USD" => Some(Self::Usd),
            "EUR" => Some(Self::Eur),
            "GBP" => Some(Self::Gbp),
            "JPY" => Some(Self::Jpy),
            "CHF" => Some(Self::Chf),
            "INR" => Some(Self::Inr),
            "CNY" => Some(Self::Cny),
            "KRW" => Some(Self::Krw),
            "CAD" => Some(Self::Cad),
            "AUD" => Some(Self::Aud),
            "BRL" => Some(Self::Brl),
            "PLN" => Some(Self::Pln),
            "RUB" => Some(Self::Rub),
            "SEK" => Some(Self::Sek),
            "CZK" => Some(Self::Czk),
            "TRY" => Some(Self::Try),
            "VND" => Some(Self::Vnd),
            "AED" => Some(Self::Aed),
            "SAR" => Some(Self::Sar),
            "KWD" => Some(Self::Kwd),
            "BHD" => Some(Self::Bhd),
            _ => None,
        }
    }

    /// Standard ISO-4217 minor-unit decimal digits (e.g. `2` for USD/EUR, `0` for JPY/KRW/VND, `3` for KWD/BHD).
    pub const fn minor_units(self) -> u32 {
        match self {
            Self::Jpy | Self::Krw | Self::Vnd => 0,
            Self::Kwd | Self::Bhd => 3,
            _ => 2,
        }
    }

    /// Default display symbol for this currency.
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Usd => "$",
            Self::Eur => "€",
            Self::Gbp => "£",
            Self::Jpy | Self::Cny => "¥",
            Self::Chf => "CHF",
            Self::Inr => "₹",
            Self::Krw => "₩",
            Self::Cad => "CA$",
            Self::Aud => "A$",
            Self::Brl => "R$",
            Self::Pln => "zł",
            Self::Rub => "₽",
            Self::Sek => "kr",
            Self::Czk => "Kč",
            Self::Try => "₺",
            Self::Vnd => "₫",
            Self::Aed => "AED",
            Self::Sar => "SAR",
            Self::Kwd => "KWD",
            Self::Bhd => "BHD",
        }
    }
}

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
    /// Resolves CLDR decimal and grouping conventions for `locale`.
    pub fn for_locale(locale: &LocaleId) -> Self {
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
        if matches!(lang, "pl" | "ru" | "uk" | "cs" | "sk" | "sv" | "fi" | "nb" | "nn" | "no" | "et") {
            return Self {
                decimal_sep: ",",
                group_sep: "\u{00A0}",
                primary_group: 3,
                secondary_group: 3,
            };
        }

        // Dot-grouped comma-decimal locales (Germanic/Romance/Turkish/Vietnamese/Greek)
        if matches!(lang, "de" | "nl" | "es" | "it" | "pt" | "tr" | "vi" | "el" | "da" | "id") {
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

#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_format_integer(locale: &LocaleId, value: i64) -> String {
    let syms = NumberSymbols::for_locale(locale);
    let neg = value < 0;
    let abs_digits = value.unsigned_abs().to_string();
    let grouped = group_digits(&abs_digits, syms);
    if neg {
        format!("-{grouped}")
    } else {
        grouped
    }
}

#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_format_number(
    locale: &LocaleId,
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

    let syms = NumberSymbols::for_locale(locale);
    let normalized = if value == 0.0 { 0.0 } else { value };
    let negative = normalized.is_sign_negative();
    let abs = normalized.abs();

    let (int_digits, frac_digits) = format_abs_decimal_parts(abs, opts);
    let is_zero = int_digits.bytes().all(|b| b == b'0')
        && frac_digits.as_deref().is_none_or(|f| f.bytes().all(|b| b == b'0'));

    let int_out = if opts.use_grouping {
        group_digits(&int_digits, syms)
    } else {
        int_digits
    };

    let mut out = String::with_capacity(int_out.len() + 16);
    if negative && !is_zero {
        out.push('-');
    }
    out.push_str(&int_out);
    if let Some(frac) = frac_digits
        && !frac.is_empty()
    {
        out.push_str(syms.decimal_sep);
        out.push_str(&frac);
    }
    out
}

#[cfg(feature = "builtin-fallback")]
fn format_abs_decimal_parts(abs: f64, opts: NumberFormatOptions) -> (String, Option<String>) {
    let raw = match opts.max_fraction_digits {
        Some(max_f) => {
            let p = (max_f as usize).min(18);
            round_half_away_from_zero(abs, p)
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

/// Formats `abs` to `prec` fractional digits using half-away-from-zero decimal
/// rounding with exact digit carry propagation (e.g. `999.995` -> `1000.00`).
#[cfg(feature = "builtin-fallback")]
fn round_half_away_from_zero(abs: f64, prec: usize) -> String {
    // Expand with 2 extra digits to determine exact rounding digit reliably.
    let work_prec = (prec + 2).min(20);
    let expanded = format!("{abs:.work_prec$}");
    let (int_str, frac_str) = expanded.split_once('.').unwrap_or((&expanded, ""));

    let mut digits: Vec<u8> = Vec::with_capacity(int_str.len() + prec + 1);
    digits.extend(int_str.bytes().map(|b| b - b'0'));
    let keep_frac = frac_str.len().min(prec);
    digits.extend(frac_str.as_bytes()[..keep_frac].iter().map(|b| b - b'0'));

    let round_up = frac_str
        .as_bytes()
        .get(prec)
        .is_some_and(|&b| b >= b'5');

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
fn group_digits(digits: &str, syms: NumberSymbols) -> String {
    let len = digits.len();
    if len <= syms.primary_group || syms.group_sep.is_empty() {
        return digits.to_owned();
    }

    let mut groups: Vec<&str> = Vec::new();
    let mut end = len;
    let first_start = end.saturating_sub(syms.primary_group);
    groups.push(&digits[first_start..end]);
    end = first_start;

    let step = syms.secondary_group.max(1);
    while end > 0 {
        let start = end.saturating_sub(step);
        groups.push(&digits[start..end]);
        end = start;
    }

    groups.reverse();
    groups.join(syms.group_sep)
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
    };
    let mag_str = builtin_format_number(locale, normalized.abs(), opts);
    assemble_currency(locale, negative, &mag_str, symbol)
}

/// Formats an exact integer amount in currency minor units (e.g. cents for USD,
/// yen for JPY, fils for KWD) with zero floating-point drift.
#[cfg(feature = "builtin-fallback")]
pub fn format_currency_minor(locale: &LocaleId, minor_amount: i64, currency: Currency) -> String {
    let syms = NumberSymbols::for_locale(locale);
    let (symbol, minor_units) = resolve_currency_symbol(locale, currency.as_code());
    let negative = minor_amount < 0;
    let abs = minor_amount.unsigned_abs();

    let mag_str = if minor_units == 0 {
        group_digits(&abs.to_string(), syms)
    } else {
        let scale = 10_u64.pow(minor_units);
        let whole = abs / scale;
        let frac = abs % scale;
        let whole_grouped = group_digits(&whole.to_string(), syms);
        let width = minor_units as usize;
        format!("{whole_grouped}{}{frac:0width$}", syms.decimal_sep)
    };

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
