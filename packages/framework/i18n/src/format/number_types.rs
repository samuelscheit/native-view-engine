//! Typed number formatting options (`NumberStyle`, `RoundingMode`,
//! `NumberFormatOptions`), ISO-4217 [`Currency`] metadata, [`LocaleMetadata`] /
//! [`MeasurementSystem`], and CLDR `-u-nu-*` numbering-system digit mapping.

/// Display style for [`NumberFormatOptions`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NumberStyle {
    /// Standard decimal formatting (`1,234.56`).
    #[default]
    Decimal = 0,
    /// Percentage formatting (multiplies by 100 and appends locale percent sign, e.g. `0.256` -> `26%`).
    Percent = 1,
    /// Scientific notation (`1.23456E4`).
    Scientific = 2,
}

/// Rounding strategy for fractional number formatting.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RoundingMode {
    /// Ties round away from zero (default, e.g. `999.995` -> `1,000.00`).
    #[default]
    HalfAwayFromZero = 0,
    /// Ties round to the nearest even digit (banker's rounding).
    HalfEven = 1,
    /// Rounds toward negative infinity (`-1.2` -> `-2`, `1.2` -> `1`).
    Floor = 2,
    /// Rounds toward positive infinity (`1.2` -> `2`, `-1.2` -> `-1`).
    Ceiling = 3,
    /// Truncates fractional digits toward zero (`1.9` -> `1`, `-1.9` -> `-1`).
    Truncate = 4,
}

/// Formatting options for decimal, percentage, and scientific numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NumberFormatOptions {
    /// Whether to insert locale thousands/lakh grouping separators (default `true`).
    pub use_grouping: bool,
    /// Minimum fraction digits to display (pads with trailing zeros).
    pub min_fraction_digits: Option<u32>,
    /// Maximum fraction digits to display.
    pub max_fraction_digits: Option<u32>,
    /// Display style (`Decimal`, `Percent`, `Scientific`).
    pub style: NumberStyle,
    /// Rounding mode when truncating fractional digits.
    pub rounding_mode: RoundingMode,
}

impl Default for NumberFormatOptions {
    fn default() -> Self {
        Self {
            use_grouping: true,
            min_fraction_digits: None,
            max_fraction_digits: None,
            style: NumberStyle::Decimal,
            rounding_mode: RoundingMode::HalfAwayFromZero,
        }
    }
}

impl NumberFormatOptions {
    /// Sets whether grouping separators are emitted.
    #[inline]
    pub const fn with_grouping(mut self, use_grouping: bool) -> Self {
        self.use_grouping = use_grouping;
        self
    }

    /// Sets minimum and maximum fraction digits.
    #[inline]
    pub const fn with_fraction_digits(mut self, min: Option<u32>, max: Option<u32>) -> Self {
        self.min_fraction_digits = min;
        self.max_fraction_digits = max;
        self
    }

    /// Sets [`NumberStyle`] (`Decimal`, `Percent`, `Scientific`).
    #[inline]
    pub const fn with_style(mut self, style: NumberStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets [`RoundingMode`].
    #[inline]
    pub const fn with_rounding_mode(mut self, rounding_mode: RoundingMode) -> Self {
        self.rounding_mode = rounding_mode;
        self
    }
}

/// Locale measurement system convention (`Metric`, `Uk`, `Us`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MeasurementSystem {
    #[default]
    Metric,
    Uk,
    Us,
}

/// Resolved locale formatting & regional metadata (populated via `CFLocaleGetValue`
/// on Apple platforms, or CLDR builtin tables elsewhere).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleMetadata {
    pub decimal_separator: String,
    pub grouping_separator: String,
    pub currency_symbol: Option<String>,
    pub currency_code: Option<String>,
    pub measurement_system: MeasurementSystem,
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

    /// All built-in ISO-4217 [`Currency`] variants (single source of truth for code lookup).
    pub const ALL: [Self; 21] = [
        Self::Usd,
        Self::Eur,
        Self::Gbp,
        Self::Jpy,
        Self::Chf,
        Self::Inr,
        Self::Cny,
        Self::Krw,
        Self::Cad,
        Self::Aud,
        Self::Brl,
        Self::Pln,
        Self::Rub,
        Self::Sek,
        Self::Czk,
        Self::Try,
        Self::Vnd,
        Self::Aed,
        Self::Sar,
        Self::Kwd,
        Self::Bhd,
    ];

    /// Parses an ISO-4217 3-letter currency code (case-insensitive, zero-alloc).
    pub fn from_code(code: &str) -> Option<Self> {
        if code.len() != 3 {
            return None;
        }
        Self::ALL
            .into_iter()
            .find(|c| c.as_code().eq_ignore_ascii_case(code))
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

/// Returns the 10 decimal digit characters for a CLDR `-u-nu-*` numbering system,
/// or `None` for standard ASCII `latn` / unknown systems.
#[cfg(feature = "builtin-fallback")]
pub(crate) fn numbering_system_digits(nu: Option<&str>) -> Option<[char; 10]> {
    match nu? {
        "arab" => Some(['٠', '١', '٢', '٣', '٤', '٥', '٦', '٧', '٨', '٩']),
        "arabext" => Some(['۰', '۱', '۲', '۳', '۴', '۵', '۶', '۷', '۸', '۹']),
        "deva" => Some(['०', '१', '२', '३', '४', '५', '६', '७', '८', '९']),
        "beng" => Some(['০', '১', '২', '৩', '৪', '৫', '৬', '৭', '৮', '৯']),
        "thai" => Some(['๐', '๑', '๒', '๓', '๔', '๕', '๖', '๗', '๘', '๙']),
        "hanidec" => Some(['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九']),
        "fullwide" => Some(['０', '１', '２', '３', '４', '５', '６', '７', '８', '９']),
        _ => None,
    }
}

/// Appends ASCII text into `out`, transliterating `'0'..='9'` when `digits` is `Some`.
#[cfg(feature = "builtin-fallback")]
#[inline]
pub(crate) fn push_digits_mapped(text: &str, digits: Option<[char; 10]>, out: &mut String) {
    let Some(table) = digits else {
        out.push_str(text);
        return;
    };
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            out.push(table[(ch as u8 - b'0') as usize]);
        } else {
            out.push(ch);
        }
    }
}
