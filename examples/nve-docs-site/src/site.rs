//! Embedded build-time `docs/content/` loader and official `DocsSiteConfig` for `native-view-engine`.

use std::sync::{Arc, OnceLock};

use nve_docs::{DocsNavSection, DocsSiteConfig};
use nve_mdx::{CompileOptions, CompiledSite, SiteSource, compile_site};

include!(concat!(
    env!("OUT_DIR"),
    "/embedded_docs_content.generated.rs"
));

/// Official repository URL used by navbar links and "Edit this page" links.
pub const GITHUB_REPO_URL: &str = "https://github.com/samuelscheit/native-view-engine";

/// Public route prefix where the 30 compiled MDX docs pages are mounted.
pub const DOCS_ROUTE_PREFIX: &str = "/docs";

/// Base URL joined with `page.source_path` for "Edit this page on GitHub".
pub const GITHUB_EDIT_BASE_URL: &str =
    "https://github.com/samuelscheit/native-view-engine/edit/main/docs/content";

/// Build an in-memory [`SiteSource`] from all build-time embedded `docs/content/` files.
///
/// Performs zero runtime filesystem I/O, working identically on WASM, iOS, Android, and desktop.
pub fn embedded_site_source() -> SiteSource {
    let mut source = SiteSource::new();
    for &(rel_path, content) in EMBEDDED_DOCS_FILES {
        source.insert(rel_path, content);
    }
    source
}

/// Returns the cached, pre-compiled [`CompiledSite`] for the official `docs/content/` tree.
pub fn embedded_compiled_site() -> Arc<CompiledSite> {
    static SITE: OnceLock<Arc<CompiledSite>> = OnceLock::new();
    Arc::clone(SITE.get_or_init(|| {
        let source = embedded_site_source();
        Arc::new(compile_site(&source, &CompileOptions::default()))
    }))
}

/// Official Fumadocs × Linear [`DocsSiteConfig`] for `native-view-engine`.
pub fn nve_docs_site_config(base_path: &str) -> DocsSiteConfig {
    nve_docs_site_config_with_theme(base_path, true)
}

/// Official [`DocsSiteConfig`] with explicit initial `default_dark` setting.
pub fn nve_docs_site_config_with_theme(base_path: &str, default_dark: bool) -> DocsSiteConfig {
    DocsSiteConfig::new("Native View Engine")
        .with_tagline(
            "Zero-overhead Rust + Dioxus UI runtime across Native, Skia, and Web SSR/WASM",
        )
        .with_brand_mark("NVE")
        .with_github_repo(GITHUB_REPO_URL)
        .with_edit_base_url(GITHUB_EDIT_BASE_URL)
        .with_base_path(base_path)
        .with_docs_prefix(DOCS_ROUTE_PREFIX)
        .with_default_dark(default_dark)
        .with_sections(vec![
            DocsNavSection::new("docs", "Docs", "/docs", "/docs"),
            DocsNavSection::new("examples", "Examples", "/examples", "/examples"),
            DocsNavSection::new(
                "benchmarks",
                "Benchmarks",
                "/docs/reference/benchmarks",
                "/docs/reference/benchmarks",
            ),
        ])
}
