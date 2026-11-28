//! BCP-47 `-u-` Unicode extension subtags (`ca`, `fw`, `hc`, `nu`) and
//! fixed-capacity inline ASCII subtag storage for zero-allocation parsing.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::error::I18nError;
use crate::format::Weekday;

/// Fixed-capacity stack-allocated ASCII subtag (zero heap allocations).
#[derive(Clone, Copy, Eq)]
pub(crate) struct InlineSubtag<const N: usize> {
    buf: [u8; N],
    len: u8,
}

impl<const N: usize> InlineSubtag<N> {
    /// Constructs an inline subtag from a compile-time ASCII byte slice.
    pub(crate) const fn from_ascii_static(bytes: &[u8]) -> Self {
        assert!(bytes.len() <= N, "static subtag exceeds InlineSubtag capacity");
        let mut buf = [0u8; N];
        let mut i = 0;
        while i < bytes.len() {
            buf[i] = bytes[i];
            i += 1;
        }
        Self {
            buf,
            len: bytes.len() as u8,
        }
    }

    /// Copies `s` lowercased into an inline ASCII buffer, returning `None` if it
    /// exceeds `N` bytes or contains non-ASCII bytes.
    #[inline]
    pub(crate) fn from_ascii_lower(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() > N {
            return None;
        }
        let mut buf = [0u8; N];
        for (idx, &b) in bytes.iter().enumerate() {
            if !b.is_ascii() {
                return None;
            }
            buf[idx] = b.to_ascii_lowercase();
        }
        Some(Self {
            buf,
            len: bytes.len() as u8,
        })
    }

    /// Copies `s` uppercased into an inline ASCII buffer.
    #[inline]
    pub(crate) fn from_ascii_upper(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() > N {
            return None;
        }
        let mut buf = [0u8; N];
        for (idx, &b) in bytes.iter().enumerate() {
            if !b.is_ascii() {
                return None;
            }
            buf[idx] = b.to_ascii_uppercase();
        }
        Some(Self {
            buf,
            len: bytes.len() as u8,
        })
    }

    /// Copies `s` in TitleCase (`First` uppercase, remainder lowercase).
    #[inline]
    pub(crate) fn from_ascii_title(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() > N {
            return None;
        }
        let mut buf = [0u8; N];
        for (idx, &b) in bytes.iter().enumerate() {
            if !b.is_ascii() {
                return None;
            }
            buf[idx] = if idx == 0 {
                b.to_ascii_uppercase()
            } else {
                b.to_ascii_lowercase()
            };
        }
        Some(Self {
            buf,
            len: bytes.len() as u8,
        })
    }

    /// Appends `-<seg>` lowercased onto an existing subtag (used for compound
    /// BCP-47 calendar tags like `islamic-umalqura`).
    pub(crate) fn push_hyphen_lower(&mut self, seg: &str) -> bool {
        let cur = usize::from(self.len);
        let bytes = seg.as_bytes();
        if cur + 1 + bytes.len() > N {
            return false;
        }
        self.buf[cur] = b'-';
        for (idx, &b) in bytes.iter().enumerate() {
            if !b.is_ascii() {
                return false;
            }
            self.buf[cur + 1 + idx] = b.to_ascii_lowercase();
        }
        self.len = (cur + 1 + bytes.len()) as u8;
        true
    }

    /// Views the stored ASCII subtag as `&str`.
    #[inline]
    pub(crate) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("")
    }
}

impl<const N: usize> PartialEq for InlineSubtag<N> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && self.buf[..usize::from(self.len)] == other.buf[..usize::from(other.len)]
    }
}

impl<const N: usize> Ord for InlineSubtag<N> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl<const N: usize> PartialOrd for InlineSubtag<N> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<const N: usize> Hash for InlineSubtag<N> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl<const N: usize> fmt::Debug for InlineSubtag<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl<const N: usize> fmt::Display for InlineSubtag<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// BCP-47 `-u-hc-*` clock hour-cycle preference (UTS #35 Section 3.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnicodeHourCycle {
    /// 12-hour clock numbered `0..=11` (`-u-hc-h11`, e.g. Japanese 12h).
    H11,
    /// 12-hour clock numbered `1..=12` (`-u-hc-h12`, e.g. US English).
    H12,
    /// 24-hour clock numbered `0..=23` (`-u-hc-h23`, ISO-8601 / most locales).
    H23,
    /// 24-hour clock numbered `1..=24` (`-u-hc-h24`).
    H24,
}

impl UnicodeHourCycle {
    /// Canonical lowercase BCP-47 `-u-hc-*` subtag (`"h11"`, `"h12"`, `"h23"`, `"h24"`).
    #[inline]
    pub const fn as_bcp47_str(self) -> &'static str {
        match self {
            Self::H11 => "h11",
            Self::H12 => "h12",
            Self::H23 => "h23",
            Self::H24 => "h24",
        }
    }

    /// Parses a BCP-47 `-u-hc-*` hour-cycle subtag (case-insensitive, zero-alloc).
    #[inline]
    pub fn parse_bcp47(subtag: &str) -> Option<Self> {
        if subtag.eq_ignore_ascii_case("h11") {
            Some(Self::H11)
        } else if subtag.eq_ignore_ascii_case("h12") {
            Some(Self::H12)
        } else if subtag.eq_ignore_ascii_case("h23") {
            Some(Self::H23)
        } else if subtag.eq_ignore_ascii_case("h24") {
            Some(Self::H24)
        } else {
            None
        }
    }

    /// Returns `true` for 12-hour cycles (`H11`, `H12`).
    #[inline]
    pub const fn is_12_hour(self) -> bool {
        matches!(self, Self::H11 | Self::H12)
    }
}

/// Canonical BCP-47 `-u-` Unicode locale extensions (`ca`, `fw`, `hc`, `nu`),
/// stored entirely inline (zero heap allocations).
///
/// Serializes in canonical UTS #35 alphabetical key order: `ca`, `fw`, `hc`, `nu`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnicodeExtensions {
    calendar: Option<InlineSubtag<24>>,
    first_day_of_week: Option<Weekday>,
    hour_cycle: Option<UnicodeHourCycle>,
    numbering_system: Option<InlineSubtag<8>>,
}

impl UnicodeExtensions {
    /// Empty Unicode extension set (no `-u-` subtags).
    pub const EMPTY: Self = Self {
        calendar: None,
        first_day_of_week: None,
        hour_cycle: None,
        numbering_system: None,
    };

    /// Returns `true` when no Unicode extension overrides are present.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.calendar.is_none()
            && self.first_day_of_week.is_none()
            && self.hour_cycle.is_none()
            && self.numbering_system.is_none()
    }

    /// Optional `-u-ca-*` calendar identifier (canonical lowercase, e.g. `"japanese"`, `"gregory"`).
    #[inline]
    pub fn calendar(&self) -> Option<&str> {
        self.calendar.as_ref().map(InlineSubtag::as_str)
    }

    /// Optional `-u-fw-*` first day of the week override.
    #[inline]
    pub const fn first_day_of_week(&self) -> Option<Weekday> {
        self.first_day_of_week
    }

    /// Optional `-u-hc-*` hour-cycle override (`H11`, `H12`, `H23`, `H24`).
    #[inline]
    pub const fn hour_cycle(&self) -> Option<UnicodeHourCycle> {
        self.hour_cycle
    }

    /// Optional `-u-nu-*` numbering system identifier (canonical lowercase, e.g. `"arab"`, `"latn"`).
    #[inline]
    pub fn numbering_system(&self) -> Option<&str> {
        self.numbering_system.as_ref().map(InlineSubtag::as_str)
    }

    /// Writes canonical `-u-...` BCP-47 extension subtags to `f` when non-empty.
    pub(crate) fn fmt_bcp47(&self, f: &mut impl fmt::Write) -> fmt::Result {
        if self.is_empty() {
            return Ok(());
        }
        f.write_str("-u")?;
        if let Some(ca) = &self.calendar {
            f.write_str("-ca-")?;
            f.write_str(ca.as_str())?;
        }
        if let Some(fw) = self.first_day_of_week {
            f.write_str("-fw-")?;
            f.write_str(fw.as_bcp47_str())?;
        }
        if let Some(hc) = self.hour_cycle {
            f.write_str("-hc-")?;
            f.write_str(hc.as_bcp47_str())?;
        }
        if let Some(nu) = &self.numbering_system {
            f.write_str("-nu-")?;
            f.write_str(nu.as_str())?;
        }
        Ok(())
    }

    /// Writes ICU `@key=value;...` keywords (for Apple `CFLocaleCreate`) when non-empty.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    pub(crate) fn fmt_icu_keywords(&self, f: &mut impl fmt::Write) -> fmt::Result {
        if self.is_empty() {
            return Ok(());
        }
        let mut sep = '@';
        if let Some(ca) = &self.calendar {
            f.write_char(sep)?;
            f.write_str("calendar=")?;
            f.write_str(ca.as_str())?;
            sep = ';';
        }
        if let Some(fw) = self.first_day_of_week {
            f.write_char(sep)?;
            f.write_str("fw=")?;
            f.write_str(fw.as_bcp47_str())?;
            sep = ';';
        }
        if let Some(hc) = self.hour_cycle {
            f.write_char(sep)?;
            f.write_str("hours=")?;
            f.write_str(hc.as_bcp47_str())?;
            sep = ';';
        }
        if let Some(nu) = &self.numbering_system {
            f.write_char(sep)?;
            f.write_str("numbers=")?;
            f.write_str(nu.as_str())?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum UKey {
    Ca,
    Fw,
    Hc,
    Nu,
}

fn parse_u_key(subtag: &str, input: &str) -> Result<UKey, I18nError> {
    if subtag.eq_ignore_ascii_case("ca") {
        Ok(UKey::Ca)
    } else if subtag.eq_ignore_ascii_case("fw") {
        Ok(UKey::Fw)
    } else if subtag.eq_ignore_ascii_case("hc") {
        Ok(UKey::Hc)
    } else if subtag.eq_ignore_ascii_case("nu") {
        Ok(UKey::Nu)
    } else {
        Err(I18nError::InvalidLocale {
            input: input.to_owned(),
            reason: "unsupported unicode extension key (supported: ca, fw, hc, nu)",
        })
    }
}

#[inline]
fn is_valid_u_type_segment(s: &str) -> bool {
    let bytes = s.as_bytes();
    (3..=8).contains(&bytes.len())
        && bytes[0].is_ascii_alphabetic()
        && bytes.iter().all(|b| b.is_ascii_alphanumeric())
}

/// Parses all subtags following the `-u-` singleton strictly per BCP-47 / UTS #35.
pub(crate) fn parse_unicode_extensions<'a>(
    parts: &mut impl Iterator<Item = &'a str>,
    input: &str,
) -> Result<UnicodeExtensions, I18nError> {
    let mut ext = UnicodeExtensions::EMPTY;
    let mut active_key: Option<UKey> = None;
    let mut active_count: u8 = 0;

    for subtag in parts {
        if subtag.is_empty() {
            return Err(I18nError::InvalidLocale {
                input: input.to_owned(),
                reason: "empty subtag (leading, trailing, or consecutive separator)",
            });
        }
        if !subtag.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(I18nError::InvalidLocale {
                input: input.to_owned(),
                reason: "subtags must contain only ASCII letters and digits",
            });
        }

        if subtag.len() == 2 {
            // Finish any previous key and validate it received a value.
            if active_key.is_some() && active_count == 0 {
                return Err(I18nError::InvalidLocale {
                    input: input.to_owned(),
                    reason: "unicode extension key requires a value subtag",
                });
            }
            let key = parse_u_key(subtag, input)?;
            let dup = match key {
                UKey::Ca => ext.calendar.is_some(),
                UKey::Fw => ext.first_day_of_week.is_some(),
                UKey::Hc => ext.hour_cycle.is_some(),
                UKey::Nu => ext.numbering_system.is_some(),
            };
            if dup {
                return Err(I18nError::InvalidLocale {
                    input: input.to_owned(),
                    reason: "duplicate unicode extension key",
                });
            }
            active_key = Some(key);
            active_count = 0;
            continue;
        }

        let Some(key) = active_key else {
            return Err(I18nError::InvalidLocale {
                input: input.to_owned(),
                reason: "expected 2-character unicode extension key after '-u-'",
            });
        };

        match key {
            UKey::Fw => {
                if active_count > 0 {
                    return Err(I18nError::InvalidLocale {
                        input: input.to_owned(),
                        reason: "'-u-fw-' accepts exactly one weekday subtag",
                    });
                }
                let day = Weekday::parse_bcp47(subtag).ok_or_else(|| I18nError::InvalidLocale {
                    input: input.to_owned(),
                    reason: "invalid '-u-fw-' weekday (expected sun, mon, tue, wed, thu, fri, sat)",
                })?;
                ext.first_day_of_week = Some(day);
                active_count = 1;
            }
            UKey::Hc => {
                if active_count > 0 {
                    return Err(I18nError::InvalidLocale {
                        input: input.to_owned(),
                        reason: "'-u-hc-' accepts exactly one hour-cycle subtag",
                    });
                }
                let hc = UnicodeHourCycle::parse_bcp47(subtag).ok_or_else(|| {
                    I18nError::InvalidLocale {
                        input: input.to_owned(),
                        reason: "invalid '-u-hc-' hour cycle (expected h11, h12, h23, h24)",
                    }
                })?;
                ext.hour_cycle = Some(hc);
                active_count = 1;
            }
            UKey::Nu => {
                if active_count > 0 {
                    return Err(I18nError::InvalidLocale {
                        input: input.to_owned(),
                        reason: "'-u-nu-' accepts exactly one numbering-system subtag",
                    });
                }
                if !is_valid_u_type_segment(subtag) {
                    return Err(I18nError::InvalidLocale {
                        input: input.to_owned(),
                        reason: "invalid '-u-nu-' numbering system subtag (must be 3-8 ASCII alnum starting with a letter)",
                    });
                }
                ext.numbering_system = InlineSubtag::from_ascii_lower(subtag);
                active_count = 1;
            }
            UKey::Ca => {
                if !is_valid_u_type_segment(subtag) {
                    return Err(I18nError::InvalidLocale {
                        input: input.to_owned(),
                        reason: "invalid '-u-ca-' calendar subtag (must be 3-8 ASCII alnum starting with a letter)",
                    });
                }
                if let Some(ca) = &mut ext.calendar {
                    if active_count >= 3 || !ca.push_hyphen_lower(subtag) {
                        return Err(I18nError::InvalidLocale {
                            input: input.to_owned(),
                            reason: "'-u-ca-' calendar identifier exceeds maximum length",
                        });
                    }
                } else {
                    ext.calendar = InlineSubtag::from_ascii_lower(subtag);
                }
                active_count = active_count.saturating_add(1);
            }
        }
    }

    if active_key.is_none() || active_count == 0 {
        return Err(I18nError::InvalidLocale {
            input: input.to_owned(),
            reason: "unicode extension '-u-' requires at least one key-value pair",
        });
    }

    Ok(ext)
}
