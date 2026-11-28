//! Docs bridges onto the unified [`nve_ui::Theme`]: [`AppBrand::Docs`] accent, OS-following
//! polarity via `nve-system-env`, MDX syntax-token and callout-tone lookups.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use nve_abi::ColorRgba;
use nve_host_services::HostServiceProvider;
use nve_mdx::{CalloutKind, TokenKind};
use nve_system_env::{SystemEnvClient, ThemeSnapshot};
use nve_ui::{AppBrand, Appearance, Syntax, Theme, ThemeConfig, ThemeMode, Tone};

/// The user's [`ThemeMode`], shared between a mounted docs surface and its host: the surface
/// writes it (through [`crate::DocsAppState::set_theme_mode`]), hosts read it to persist the
/// choice and to paint document chrome, and rebuilt surfaces mounted with the same cell keep it.
/// Clones share one value; equality is identity.
#[derive(Debug, Clone, Default)]
pub struct ThemeModeCell(Arc<AtomicU8>);

impl ThemeModeCell {
    pub fn new(mode: ThemeMode) -> Self {
        Self(Arc::new(AtomicU8::new(mode.as_u8())))
    }

    pub fn get(&self) -> ThemeMode {
        ThemeMode::from_u8(self.0.load(Ordering::Relaxed))
    }

    pub fn set(&self, mode: ThemeMode) {
        self.0.store(mode.as_u8(), Ordering::Relaxed);
    }
}

impl PartialEq for ThemeModeCell {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ThemeModeCell {}

/// Docs theme preferences: Docs brand, following the OS unless the user toggles a mode.
pub const fn docs_theme_config(mode: ThemeMode) -> ThemeConfig {
    ThemeConfig { mode, ..ThemeConfig::new(AppBrand::Docs) }
}

/// Reads the host appearance from `nve-system-env`; `fallback_dark` applies when the host
/// does not report a color scheme.
pub fn system_appearance(provider: &dyn HostServiceProvider, fallback_dark: bool) -> Appearance {
    match SystemEnvClient::new(provider).get_theme() {
        Ok(snapshot) => snapshot_appearance(&snapshot, fallback_dark),
        Err(_) => fallback_appearance(fallback_dark),
    }
}

/// Maps an `nve-system-env` [`ThemeSnapshot`] to an [`Appearance`]; `fallback_dark` applies
/// when the host does not report a color scheme.
pub const fn snapshot_appearance(snapshot: &ThemeSnapshot, fallback_dark: bool) -> Appearance {
    Appearance {
        scheme: match snapshot.color_scheme {
            Some(scheme) => scheme,
            None => fallback_appearance(fallback_dark).scheme,
        },
        contrast: snapshot.contrast,
    }
}

const fn fallback_appearance(fallback_dark: bool) -> Appearance {
    if fallback_dark { Appearance::DARK } else { Appearance::LIGHT }
}

/// Foreground color of an MDX syntax token.
pub const fn syntax_color(syntax: &Syntax, kind: TokenKind) -> ColorRgba {
    match kind {
        TokenKind::Plain => syntax.plain,
        TokenKind::Keyword => syntax.keyword,
        TokenKind::TypeName => syntax.type_name,
        TokenKind::StringLit => syntax.string,
        TokenKind::Comment => syntax.comment,
        TokenKind::Number => syntax.number,
        TokenKind::Function => syntax.function,
        TokenKind::Macro => syntax.macro_name,
        TokenKind::Attribute => syntax.attribute,
        TokenKind::Punctuation => syntax.punctuation,
        TokenKind::DiffAdd => syntax.diff_add,
        TokenKind::DiffDel => syntax.diff_del,
    }
}

/// Semantic tone plus badge label and glyph for a `:::callout` / `<Callout>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalloutTone {
    pub colors: Tone,
    pub label: &'static str,
    pub icon: &'static str,
}

pub const fn callout_tone(theme: &Theme, kind: CalloutKind) -> CalloutTone {
    let s = &theme.status;
    let (colors, label, icon) = match kind {
        CalloutKind::Tip => (s.success, "TIP", "✦"),
        CalloutKind::Info => (s.info, "INFO", "i"),
        CalloutKind::Warning => (s.warning, "WARNING", "!"),
        CalloutKind::Danger => (s.danger, "DANGER", "×"),
        CalloutKind::Note => (s.accent, "NOTE", "•"),
    };
    CalloutTone { colors, label, icon }
}
