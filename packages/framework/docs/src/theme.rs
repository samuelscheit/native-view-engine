//! Docs bridges onto the unified [`nve_ui::Theme`]: [`AppBrand::Docs`] accent, OS-following
//! polarity via `nve-system-env`, MDX syntax-token and callout-tone lookups.

use nve_abi::ColorRgba;
use nve_host_services::HostServiceProvider;
use nve_mdx::{CalloutKind, TokenKind};
use nve_system_env::{ColorScheme, SystemEnvClient};
use nve_ui::{AppBrand, Appearance, Syntax, Theme, ThemeConfig, ThemeMode, Tone};

/// Docs theme preferences: Docs brand, following the OS unless the user toggles a mode.
pub const fn docs_theme_config(mode: ThemeMode) -> ThemeConfig {
    ThemeConfig { mode, ..ThemeConfig::new(AppBrand::Docs) }
}

/// Reads the host appearance from `nve-system-env`; `fallback_dark` applies when the host
/// does not report a color scheme.
pub fn system_appearance(provider: &dyn HostServiceProvider, fallback_dark: bool) -> Appearance {
    let fallback = if fallback_dark { Appearance::DARK } else { Appearance::LIGHT };
    match SystemEnvClient::new(provider).get_theme() {
        Ok(snapshot) => Appearance {
            scheme: match snapshot.color_scheme {
                Some(ColorScheme::Dark) => ColorScheme::Dark,
                Some(ColorScheme::Light) => ColorScheme::Light,
                None => fallback.scheme,
            },
            contrast: snapshot.contrast,
        },
        Err(_) => fallback,
    }
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
