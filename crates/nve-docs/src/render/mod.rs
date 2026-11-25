//! Complete `nve-mdx` AST → NVE/Dioxus renderer (`DocsArticle`, blocks, inlines, code blocks, directives).

pub mod blocks;
pub mod code_block;
pub mod components;
pub mod directives;
pub mod inlines;
pub mod live_demo;

pub use blocks::BlocksView;
pub use code_block::{CodeBlockView, HighlightedCodeView};
pub use components::{ComponentBlockView, InlineComponentView};
pub use directives::DirectiveBlockView;
pub use inlines::InlinesView;
pub use live_demo::LiveDemoView;

use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;
use nve_mdx::{Block, CompiledPage};

use crate::state::use_docs_state;

/// Renders a compiled MDX page inside an `accessibility_role: "article"` semantic container.
#[component]
pub fn DocsArticle(page: CompiledPage) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();

    // Avoid duplicating an H1 when the page's first block already repeats `page.title`.
    let body_blocks: Vec<Block> = match page.blocks.first() {
        Some(Block::Heading { level: 1, text, .. })
            if text.trim().eq_ignore_ascii_case(page.title.trim()) =>
        {
            page.blocks[1..].to_vec()
        }
        _ => page.blocks.clone(),
    };

    let page_badge = page.frontmatter.badge.clone();
    let page_tags = page.frontmatter.tags.clone();

    rsx! {
        view {
            test_id: "docs-article",
            accessibility_role: "article",
            accessibility_label: "{page.title}",
            "data-route": "{page.route}",
            style: "width: 100%; flex-direction: column; gap: 18px;",

            // Article header (badge, H1 title, description, tags)
            view {
                test_id: "docs-article-header",
                style: "width: 100%; flex-direction: column; gap: 10px; padding-vertical: 8px; border-width: 1px; border-color: {theme.border}; border-radius: 10px; padding-horizontal: 18px; background-color: {theme.surface};",

                if let Some(badge_txt) = page_badge {
                    view {
                        style: "flex-direction: row;",
                        view {
                            style: "padding-horizontal: 8px; padding-vertical: 2px; border-radius: 999px; background-color: {theme.accent_surface}; border-width: 1px; border-color: {theme.accent};",
                            text {
                                style: "font-size: 11px; font-weight: 600; color: {theme.accent};",
                                "{badge_txt}"
                            }
                        }
                    }
                }

                view {
                    test_id: "docs-page-title",
                    accessibility_role: "heading",
                    "aria-level": "1",
                    text {
                        style: "font-size: 32px; font-weight: 700; color: {theme.text};",
                        "{page.title}"
                    }
                }

                if !page.description.is_empty() {
                    text {
                        test_id: "docs-page-description",
                        style: "font-size: 15px; color: {theme.muted};",
                        "{page.description}"
                    }
                }

                if !page_tags.is_empty() {
                    view {
                        style: "flex-direction: row; gap: 6px; padding-vertical: 2px;",
                        for tag in page_tags {
                            view {
                                key: "tag-{tag}",
                                style: "padding-horizontal: 8px; padding-vertical: 2px; border-radius: 999px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                                text {
                                    style: "font-size: 11px; color: {theme.muted};",
                                    "#{tag}"
                                }
                            }
                        }
                    }
                }
            }

            // Rendered MDX AST blocks
            BlocksView {
                blocks: body_blocks,
                scope_prefix: page.route.clone(),
            }
        }
    }
}
