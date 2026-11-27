//! Browser appearance for the hydrated docs: `prefers-color-scheme` / `prefers-contrast` as an
//! `nve-system-env` snapshot with live change listeners, the persisted user [`ThemeMode`], and
//! the document chrome (`<html data-theme*>`, body colors, SSR appearance overrides).

use nve_system_env::{ColorScheme, ContrastLevel, ThemeSnapshot};
use nve_ui::{ColorHex, Theme, ThemeMode};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::THEME_MODE_STORAGE_KEY;
use crate::ssr::APPEARANCE_STYLE_ATTR;

const PREFERS_DARK: &str = "(prefers-color-scheme: dark)";
const PREFERS_MORE_CONTRAST: &str = "(prefers-contrast: more)";

fn media_query(query: &str) -> Option<web_sys::MediaQueryList> {
    web_sys::window()?.match_media(query).ok().flatten()
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

/// The browser's appearance as an `nve-system-env` snapshot (`None` scheme when `matchMedia` is
/// unavailable, so `DocsSiteConfig::default_dark` decides).
pub(super) fn browser_theme_snapshot() -> ThemeSnapshot {
    let color_scheme = media_query(PREFERS_DARK).map(|q| if q.matches() { ColorScheme::Dark } else { ColorScheme::Light });
    let contrast = if media_query(PREFERS_MORE_CONTRAST).is_some_and(|q| q.matches()) {
        ContrastLevel::High
    } else {
        ContrastLevel::Standard
    };
    ThemeSnapshot { color_scheme, contrast, ..ThemeSnapshot::default() }
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The user's persisted mode ([`ThemeMode::System`] when unset or unreadable).
pub(super) fn stored_theme_mode() -> ThemeMode {
    local_storage()
        .and_then(|s| s.get_item(THEME_MODE_STORAGE_KEY).ok().flatten())
        .and_then(|v| ThemeMode::parse(&v))
        .unwrap_or_default()
}

pub(super) fn persist_theme_mode(mode: ThemeMode) {
    if let Some(storage) = local_storage() {
        let _ = match mode {
            ThemeMode::System => storage.remove_item(THEME_MODE_STORAGE_KEY),
            pinned => storage.set_item(THEME_MODE_STORAGE_KEY, pinned.as_str()),
        };
    }
}

/// Mirrors the mounted theme onto the document: `<html data-theme-mode>` (absent for System, the
/// attribute the SSR chrome rules key on), `<html data-theme>`, and the body colors.
pub(super) fn sync_document_theme(mode: ThemeMode, theme: &Theme) {
    let Some(doc) = document() else { return };
    if let Some(root) = doc.document_element() {
        let _ = match mode {
            ThemeMode::System => root.remove_attribute("data-theme-mode"),
            pinned => root.set_attribute("data-theme-mode", pinned.as_str()),
        };
        let _ = root.set_attribute("data-theme", if theme.is_dark() { "dark" } else { "light" });
    }
    if let Some(body) = doc.body() {
        let style = format!("margin:0;background-color:{};color:{};", theme.surface.canvas.hex(), theme.text.primary.hex());
        let _ = body.set_attribute("style", &style);
    }
}

/// Drops the SSR node overrides once the client paints every node itself (they key on SSR
/// content classes and would otherwise pin pre-hydration colors on later user mode changes).
pub(super) fn remove_ssr_appearance_overrides() {
    if let Some(Ok(Some(style))) = document().map(|d| d.query_selector(&format!("style[{APPEARANCE_STYLE_ATTR}]"))) {
        style.remove();
    }
}

/// Live `prefers-color-scheme` / `prefers-contrast` change listeners, removed on drop.
pub(super) struct AppearanceListener {
    callback: Closure<dyn FnMut(web_sys::Event)>,
    queries: Vec<web_sys::MediaQueryList>,
}

impl AppearanceListener {
    pub(super) fn install(mut on_change: impl FnMut() + 'static) -> Self {
        let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |_ev: web_sys::Event| on_change());
        let queries: Vec<_> = [PREFERS_DARK, PREFERS_MORE_CONTRAST].into_iter().filter_map(media_query).collect();
        for query in &queries {
            let _ = query.add_event_listener_with_callback("change", callback.as_ref().unchecked_ref());
        }
        Self { callback, queries }
    }
}

impl Drop for AppearanceListener {
    fn drop(&mut self) {
        for query in &self.queries {
            let _ = query.remove_event_listener_with_callback("change", self.callback.as_ref().unchecked_ref());
        }
    }
}
