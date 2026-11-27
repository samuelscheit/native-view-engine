//! Browser OS appearance → app host events: the `prefers-color-scheme`, `prefers-contrast` and
//! `prefers-reduced-motion` media queries become one `nve_system_env::ThemeSnapshot`, delivered
//! through `NveApp::dispatch_host_events` at boot and on every OS change, so embedded apps follow
//! the four OS polarities like native hosts do.

use nve_abi::{ColorScheme, ContrastLevel};
use nve_system_env::ThemeSnapshot;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

use crate::session::WebAppSession;

const DARK_QUERY: &str = "(prefers-color-scheme: dark)";
const CONTRAST_QUERY: &str = "(prefers-contrast: more)";
const REDUCED_MOTION_QUERY: &str = "(prefers-reduced-motion: reduce)";

fn query_matches(window: &web_sys::Window, query: &str) -> bool {
    window.match_media(query).ok().flatten().is_some_and(|mql| mql.matches())
}

/// The browser's current appearance as a host [`ThemeSnapshot`] (accent and scale fields keep
/// their defaults: the apps own their accents, and the viewport carries the scale factor).
pub(crate) fn browser_theme_snapshot(window: &web_sys::Window) -> ThemeSnapshot {
    ThemeSnapshot {
        color_scheme: Some(if query_matches(window, DARK_QUERY) { ColorScheme::Dark } else { ColorScheme::Light }),
        contrast: if query_matches(window, CONTRAST_QUERY) { ContrastLevel::High } else { ContrastLevel::Standard },
        reduced_motion: query_matches(window, REDUCED_MOTION_QUERY),
        ..ThemeSnapshot::default()
    }
}

/// Delivers `theme` to the session's app (rendering when the UI changed); failures are logged.
/// Appearance is host environment, not app state, so it schedules no snapshot.
pub(crate) fn dispatch_appearance(session: &mut WebAppSession, theme: ThemeSnapshot) {
    if let Err(err) = session.dispatch_host_events(&theme.to_repr_c_bytes()) {
        web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web] appearance: {err}")));
    }
}

/// Keeps the `change` listeners of the appearance media queries alive; dropping it detaches them.
pub(crate) struct AppearanceListener {
    queries: Vec<web_sys::MediaQueryList>,
    on_change: Closure<dyn FnMut(web_sys::Event)>,
}

impl Drop for AppearanceListener {
    fn drop(&mut self) {
        for mql in &self.queries {
            let _ = mql.remove_event_listener_with_callback("change", self.on_change.as_ref().unchecked_ref());
        }
    }
}

/// Calls `on_change` with the fresh [`browser_theme_snapshot`] whenever any appearance query flips.
pub(crate) fn install_appearance_listener<F>(window: &web_sys::Window, on_change: F) -> Option<AppearanceListener>
where
    F: Fn(ThemeSnapshot) + 'static,
{
    let win = window.clone();
    let on_change = Closure::<dyn FnMut(web_sys::Event)>::new(move |_ev: web_sys::Event| {
        on_change(browser_theme_snapshot(&win));
    });
    let queries: Vec<web_sys::MediaQueryList> = [DARK_QUERY, CONTRAST_QUERY, REDUCED_MOTION_QUERY]
        .into_iter()
        .filter_map(|q| window.match_media(q).ok().flatten())
        .filter(|mql| mql.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref()).is_ok())
        .collect();
    (!queries.is_empty()).then_some(AppearanceListener { queries, on_change })
}
