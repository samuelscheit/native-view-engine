//! CLDR v45+ cardinal and ordinal plural rules and exact [`PluralOperands`]
//! (`n`, `i`, `v`, `w`, `f`, `t`, `e`).

use std::fmt;

use crate::backend::{I18nBackend, NativeI18nBackend};
use crate::error::I18nError;
use crate::locale::LocaleId;

/// CLDR plural category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluralCategory {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl PluralCategory {
    /// All CLDR plural categories in canonical order.
    pub const ALL: [Self; 6] = [
        Self::Zero,
        Self::One,
        Self::Two,
        Self::Few,
        Self::Many,
        Self::Other,
    ];

    /// Returns the canonical CLDR keyword (`"zero"`, `"one"`, `"two"`, `"few"`, `"many"`, `"other"`).
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::One => "one",
            Self::Two => "two",
            Self::Few => "few",
            Self::Many => "many",
            Self::Other => "other",
        }
    }

    /// Parses a CLDR plural category keyword.
    #[inline]
    pub fn parse(keyword: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.as_str() == keyword)
    }
}

impl fmt::Display for PluralCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether to evaluate cardinal (`"1 item"`, `"2 items"`) or ordinal (`"1st"`, `"2nd"`) rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluralKind {
    Cardinal,
    Ordinal,
}

/// Exact CLDR LDML plural operands (UTS #35 Section 5.1.1).
///
/// Retaining visible fractional digit counts (`v`, `w`) distinguishes integer
/// `"1"` (`v = 0`, English `One`) from decimal `"1.0"` (`v = 1`, English `Other`)
/// and fractional Polish/Russian decimals (`v > 0`, `Other`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PluralOperands {
    /// Absolute value of the source number (`n`).
    pub n: f64,
    /// Integer digits of `n` (`i`).
    pub i: u64,
    /// Number of visible fraction digits in `n`, with trailing zeros (`v`).
    pub v: u32,
    /// Number of visible fraction digits in `n`, without trailing zeros (`w`).
    pub w: u32,
    /// Visible fractional digits in `n`, with trailing zeros (`f`).
    pub f: u64,
    /// Visible fractional digits in `n`, without trailing zeros (`t`).
    pub t: u64,
    /// Compact decimal exponent value (`e` / `c` in CLDR 40+).
    pub e: u32,
}

impl PluralOperands {
    /// Constructs exact operands for a signed integer (`v = w = f = t = e = 0`).
    #[inline]
    pub const fn from_int(value: i64) -> Self {
        Self::from_u64(value.unsigned_abs())
    }

    /// Constructs exact operands for an unsigned integer (`v = w = f = t = e = 0`).
    #[inline]
    pub const fn from_u64(value: u64) -> Self {
        Self {
            n: value as f64,
            i: value,
            v: 0,
            w: 0,
            f: 0,
            t: 0,
            e: 0,
        }
    }

    /// Constructs operands from a finite floating-point number using its
    /// shortest decimal representation (e.g. `1.5` -> `v = 1`, `1.0` -> `v = 0`).
    ///
    /// Use [`Self::from_f64_with_precision`] or [`Self::parse`] when visible
    /// trailing zeros such as `"1.0"` must be preserved (`v = 1`).
    pub fn from_f64(value: f64) -> Result<Self, I18nError> {
        if !value.is_finite() {
            return Err(I18nError::InvalidNumber {
                input: value.to_string(),
            });
        }
        let abs = value.abs();
        let text = format!("{abs}");
        Self::parse(&text)
    }

    /// Constructs operands from a finite floating-point number formatted with
    /// an exact number of visible fractional digits (preserving trailing zeros).
    pub fn from_f64_with_precision(value: f64, fraction_digits: u32) -> Result<Self, I18nError> {
        if !value.is_finite() {
            return Err(I18nError::InvalidNumber {
                input: value.to_string(),
            });
        }
        let prec = (fraction_digits as usize).min(18);
        let abs = value.abs();
        let text = format!("{abs:.prec$}");
        Self::parse(&text)
    }

    /// Parses exact CLDR operands from a decimal string (e.g. `"0"`, `"-1"`,
    /// `"1.0"`, `"1.20"`, `"1000000"`, `"1c6"`, `"1e6"`).
    pub fn parse(input: &str) -> Result<Self, I18nError> {
        let s = input.trim();
        if s.is_empty() {
            return Err(I18nError::InvalidNumber {
                input: input.to_owned(),
            });
        }
        let unsigned = s
            .strip_prefix('-')
            .or_else(|| s.strip_prefix('+'))
            .unwrap_or(s);

        let (mantissa, exp) = if let Some(pos) = unsigned.find(['e', 'E', 'c', 'C']) {
            let exp_str = &unsigned[pos + 1..];
            let exp_unsigned = exp_str.strip_prefix('+').unwrap_or(exp_str);
            if exp_unsigned.is_empty() || !exp_unsigned.bytes().all(|b| b.is_ascii_digit()) {
                return Err(I18nError::InvalidNumber {
                    input: input.to_owned(),
                });
            }
            let e = exp_unsigned.parse::<u32>().map_err(|_| I18nError::InvalidNumber {
                input: input.to_owned(),
            })?;
            (&unsigned[..pos], e)
        } else {
            (unsigned, 0)
        };

        let (int_str, frac_str) = match mantissa.split_once('.') {
            Some((int_part, frac_part)) => {
                if frac_part.is_empty() || frac_part.contains('.') {
                    return Err(I18nError::InvalidNumber {
                        input: input.to_owned(),
                    });
                }
                (int_part, Some(frac_part))
            }
            None => (mantissa, None),
        };

        if int_str.is_empty() || !int_str.bytes().all(|b| b.is_ascii_digit()) {
            return Err(I18nError::InvalidNumber {
                input: input.to_owned(),
            });
        }
        if let Some(frac) = frac_str
            && !frac.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(I18nError::InvalidNumber {
                input: input.to_owned(),
            });
        }

        let i = parse_clamped_u64(int_str);
        let (v, w, f, t) = if let Some(frac) = frac_str {
            let v = u32::try_from(frac.len()).unwrap_or(u32::MAX);
            let trimmed = frac.trim_end_matches('0');
            let w = u32::try_from(trimmed.len()).unwrap_or(u32::MAX);
            let f = parse_clamped_u64(frac);
            let t = if trimmed.is_empty() {
                0
            } else {
                parse_clamped_u64(trimmed)
            };
            (v, w, f, t)
        } else {
            (0, 0, 0, 0)
        };

        let base_n = mantissa.parse::<f64>().map_err(|_| I18nError::InvalidNumber {
            input: input.to_owned(),
        })?;
        if !base_n.is_finite() {
            return Err(I18nError::InvalidNumber {
                input: input.to_owned(),
            });
        }
        let n = if exp == 0 {
            base_n
        } else {
            let scaled = base_n * 10_f64.powi(exp.min(300) as i32);
            if !scaled.is_finite() {
                return Err(I18nError::InvalidNumber {
                    input: input.to_owned(),
                });
            }
            scaled
        };

        Ok(Self {
            n,
            i,
            v,
            w,
            f,
            t,
            e: exp,
        })
    }

    /// Returns `true` when `n` has no non-zero fractional part and no compact exponent.
    #[inline]
    pub const fn is_integer_value(self) -> bool {
        self.w == 0 && self.e == 0
    }

    /// CLDR 45+ Romance `many` rule:
    /// `e = 0 and i != 0 and i % 1000000 = 0 and v = 0 or e != 0..5`.
    #[cfg(feature = "builtin-fallback")]
    #[inline]
    const fn is_cldr_millions_many(self) -> bool {
        (self.e == 0 && self.i != 0 && self.i.is_multiple_of(1_000_000) && self.v == 0)
            || self.e >= 6
    }
}

impl From<i64> for PluralOperands {
    #[inline]
    fn from(value: i64) -> Self {
        Self::from_int(value)
    }
}

impl From<i32> for PluralOperands {
    #[inline]
    fn from(value: i32) -> Self {
        Self::from_int(i64::from(value))
    }
}

impl From<u64> for PluralOperands {
    #[inline]
    fn from(value: u64) -> Self {
        Self::from_u64(value)
    }
}

impl From<usize> for PluralOperands {
    #[inline]
    fn from(value: usize) -> Self {
        Self::from_u64(value as u64)
    }
}

fn parse_clamped_u64(digits: &str) -> u64 {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        return 0;
    }
    if let Ok(val) = trimmed.parse::<u64>() {
        return val;
    }
    let tail = &trimmed[trimmed.len().saturating_sub(18)..];
    tail.parse::<u64>().unwrap_or(u64::MAX)
}

/// Evaluates the CLDR plural category using [`NativeI18nBackend`].
#[inline]
pub fn plural_category(
    locale: &LocaleId,
    kind: PluralKind,
    operands: PluralOperands,
) -> PluralCategory {
    NativeI18nBackend.plural_category(locale, kind, operands)
}

/// Convenience helper for integer values using [`NativeI18nBackend`].
#[inline]
pub fn plural_category_for_int(
    locale: &LocaleId,
    kind: PluralKind,
    value: i64,
) -> PluralCategory {
    plural_category(locale, kind, PluralOperands::from_int(value))
}

#[cfg(feature = "builtin-fallback")]
pub(crate) fn builtin_plural_category(
    locale: &LocaleId,
    kind: PluralKind,
    operands: PluralOperands,
) -> PluralCategory {
    match kind {
        PluralKind::Cardinal => cardinal_category(locale, operands),
        PluralKind::Ordinal => ordinal_category(locale, operands),
    }
}

#[cfg(feature = "builtin-fallback")]
fn cardinal_category(locale: &LocaleId, op: PluralOperands) -> PluralCategory {
    let lang = locale.language();
    let region = locale.region();

    match lang {
        // Germanic & standard `i = 1 and v = 0` languages
        "en" | "de" | "nl" | "sv" | "da" | "fi" | "nb" | "nn" | "no" | "el" | "hu" | "bg"
        | "et" => {
            if op.i == 1 && op.v == 0 {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }

        // Spanish: `one: n = 1`, `many: millions` (CLDR 45+)
        "es" => {
            if op.is_integer_value() && op.i == 1 {
                PluralCategory::One
            } else if op.is_cldr_millions_many() {
                PluralCategory::Many
            } else {
                PluralCategory::Other
            }
        }

        // Italian & Catalan: `one: i = 1 and v = 0`, `many: millions` (CLDR 45+)
        "it" | "ca" => {
            if op.i == 1 && op.v == 0 {
                PluralCategory::One
            } else if op.is_cldr_millions_many() {
                PluralCategory::Many
            } else {
                PluralCategory::Other
            }
        }

        // Portuguese: `pt-PT` uses `i = 1 and v = 0`; default/`pt-BR` uses `i = 0..1`
        "pt" => {
            let is_one = if region == Some("PT") {
                op.i == 1 && op.v == 0
            } else {
                op.e == 0 && (op.i == 0 || op.i == 1)
            };
            if is_one {
                PluralCategory::One
            } else if op.is_cldr_millions_many() {
                PluralCategory::Many
            } else {
                PluralCategory::Other
            }
        }

        // French: CLDR 45+: `one: i = 0,1`, `many: millions`
        "fr" => {
            if op.e == 0 && (op.i == 0 || op.i == 1) {
                PluralCategory::One
            } else if op.is_cldr_millions_many() {
                PluralCategory::Many
            } else {
                PluralCategory::Other
            }
        }

        // Arabic: all 6 CLDR categories
        "ar" => {
            if !op.is_integer_value() {
                return PluralCategory::Other;
            }
            let mod100 = op.i % 100;
            if op.i == 0 {
                PluralCategory::Zero
            } else if op.i == 1 {
                PluralCategory::One
            } else if op.i == 2 {
                PluralCategory::Two
            } else if (3..=10).contains(&mod100) {
                PluralCategory::Few
            } else if (11..=99).contains(&mod100) {
                PluralCategory::Many
            } else {
                PluralCategory::Other
            }
        }

        // Polish: One / Few / Many / Other (fractional `v > 0` -> `Other`)
        "pl" => {
            if op.v != 0 || op.e != 0 {
                return PluralCategory::Other;
            }
            let mod10 = op.i % 10;
            let mod100 = op.i % 100;
            if op.i == 1 {
                PluralCategory::One
            } else if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
                PluralCategory::Few
            } else {
                PluralCategory::Many
            }
        }

        // East Slavic (Russian, Ukrainian): One / Few / Many / Other (fractional -> `Other`)
        "ru" | "uk" => {
            if op.v != 0 || op.e != 0 {
                return PluralCategory::Other;
            }
            let mod10 = op.i % 10;
            let mod100 = op.i % 100;
            if mod10 == 1 && mod100 != 11 {
                PluralCategory::One
            } else if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
                PluralCategory::Few
            } else {
                PluralCategory::Many
            }
        }

        // West Slavic (Czech, Slovak): `one: i=1,v=0`, `few: i=2..4,v=0`, `many: v!=0`
        "cs" | "sk" => {
            if op.v != 0 {
                PluralCategory::Many
            } else if op.e == 0 && op.i == 1 {
                PluralCategory::One
            } else if op.e == 0 && (2..=4).contains(&op.i) {
                PluralCategory::Few
            } else {
                PluralCategory::Other
            }
        }

        // Welsh: all 6 CLDR categories (`0`, `1`, `2`, `3`, `6`, `other`)
        "cy" => {
            if !op.is_integer_value() {
                return PluralCategory::Other;
            }
            match op.i {
                0 => PluralCategory::Zero,
                1 => PluralCategory::One,
                2 => PluralCategory::Two,
                3 => PluralCategory::Few,
                6 => PluralCategory::Many,
                _ => PluralCategory::Other,
            }
        }

        // Hebrew: `one: (i=1,v=0) or (i=0,v!=0)`, `two: i=2,v=0`, `other`
        "he" | "iw" => {
            if (op.i == 1 && op.v == 0) || (op.i == 0 && op.v != 0) {
                PluralCategory::One
            } else if op.i == 2 && op.v == 0 {
                PluralCategory::Two
            } else {
                PluralCategory::Other
            }
        }

        // Hindi, Bengali, Persian: `i = 0 or n = 1` -> `One`
        "hi" | "bn" | "fa" => {
            if op.e == 0 && (op.i == 0 || (op.is_integer_value() && op.i == 1)) {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }

        // `Other`-only language group
        "ja" | "zh" | "ko" | "tr" | "vi" | "id" | "th" | "ms" | "my" | "km" | "lo" => {
            PluralCategory::Other
        }

        _ => PluralCategory::Other,
    }
}

#[cfg(feature = "builtin-fallback")]
fn ordinal_category(locale: &LocaleId, op: PluralOperands) -> PluralCategory {
    if !op.is_integer_value() {
        return PluralCategory::Other;
    }
    let i = op.i;
    let mod10 = i % 10;
    let mod100 = i % 100;

    match locale.language() {
        "en" => {
            if mod10 == 1 && mod100 != 11 {
                PluralCategory::One
            } else if mod10 == 2 && mod100 != 12 {
                PluralCategory::Two
            } else if mod10 == 3 && mod100 != 13 {
                PluralCategory::Few
            } else {
                PluralCategory::Other
            }
        }
        "sv" => {
            if (mod10 == 1 || mod10 == 2) && mod100 != 11 && mod100 != 12 {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }
        "fr" => {
            if i == 1 {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }
        "it" => {
            if matches!(i, 8 | 11 | 80 | 800) {
                PluralCategory::Many
            } else {
                PluralCategory::Other
            }
        }
        "uk" => {
            if mod10 == 3 && mod100 != 13 {
                PluralCategory::Few
            } else {
                PluralCategory::Other
            }
        }
        "cy" => match i {
            0 | 7 | 8 | 9 => PluralCategory::Zero,
            1 => PluralCategory::One,
            2 => PluralCategory::Two,
            3 | 4 => PluralCategory::Few,
            5 | 6 => PluralCategory::Many,
            _ => PluralCategory::Other,
        },
        "de" | "nl" | "es" | "pt" | "ar" | "pl" | "ru" | "cs" | "sk" | "ja" | "zh" | "ko"
        | "tr" | "vi" => PluralCategory::Other,
        _ => PluralCategory::Other,
    }
}
