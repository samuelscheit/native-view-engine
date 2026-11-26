//! BCP-47 locale parsing, canonicalization, RTL detection, selective icon
//! mirroring, and deterministic locale fallback chains.

use std::fmt;
use std::str::FromStr;

use crate::error::I18nError;

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
    matches!((direction, semantic), (TextDirection::Rtl, IconMirrorPolicy::Directional))
}

/// Canonical BCP-47 locale identifier (`language[-Script][-REGION][-variant]*`).
///
/// Subtags are normalized on parse:
/// - `language`: lowercase ISO 639 (`en`, `zh`, `ar`, `ckb`)
/// - `script`: TitleCase ISO 15924 (`Hans`, `Hant`, `Arab`, `Latn`)
/// - `region`: UPPERCASE ISO 3166-1 alpha-2 or 3-digit UN M.49 (`US`, `CN`, `419`)
/// - `variants`: lowercase BCP-47 variant subtags (`1901`, `valencia`)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocaleId {
    language: String,
    script: Option<String>,
    region: Option<String>,
    variants: Vec<String>,
}

impl LocaleId {
    /// Parses and canonicalizes a BCP-47 locale tag (`-` or `_` separators).
    ///
    /// # Examples
    /// - `"en-us"` / `"en_US"` -> `en-US`
    /// - `"ZH-hans-cn"` -> `zh-Hans-CN`
    /// - `"es-419"` -> `es-419`
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
        #[expect(
            clippy::disallowed_methods,
            reason = "BCP-47 canonical locale owns normalized lowercase language subtag"
        )]
        let language = lang_raw.to_ascii_lowercase();

        let mut script: Option<String> = None;
        let mut region: Option<String> = None;
        let mut variants: Vec<String> = Vec::new();

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

            if script.is_none()
                && region.is_none()
                && variants.is_empty()
                && is_valid_script(subtag)
            {
                script = Some(title_case_ascii(subtag));
                continue;
            }

            if region.is_none() && variants.is_empty() && is_valid_region(subtag) {
                region = Some(subtag.to_ascii_uppercase());
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
        })
    }

    /// Convenience constructor for `en`.
    pub fn en() -> Self {
        Self {
            language: "en".to_owned(),
            script: None,
            region: None,
            variants: Vec::new(),
        }
    }

    /// Convenience constructor for `en-US`.
    pub fn en_us() -> Self {
        Self {
            language: "en".to_owned(),
            script: None,
            region: Some("US".to_owned()),
            variants: Vec::new(),
        }
    }

    /// Lowercase primary language subtag (e.g. `"en"`, `"zh"`, `"ar"`).
    #[inline]
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Optional TitleCase script subtag (e.g. `Some("Hans")`, `Some("Arab")`).
    #[inline]
    pub fn script(&self) -> Option<&str> {
        self.script.as_deref()
    }

    /// Optional UPPERCASE region subtag (e.g. `Some("US")`, `Some("CN")`, `Some("419")`).
    #[inline]
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// Optional lowercase variant subtags.
    #[inline]
    pub fn variants(&self) -> &[String] {
        &self.variants
    }

    /// Returns the canonical hyphen-separated BCP-47 tag (e.g. `"zh-Hans-CN"`).
    pub fn canonical_tag(&self) -> String {
        self.to_string()
    }

    /// Returns `true` if this locale uses a right-to-left script or language.
    ///
    /// When an explicit script subtag is present, it takes precedence over the
    /// primary language (so `ar-Latn` is LTR while `ks-Arab` is RTL).
    pub fn is_rtl(&self) -> bool {
        if let Some(script) = self.script.as_deref() {
            return is_rtl_script(script);
        }
        is_rtl_language(&self.language)
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

    /// Returns the immediate parent locale by stripping the most specific subtag,
    /// or `None` if only the primary language remains.
    ///
    /// Examples:
    /// - `zh-Hans-CN` -> `Some(zh-Hans)`
    /// - `zh-Hans` -> `Some(zh)`
    /// - `zh` -> `None`
    pub fn parent(&self) -> Option<Self> {
        if !self.variants.is_empty() {
            let mut next = self.clone();
            next.variants.pop();
            return Some(next);
        }
        if self.region.is_some() {
            return Some(Self {
                language: self.language.clone(),
                script: self.script.clone(),
                region: None,
                variants: Vec::new(),
            });
        }
        if self.script.is_some() {
            return Some(Self {
                language: self.language.clone(),
                script: None,
                region: None,
                variants: Vec::new(),
            });
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
    /// same primary language).
    pub fn best_match<'a>(&self, available: &'a [LocaleId]) -> Option<&'a LocaleId> {
        for candidate in self.fallback_chain(None) {
            if let Some(found) = available.iter().find(|loc| **loc == candidate) {
                return Some(found);
            }
        }
        if let Some(script) = self.script.as_deref()
            && let Some(found) = available
                .iter()
                .find(|loc| loc.language == self.language && loc.script.as_deref() == Some(script))
        {
            return Some(found);
        }
        available.iter().find(|loc| loc.language == self.language)
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
        f.write_str(&self.language)?;
        if let Some(script) = &self.script {
            f.write_str("-")?;
            f.write_str(script)?;
        }
        if let Some(region) = &self.region {
            f.write_str("-")?;
            f.write_str(region)?;
        }
        for variant in &self.variants {
            f.write_str("-")?;
            f.write_str(variant)?;
        }
        Ok(())
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
    (5..=8).contains(&bytes.len())
        || (bytes.len() == 4 && bytes[0].is_ascii_digit())
}

fn title_case_ascii(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (idx, b) in s.bytes().enumerate() {
        let c = if idx == 0 {
            b.to_ascii_uppercase()
        } else {
            b.to_ascii_lowercase()
        };
        out.push(c as char);
    }
    out
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
