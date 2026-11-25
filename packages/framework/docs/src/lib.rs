//! Reusable Fumadocs × Linear documentation framework (`nve-docs`) built 100% on
//! `nve-ui`, `nve-router`, `nve-host-services`, `nve-clipboard`, `nve-system-env`, and `nve-mdx`.
//!
//! The exact same compiled docs application renders cleanly for Web SSR + Progressive Hydration
//! as well as Native Desktop/Mobile hosts (`nve-skia-shell`, macOS, Windows, iOS, Android).

pub mod config;
pub mod registry;
pub mod render;
pub mod route;
pub mod shell;
pub mod state;
pub mod theme;

use std::collections::BTreeMap;
use std::sync::Arc;

use dioxus_core::{provide_context, use_hook};
use nve_abi::SurfaceId;
use nve_clipboard::MemoryClipboard;
use nve_dioxus::DioxusSurface;
use nve_dioxus::prelude::*;
use nve_host_services::{HostServiceProvider, MemoryHostServices};
use nve_mdx::{CompiledSite, NavTree, SearchIndex};
use nve_router::{RouteRenderer, RouterProvider};
use nve_system_env::MemorySystemEnv;

pub use config::{DocsNavSection, DocsSiteConfig};
pub use registry::{
    CustomPageRegistry, CustomPageRenderer, PlaygroundEntry, PlaygroundMeta, PlaygroundRegistry,
    PlaygroundRenderer, default_component_registry,
};
pub use route::DocsRoute;
pub use shell::{Breadcrumbs, DocsNavbar, DocsShell, DocsSidebar, DocsToc, PageFooter, SearchModal};
pub use state::{
    DocsAppState, DocsViewport, MOBILE_BREAKPOINT_PT, TOC_BREAKPOINT_PT, navigate_to,
    navigate_to_target, use_docs_state,
};
pub use theme::{CalloutTone, DocsTheme, SyntaxPalette};

/// Root Dioxus component for a mounted `nve-docs` documentation application.
///
/// Consumes its initial [`DocsAppState`] from the thread-local surface bootstrap queue (seeded
/// by [`create_docs_surface`] / [`create_docs_surface_at_route`] / [`create_docs_surface_with_viewport`]),
/// wraps the shell in `nve_router::RouterProvider<DocsRoute>`, and provides reactive
/// `Signal<DocsAppState>` context to all shell and MDX AST renderer components.
#[component]
pub fn DocsApp() -> Element {
    let initial = use_hook(|| state::take_bootstrap().unwrap_or_else(fallback_empty_state));
    let initial_route = initial.active_route.clone();
    let state_sig = use_hook(move || provide_context(Signal::new(initial)));

    let renderer = RouteRenderer::new(move |route: &DocsRoute| {
        state::sync_route_from_router(state_sig, route);
        rsx! { DocsShell {} }
    });

    rsx! {
        RouterProvider::<DocsRoute> {
            initial: initial_route,
            render: renderer,
        }
    }
}

/// Create a mounted [`DioxusSurface`] for a compiled documentation site at the root route `"/"`.
pub fn create_docs_surface(
    surface_id: SurfaceId,
    config: DocsSiteConfig,
    site: Arc<CompiledSite>,
    playgrounds: Arc<PlaygroundRegistry>,
    host_services: Arc<dyn HostServiceProvider>,
) -> DioxusSurface {
    create_docs_surface_with_viewport(
        surface_id,
        "/",
        DocsViewport::desktop_default(),
        config,
        site,
        playgrounds,
        host_services,
    )
}

/// Create a [`DioxusSurface`] initialized directly at `initial_route` (ideal for per-route static SSR).
pub fn create_docs_surface_at_route(
    surface_id: SurfaceId,
    initial_route: impl AsRef<str>,
    config: DocsSiteConfig,
    site: Arc<CompiledSite>,
    playgrounds: Arc<PlaygroundRegistry>,
    host_services: Arc<dyn HostServiceProvider>,
) -> DioxusSurface {
    create_docs_surface_with_viewport(
        surface_id,
        initial_route,
        DocsViewport::desktop_default(),
        config,
        site,
        playgrounds,
        host_services,
    )
}

/// Create a [`DioxusSurface`] initialized at `initial_route` with an explicit responsive [`DocsViewport`].
pub fn create_docs_surface_with_viewport(
    surface_id: SurfaceId,
    initial_route: impl AsRef<str>,
    viewport: DocsViewport,
    config: DocsSiteConfig,
    site: Arc<CompiledSite>,
    playgrounds: Arc<PlaygroundRegistry>,
    host_services: Arc<dyn HostServiceProvider>,
) -> DioxusSurface {
    create_docs_surface_with_pages(
        surface_id,
        initial_route,
        viewport,
        config,
        site,
        playgrounds,
        Arc::new(CustomPageRegistry::new()),
        host_services,
    )
}

/// Create a [`DioxusSurface`] with full-bleed custom routes (e.g. `/` landing and `/examples`
/// gallery) sharing the unified navbar, theme toggle, and `⌘K` search modal with MDX docs pages.
#[allow(clippy::too_many_arguments)]
pub fn create_docs_surface_with_pages(
    surface_id: SurfaceId,
    initial_route: impl AsRef<str>,
    viewport: DocsViewport,
    config: DocsSiteConfig,
    site: Arc<CompiledSite>,
    playgrounds: Arc<PlaygroundRegistry>,
    custom_pages: Arc<CustomPageRegistry>,
    host_services: Arc<dyn HostServiceProvider>,
) -> DioxusSurface {
    let initial_state = DocsAppState::new_with_pages(
        config,
        site,
        playgrounds,
        custom_pages,
        host_services,
        DocsRoute::new(initial_route),
        viewport,
    );
    state::push_bootstrap(initial_state);
    DioxusSurface::new(surface_id, DocsApp)
}

fn fallback_empty_state() -> DocsAppState {
    let empty_site = Arc::new(CompiledSite {
        pages: Vec::new(),
        nav: NavTree::default(),
        prev_next: BTreeMap::new(),
        search_index: SearchIndex::build(&[]),
    });
    let services = MemoryHostServices::new();
    services.register(MemoryClipboard::new());
    services.register(MemorySystemEnv::new());
    DocsAppState::new(
        DocsSiteConfig::default(),
        empty_site,
        Arc::new(PlaygroundRegistry::new()),
        Arc::new(services),
        DocsRoute::root(),
        DocsViewport::desktop_default(),
    )
}
