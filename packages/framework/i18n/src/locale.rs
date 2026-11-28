//! BCP-47 locale parsing, canonicalization, `-u-` Unicode extension support,
//! RTL detection, selective icon mirroring, and zero-alloc fallback matching.

mod unicode_ext;

use std::fmt;
use std::str::FromStr;

use crate::error::I18nError;
use crate::format::Weekday;

pub use unicode_ext::{UnicodeExtensions, UnicodeHourCycle};

use unicode_ext::{InlineSubtag, parse_unicode_extensions};

/// Base inline text and layout reading direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextDirection {
    #[default]
    Ltr,
    Rtl,
}

impl TextDirection {
    /// Returns `true` when the reading direction is right-to-left.
    #[inline]
    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }

    /// Returns the CSS/HTML `dir` attribute value (`"ltr"` or `"rtl"`).
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ltr => "ltr",
            Self::Rtl => "rtl",
        }
    }

    /// Returns whether an icon with the given semantic policy should be
    /// horizontally mirrored for this reading direction.
    #[inline]
    pub const fn should_mirror_icon(self, semantic: IconMirrorPolicy) -> bool {
        should_mirror_icon(self, semantic)
    }
}

/// Semantic icon mirroring policy for RTL layout support.
///
/// UI icon mirroring is strictly selective: directional icons reverse in RTL,
/// whereas universal symbols (checkmarks, clocks, search, close `×`, logos)
/// remain unmirrored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconMirrorPolicy {
    /// Directional icon that conveys reading/navigation flow (e.g., chevrons,
    /// back/forward arrows, indent/outdent, reply, list alignment).
    /// Mirrored in RTL locales.
    Directional,
    /// Universal or brand icon that must never be horizontally flipped
    /// (e.g., checkmarks, clocks, search magnifying glass, close `×`, logos).
    Fixed,
}

/// Returns `true` when an icon should be horizontally mirrored for `direction`.
#[inline]
pub const fn should_mirror_icon(direction: TextDirection, semantic: IconMirrorPolicy) -> bool {
    matches!(
        (direction, semantic),
        (TextDirection::Rtl, IconMirrorPolicy::Directional)
    )
}

/// Canonical BCP-47 locale identifier
/// (`language[-Script][-REGION][-variant]*[-u-ca-...-fw-...-hc-...-nu-...]`).
///
/// Subtags are normalized on parse with zero heap allocations for all locales
/// without variant subtags (including locales carrying `-u-` extensions):
/// - `language`: lowercase ISO 639 (`en`, `zh`, `ar`, `ckb`) — stored inline
/// - `script`: TitleCase ISO 15924 (`Hans`, `Hant`, `Arab`, `Latn`) — stored inline
/// - `region`: UPPERCASE ISO 3166-1 alpha-2 or 3-digit UN M.49 (`US`, `CN`, `419`) — stored inline
/// - `variants`: lowercase BCP-47 variant subtags (`1901`, `valencia`)
/// - `unicode_extensions`: canonical `-u-` subtags (`ca`, `fw`, `hc`, `nu`) — stored inline
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocaleId {
    language: InlineSubtag<3>,
    script: Option<InlineSubtag<4>>,
    region: Option<InlineSubtag<3>>,
    variants: Vec<String>,
    unicode_ext: UnicodeExtensions,
}

impl LocaleId {
    /// Parses and canonicalizes a BCP-47 locale tag (`-` or `_` separators),
    /// including `-u-` Unicode extension subtags (`ca`, `fw`, `hc`, `nu`).
    ///
    /// Performs zero heap allocations on valid locales without variant subtags.
    ///
    /// # Examples
    /// - `"en-us"` / `"en_US"` -> `en-US`
    /// - `"ZH-hans-cn"` -> `zh-Hans-CN`
    /// - `"es-419"` -> `es-419`
    /// - `"ja-JP-u-ca-japanese"` -> `ja-JP-u-ca-japanese`
    /// - `"en-US-u-hc-H23-fw-MON"` -> `en-US-u-fw-mon-hc-h23`
    pub fn parse(input: &str) -> Result<Self, I18nError> {
        if input.is_empty() {
            return Err(I18nError::InvalidLocale {
                input: input.to_owned(),
                reason: "locale tag cannot be empty",
            });
        }

        let mut parts = input.split(['-', '_']);
        let lang_raw = parts.next().unwrap_or("");
        if !is_valid_language(lang_raw) {
            return Err(I18nError::InvalidLocale {
                input: input.to_owned(),
                reason: "primary language subtag must be 2-3 ASCII letters",
            });
        }
        let Some(language) = InlineSubtag::from_ascii_lower(lang_raw) else {
            return Err(I18nError::InvalidLocale {
                input: input.to_owned(),
                reason: "primary language subtag must be 2-3 ASCII letters",
            });
        };

        let mut script: Option<InlineSubtag<4>> = None;
        let mut region: Option<InlineSubtag<3>> = None;
        let mut variants: Vec<String> = Vec::new();
        let mut unicode_ext = UnicodeExtensions::EMPTY;

        while let Some(subtag) = parts.next() {
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

            if subtag.len() == 1 {
                if subtag.eq_ignore_ascii_case("u") {
                    unicode_ext = parse_unicode_extensions(&mut parts, input)?;
                    break;
                }
                return Err(I18nError::InvalidLocale {
                    input: input.to_owned(),
                    reason: "unsupported extension singleton (only '-u-' is supported)",
                });
            }

            if script.is_none()
                && region.is_none()
                && variants.is_empty()
                && is_valid_script(subtag)
            {
                script = InlineSubtag::from_ascii_title(subtag);
                continue;
            }

            if region.is_none() && variants.is_empty() && is_valid_region(subtag) {
                region = InlineSubtag::from_ascii_upper(subtag);
                continue;
            }

            if is_valid_variant(subtag) {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "BCP-47 canonical locale owns normalized lowercase variant subtag"
                )]
                let canonical_variant = subtag.to_ascii_lowercase();
                if variants.contains(&canonical_variant) {
                    return Err(I18nError::InvalidLocale {
                        input: input.to_owned(),
                        reason: "duplicate variant subtag",
                    });
                }
                variants.push(canonical_variant);
                continue;
            }

            return Err(I18nError::InvalidLocale {
                input: input.to_owned(),
                reason: "malformed or out-of-order subtag",
            });
        }

        Ok(Self {
            language,
            script,
            region,
            variants,
            unicode_ext,
        })
    }

    /// Zero-allocation convenience constructor for `en`.
    #[inline]
    pub const fn en() -> Self {
        Self {
            language: InlineSubtag::from_ascii_static(b"en"),
            script: None,
            region: None,
            variants: Vec::new(),
            unicode_ext: UnicodeExtensions::EMPTY,
        }
    }

    /// Zero-allocation convenience constructor for `en-US`.
    #[inline]
    pub const fn en_us() -> Self {
        Self {
            language: InlineSubtag::from_ascii_static(b"en"),
            script: None,
            region: Some(InlineSubtag::from_ascii_static(b"US")),
            variants: Vec::new(),
            unicode_ext: UnicodeExtensions::EMPTY,
        }
    }

    /// Lowercase primary language subtag (e.g. `"en"`, `"zh"`, `"ar"`).
    #[inline]
    pub fn language(&self) -> &str {
        self.language.as_str()
    }

    /// Optional TitleCase script subtag (e.g. `Some("Hans")`, `Some("Arab")`).
    #[inline]
    pub fn script(&self) -> Option<&str> {
        self.script.as_ref().map(InlineSubtag::as_str)
    }

    /// Optional UPPERCASE region subtag (e.g. `Some("US")`, `Some("CN")`, `Some("419")`).
    #[inline]
    pub fn region(&self) -> Option<&str> {
        self.region.as_ref().map(InlineSubtag::as_str)
    }

    /// Optional lowercase variant subtags.
    #[inline]
    pub fn variants(&self) -> &[String] {
        &self.variants
    }

    /// Parsed BCP-47 `-u-` Unicode extension subtags (`ca`, `fw`, `hc`, `nu`).
    #[inline]
    pub const fn unicode_extensions(&self) -> &UnicodeExtensions {
        &self.unicode_ext
    }

    /// Returns `true` when any BCP-47 `-u-` Unicode extension override is present.
    #[inline]
    pub const fn has_extensions(&self) -> bool {
        !self.unicode_ext.is_empty()
    }

    /// Optional `-u-ca-*` calendar override (e.g. `Some("japanese")`, `Some("gregory")`).
    #[inline]
    pub fn calendar(&self) -> Option<&str> {
        self.unicode_ext.calendar()
    }

    /// Optional `-u-fw-*` first day of week override (e.g. `Some(Weekday::Monday)`).
    #[inline]
    pub const fn first_day_of_week_override(&self) -> Option<Weekday> {
        self.unicode_ext.first_day_of_week()
    }

    /// Optional `-u-hc-*` hour-cycle override (e.g. `Some(UnicodeHourCycle::H12)`).
    #[inline]
    pub const fn hour_cycle(&self) -> Option<UnicodeHourCycle> {
        self.unicode_ext.hour_cycle()
    }

    /// Optional `-u-nu-*` numbering system override (e.g. `Some("arab")`, `Some("latn")`).
    #[inline]
    pub fn numbering_system(&self) -> Option<&str> {
        self.unicode_ext.numbering_system()
    }

    /// Writes the canonical hyphen-separated BCP-47 tag directly into `out`
    /// (zero allocations when `out` has sufficient capacity).
    pub fn write_canonical_tag(&self, out: &mut impl fmt::Write) -> fmt::Result {
        out.write_str(self.language.as_str())?;
        if let Some(script) = &self.script {
            out.write_str("-")?;
            out.write_str(script.as_str())?;
        }
        if let Some(region) = &self.region {
            out.write_str("-")?;
            out.write_str(region.as_str())?;
        }
        for variant in &self.variants {
            out.write_str("-")?;
            out.write_str(variant)?;
        }
        self.unicode_ext.fmt_bcp47(out)
    }

    /// Writes the ICU identifier (`lang[_Script][_REGION][_VAR][@calendar=...;fw=...;hours=...;numbers=...]`)
    /// for Apple `CoreFoundation` `CFLocaleCreate`.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    pub(crate) fn write_icu_identifier(&self, out: &mut impl fmt::Write) -> fmt::Result {
        out.write_str(self.language.as_str())?;
        if let Some(script) = &self.script {
            out.write_str("_")?;
            out.write_str(script.as_str())?;
        }
        if let Some(region) = &self.region {
            out.write_str("_")?;
            out.write_str(region.as_str())?;
        }
        for variant in &self.variants {
            out.write_str("_")?;
            out.write_str(variant)?;
        }
        self.unicode_ext.fmt_icu_keywords(out)
    }

    /// Returns the canonical hyphen-separated BCP-47 tag (e.g. `"zh-Hans-CN"`,
    /// `"ja-JP-u-ca-japanese"`).
    pub fn canonical_tag(&self) -> String {
        let mut out = String::with_capacity(24);
        let _ = self.write_canonical_tag(&mut out);
        out
    }

    /// Returns `true` if this locale uses a right-to-left script or language.
    ///
    /// When an explicit script subtag is present, it takes precedence over the
    /// primary language (so `ar-Latn` is LTR while `ks-Arab` is RTL).
    pub fn is_rtl(&self) -> bool {
        if let Some(script) = self.script() {
            return is_rtl_script(script);
        }
        is_rtl_language(self.language())
    }

    /// Returns [`TextDirection::Rtl`] or [`TextDirection::Ltr`] for this locale.
    #[inline]
    pub fn text_direction(&self) -> TextDirection {
        if self.is_rtl() {
            TextDirection::Rtl
        } else {
            TextDirection::Ltr
        }
    }

    /// Returns whether an icon with `semantic` policy should mirror in this locale.
    #[inline]
    pub fn should_mirror_icon(&self, semantic: IconMirrorPolicy) -> bool {
        should_mirror_icon(self.text_direction(), semantic)
    }

    /// Returns the immediate parent locale by stripping the most specific subtag
    /// (`-u-` extensions -> variants -> region -> script), or `None` if only the
    /// primary language remains.
    ///
    /// Performs zero heap allocations when `self.variants` is empty.
    pub fn parent(&self) -> Option<Self> {
        if !self.unicode_ext.is_empty() {
            return Some(Self {
                language: self.language,
                script: self.script,
                region: self.region,
                variants: self.variants.clone(),
                unicode_ext: UnicodeExtensions::EMPTY,
            });
        }
        if !self.variants.is_empty() {
            let mut next = self.clone();
            next.variants.pop();
            return Some(next);
        }
        if self.region.is_some() {
            return Some(Self {
                language: self.language,
                script: self.script,
                region: None,
                variants: Vec::new(),
                unicode_ext: UnicodeExtensions::EMPTY,
            });
        }
        if self.script.is_some() {
            return Some(Self {
                language: self.language,
                script: None,
                region: None,
                variants: Vec::new(),
                unicode_ext: UnicodeExtensions::EMPTY,
            });
        }
        None
    }

    /// Visits `self` followed by each parent in order without allocating a `Vec`.
    /// Stops early and returns `Some(T)` on the first match.
    pub(crate) fn find_in_ancestry<T>(&self, mut f: impl FnMut(&LocaleId) -> Option<T>) -> Option<T> {
        if let Some(hit) = f(self) {
            return Some(hit);
        }
        let mut current = self.parent();
        while let Some(loc) = current {
            if let Some(hit) = f(&loc) {
                return Some(hit);
            }
            current = loc.parent();
        }
        None
    }

    /// Produces the deterministic fallback chain from most specific to least
    /// specific, optionally followed by `default_fallback` and its parents,
    /// without duplicates.
    ///
    /// Example: `zh-Hans-CN` with default `en` yields `[zh-Hans-CN, zh-Hans, zh, en]`.
    pub fn fallback_chain(&self, default_fallback: Option<&LocaleId>) -> Vec<LocaleId> {
        let mut chain = Vec::with_capacity(4);
        push_parents(self, &mut chain);
        if let Some(default) = default_fallback {
            push_parents(default, &mut chain);
        }
        chain
    }

    /// Resolves the best available locale from `available` using this locale's
    /// fallback chain (exact chain hit first, then same language+script, then
    /// same primary language), with zero heap allocations for locales without variants.
    pub fn best_match<'a>(&self, available: &'a [LocaleId]) -> Option<&'a LocaleId> {
        if let Some(found) =
            self.find_in_ancestry(|cand| available.iter().find(|loc| **loc == *cand))
        {
            return Some(found);
        }
        if let Some(script) = self.script()
            && let Some(found) = available
                .iter()
                .find(|loc| loc.language() == self.language() && loc.script() == Some(script))
        {
            return Some(found);
        }
        available
            .iter()
            .find(|loc| loc.language() == self.language())
    }
}

impl Default for LocaleId {
    fn default() -> Self {
        Self::en_us()
    }
}

impl FromStr for LocaleId {
    type Err = I18nError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for LocaleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_canonical_tag(f)
    }
}

fn push_parents(start: &LocaleId, out: &mut Vec<LocaleId>) {
    let mut current = Some(start.clone());
    while let Some(loc) = current {
        if !out.contains(&loc) {
            out.push(loc.clone());
        }
        current = loc.parent();
    }
}

#[inline]
fn is_valid_language(s: &str) -> bool {
    (2..=3).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphabetic())
}

#[inline]
fn is_valid_script(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|b| b.is_ascii_alphabetic())
}

#[inline]
fn is_valid_region(s: &str) -> bool {
    (s.len() == 2 && s.bytes().all(|b| b.is_ascii_alphabetic()))
        || (s.len() == 3 && s.bytes().all(|b| b.is_ascii_digit()))
}

#[inline]
fn is_valid_variant(s: &str) -> bool {
    let bytes = s.as_bytes();
    (5..=8).contains(&bytes.len()) || (bytes.len() == 4 && bytes[0].is_ascii_digit())
}

/// Returns `true` if `script` (TitleCase ISO 15924) is right-to-left.
pub fn is_rtl_script(script: &str) -> bool {
    matches!(
        script,
        "Arab"
            | "Hebr"
            | "Syrc"
            | "Thaa"
            | "Nkoo"
            | "Adlm"
            | "Mand"
            | "Samr"
            | "Mend"
            | "Rohg"
            | "Yezi"
            | "Chrs"
    )
}

/// Returns `true` if primary `language` (lowercase ISO 639) defaults to RTL.
pub fn is_rtl_language(language: &str) -> bool {
    matches!(
        language,
        "ar" | "he"
            | "iw"
            | "fa"
            | "ur"
            | "ps"
            | "yi"
            | "ji"
            | "dv"
            | "sd"
            | "ug"
            | "ckb"
            | "syr"
            | "nqo"
            | "arc"
            | "ks"
    )
}
