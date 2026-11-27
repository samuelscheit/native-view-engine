//! Official `native-view-engine` documentation site & native desktop/mobile docs app.
//!
//! Shares one compiled Rust/Dioxus `nve-docs` application across:
//! - Static Semantic HTML5 + Option 5 Hybrid CSS SSR (`cargo nve docs build`)
//! - Progressive `wasm32` browser hydration + SPA routing (`wasm::hydrate_docs_app`)
//! - Native Desktop/Mobile hosts via `export_nve_app!` (`native_app::DocsSiteModule`)

pub mod landing;
pub mod native_app;
pub mod playgrounds;
pub mod site;

/// The docs site's web table plugins (`nve-web-html` + canvas), shared by its SSR/SSG builders and
/// browser hydration.
pub fn register_web_systems(registry: &mut nve_backend_web::WebTableRegistry) {
    nve_web_html_web::register_web_html_systems(registry);
    registry.register(nve_canvas_web::CanvasWebSystem);
}
#[cfg(target_arch = "wasm32")]
pub mod wasm;

use std::sync::Arc;

use nve_abi::SurfaceId;
use nve_backend_web::{SsrOutput, WebBackend};
use nve_clipboard::MemoryClipboard;
use nve_dioxus::DioxusSurface;
use nve_components::tables::DebugId;
use nve_docs::{DocsSiteConfig, DocsViewport, PlaygroundRegistry, create_docs_surface_with_pages};
use nve_host_services::{HostServiceHandler, MemoryHostServices};
use nve_mdx::CompiledSite;
use nve_store::NodeId;
use nve_system_env::{LocaleSnapshot, MemorySystemEnv, ThemeSnapshot};
use nve_ui::ThemeMode;
use nve_web_html::{DataAttrList, HtmlDataAttrs};

pub use landing::nve_site_custom_pages;
pub use native_app::DocsSiteModule;
pub use playgrounds::{build_playground_registry, nve_docs_playground_registry};
pub use site::{
    DOCS_ROUTE_PREFIX, EMBEDDED_DOCS_FILES, GITHUB_EDIT_BASE_URL, GITHUB_REPO_URL,
    embedded_compiled_site, embedded_site_source, nve_docs_site_config,
    nve_docs_site_config_with_theme,
};

/// Return the shared playground registry for CLI validation and site mounting.
pub fn playground_registry() -> Arc<PlaygroundRegistry> {
    nve_docs_playground_registry()
}

/// Create a `DioxusSurface` for the official embedded `docs/content/` at `route`.
pub fn create_nve_docs_surface(
    route: &str,
    base_path: &str,
    viewport: DocsViewport,
) -> DioxusSurface {
    create_nve_docs_surface_on(1, route, base_path, viewport)
}

/// Create a `DioxusSurface` for the official embedded `docs/content/` on an explicit `surface_id`.
pub fn create_nve_docs_surface_on(
    surface_id: SurfaceId,
    route: &str,
    base_path: &str,
    viewport: DocsViewport,
) -> DioxusSurface {
    create_nve_docs_surface_full(
        surface_id,
        embedded_compiled_site(),
        route,
        base_path,
        viewport,
        true,
    )
}

/// Create a `DioxusSurface` for an arbitrary [`CompiledSite`] (used by `cargo nve docs build`
/// and `docs dev` when compiling custom or edited content directories).
pub fn create_nve_docs_surface_full(
    surface_id: SurfaceId,
    compiled_site: Arc<CompiledSite>,
    route: &str,
    base_path: &str,
    viewport: DocsViewport,
    default_dark: bool,
) -> DioxusSurface {
    create_nve_docs_surface_with_host(
        surface_id,
        compiled_site,
        route,
        viewport,
        nve_docs_site_config_with_theme(base_path, default_dark),
        docs_host_services(Arc::new(headless_system_env())),
    )
}

/// Create a `DioxusSurface` on caller-owned host services (live `nve-system-env` backend) and
/// config (e.g. a preserved [`DocsSiteConfig::theme_mode`] across rebuilds).
pub fn create_nve_docs_surface_with_host(
    surface_id: SurfaceId,
    compiled_site: Arc<CompiledSite>,
    route: &str,
    viewport: DocsViewport,
    config: DocsSiteConfig,
    host_services: Arc<MemoryHostServices>,
) -> DioxusSurface {
    create_docs_surface_with_pages(
        surface_id,
        route,
        viewport,
        config,
        compiled_site,
        nve_docs_playground_registry(),
        nve_site_custom_pages(),
        host_services,
    )
}

/// `nve-system-env` stand-in for hosts without an OS appearance (SSR, static builds, tests): it
/// reports no color scheme, so [`DocsSiteConfig::default_dark`] decides.
pub fn headless_system_env() -> MemorySystemEnv {
    MemorySystemEnv::with_initial(
        ThemeSnapshot { color_scheme: None, ..ThemeSnapshot::default() },
        LocaleSnapshot::default(),
    )
}

/// Host services for a docs surface: in-memory clipboard plus the given `nve-system-env` backend.
pub fn docs_host_services(system_env: Arc<dyn HostServiceHandler>) -> Arc<MemoryHostServices> {
    let services = MemoryHostServices::new();
    services.register(MemoryClipboard::new());
    services.register_arc(system_env);
    Arc::new(services)
}

/// The user's [`ThemeMode`] on a mounted docs surface (`docs-root` `data-theme-mode`), so hosts
/// that rebuild the surface keep a toggled scheme. [`ThemeMode::System`] when unmounted.
pub fn surface_theme_mode(surface: &DioxusSurface) -> ThemeMode {
    docs_root_data_attr(surface, "theme-mode")
        .and_then(|v| ThemeMode::parse(&v))
        .unwrap_or_default()
}

/// A `data-*` attribute (without the prefix) of the mounted `docs-root` node.
pub fn docs_root_data_attr(surface: &DioxusSurface, key: &str) -> Option<String> {
    let store = surface.store();
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        if store.try_row::<DebugId>(id).is_some_and(|d| d.test_id() == "docs-root") {
            let list = store.resource::<DataAttrList>(store.try_row::<HtmlDataAttrs>(id)?.entries())?;
            return list.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        }
        stack.extend_from_slice(store.children(id));
    }
    None
}

/// Render a single embedded route via `WebBackend::render_ssr` (convenience helper for tests & SSR).
pub fn render_nve_docs_ssr(route: &str, base_path: &str, viewport: DocsViewport) -> SsrOutput {
    let mut surface = create_nve_docs_surface(route, base_path, viewport);
    let mut backend = WebBackend::new();
    register_web_systems(backend.table_registry_mut());
    surface
        .commit_to(1, &mut backend)
        .expect("WebBackend commit_to for docs SSR");
    backend.render_ssr(surface.store())
}
