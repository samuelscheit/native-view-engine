//! Zero-dependency pure-Rust Markdown & declarative `.mdx` compiler, syntax highlighter,
//! `_meta.toml` navigation builder, compile-time validator, and binary full-text search index
//! (`NVS1`) for `native-view-engine`.

pub mod ast;
pub mod codegen;
mod directive;
pub mod frontmatter;
pub mod highlight;
mod highlight_langs;
mod highlight_scan;
pub mod inline;
mod mdx_tag;
mod meta_toml;
pub mod nav;
pub mod parser;
mod parser_blocks;
pub mod search;
mod search_codec;
pub mod validate;

pub use ast::{
    Block, CalloutKind, CardItem, CodeBlock, CompiledPage, CompiledSite, ComponentNode,
    ComponentProp, DirectiveBlock, Frontmatter, HydrateStrategy, Inline, ListItem, LiveBlock,
    PropValue, SlugGenerator, SourcePos, StepItem, TabItem, TableAlign, TocHeading,
    normalize_route, slugify,
};
pub use codegen::{SiteCodegenOptions, generate_rust_module};
pub use frontmatter::parse_frontmatter;
pub use highlight::{HighlightLine, HighlightSpan, Language, TokenKind, highlight_code};
pub use inline::{inline_plain_text, parse_inlines};
pub use nav::{
    MetaEntry, NavLink, NavNode, NavTree, PrevNext, SiteSource, build_navigation, parse_meta_toml,
    route_from_source_path,
};
pub use parser::{ParsedBody, parse_markdown_body};
pub use search::{
    NVS1_MAGIC, NVS1_VERSION, Posting, SearchDoc, SearchHit, SearchIndex, SearchIndexError, fnv1a64,
};
pub use validate::{
    ComponentRegistry, ComponentSchema, Diagnostic, DiagnosticKind, DiagnosticSeverity, PropSchema,
    PropType, validate_site,
};

/// Compilation options for [`compile_site`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CompileOptions {
    /// Whether pages with `draft: true` in frontmatter are included in the compiled site.
    pub include_drafts: bool,
}

/// Compile a single `.md` or `.mdx` document into a [`CompiledPage`].
pub fn compile_document(route: &str, source_path: &str, source: &str) -> CompiledPage {
    let (frontmatter, body, body_start_line) = parse_frontmatter(source);
    let norm_route = normalize_route(route);
    let parsed = parse_markdown_body(body, body_start_line, &frontmatter, &norm_route);

    let title = frontmatter
        .title
        .clone()
        .or_else(|| {
            parsed
                .toc
                .iter()
                .find(|h| h.level == 1)
                .map(|h| h.text.clone())
        })
        .unwrap_or_default();

    let description = frontmatter.description.clone().unwrap_or_else(|| {
        parsed
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Paragraph { content, .. } => {
                    let text = inline_plain_text(content).trim().to_string();
                    if text.is_empty() { None } else { Some(text) }
                }
                _ => None,
            })
            .unwrap_or_default()
    });

    CompiledPage {
        route: norm_route,
        source_path: source_path.to_string(),
        frontmatter,
        title,
        description,
        toc: parsed.toc,
        blocks: parsed.blocks,
        live_blocks: parsed.live_blocks,
        plain_text: parsed.plain_text,
    }
}

/// Compile an entire [`SiteSource`] into a [`CompiledSite`] (pages, navigation, pagination, and
/// `NVS1` search index).
pub fn compile_site(source: &SiteSource, options: &CompileOptions) -> CompiledSite {
    let mut pages = Vec::new();
    for (rel_path, content) in source.doc_files() {
        let route = route_from_source_path(rel_path);
        let page = compile_document(&route, rel_path, content);
        if page.frontmatter.draft && !options.include_drafts {
            continue;
        }
        pages.push(page);
    }

    let (nav, prev_next) = build_navigation(source, &pages);
    let search_index = SearchIndex::build(&pages);

    CompiledSite {
        pages,
        nav,
        prev_next,
        search_index,
    }
}

/// Compile a site and immediately run compile-time schema & cross-link validation.
pub fn compile_site_with_validation(
    source: &SiteSource,
    options: &CompileOptions,
    registry: &ComponentRegistry,
) -> (CompiledSite, Vec<Diagnostic>) {
    let site = compile_site(source, options);
    let diagnostics = validate_site(&site, registry);
    (site, diagnostics)
}
