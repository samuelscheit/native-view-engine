//! Official `native-view-engine` documentation site & native desktop/mobile docs app.
//!
//! Shares one compiled Rust/Dioxus `nve-docs` application across:
//! - Static Semantic HTML5 + Option 5 Hybrid CSS SSR (`cargo nve docs build`)
//! - Progressive `wasm32` browser hydration + SPA routing (`wasm::hydrate_docs_app`)
//! - Native Desktop/Mobile hosts via `export_nve_app!` (`native_app::DocsSiteModule`)

pub mod native_app;
pub mod playgrounds;
pub mod site;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

use std::sync::Arc;

use nve_abi::SurfaceId;
use nve_backend_web::{SsrOutput, WebBackend};
use nve_dioxus::DioxusSurface;
use nve_docs::{DocsViewport, PlaygroundRegistry, create_docs_surface_with_viewport};
use nve_engine::RenderBackend;
use nve_mdx::CompiledSite;
use nve_sdk::{HostServiceProvider, MemoryHostServices};

pub use native_app::DocsSiteModule;
pub use playgrounds::{build_playground_registry, nve_docs_playground_registry};
pub use site::{
    EMBEDDED_DOCS_FILES, GITHUB_EDIT_BASE_URL, GITHUB_REPO_URL, embedded_compiled_site,
    embedded_site_source, nve_docs_site_config, nve_docs_site_config_with_theme,
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
    let config = nve_docs_site_config_with_theme(base_path, default_dark);
    let playgrounds = nve_docs_playground_registry();
    let host_services: Arc<dyn HostServiceProvider> = Arc::new(MemoryHostServices::new());
    create_docs_surface_with_viewport(
        surface_id,
        route,
        viewport,
        config,
        compiled_site,
        playgrounds,
        host_services,
    )
}

/// Render a single embedded route via `WebBackend::render_ssr` (convenience helper for tests & SSR).
pub fn render_nve_docs_ssr(route: &str, base_path: &str, viewport: DocsViewport) -> SsrOutput {
    let mut surface = create_nve_docs_surface(route, base_path, viewport);
    let (_, _, frame_bytes) = surface.render_frame(1);
    let mut backend = WebBackend::new();
    backend
        .submit_frame(&frame_bytes)
        .expect("WebBackend submit_frame for docs SSR");
    backend.render_ssr()
}
