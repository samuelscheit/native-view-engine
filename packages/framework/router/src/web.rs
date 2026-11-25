//! Portable, zero-JSON Browser History API adapter for `nve-router`.
//!
//! Provides [`HistoryBackend`], [`MemoryHistoryBackend`] (all targets), [`BrowserHistoryBackend`]
//! (`wasm32` `web-sys` with RAII `popstate` teardown on `Drop`), and [`WebHistoryAdapter`].

use std::fmt;
use std::marker::PhantomData;

use crate::query::{percent_decode, percent_encode_into, split_uri, QueryParams};
use crate::route::{normalize_path, Routable};
use crate::stack::{NavigationAction, NavigationError, NavigationTransition, RouterState};

/// Errors returned by browser History API operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebHistoryError {
    NoWindow,
    HistoryUnavailable,
    PopStateRegistrationFailed,
}

impl fmt::Display for WebHistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoWindow => "browser window object is unavailable",
            Self::HistoryUnavailable => "browser window.history is unavailable",
            Self::PopStateRegistrationFailed => "failed to register popstate listener",
        })
    }
}

impl std::error::Error for WebHistoryError {}

/// Recorded viewport scroll action for [`MemoryHistoryBackend`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrollTarget {
    Top,
    Anchor(String),
}

/// Cross-platform abstraction over browser location, History API, and viewport scrolling.
pub trait HistoryBackend {
    fn current_href(&self) -> String;
    fn push_state(&mut self, url: &str);
    fn replace_state(&mut self, url: &str);
    fn scroll_to_anchor(&mut self, id: &str) -> bool;
    fn scroll_to_top(&mut self);
    fn origin(&self) -> Option<String> { None }
}

/// Deterministic in-memory [`HistoryBackend`] available on all targets for native host tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryHistoryBackend {
    entries: Vec<String>,
    cursor: usize,
    origin: Option<String>,
    push_count: usize,
    replace_count: usize,
    scroll_events: Vec<ScrollTarget>,
}

impl Default for MemoryHistoryBackend {
    fn default() -> Self { Self::new("/") }
}

impl MemoryHistoryBackend {
    pub fn new(initial_href: impl Into<String>) -> Self {
        let initial = initial_href.into();
        Self {
            entries: vec![if initial.is_empty() { "/".to_owned() } else { initial }],
            cursor: 0,
            origin: None,
            push_count: 0,
            replace_count: 0,
            scroll_events: Vec::new(),
        }
    }

    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        let raw = origin.into();
        self.origin = (!raw.is_empty()).then_some(raw);
        self
    }

    #[inline] pub fn entries(&self) -> &[String] { &self.entries }
    #[inline] pub fn cursor(&self) -> usize { self.cursor }
    #[inline] pub fn len(&self) -> usize { self.entries.len() }
    #[inline] pub fn is_empty(&self) -> bool { self.entries.is_empty() }
    #[inline] pub fn can_go_back(&self) -> bool { self.cursor > 0 }
    #[inline] pub fn can_go_forward(&self) -> bool { self.cursor + 1 < self.entries.len() }
    #[inline] pub fn push_count(&self) -> usize { self.push_count }
    #[inline] pub fn replace_count(&self) -> usize { self.replace_count }
    #[inline] pub fn scroll_events(&self) -> &[ScrollTarget] { &self.scroll_events }
    #[inline] pub fn last_scroll(&self) -> Option<&ScrollTarget> { self.scroll_events.last() }
    pub fn clear_scroll_events(&mut self) { self.scroll_events.clear(); }

    /// Simulates clicking browser Back/Forward or `history.go(delta)` before `popstate`.
    pub fn go_back(&mut self) -> Option<&str> { self.go(-1) }
    pub fn go_forward(&mut self) -> Option<&str> { self.go(1) }

    pub fn go(&mut self, delta: isize) -> Option<&str> {
        let target = self.cursor.checked_add_signed(delta)?;
        (target < self.entries.len()).then(|| {
            self.cursor = target;
            self.entries[self.cursor].as_str()
        })
    }

    /// Simulates a browser-initiated URL/hash jump prior to `popstate`.
    pub fn navigate_browser(&mut self, href: impl Into<String>) {
        self.entries.truncate(self.cursor + 1);
        self.entries.push(href.into());
        self.cursor = self.entries.len() - 1;
    }
}

impl HistoryBackend for MemoryHistoryBackend {
    fn current_href(&self) -> String { self.entries[self.cursor].clone() }
    fn push_state(&mut self, url: &str) { self.navigate_browser(url); self.push_count += 1; }
    fn replace_state(&mut self, url: &str) { self.entries[self.cursor] = url.to_owned(); self.replace_count += 1; }
    fn scroll_to_anchor(&mut self, id: &str) -> bool {
        let id = id.strip_prefix('#').unwrap_or(id);
        if id.is_empty() { return false; }
        self.scroll_events.push(ScrollTarget::Anchor(id.to_owned()));
        true
    }
    fn scroll_to_top(&mut self) { self.scroll_events.push(ScrollTarget::Top); }
    fn origin(&self) -> Option<String> { self.origin.clone() }
}

/// Browser `wasm32` [`HistoryBackend`] backed by `web_sys::Window`, `History`, `Location`,
/// `Document::get_element_by_id().scroll_into_view()`, and RAII `popstate` teardown on `Drop`.
#[cfg(target_arch = "wasm32")]
pub struct BrowserHistoryBackend {
    window: web_sys::Window,
    history: web_sys::History,
    location: web_sys::Location,
    popstate_listener: Option<wasm_bindgen::closure::Closure<dyn FnMut(web_sys::PopStateEvent)>>,
}

#[cfg(target_arch = "wasm32")]
impl BrowserHistoryBackend {
    pub fn new() -> Result<Self, WebHistoryError> {
        Self::from_window(web_sys::window().ok_or(WebHistoryError::NoWindow)?)
    }
    pub fn from_window(window: web_sys::Window) -> Result<Self, WebHistoryError> {
        let history = window.history().map_err(|_| WebHistoryError::HistoryUnavailable)?;
        let location = window.location();
        Ok(Self { window, history, location, popstate_listener: None })
    }
    /// Registers a `window` `"popstate"` listener, detaching any previous listener first.
    pub fn register_popstate<F>(&mut self, mut callback: F) -> Result<(), WebHistoryError>
    where
        F: FnMut() + 'static,
    {
        use wasm_bindgen::JsCast;
        self.unregister_popstate();
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::PopStateEvent)>::new(
            move |_ev: web_sys::PopStateEvent| callback(),
        );
        self.window
            .add_event_listener_with_callback("popstate", closure.as_ref().unchecked_ref())
            .map_err(|_| WebHistoryError::PopStateRegistrationFailed)?;
        self.popstate_listener = Some(closure);
        Ok(())
    }
    /// Detaches the active `"popstate"` listener, if any.
    pub fn unregister_popstate(&mut self) {
        use wasm_bindgen::JsCast;
        if let Some(closure) = self.popstate_listener.take() {
            let _ = self
                .window
                .remove_event_listener_with_callback("popstate", closure.as_ref().unchecked_ref());
        }
    }
    #[inline] pub fn has_popstate_listener(&self) -> bool { self.popstate_listener.is_some() }
}

#[cfg(target_arch = "wasm32")]
impl Drop for BrowserHistoryBackend {
    fn drop(&mut self) { self.unregister_popstate(); }
}

#[cfg(target_arch = "wasm32")]
impl HistoryBackend for BrowserHistoryBackend {
    fn current_href(&self) -> String {
        let path = self.location.pathname().unwrap_or_else(|_| "/".to_owned());
        let search = self.location.search().unwrap_or_default();
        let hash = self.location.hash().unwrap_or_default();
        format!("{path}{search}{hash}")
    }
    fn push_state(&mut self, url: &str) {
        let _ = self.history.push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(url));
    }
    fn replace_state(&mut self, url: &str) {
        let _ = self.history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(url));
    }
    fn scroll_to_anchor(&mut self, id: &str) -> bool {
        let id = id.strip_prefix('#').unwrap_or(id);
        let Some(el) = (!id.is_empty())
            .then(|| self.window.document().and_then(|d| d.get_element_by_id(id)))
            .flatten()
        else { return false; };
        el.scroll_into_view();
        true
    }
    fn scroll_to_top(&mut self) { self.window.scroll_to_with_x_and_y(0.0, 0.0); }
    fn origin(&self) -> Option<String> {
        self.location.origin().ok().filter(|o| !o.is_empty() && o != "null")
    }
}

/// Normalizes a deployment `base_path` into `""` (root) or `"/seg1/seg2"` (no trailing slash).
pub fn normalize_base_path(raw: &str) -> String {
    let mut out = String::new();
    for seg in raw.trim().split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        out.push_str(seg);
    }
    out
}

/// `base_path`-aware Browser History adapter synchronizing [`RouterState<R>`] with [`HistoryBackend`].
#[derive(Debug, Clone)]
pub struct WebHistoryAdapter<R: Routable, H: HistoryBackend> {
    base_path: String,
    backend: H,
    _route: PhantomData<fn() -> R>,
}

impl<R: Routable, H: HistoryBackend> WebHistoryAdapter<R, H> {
    pub fn new(base_path: impl AsRef<str>, backend: H) -> Self {
        Self {
            base_path: normalize_base_path(base_path.as_ref()),
            backend,
            _route: PhantomData,
        }
    }

    #[inline] pub fn base_path(&self) -> &str { &self.base_path }
    #[inline] pub fn backend(&self) -> &H { &self.backend }
    #[inline] pub fn backend_mut(&mut self) -> &mut H { &mut self.backend }
    #[inline] pub fn into_backend(self) -> H { self.backend }

    /// Formats the browser-facing `href` for `route`, prefixing `base_path` consistently for SSR
    /// `<a href="...">` links and client History API updates.
    pub fn href_for(&self, route: &R) -> String { self.href_for_path(&route.to_path()) }
    pub fn href_for_path(&self, app_path: &str) -> String {
        if app_path.starts_with('/') {
            format!("{}{app_path}", self.base_path)
        } else {
            format!("{}/{app_path}", self.base_path)
        }
    }
    pub fn strip_base_path(&self, href: &str) -> Option<String> {
        strip_base_from_relative(&self.base_path, extract_relative_part(href))
    }
    pub fn parse_href(&self, href: &str) -> Option<(R, Option<String>)> {
        let app_uri = self.strip_base_path(href)?;
        let (path, query) = QueryParams::from_uri(&app_uri).ok()?;
        let anchor = query.fragment().filter(|f| !f.is_empty()).map(str::to_owned);
        Some((R::parse_path(path, &query)?, anchor))
    }
    /// Parses current `pathname + search + hash` (stripping `base_path`) into the initial route.
    pub fn initial_route(&self) -> Option<R> { self.initial_route_with_anchor().map(|(r, _)| r) }
    pub fn initial_route_with_anchor(&self) -> Option<(R, Option<String>)> {
        self.parse_href(&self.backend.current_href())
    }
    pub fn current_anchor(&self) -> Option<String> {
        decode_anchor_from_href(&self.backend.current_href())
    }
    /// Applies current location onto `router` via `router.replace`, running guards (updating
    /// browser URL via `replace_state` on redirect/block) and scrolling to any `#anchor`.
    pub fn apply_initial_route(&mut self, router: &mut RouterState<R>) -> Option<R> {
        let (target, anchor) = self.initial_route_with_anchor()?;
        let outcome = router.replace(target.clone());
        self.reconcile_browser_transition(router, &target, anchor, outcome)
    }
    /// Pushes `route` into browser history unless already at that exact URL.
    pub fn sync_push(&mut self, route: &R) -> bool { self.apply_history_url(&self.href_for(route), false) }
    pub fn sync_push_with_anchor(&mut self, route: &R, anchor: Option<&str>) -> bool {
        self.apply_history_url(&append_or_replace_anchor(self.href_for(route), anchor), false)
    }
    /// Replaces the current browser history entry with `route` unless already at that exact URL.
    pub fn sync_replace(&mut self, route: &R) -> bool { self.apply_history_url(&self.href_for(route), true) }
    pub fn sync_replace_with_anchor(&mut self, route: &R, anchor: Option<&str>) -> bool {
        self.apply_history_url(&append_or_replace_anchor(self.href_for(route), anchor), true)
    }
    /// Synchronizes a committed [`NavigationTransition`] into browser history.
    pub fn sync_transition(&mut self, transition: &NavigationTransition<R>) -> bool {
        match transition.action {
            NavigationAction::Replace => self.sync_replace(&transition.route),
            NavigationAction::Push | NavigationAction::DeepLink => self.sync_push(&transition.route),
            _ => false,
        }
    }

    /// Synchronizes browser Back/Forward `popstate` into `router` without re-pushing history.
    pub fn handle_popstate(&mut self, router: &mut RouterState<R>) -> Option<R> {
        let (target, anchor) = self.parse_href(&self.backend.current_href())?;
        if router.current_route() == &target {
            if let Some(id) = anchor { self.backend.scroll_to_anchor(&id); }
            return Some(target);
        }
        let outcome = match find_nearest_history_delta(router, &target) {
            Some(delta) => router.go(delta),
            None => router.replace(target.clone()),
        };
        self.reconcile_browser_transition(router, &target, anchor, outcome)
    }

    /// Evaluates browser SPA link-click interception rules, returning the normalized app-relative
    /// URI when safe to intercept, or `None` to let the browser handle the click natively.
    #[allow(clippy::too_many_arguments)]
    pub fn should_intercept_link_click(
        &self,
        href: &str,
        ctrl: bool,
        meta: bool,
        shift: bool,
        alt: bool,
        button: i16,
        target: Option<&str>,
    ) -> Option<String> {
        if ctrl || meta || shift || alt || button != 0 { return None; }
        if let Some(t) = target.map(str::trim)
            && !t.is_empty()
            && !t.eq_ignore_ascii_case("_self")
        {
            return None;
        }
        let trimmed = href.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.bytes().any(|b| b < 0x20 || b == 0x7f)
        {
            return None;
        }

        let current_href = self.backend.current_href();
        let current_parts = split_uri(extract_relative_part(&current_href));
        let current_path = if current_parts.path.is_empty() { "/" } else { current_parts.path };

        let resolved = if let Some(after_slashes) = trimmed.strip_prefix("//") {
            let active_origin = self.backend.origin()?;
            let (scheme, _, _) = parse_origin(&active_origin)?;
            extract_same_origin_relative(&format!("{scheme}://{after_slashes}"), &active_origin)?
        } else if let Some(scheme) = leading_scheme(trimmed) {
            if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
                extract_same_origin_relative(trimmed, &self.backend.origin()?)?
            } else {
                return None;
            }
        } else if trimmed.starts_with('?') {
            format!("{current_path}{trimmed}")
        } else if trimmed.starts_with('/') {
            trimmed.to_owned()
        } else {
            resolve_relative_reference(current_path, trimmed)
        };

        let rp = split_uri(&resolved);
        if rp.raw_fragment.is_some()
            && rp.path == current_path
            && rp.raw_query == current_parts.raw_query
        {
            return None;
        }
        strip_base_from_relative(&self.base_path, &resolved)
    }

    pub fn scroll_to_anchor(&mut self, id: &str) -> bool { self.backend.scroll_to_anchor(id) }
    pub fn scroll_to_top(&mut self) { self.backend.scroll_to_top(); }

    fn reconcile_browser_transition(
        &mut self,
        router: &RouterState<R>,
        requested: &R,
        anchor: Option<String>,
        outcome: Result<NavigationTransition<R>, NavigationError>,
    ) -> Option<R> {
        match outcome {
            Ok(tr) => {
                if &tr.route != requested {
                    self.sync_replace(&tr.route);
                } else if let Some(id) = anchor {
                    self.backend.scroll_to_anchor(&id);
                }
                Some(tr.route)
            }
            Err(_) => {
                self.sync_replace(router.current_route());
                None
            }
        }
    }

    fn apply_history_url(&mut self, target_url: &str, replace: bool) -> bool {
        if extract_relative_part(&self.backend.current_href()) == target_url {
            return false;
        }
        if replace { self.backend.replace_state(target_url); } else { self.backend.push_state(target_url); }
        if let Some(anchor) = decode_anchor_from_href(target_url) {
            self.backend.scroll_to_anchor(&anchor);
        } else {
            self.backend.scroll_to_top();
        }
        true
    }
}

fn find_nearest_history_delta<R: Routable>(router: &RouterState<R>, target: &R) -> Option<isize> {
    let (entries, cursor) = (router.entries(), router.cursor());
    let max_dist = cursor.max(entries.len().saturating_sub(1).saturating_sub(cursor));
    for dist in 1..=max_dist {
        if dist <= cursor && &entries[cursor - dist].route == target { return Some(-(dist as isize)); }
        if cursor + dist < entries.len() && &entries[cursor + dist].route == target { return Some(dist as isize); }
    }
    None
}

fn extract_relative_part(href: &str) -> &str {
    if let Some(scheme) = leading_scheme(href)
        && let Some(after) = href[scheme.len()..].strip_prefix("://")
    {
        return after.find(['/', '?', '#']).map_or("/", |idx| &after[idx..]);
    }
    href
}

fn reassemble_uri(path: &str, raw_query: Option<&str>, raw_fragment: Option<&str>) -> String {
    let mut out = String::with_capacity(
        path.len() + raw_query.map_or(0, |q| q.len() + 1) + raw_fragment.map_or(0, |f| f.len() + 1),
    );
    out.push_str(path);
    if let Some(q) = raw_query { out.push('?'); out.push_str(q); }
    if let Some(f) = raw_fragment { out.push('#'); out.push_str(f); }
    out
}

fn strip_base_from_relative(base_path: &str, relative: &str) -> Option<String> {
    let parts = split_uri(relative);
    let normalized = normalize_path(if parts.path.is_empty() { "/" } else { parts.path })?;
    let app_path: &str = if base_path.is_empty() {
        &normalized
    } else if normalized == base_path {
        "/"
    } else if let Some(rest) = normalized.strip_prefix(base_path) && rest.starts_with('/') {
        rest
    } else {
        return None;
    };
    Some(reassemble_uri(app_path, parts.raw_query, parts.raw_fragment))
}

fn decode_anchor_from_href(href: &str) -> Option<String> {
    let raw = split_uri(extract_relative_part(href)).raw_fragment?;
    if raw.is_empty() { return None; }
    let decoded = percent_decode(raw).ok()?;
    (!decoded.is_empty()).then(|| decoded.into_owned())
}

fn append_or_replace_anchor(mut url: String, anchor: Option<&str>) -> String {
    if let Some(hash_idx) = url.find('#') {
        if anchor.is_some() { url.truncate(hash_idx); } else { return url; }
    }
    if let Some(clean) = anchor.map(|a| a.strip_prefix('#').unwrap_or(a)).filter(|a| !a.is_empty()) {
        url.push('#');
        percent_encode_into(clean, &mut url);
    }
    url
}

fn leading_scheme(input: &str) -> Option<&str> {
    let colon = input.find(':')?;
    if input[..colon].contains(['/', '?', '#']) { return None; }
    let scheme = &input[..colon];
    let mut bytes = scheme.bytes();
    if !bytes.next()?.is_ascii_alphabetic() { return None; }
    bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')).then_some(scheme)
}

fn parse_origin(url: &str) -> Option<(String, String, u16)> {
    let scheme_raw = leading_scheme(url)?;
    let after = url[scheme_raw.len()..].strip_prefix("://")?;
    let scheme = scheme_raw.to_ascii_lowercase();
    let default_port = match scheme.as_str() { "http" => 80u16, "https" => 443u16, _ => return None };
    let authority = &after[..after.find(['/', '?', '#']).unwrap_or(after.len())];
    if authority.is_empty() || authority.contains('@') { return None; }
    let (host, port) = if let Some(end) = authority.rfind(']') {
        if !authority.starts_with('[') { return None; }
        let rest = &authority[end + 1..];
        let port = if rest.is_empty() { default_port } else { rest.strip_prefix(':')?.parse().ok()? };
        (&authority[..=end], port)
    } else if let Some((h, p)) = authority.rsplit_once(':') {
        if h.is_empty() { return None; }
        (h, p.parse().ok()?)
    } else {
        (authority, default_port)
    };
    Some((scheme, host.to_ascii_lowercase(), port))
}

fn extract_same_origin_relative(href: &str, active_origin: &str) -> Option<String> {
    if parse_origin(href)? != parse_origin(active_origin)? { return None; }
    let rel = extract_relative_part(href);
    Some(if rel.starts_with('/') { rel.to_owned() } else { format!("/{rel}") })
}

fn resolve_relative_reference(current_path: &str, relative: &str) -> String {
    let parts = split_uri(relative);
    let base_dir = current_path.rfind('/').map_or("/", |idx| &current_path[..=idx]);
    let combined = format!("{base_dir}{}", parts.path);
    let mut stack: Vec<&str> = Vec::new();
    for seg in combined.split('/') {
        match seg {
            "" | "." => {}
            ".." => { stack.pop(); }
            s => stack.push(s),
        }
    }
    let path = if stack.is_empty() { "/".to_owned() } else { format!("/{}", stack.join("/")) };
    reassemble_uri(&path, parts.raw_query, parts.raw_fragment)
}
