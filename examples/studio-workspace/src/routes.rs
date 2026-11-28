//! Typed deep-linkable routing (`packages/framework/router`) for **Aether Workspace** + **NVE Studio**.
//!
//! Maps every [`ActiveView`] plus typed detail endpoints (`/task/:id`, `/doc/:id`) onto clean
//! URLs with a Back/Forward history stack synchronized with [`crate::state::WorkspaceState`].

use nve_router::{define_routes, DeepLinkConfig, NavigationError, Routable, RouterState};

use crate::state::{ActiveView, ChannelKind, WorkspaceSnapshot};

/// Canonical custom URL scheme for Studio Workspace deep links (`aether://studio/...`).
pub const STUDIO_DEEP_LINK_SCHEME: &str = "aether";
/// Canonical authority host for custom-scheme deep links.
pub const STUDIO_DEEP_LINK_HOST: &str = "studio";
/// Canonical HTTPS universal-link hostname for Studio Workspace deep links.
pub const STUDIO_UNIVERSAL_LINK_HOST: &str = "studio.nve.dev";

define_routes! {
    /// Typed routes for Aether Studio Workspace views and deep-linked entities.
    pub enum StudioRoute {
        #[route("/board", title = "Issue Board")]
        Board,
        #[route("/task/:id", title = "Issue Detail")]
        Task { id: u32 },
        #[route("/channels", title = "Channels")]
        Channels {
            #[query("channel")]
            channel: Option<String>,
        },
        #[route("/docs", title = "Workspace Docs")]
        Docs,
        #[route("/doc/:id", title = "Document Detail")]
        Doc { id: u32 },
        #[route("/workbench", title = "Meta Workbench")]
        Workbench,
        #[route("/stream", title = "100k Stream")]
        Stream,
        #[route("/islands", title = "Multi-Host Islands")]
        Islands,
        #[route("/metrics", title = "Live Metrics")]
        Metrics,
    }
}

/// Single source of truth pairing each non-parameterized [`ActiveView`] with its unit [`StudioRoute`].
const UNIT_VIEW_ROUTES: [(ActiveView, StudioRoute); 6] = [
    (ActiveView::Board, StudioRoute::Board),
    (ActiveView::Docs, StudioRoute::Docs),
    (ActiveView::Workbench, StudioRoute::Workbench),
    (ActiveView::Stream100k, StudioRoute::Stream),
    (ActiveView::Islands, StudioRoute::Islands),
    (ActiveView::Metrics, StudioRoute::Metrics),
];

impl StudioRoute {
    /// Builds the default [`StudioRoute`] corresponding to an [`ActiveView`].
    pub fn from_view(view: ActiveView) -> Self {
        match view {
            ActiveView::ChannelEngineering => Self::Channels { channel: None },
            ActiveView::ChannelDesign => Self::Channels {
                channel: Some("design".into()),
            },
            unit => UNIT_VIEW_ROUTES
                .iter()
                .find(|(v, _)| *v == unit)
                .map(|(_, r)| r.clone())
                .unwrap_or(Self::Board),
        }
    }

    /// Builds the canonical [`StudioRoute`] for a full [`WorkspaceSnapshot`].
    pub fn from_snapshot(snapshot: &WorkspaceSnapshot) -> Self {
        match snapshot.active_view {
            ActiveView::Docs => Self::Doc {
                id: snapshot.active_doc_id,
            },
            other => Self::from_view(other),
        }
    }

    /// Resolves this route back to its workspace [`ActiveView`].
    pub fn active_view(&self) -> ActiveView {
        match self {
            Self::Task { .. } => ActiveView::Board,
            Self::Doc { .. } => ActiveView::Docs,
            Self::Channels { channel } => match channel.as_deref() {
                Some(ch) if ch.eq_ignore_ascii_case("design") => ActiveView::ChannelDesign,
                _ => ActiveView::ChannelEngineering,
            },
            unit => UNIT_VIEW_ROUTES
                .iter()
                .find(|(_, r)| r == unit)
                .map_or(ActiveView::Board, |(v, _)| *v),
        }
    }

    /// Returns the target channel if this route points to `/channels`.
    pub fn channel_kind(&self) -> Option<ChannelKind> {
        match self {
            Self::Channels { channel } => match channel.as_deref() {
                Some(ch) if ch.eq_ignore_ascii_case("design") => Some(ChannelKind::Design),
                _ => Some(ChannelKind::Engineering),
            },
            _ => None,
        }
    }

    /// Returns the focused task ID for `/task/:id` routes.
    pub const fn task_id(&self) -> Option<u32> {
        match self {
            Self::Task { id } => Some(*id),
            _ => None,
        }
    }

    /// Returns the targeted document ID for `/doc/:id` routes.
    pub const fn doc_id(&self) -> Option<u32> {
        match self {
            Self::Doc { id } => Some(*id),
            _ => None,
        }
    }

    /// Formats this route as a canonical `aether://studio/...` deep-link URI.
    pub fn deep_link_uri(&self) -> String {
        format!(
            "{STUDIO_DEEP_LINK_SCHEME}://{STUDIO_DEEP_LINK_HOST}{}",
            self.to_path()
        )
    }

    /// Returns the standard [`DeepLinkConfig`] accepted by Aether Studio.
    pub fn deep_link_config() -> DeepLinkConfig {
        DeepLinkConfig::new()
            .with_scheme(STUDIO_DEEP_LINK_SCHEME)
            .with_scheme_host(STUDIO_DEEP_LINK_HOST)
            .with_universal_host(STUDIO_UNIVERSAL_LINK_HOST)
            .allow_web_paths(true)
    }

    /// Parses either a relative path (`/docs`, `/task/1`, `/doc/2`, `/channels?channel=design`),
    /// custom-scheme URI (`aether://studio/doc/2`), or HTTPS universal link
    /// (`https://studio.nve.dev/stream`) into a typed [`StudioRoute`].
    pub fn parse_deep_link(uri: &str) -> Option<Self> {
        let trimmed = uri.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed == "/" {
            return Some(Self::Board);
        }
        Self::deep_link_config()
            .match_uri::<Self>(trimmed)
            .ok()
            .map(|m| m.route)
    }
}

/// Synchronized Back/Forward navigation stack wrapping [`nve_router::RouterState<StudioRoute>`].
#[derive(Debug, Clone)]
pub struct StudioRouter {
    inner: RouterState<StudioRoute>,
}

impl Default for StudioRouter {
    fn default() -> Self {
        Self::new(StudioRoute::Board)
    }
}

impl PartialEq for StudioRouter {
    fn eq(&self, other: &Self) -> bool {
        self.inner.cursor() == other.inner.cursor()
            && self.inner.entries().len() == other.inner.entries().len()
            && self
                .inner
                .entries()
                .iter()
                .zip(other.inner.entries().iter())
                .all(|(a, b)| a.route == b.route)
    }
}

impl Eq for StudioRouter {}

impl StudioRouter {
    /// Creates a new [`StudioRouter`] rooted at `initial`.
    pub fn new(initial: StudioRoute) -> Self {
        Self {
            inner: RouterState::new(initial),
        }
    }

    /// Reconstructs a [`StudioRouter`] from persisted route history and cursor.
    pub fn from_history(routes: Vec<StudioRoute>, cursor: usize) -> Self {
        let mut iter = routes.into_iter();
        let Some(first) = iter.next() else {
            return Self::default();
        };
        let mut state = RouterState::new(first);
        for route in iter {
            let _ = state.push(route);
        }
        let max_cursor = state.history_len().saturating_sub(1);
        let target = cursor.min(max_cursor);
        let delta = target as isize - state.cursor() as isize;
        if delta != 0 {
            let _ = state.go(delta);
        }
        Self { inner: state }
    }

    /// Returns the underlying [`RouterState<StudioRoute>`].
    pub fn inner(&self) -> &RouterState<StudioRoute> {
        &self.inner
    }

    /// Returns the active [`StudioRoute`].
    pub fn current_route(&self) -> &StudioRoute {
        self.inner.current_route()
    }

    /// Returns the active canonical URL path (e.g. `"/board"`, `"/doc/2"`, `"/task/1"`).
    pub fn current_path(&self) -> String {
        self.inner.current_route().to_path()
    }

    /// Returns the active deep-link URI (e.g. `"aether://studio/doc/2"`).
    pub fn current_deep_link(&self) -> String {
        self.inner.current_route().deep_link_uri()
    }

    /// Returns all routes currently stored in the navigation history stack.
    pub fn history_routes(&self) -> Vec<StudioRoute> {
        self.inner
            .entries()
            .iter()
            .map(|e| e.route.clone())
            .collect()
    }

    /// Zero-based index of the active entry within the history stack.
    pub fn cursor(&self) -> usize {
        self.inner.cursor()
    }

    /// Total number of entries in the navigation stack (including forward entries).
    pub fn history_len(&self) -> usize {
        self.inner.history_len()
    }

    /// Returns `true` when navigating Back (`go(-1)`) is available.
    pub fn can_go_back(&self) -> bool {
        self.inner.can_go_back()
    }

    /// Returns `true` when navigating Forward (`go(1)`) is available.
    pub fn can_go_forward(&self) -> bool {
        self.inner.can_go_forward()
    }

    /// Pushes `route` onto the history stack (truncating any forward entries).
    /// Returns `false` if `route` equals the current route.
    pub fn push(&mut self, route: StudioRoute) -> bool {
        if self.inner.current_route() == &route {
            return false;
        }
        self.inner.push(route).is_ok()
    }

    /// Replaces the active entry without growing the history stack.
    pub fn replace(&mut self, route: StudioRoute) -> bool {
        if self.inner.current_route() == &route {
            return false;
        }
        self.inner.replace(route).is_ok()
    }

    /// Navigates to a deep-link URI via `nve_router::DeepLinkConfig`.
    pub fn apply_deep_link(&mut self, uri: &str) -> Result<StudioRoute, NavigationError> {
        let route = StudioRoute::parse_deep_link(uri).ok_or(NavigationError::Blocked)?;
        if self.inner.current_route() == &route {
            return Ok(route);
        }
        let transition = self.inner.apply_deep_link(route)?;
        Ok(transition.route)
    }

    /// Moves one step backward in the history stack without truncating forward entries.
    pub fn back(&mut self) -> Option<StudioRoute> {
        self.inner.go(-1).ok().map(|t| t.route)
    }

    /// Moves one step forward in the history stack.
    pub fn forward(&mut self) -> Option<StudioRoute> {
        self.inner.go(1).ok().map(|t| t.route)
    }

    /// Synchronizes the active route with `view` when time-travel or direct state mutations change
    /// `active_view` outside of router navigation.
    pub fn sync_to_view(&mut self, view: ActiveView) {
        if self.inner.current_route().active_view() != view {
            let _ = self.inner.replace(StudioRoute::from_view(view));
        }
    }
}
