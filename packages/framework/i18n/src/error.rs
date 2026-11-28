//! Error types for `nve-i18n`.

use std::fmt;

use crate::locale::LocaleId;

/// Diagnostic record emitted when a translation key cannot be found in a catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingKeyDiagnostic {
    /// Requested translation key.
    pub key: String,
    /// Active locale requested by the caller.
    pub requested_locale: LocaleId,
    /// Ordered locale fallback chain that was searched.
    pub tried_locales: Vec<LocaleId>,
}

impl fmt::Display for MissingKeyDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "missing translation key '{}' for locale '{}' (tried: ",
            self.key, self.requested_locale
        )?;
        for (idx, loc) in self.tried_locales.iter().enumerate() {
            if idx > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{loc}")?;
        }
        f.write_str(")")
    }
}

/// Errors returned by BCP-47 parsing, plural operand parsing, ICU MessageFormat,
/// calendar validation, and catalog resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum I18nError {
    /// Malformed BCP-47 locale identifier.
    InvalidLocale { input: String, reason: &'static str },
    /// Malformed decimal or non-finite numeric input.
    InvalidNumber { input: String },
    /// Invalid Gregorian calendar date (month/day out of range).
    InvalidDate { year: i32, month: u8, day: u8 },
    /// Invalid wall-clock time (hour/minute/second out of range).
    InvalidTime { hour: u8, minute: u8, second: u8 },
    /// Unclosed `{` in an ICU MessageFormat template.
    UnmatchedOpenBrace { offset: usize },
    /// Stray `}` without a matching `{` in an ICU MessageFormat template.
    UnmatchedCloseBrace { offset: usize },
    /// Unclosed apostrophe-quoted literal span (`'{...`) in an ICU template.
    UnterminatedQuote { offset: usize },
    /// Missing or invalid argument identifier inside `{...}`.
    EmptyArgumentName { offset: usize },
    /// Unsupported argument format type (expected `plural`, `selectordinal`, `select`, or `number`).
    InvalidArgumentType { arg_type: String, offset: usize },
    /// Invalid plural/selectordinal selector keyword or `=N` literal.
    InvalidPluralSelector { selector: String, offset: usize },
    /// Duplicate branch selector inside a single `plural`, `selectordinal`, or `select` block.
    DuplicateSelector { selector: String, offset: usize },
    /// Invalid `offset:N` clause inside a plural block.
    InvalidOffset { offset: usize },
    /// A `plural`, `selectordinal`, or `select` block is missing its mandatory `other` branch.
    MissingOtherBranch { arg: String, offset: usize },
    /// Expected a specific syntax token while parsing an ICU template.
    ExpectedToken { expected: &'static str, offset: usize },
    /// ICU template exceeded the strict maximum recursion depth.
    MaxDepthExceeded { max_depth: usize, offset: usize },
    /// Required message argument was not provided when formatting.
    MissingArgument { name: String },
    /// Message argument had an incompatible type (e.g. non-numeric value for `plural`).
    InvalidArgumentValue { name: String, expected: &'static str },
    /// Translation key was not found in the catalog across the locale fallback chain.
    MissingKey(Box<MissingKeyDiagnostic>),
}

impl fmt::Display for I18nError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLocale { input, reason } => {
                write!(f, "invalid BCP-47 locale '{input}': {reason}")
            }
            Self::InvalidNumber { input } => write!(f, "invalid numeric value '{input}'"),
            Self::InvalidDate { year, month, day } => {
                write!(f, "invalid calendar date {year:04}-{month:02}-{day:02}")
            }
            Self::InvalidTime {
                hour,
                minute,
                second,
            } => {
                write!(f, "invalid wall-clock time {hour:02}:{minute:02}:{second:02}")
            }
            Self::UnmatchedOpenBrace { offset } => {
                write!(f, "unmatched '{{' at byte offset {offset}")
            }
            Self::UnmatchedCloseBrace { offset } => {
                write!(f, "unmatched '}}' at byte offset {offset}")
            }
            Self::UnterminatedQuote { offset } => {
                write!(f, "unterminated quoted literal starting at byte offset {offset}")
            }
            Self::EmptyArgumentName { offset } => {
                write!(f, "empty or invalid argument name at byte offset {offset}")
            }
            Self::InvalidArgumentType { arg_type, offset } => {
                write!(f, "unsupported argument type '{arg_type}' at byte offset {offset}")
            }
            Self::InvalidPluralSelector { selector, offset } => {
                write!(f, "invalid plural selector '{selector}' at byte offset {offset}")
            }
            Self::DuplicateSelector { selector, offset } => {
                write!(f, "duplicate selector '{selector}' at byte offset {offset}")
            }
            Self::InvalidOffset { offset } => {
                write!(f, "invalid plural offset at byte offset {offset}")
            }
            Self::MissingOtherBranch { arg, offset } => {
                write!(f, "missing required 'other' branch for argument '{arg}' at byte offset {offset}")
            }
            Self::ExpectedToken { expected, offset } => {
                write!(f, "expected {expected} at byte offset {offset}")
            }
            Self::MaxDepthExceeded { max_depth, offset } => {
                write!(f, "message nesting exceeded max depth {max_depth} at byte offset {offset}")
            }
            Self::MissingArgument { name } => write!(f, "missing message argument '{name}'"),
            Self::InvalidArgumentValue { name, expected } => {
                write!(f, "argument '{name}' requires {expected}")
            }
            Self::MissingKey(diag) => fmt::Display::fmt(diag, f),
        }
    }
}

impl std::error::Error for I18nError {}
