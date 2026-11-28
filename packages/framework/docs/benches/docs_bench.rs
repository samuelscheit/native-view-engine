//! Criterion micro-benchmarks for `nve-docs`: theme/syntax/callout resolution, route & href
//! resolution, compiled-site lookup/breadcrumbs/search, and MDX component registry construction.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;
    use std::sync::Arc;

    use criterion::{criterion_group, Criterion};
    use nve_docs::{
        callout_tone, default_component_registry, docs_theme_config, syntax_color, DocsAppState,
        DocsNavSection, DocsRoute, DocsSiteConfig, DocsViewport, PlaygroundRegistry,
    };
    use nve_host_services::MemoryHostServices;
    use nve_mdx::{compile_site_with_validation, CalloutKind, CompileOptions, SiteSource, TokenKind};
    use nve_ui::{Appearance, ThemeMode};

    fn build_bench_site() -> Arc<nve_mdx::CompiledSite> {
        let mut src = SiteSource::new();
        src.insert(
            "_meta.toml",
            "index = \"Overview\"\nguides = \"Guides\"\nreference = \"Reference\"\n",
        );
        src.insert(
            "guides/_meta.toml",
            "quickstart = \"Quickstart\"\nlayout = \"Layout Engine\"\n",
        );
        src.insert(
            "index.mdx",
            "---\ntitle: \"Overview\"\ndescription: \"Single binary IR across Web and Native.\"\n---\n\n# Overview\n\nWelcome to native-view-engine.\n",
        );
        src.insert(
            "guides/quickstart.mdx",
            "---\ntitle: \"Quickstart\"\ndescription: \"Boot your first surface in 60 seconds.\"\n---\n\n# Quickstart\n\nInstall the toolchain and run cargo nve dev.\n",
        );
        src.insert(
            "guides/layout.mdx",
            "---\ntitle: \"Layout Engine\"\ndescription: \"Direct Taffy Flexbox and CSS Grid over NodeStore.\"\n---\n\n# Layout Engine\n\nZero mirror trees and zero-copy scene publication.\n",
        );
        src.insert(
            "reference/abi.mdx",
            "---\ntitle: \"Runtime ABI\"\ndescription: \"Single flat NveSceneView and 64-byte NveEvent ring.\"\n---\n\n# Runtime ABI\n\nEvery host shares one binary ABI.\n",
        );
        let (site, diags) = compile_site_with_validation(
            &src,
            &CompileOptions::default(),
            &default_component_registry(),
        );
        assert!(diags.is_empty(), "bench site diagnostics: {diags:?}");
        Arc::new(site)
    }

    fn bench_docs_hot_paths(c: &mut Criterion) {
        let token_kinds = [
            TokenKind::Plain,
            TokenKind::Keyword,
            TokenKind::TypeName,
            TokenKind::StringLit,
            TokenKind::Comment,
            TokenKind::Number,
            TokenKind::Function,
            TokenKind::Macro,
            TokenKind::Attribute,
            TokenKind::Punctuation,
            TokenKind::DiffAdd,
            TokenKind::DiffDel,
        ];
        let callout_kinds = [
            CalloutKind::Tip,
            CalloutKind::Info,
            CalloutKind::Warning,
            CalloutKind::Danger,
            CalloutKind::Note,
        ];

        c.bench_function("docs_theme_syntax_and_callouts", |b| {
            b.iter(|| {
                let theme = docs_theme_config(black_box(ThemeMode::System))
                    .resolve(black_box(Appearance::DARK));
                let mut acc = 0u32;
                for &tk in &token_kinds {
                    let c = syntax_color(&theme.syntax, tk);
                    acc = acc.wrapping_add(u32::from(c.r) + u32::from(c.g) + u32::from(c.b));
                }
                for &ck in &callout_kinds {
                    let c = callout_tone(&theme, ck).colors.text;
                    acc = acc.wrapping_add(u32::from(c.r) + u32::from(c.g) + u32::from(c.b));
                }
                black_box(acc)
            });
        });

        let config = DocsSiteConfig::new("NVE Docs")
            .with_base_path("/native-view-engine")
            .with_docs_prefix("/docs")
            .add_section(DocsNavSection::new("guides", "Guides", "/docs/guides", "/docs/guides/quickstart"))
            .add_section(DocsNavSection::new("ref", "Reference", "/docs/reference", "/docs/reference/abi"));

        c.bench_function("docs_route_and_href_resolution", |b| {
            b.iter(|| {
                let href = config.resolve_docs_href(black_box("/guides/layout#flexbox"));
                let sec = config.active_section_for(black_box("/docs/guides/layout"));
                black_box((href.len(), sec.map(|s| s.id.as_str())))
            });
        });

        let site = build_bench_site();
        let mut state = DocsAppState::new(
            config,
            site,
            Arc::new(PlaygroundRegistry::new()),
            Arc::new(MemoryHostServices::new()),
            DocsRoute::new("/docs/guides/layout"),
            DocsViewport::desktop_default(),
        );
        state.set_search_query("binary ABI".to_string());

        c.bench_function("docs_state_page_breadcrumbs_and_search", |b| {
            b.iter(|| {
                let page_len = state.current_page().map_or(0, |p| p.title.len());
                let crumbs = state.breadcrumbs();
                let hits = state.search_hits(black_box(8));
                black_box((page_len, crumbs.len(), hits.len()))
            });
        });

        c.bench_function("docs_default_component_registry", |b| {
            b.iter(|| black_box(default_component_registry()));
        });
    }

    criterion_group!(benches, bench_docs_hot_paths);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
