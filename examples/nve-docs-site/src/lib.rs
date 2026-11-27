//! Official `native-view-engine` documentation site & native desktop/mobile docs app.
//!
//! Shares one compiled Rust/Dioxus `nve-docs` application across:
//! - Static Semantic HTML5 + Option 5 Hybrid CSS SSR (`cargo nve docs build`)
//! - Progressive `wasm32` browser hydration + SPA routing (`wasm::hydrate_docs_app`)
//! - Native Desktop/Mobile hosts via `export_nve_app!` (`native_app::DocsSiteModule`)

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod host;
pub mod landing;
pub mod native_app;
pub mod playgrounds;
pub mod site;
pub mod ssr;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// The docs site's web table plugins (`nve-web-html` + canvas), shared by its SSR/SSG builders and
/// browser hydration.
pub fn register_web_systems(registry: &mut nve_backend_web::WebTableRegistry) {
    nve_web_html_web::register_web_html_systems(registry);
    registry.register(nve_canvas_web::CanvasWebSystem);
}

use std::sync::Arc;

use nve_abi::SurfaceId;
use nve_dioxus::DioxusSurface;
use nve_docs::{DocsViewport, PlaygroundRegistry};
use nve_mdx::CompiledSite;

pub use host::{DocsHost, THEME_MODE_STORAGE_KEY};
pub use landing::nve_site_custom_pages;
pub use ssr::{DocsSsr, render_docs_ssr, render_nve_docs_ssr};
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

/// Create a `DioxusSurface` for an arbitrary [`CompiledSite`] on a headless host (no OS
/// appearance: `default_dark` decides). Used by `cargo nve docs dev` and tests.
pub fn create_nve_docs_surface_full(
    surface_id: SurfaceId,
    compiled_site: Arc<CompiledSite>,
    route: &str,
    base_path: &str,
    viewport: DocsViewport,
    default_dark: bool,
) -> DioxusSurface {
    DocsHost::headless(default_dark).mount(surface_id, compiled_site, route, base_path, viewport)
}
