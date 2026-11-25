//! Multi-locale [`MessageCatalog`] and statically-dispatched
//! [`Translator<B = NativeI18nBackend>`] with fallback chain resolution,
//! pseudo-localization mode, and missing-key diagnostics.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::backend::{I18nBackend, NativeI18nBackend};
use crate::error::{I18nError, MissingKeyDiagnostic};
use crate::format::{CalendarDate, DateStyle, NumberFormatOptions, RelativeUnit, Weekday};
use crate::locale::{IconMirrorPolicy, LocaleId, TextDirection};
use crate::message::{ArgLookup, format_message_with_backend, parse_message};
use crate::plural::{PluralCategory, PluralKind, PluralOperands};
use crate::pseudo::{PseudoLocaleOptions, PseudoLocalizer};

/// Multi-locale catalog of ICU MessageFormat templates with syntax validation
/// on insert and deterministic BCP-47 fallback lookup.
#[derive(Debug, Clone)]
pub struct MessageCatalog {
    default_locale: LocaleId,
    bundles: BTreeMap<LocaleId, BTreeMap<String, String>>,
}

impl MessageCatalog {
    /// Creates an empty catalog with `default_locale` as the final fallback.
    pub fn new(default_locale: LocaleId) -> Self {
        Self {
            default_locale,
            bundles: BTreeMap::new(),
        }
    }

    /// Returns the catalog's default fallback locale.
    #[inline]
    pub fn default_locale(&self) -> &LocaleId {
        &self.default_locale
    }

    /// Validates and inserts a single message template for `(locale, key)`.
    pub fn insert(
        &mut self,
        locale: LocaleId,
        key: impl Into<String>,
        template: impl Into<String>,
    ) -> Result<(), I18nError> {
        let template = template.into();
        let _ = parse_message(&template)?;
        self.bundles
            .entry(locale)
            .or_default()
            .insert(key.into(), template);
        Ok(())
    }

    /// Validates and inserts a batch of `(key, template)` entries for `locale`.
    pub fn insert_bundle<K, V>(
        &mut self,
        locale: LocaleId,
        entries: impl IntoIterator<Item = (K, V)>,
    ) -> Result<(), I18nError>
    where
        K: Into<String>,
        V: Into<String>,
    {
        let mut validated: Vec<(String, String)> = Vec::new();
        for (k, v) in entries {
            let tpl = v.into();
            let _ = parse_message(&tpl)?;
            validated.push((k.into(), tpl));
        }
        let map = self.bundles.entry(locale).or_default();
        for (k, tpl) in validated {
            map.insert(k, tpl);
        }
        Ok(())
    }

    /// Iterates over all locales currently stored in the catalog.
    pub fn locales(&self) -> impl Iterator<Item = &LocaleId> {
        self.bundles.keys()
    }

    /// Resolves `key` for `locale` by walking `locale.fallback_chain(Some(default_locale))`.
    pub fn lookup<'a>(&'a self, locale: &LocaleId, key: &str) -> Option<(&'a LocaleId, &'a str)> {
        let chain = locale.fallback_chain(Some(&self.default_locale));
        for candidate in &chain {
            if let Some((stored_locale, entries)) = self.bundles.get_key_value(candidate)
                && let Some(template) = entries.get(key)
            {
                return Some((stored_locale, template.as_str()));
            }
        }
        None
    }

    /// Resolves `key` for `locale` or returns a structured [`MissingKeyDiagnostic`]
    /// recording the exact fallback chain searched.
    pub fn lookup_with_trace<'a>(
        &'a self,
        locale: &LocaleId,
        key: &str,
    ) -> Result<(&'a LocaleId, &'a str), Box<MissingKeyDiagnostic>> {
        let chain = locale.fallback_chain(Some(&self.default_locale));
        for candidate in &chain {
            if let Some((stored_locale, entries)) = self.bundles.get_key_value(candidate)
                && let Some(template) = entries.get(key)
            {
                return Ok((stored_locale, template.as_str()));
            }
        }
        Err(Box::new(MissingKeyDiagnostic {
            key: key.to_owned(),
            requested_locale: locale.clone(),
            tried_locales: chain,
        }))
    }

    /// Audits `target_locale` against the default locale bundle, returning all
    /// keys present in `default_locale` that are not translated anywhere in
    /// `target_locale`'s own fallback chain (excluding `default_locale`).
    pub fn missing_keys(&self, target_locale: &LocaleId) -> Vec<String> {
        let Some(base_entries) = self.bundles.get(&self.default_locale) else {
            return Vec::new();
        };
        let target_chain = target_locale.fallback_chain(None);
        let mut missing = Vec::new();
        for key in base_entries.keys() {
            let found = target_chain.iter().any(|candidate| {
                self.bundles
                    .get(candidate)
                    .is_some_and(|bundle| bundle.contains_key(key))
            });
            if !found {
                missing.push(key.clone());
            }
        }
        missing
    }

    /// Converts this catalog into a [`Translator`] backed by [`NativeI18nBackend`].
    pub fn into_translator(self, locale: LocaleId) -> Translator<NativeI18nBackend> {
        Translator::new(self, locale)
    }

    /// Converts this catalog into a [`Translator`] backed by a custom [`I18nBackend`].
    pub fn into_translator_with_backend<B: I18nBackend>(
        self,
        locale: LocaleId,
        backend: B,
    ) -> Translator<B> {
        Translator::with_backend(self, locale, backend)
    }
}

/// Statically-dispatched translator (`Translator<B = NativeI18nBackend>`)
/// combining [`MessageCatalog`] fallback resolution, ICU MessageFormat,
/// platform-first [`I18nBackend`] formatting, pseudo-localization QA mode,
/// and missing-key diagnostics.
#[derive(Debug)]
pub struct Translator<B: I18nBackend = NativeI18nBackend> {
    catalog: MessageCatalog,
    locale: LocaleId,
    backend: B,
    pseudo_localize: bool,
    pseudo_localizer: PseudoLocalizer,
    diagnostics: Mutex<Vec<MissingKeyDiagnostic>>,
}

impl Translator<NativeI18nBackend> {
    /// Creates a [`Translator`] using the platform-first [`NativeI18nBackend`].
    pub fn new(catalog: MessageCatalog, locale: LocaleId) -> Self {
        Self::with_backend(catalog, locale, NativeI18nBackend)
    }
}

impl<B: I18nBackend> Translator<B> {
    /// Creates a [`Translator`] with an explicit [`I18nBackend`] (static generic dispatch).
    pub fn with_backend(catalog: MessageCatalog, locale: LocaleId, backend: B) -> Self {
        Self {
            catalog,
            locale,
            backend,
            pseudo_localize: false,
            pseudo_localizer: PseudoLocalizer::default(),
            diagnostics: Mutex::new(Vec::new()),
        }
    }

    /// Returns the active [`LocaleId`].
    #[inline]
    pub fn locale(&self) -> &LocaleId {
        &self.locale
    }

    /// Updates the active locale.
    #[inline]
    pub fn set_locale(&mut self, locale: LocaleId) {
        self.locale = locale;
    }

    /// Returns `true` if the active locale is right-to-left.
    #[inline]
    pub fn is_rtl(&self) -> bool {
        self.locale.is_rtl()
    }

    /// Returns the active [`TextDirection`].
    #[inline]
    pub fn text_direction(&self) -> TextDirection {
        self.locale.text_direction()
    }

    /// Returns whether an icon with `semantic` policy should mirror in the active locale.
    #[inline]
    pub fn should_mirror_icon(&self, semantic: IconMirrorPolicy) -> bool {
        self.locale.should_mirror_icon(semantic)
    }

    /// Returns a reference to the underlying [`I18nBackend`].
    #[inline]
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Returns a reference to the underlying [`MessageCatalog`].
    #[inline]
    pub fn catalog(&self) -> &MessageCatalog {
        &self.catalog
    }

    /// Returns a mutable reference to the underlying [`MessageCatalog`].
    #[inline]
    pub fn catalog_mut(&mut self) -> &mut MessageCatalog {
        &mut self.catalog
    }

    /// Enables or disables pseudo-localization QA mode.
    #[inline]
    pub fn set_pseudo_localize(&mut self, enabled: bool) {
        self.pseudo_localize = enabled;
    }

    /// Builder-style toggle for pseudo-localization QA mode.
    #[inline]
    pub fn with_pseudo_localize(mut self, enabled: bool) -> Self {
        self.pseudo_localize = enabled;
        self
    }

    /// Customizes [`PseudoLocaleOptions`] used when pseudo-localization is active.
    #[inline]
    pub fn set_pseudo_options(&mut self, options: PseudoLocaleOptions) {
        self.pseudo_localizer = PseudoLocalizer::new(options);
    }

    /// Returns whether pseudo-localization mode is currently active.
    #[inline]
    pub fn is_pseudo_localize(&self) -> bool {
        self.pseudo_localize
    }

    /// Strictly resolves and formats `key` with `args`.
    ///
    /// Records a [`MissingKeyDiagnostic`] whenever `key` cannot be resolved.
    pub fn try_t<A: ArgLookup + ?Sized>(&self, key: &str, args: &A) -> Result<String, I18nError> {
        let (_resolved_locale, template) = match self.catalog.lookup_with_trace(&self.locale, key) {
            Ok(hit) => hit,
            Err(diag) => {
                if let Ok(mut guard) = self.diagnostics.lock() {
                    guard.push((*diag).clone());
                }
                return Err(I18nError::MissingKey(diag));
            }
        };

        if self.pseudo_localize {
            let pseudo_tpl = self.pseudo_localizer.transform(template);
            format_message_with_backend(&self.backend, &self.locale, &pseudo_tpl, args)
        } else {
            format_message_with_backend(&self.backend, &self.locale, template, args)
        }
    }

    /// UI-safe translation lookup with zero arguments.
    ///
    /// On a missing key (or argument error), records diagnostics and returns
    /// `key` directly without panicking.
    pub fn t(&self, key: &str) -> String {
        self.t_args(key, &())
    }

    /// UI-safe translation lookup with arguments.
    ///
    /// On a missing key (or argument error), records diagnostics and returns
    /// `key` directly without panicking.
    pub fn t_args<A: ArgLookup + ?Sized>(&self, key: &str, args: &A) -> String {
        self.try_t(key, args).unwrap_or_else(|_| key.to_owned())
    }

    /// Returns a snapshot of recorded missing-key diagnostics.
    pub fn missing_diagnostics(&self) -> Vec<MissingKeyDiagnostic> {
        self.diagnostics
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Drains and returns all recorded missing-key diagnostics.
    pub fn take_missing_diagnostics(&self) -> Vec<MissingKeyDiagnostic> {
        self.diagnostics
            .lock()
            .map(|mut guard| std::mem::take(&mut *guard))
            .unwrap_or_default()
    }

    /// Clears all recorded missing-key diagnostics.
    pub fn clear_diagnostics(&self) {
        if let Ok(mut guard) = self.diagnostics.lock() {
            guard.clear();
        }
    }

    /// Evaluates the CLDR plural category for the active locale via `self.backend`.
    #[inline]
    pub fn plural_category(&self, kind: PluralKind, operands: PluralOperands) -> PluralCategory {
        self.backend.plural_category(&self.locale, kind, operands)
    }

    /// Formats a number for the active locale via `self.backend`.
    #[inline]
    pub fn format_number(&self, value: f64, opts: NumberFormatOptions) -> String {
        self.backend.format_number(&self.locale, value, opts)
    }

    /// Formats an integer for the active locale via `self.backend`.
    #[inline]
    pub fn format_integer(&self, value: i64) -> String {
        self.backend.format_integer(&self.locale, value)
    }

    /// Formats a currency amount (ISO-4217 code) for the active locale via `self.backend`.
    #[inline]
    pub fn format_currency(&self, amount: f64, currency: &str) -> String {
        self.backend.format_currency(&self.locale, amount, currency)
    }

    /// Formats a calendar date for the active locale via `self.backend`.
    #[inline]
    pub fn format_date(&self, date: CalendarDate, style: DateStyle) -> String {
        self.backend.format_date(&self.locale, date, style)
    }

    /// Formats a relative time offset for the active locale via `self.backend`.
    #[inline]
    pub fn format_relative_time(&self, value: i64, unit: RelativeUnit) -> String {
        self.backend.format_relative_time(&self.locale, value, unit)
    }

    /// Returns the first day of the week for the active locale via `self.backend`.
    #[inline]
    pub fn first_day_of_week(&self) -> Weekday {
        self.backend.first_day_of_week(&self.locale)
    }
}
