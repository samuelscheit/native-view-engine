//! Fumadocs × Linear design tokens, dark/light palettes, syntax colors, and `UiTheme`/`SystemEnv` sync.

use std::borrow::Cow;

use nve_mdx::{CalloutKind, TokenKind};
use nve_sdk::{ColorScheme as SdkColorScheme, HostServiceProvider, SystemEnvClient};
use nve_ui::{UiPalette, UiTheme};

/// Syntax highlighting palette covering every [`nve_mdx::TokenKind`] variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxPalette {
    pub plain: &'static str,
    pub keyword: &'static str,
    pub type_name: &'static str,
    pub string_lit: &'static str,
    pub comment: &'static str,
    pub number: &'static str,
    pub function: &'static str,
    pub macro_name: &'static str,
    pub attribute: &'static str,
    pub punctuation: &'static str,
    pub diff_add: &'static str,
    pub diff_del: &'static str,
}

impl SyntaxPalette {
    /// High-contrast dark-mode syntax tokens tuned for `#0c0c0f` code surfaces.
    pub const fn dark() -> Self {
        Self {
            plain: "#e4e4e7",
            keyword: "#c084fc",
            type_name: "#38bdf8",
            string_lit: "#4ade80",
            comment: "#71717a",
            number: "#fb923c",
            function: "#60a5fa",
            macro_name: "#f472b6",
            attribute: "#facc15",
            punctuation: "#a1a1aa",
            diff_add: "#22c55e",
            diff_del: "#f87171",
        }
    }

    /// Crisp light-mode syntax tokens tuned for `#f8fafc` code surfaces.
    pub const fn light() -> Self {
        Self {
            plain: "#18181b",
            keyword: "#7c3aed",
            type_name: "#0284c7",
            string_lit: "#15803d",
            comment: "#71717a",
            number: "#c2410c",
            function: "#2563eb",
            macro_name: "#be185d",
            attribute: "#a16207",
            punctuation: "#52525b",
            diff_add: "#16a34a",
            diff_del: "#dc2626",
        }
    }

    /// Resolve the foreground hex color for any [`TokenKind`] exhaustively.
    pub const fn color_for(&self, kind: TokenKind) -> &'static str {
        match kind {
            TokenKind::Plain => self.plain,
            TokenKind::Keyword => self.keyword,
            TokenKind::TypeName => self.type_name,
            TokenKind::StringLit => self.string_lit,
            TokenKind::Comment => self.comment,
            TokenKind::Number => self.number,
            TokenKind::Function => self.function,
            TokenKind::Macro => self.macro_name,
            TokenKind::Attribute => self.attribute,
            TokenKind::Punctuation => self.punctuation,
            TokenKind::DiffAdd => self.diff_add,
            TokenKind::DiffDel => self.diff_del,
        }
    }
}

/// Visual tokens for a `:::callout` or `<Callout>` semantic tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalloutTone {
    pub border: &'static str,
    pub bg: &'static str,
    pub title: &'static str,
    pub badge_text: &'static str,
    pub icon: &'static str,
}

/// Fumadocs × Linear documentation theme with synchronized dark and light palettes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocsTheme {
    pub is_dark: bool,
    pub canvas: &'static str,
    pub surface: &'static str,
    pub surface_subtle: &'static str,
    pub border: &'static str,
    pub text: &'static str,
    pub muted: &'static str,
    pub accent: &'static str,
    pub accent_alt: &'static str,
    pub accent_fg: &'static str,
    pub accent_surface: &'static str,
    pub code_bg: &'static str,
    pub code_line_highlight: &'static str,
    pub danger: &'static str,
    pub success: &'static str,
    pub warning: &'static str,
    pub syntax: SyntaxPalette,
}

impl Default for DocsTheme {
    fn default() -> Self {
        Self::dark()
    }
}

impl DocsTheme {
    /// Fumadocs × Linear dark-first palette.
    pub const fn dark() -> Self {
        Self {
            is_dark: true,
            canvas: "#09090b",
            surface: "#111114",
            surface_subtle: "#18181b",
            border: "#27272a",
            text: "#fafafa",
            muted: "#a1a1aa",
            accent: "#38bdf8",
            accent_alt: "#818cf8",
            accent_fg: "#09090b",
            accent_surface: "#0c2231",
            code_bg: "#0c0c0f",
            code_line_highlight: "#172536",
            danger: "#f87171",
            success: "#4ade80",
            warning: "#fbbf24",
            syntax: SyntaxPalette::dark(),
        }
    }

    /// Crisp Fumadocs × Linear light palette.
    pub const fn light() -> Self {
        Self {
            is_dark: false,
            canvas: "#ffffff",
            surface: "#fafafa",
            surface_subtle: "#f4f4f5",
            border: "#e4e4e7",
            text: "#18181b",
            muted: "#52525b",
            accent: "#0284c7",
            accent_alt: "#4f46e5",
            accent_fg: "#ffffff",
            accent_surface: "#e0f2fe",
            code_bg: "#f8fafc",
            code_line_highlight: "#e0f2fe",
            danger: "#dc2626",
            success: "#16a34a",
            warning: "#d97706",
            syntax: SyntaxPalette::light(),
        }
    }

    /// Select dark or light theme explicitly.
    pub const fn for_dark(dark: bool) -> Self {
        if dark {
            Self::dark()
        } else {
            Self::light()
        }
    }

    /// Resolve initial dark/light mode from `nve_sdk::SystemEnvClient`, falling back to `default_dark`.
    pub fn resolve_dark_from_system_env(
        provider: &dyn HostServiceProvider,
        default_dark: bool,
    ) -> bool {
        let client = SystemEnvClient::new(provider);
        match client.get_theme() {
            Ok(snapshot) => match snapshot.color_scheme {
                SdkColorScheme::Dark => true,
                SdkColorScheme::Light => false,
                SdkColorScheme::Unspecified => default_dark,
            },
            Err(_) => default_dark,
        }
    }

    /// Construct a `DocsTheme` synchronized with the host's `SystemEnv` capability.
    pub fn from_system_env(provider: &dyn HostServiceProvider, default_dark: bool) -> Self {
        Self::for_dark(Self::resolve_dark_from_system_env(provider, default_dark))
    }

    /// Build a matching [`nve_ui::UiTheme`] so `Dialog`, `Input`, `Button`, `Badge`, `Card`, and
    /// `TabsRoot` inherit the exact Fumadocs × Linear palette.
    pub fn to_ui_theme(&self) -> UiTheme {
        let mut base = if self.is_dark {
            UiTheme::dark()
        } else {
            UiTheme::light()
        };
        base.palette = UiPalette {
            surface: Cow::Borrowed(self.canvas),
            surface_elevated: Cow::Borrowed(self.surface),
            surface_subtle: Cow::Borrowed(self.surface_subtle),
            text_primary: Cow::Borrowed(self.text),
            text_secondary: Cow::Borrowed(self.muted),
            text_muted: Cow::Borrowed(self.muted),
            border: Cow::Borrowed(self.border),
            border_focus: Cow::Borrowed(self.accent),
            accent: Cow::Borrowed(self.accent),
            accent_fg: Cow::Borrowed(self.accent_fg),
            danger: Cow::Borrowed(self.danger),
            danger_fg: Cow::Borrowed(if self.is_dark { "#09090b" } else { "#ffffff" }),
            success: Cow::Borrowed(self.success),
            warning: Cow::Borrowed(self.warning),
        };
        base
    }

    /// Resolve syntax color for a single [`TokenKind`].
    pub const fn token_color(&self, kind: TokenKind) -> &'static str {
        self.syntax.color_for(kind)
    }

    /// Semantic border, tint background, heading color, and icon glyph for a callout block.
    pub const fn callout_tone(&self, kind: CalloutKind) -> CalloutTone {
        if self.is_dark {
            match kind {
                CalloutKind::Tip => CalloutTone {
                    border: "#10b981",
                    bg: "#071f17",
                    title: "#34d399",
                    badge_text: "TIP",
                    icon: "✦",
                },
                CalloutKind::Info => CalloutTone {
                    border: "#38bdf8",
                    bg: "#0a1c28",
                    title: "#7dd3fc",
                    badge_text: "INFO",
                    icon: "i",
                },
                CalloutKind::Warning => CalloutTone {
                    border: "#f59e0b",
                    bg: "#241907",
                    title: "#fbbf24",
                    badge_text: "WARNING",
                    icon: "!",
                },
                CalloutKind::Danger => CalloutTone {
                    border: "#ef4444",
                    bg: "#270d10",
                    title: "#f87171",
                    badge_text: "DANGER",
                    icon: "×",
                },
                CalloutKind::Note => CalloutTone {
                    border: "#818cf8",
                    bg: "#15152b",
                    title: "#a5b4fc",
                    badge_text: "NOTE",
                    icon: "•",
                },
            }
        } else {
            match kind {
                CalloutKind::Tip => CalloutTone {
                    border: "#059669",
                    bg: "#ecfdf5",
                    title: "#047857",
                    badge_text: "TIP",
                    icon: "✦",
                },
                CalloutKind::Info => CalloutTone {
                    border: "#0284c7",
                    bg: "#f0f9ff",
                    title: "#0369a1",
                    badge_text: "INFO",
                    icon: "i",
                },
                CalloutKind::Warning => CalloutTone {
                    border: "#d97706",
                    bg: "#fffbeb",
                    title: "#b45309",
                    badge_text: "WARNING",
                    icon: "!",
                },
                CalloutKind::Danger => CalloutTone {
                    border: "#dc2626",
                    bg: "#fef2f2",
                    title: "#b91c1c",
                    badge_text: "DANGER",
                    icon: "×",
                },
                CalloutKind::Note => CalloutTone {
                    border: "#4f46e5",
                    bg: "#eef2ff",
                    title: "#4338ca",
                    badge_text: "NOTE",
                    icon: "•",
                },
            }
        }
    }
}
