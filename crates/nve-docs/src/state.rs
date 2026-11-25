//! Reactive documentation application state, responsive breakpoints, and surface bootstrap handoff.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use dioxus_core::consume_context;
use dioxus_signals::{ReadableExt, Signal, WritableExt};
use nve_mdx::{CompiledPage, CompiledSite, NavNode, PrevNext, SearchHit, SearchIndex};
use nve_router::try_use_navigator;
use nve_sdk::{ClipboardClient, HostServiceProvider};

use crate::config::DocsSiteConfig;
use crate::registry::PlaygroundRegistry;
use crate::route::DocsRoute;
use crate::theme::DocsTheme;

/// Viewport width threshold (pt) below which the sidebar collapses into a slide-over drawer.
pub const MOBILE_BREAKPOINT_PT: f32 = 960.0;

/// Viewport width threshold (pt) below which the sticky right "On This Page" TOC hides.
pub const TOC_BREAKPOINT_PT: f32 = 1200.0;

/// Logical viewport dimensions driving the responsive 3-column Fumadocs shell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DocsViewport {
    pub width: f32,
    pub height: f32,
}

impl Default for DocsViewport {
    fn default() -> Self {
        Self::desktop_default()
    }
}

impl DocsViewport {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub const fn desktop_default() -> Self {
        Self {
            width: 1280.0,
            height: 800.0,
        }
    }

    pub const fn mobile_default() -> Self {
        Self {
            width: 390.0,
            height: 844.0,
        }
    }

    /// True when the viewport collapses the left sidebar into a mobile drawer (`< 960pt`).
    pub fn is_mobile(&self) -> bool {
        self.width < MOBILE_BREAKPOINT_PT
    }

    /// True when the sticky right "On This Page" TOC should render (`>= 1200pt`).
    pub fn show_toc(&self) -> bool {
        self.width >= TOC_BREAKPOINT_PT
    }

    /// Diagnostic label emitted on the root node as `"viewport-mode"`.
    pub fn mode_label(&self) -> &'static str {
        if self.is_mobile() {
            "mobile"
        } else if self.show_toc() {
            "desktop"
        } else {
            "tablet"
        }
    }
}

/// Complete reactive state for a mounted `nve-docs` application surface.
#[derive(Clone)]
pub struct DocsAppState {
    pub site: Arc<CompiledSite>,
    pub search_index: Arc<SearchIndex>,
    pub config: DocsSiteConfig,
    pub playgrounds: Arc<PlaygroundRegistry>,
    pub host_services: Arc<dyn HostServiceProvider>,
    pub active_route: DocsRoute,
    pub dark_mode: bool,
    pub viewport: DocsViewport,
    pub mobile_drawer_open: bool,
    pub collapsed_folders: BTreeSet<String>,
    pub search_open: bool,
    pub search_query: String,
    pub search_selected: usize,
    pub active_tabs: BTreeMap<String, String>,
    pub copied_code_key: Option<String>,
    pub active_anchor: Option<String>,
}

impl DocsAppState {
    /// Construct initial state synchronized with `SystemEnv` and compiled navigation metadata.
    pub fn new(
        config: DocsSiteConfig,
        site: Arc<CompiledSite>,
        playgrounds: Arc<PlaygroundRegistry>,
        host_services: Arc<dyn HostServiceProvider>,
        initial_route: DocsRoute,
        viewport: DocsViewport,
    ) -> Self {
        let dark_mode = config.default_dark;
        let search_index = Arc::new(site.search_index.clone());
        let mut collapsed_folders = BTreeSet::new();
        collect_initially_collapsed(&site.nav.items, &mut collapsed_folders);
        let resolved_route = if site.page(initial_route.as_str()).is_some() {
            initial_route
        } else if let Some(first) = site.pages.first() {
            DocsRoute::new(&first.route)
        } else {
            initial_route
        };

        Self {
            site,
            search_index,
            config,
            playgrounds,
            host_services,
            active_route: resolved_route,
            dark_mode,
            viewport,
            mobile_drawer_open: false,
            collapsed_folders,
            search_open: false,
            search_query: String::new(),
            search_selected: 0,
            active_tabs: BTreeMap::new(),
            copied_code_key: None,
            active_anchor: None,
        }
    }

    /// Resolve the active [`DocsTheme`] palette for the current dark/light toggle.
    pub const fn theme(&self) -> DocsTheme {
        DocsTheme::for_dark(self.dark_mode)
    }

    /// Resolve the active compiled page, falling back to the first page if not found.
    pub fn current_page(&self) -> Option<&CompiledPage> {
        self.site
            .page(self.active_route.as_str())
            .or_else(|| self.site.pages.first())
    }

    /// Return Prev/Next pagination links for the active route.
    pub fn prev_next(&self) -> PrevNext {
        self.site
            .prev_next
            .get(self.active_route.as_str())
            .cloned()
            .unwrap_or_default()
    }

    /// Navigate to a new documentation route and reset transient overlays/feedback.
    pub fn navigate(&mut self, route: DocsRoute) {
        self.active_route = route;
        self.active_anchor = None;
        self.mobile_drawer_open = false;
        self.search_open = false;
        self.search_query.clear();
        self.search_selected = 0;
        self.copied_code_key = None;
    }

    /// Navigate to an internal route and optional heading anchor.
    pub fn navigate_with_anchor(&mut self, route: Option<DocsRoute>, anchor: Option<String>) {
        if let Some(target) = route {
            self.navigate(target);
        }
        if anchor.is_some() {
            self.active_anchor = anchor;
        }
    }

    /// Toggle dark/light theme mode.
    pub fn toggle_theme(&mut self) {
        self.dark_mode = !self.dark_mode;
    }

    /// Synchronize dark/light mode with the host's `nve_sdk::SystemEnv` capability.
    ///
    /// Returns `true` if `dark_mode` changed.
    pub fn sync_with_system_env(&mut self) -> bool {
        let resolved = DocsTheme::resolve_dark_from_system_env(
            self.host_services.as_ref(),
            self.config.default_dark,
        );
        if resolved != self.dark_mode {
            self.dark_mode = resolved;
            true
        } else {
            false
        }
    }

    /// Toggle the mobile slide-over sidebar drawer.
    pub fn toggle_mobile_drawer(&mut self) {
        self.mobile_drawer_open = !self.mobile_drawer_open;
    }

    /// Explicitly open or close the mobile sidebar drawer.
    pub fn set_mobile_drawer(&mut self, open: bool) {
        self.mobile_drawer_open = open;
    }

    /// Update the responsive viewport dimensions.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        if width.is_finite() && width > 0.0 {
            self.viewport.width = width;
        }
        if height.is_finite() && height > 0.0 {
            self.viewport.height = height;
        }
        if !self.viewport.is_mobile() {
            self.mobile_drawer_open = false;
        }
    }

    /// Toggle collapse state for a sidebar folder group by slug.
    pub fn toggle_folder(&mut self, slug: &str) {
        if !self.collapsed_folders.remove(slug) {
            self.collapsed_folders.insert(slug.to_string());
        }
    }

    pub fn is_folder_collapsed(&self, slug: &str) -> bool {
        self.collapsed_folders.contains(slug)
    }

    /// Open the `⌘K` search modal and reset query selection.
    pub fn open_search(&mut self) {
        self.search_open = true;
        self.search_selected = 0;
    }

    /// Close the `⌘K` search modal.
    pub fn close_search(&mut self) {
        self.search_open = false;
        self.search_selected = 0;
    }

    /// Update the search query and clamp selection back to index `0`.
    pub fn set_search_query(&mut self, query: String) {
        self.search_query = query;
        self.search_selected = 0;
    }

    /// Execute a live query against the `NVS1` [`SearchIndex`].
    ///
    /// When the query is empty, returns the first `limit` pages as suggested quick-jumps.
    pub fn search_hits(&self, limit: usize) -> Vec<SearchHit> {
        let trimmed = self.search_query.trim();
        if trimmed.is_empty() {
            return self
                .site
                .pages
                .iter()
                .take(limit)
                .map(|p| SearchHit {
                    route: p.route.clone(),
                    title: p.title.clone(),
                    section: p.title.clone(),
                    anchor: String::new(),
                    excerpt: if !p.description.is_empty() {
                        p.description.clone()
                    } else {
                        p.plain_text.chars().take(120).collect()
                    },
                    score: 1.0,
                })
                .collect();
        }
        self.search_index.query(trimmed, limit)
    }

    /// Move the active search hit cursor by `delta` with wrap-around.
    pub fn move_search_selection(&mut self, delta: isize, result_count: usize) {
        if result_count == 0 {
            self.search_selected = 0;
            return;
        }
        let len = result_count as isize;
        let cur = (self.search_selected as isize).rem_euclid(len);
        self.search_selected = (cur + delta).rem_euclid(len) as usize;
    }

    /// Navigate to a specific search hit and close the modal.
    pub fn select_search_hit(&mut self, hit: &SearchHit) -> DocsRoute {
        let route = DocsRoute::new(&hit.route);
        let anchor = (!hit.anchor.is_empty()).then(|| hit.anchor.clone());
        self.navigate_with_anchor(Some(route.clone()), anchor);
        route
    }

    /// Resolve the active tab value for a given tab group key.
    pub fn active_tab(&self, group_key: &str, default_value: &str) -> String {
        self.active_tabs
            .get(group_key)
            .cloned()
            .unwrap_or_else(|| default_value.to_string())
    }

    /// Set the active tab value for a given tab group key.
    pub fn set_active_tab(&mut self, group_key: impl Into<String>, value: impl Into<String>) {
        self.active_tabs.insert(group_key.into(), value.into());
    }

    /// Write `code` to the host clipboard via `nve_sdk::ClipboardClient` and record copied feedback.
    pub fn copy_code(&mut self, block_key: &str, code: &str) -> bool {
        let client = ClipboardClient::new(self.host_services.as_ref());
        if client.write_text(code).is_ok() {
            self.copied_code_key = Some(block_key.to_string());
            true
        } else {
            false
        }
    }

    /// Check whether `block_key` is the most recently copied code block.
    pub fn is_code_copied(&self, block_key: &str) -> bool {
        self.copied_code_key.as_deref() == Some(block_key)
    }

    /// Compute the breadcrumb trail `(title, route)` for the active route from `CompiledSite::nav`.
    pub fn breadcrumbs(&self) -> Vec<(String, String)> {
        let target = self.active_route.as_str();
        let mut trail = Vec::new();
        if find_breadcrumbs(&self.site.nav.items, target, &mut trail) {
            return trail;
        }
        if let Some(page) = self.current_page() {
            vec![(page.title.clone(), page.route.clone())]
        } else {
            Vec::new()
        }
    }
}

fn collect_initially_collapsed(nodes: &[NavNode], out: &mut BTreeSet<String>) {
    for node in nodes {
        if let NavNode::Folder {
            slug,
            collapsed,
            children,
            ..
        } = node
        {
            if *collapsed {
                out.insert(slug.clone());
            }
            collect_initially_collapsed(children, out);
        }
    }
}

fn find_breadcrumbs(
    nodes: &[NavNode],
    target_route: &str,
    trail: &mut Vec<(String, String)>,
) -> bool {
    for node in nodes {
        match node {
            NavNode::Page { title, route, .. } if route == target_route => {
                trail.push((title.clone(), route.clone()));
                return true;
            }
            NavNode::Folder {
                title,
                index_route,
                children,
                ..
            } => {
                let folder_route = index_route.clone().unwrap_or_default();
                if index_route.as_deref() == Some(target_route) {
                    trail.push((title.clone(), folder_route));
                    return true;
                }
                trail.push((title.clone(), folder_route));
                if find_breadcrumbs(children, target_route, trail) {
                    return true;
                }
                trail.pop();
            }
            _ => {}
        }
    }
    false
}

/// Consume the shared reactive [`DocsAppState`] signal inside any `nve-docs` component.
pub fn use_docs_state() -> Signal<DocsAppState> {
    consume_context::<Signal<DocsAppState>>()
}

/// Navigate both [`DocsAppState`] and the surrounding `nve-router` [`Navigator`](nve_router::Navigator).
pub fn navigate_to(mut state: Signal<DocsAppState>, route: DocsRoute) {
    let nav_route = route.clone();
    state.write().navigate(route);
    if let Some(nav) = try_use_navigator::<DocsRoute>()
        && nav.current_route() != nav_route
    {
        let _ = nav.push(nav_route);
    }
}

/// Navigate to a route and optional heading anchor across state and router.
pub fn navigate_to_target(
    mut state: Signal<DocsAppState>,
    route: Option<DocsRoute>,
    anchor: Option<String>,
) {
    let nav_route = route.clone();
    state.write().navigate_with_anchor(route, anchor);
    if let Some(target) = nav_route
        && let Some(nav) = try_use_navigator::<DocsRoute>()
        && nav.current_route() != target
    {
        let _ = nav.push(target);
    }
}

/// Sync `DocsAppState::active_route` when `nve-router` drives navigation externally (e.g. Back/Forward).
pub fn sync_route_from_router(mut state: Signal<DocsAppState>, route: &DocsRoute) {
    if state.read().active_route != *route {
        state.write().navigate(route.clone());
    }
}

// ---------------------------------------------------------------------------
// Thread-local FIFO bootstrap queue for `DioxusSurface::new(surface_id, fn)`
// ---------------------------------------------------------------------------

thread_local! {
    static PENDING_BOOTSTRAPS: RefCell<VecDeque<DocsAppState>> = const { RefCell::new(VecDeque::new()) };
}

pub(crate) fn push_bootstrap(initial: DocsAppState) {
    PENDING_BOOTSTRAPS.with(|queue| {
        queue.borrow_mut().push_back(initial);
    });
}

pub(crate) fn take_bootstrap() -> Option<DocsAppState> {
    PENDING_BOOTSTRAPS.with(|queue| queue.borrow_mut().pop_front())
}
