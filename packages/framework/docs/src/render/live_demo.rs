//! Interactive `:::demo{id="..."}` / `<LiveDemo id="..." />` playground frame with `Preview | Code` tabs
//! and `data-nve-island` / `data-nve-hydrate` progressive hydration attributes.

use std::sync::Arc;
use nve_ui::{ColorHex, tab_colors};

use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;
use nve_mdx::{Block, HydrateStrategy, highlight_code};

use crate::registry::PlaygroundRenderer;
use crate::render::code_block::HighlightedCodeView;
use crate::state::use_docs_state;

#[derive(Clone)]
struct PlaygroundRenderProp(PlaygroundRenderer);

impl PartialEq for PlaygroundRenderProp {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Dedicated component boundary so each registered playground closure owns an isolated Dioxus hook scope.
#[component]
fn PlaygroundHost(render: PlaygroundRenderProp) -> Element {
    (render.0)()
}

/// Interactive playground frame rendered for `:::demo{id="..."}` and `<LiveDemo id="..." />`.
#[component]
pub fn LiveDemoView(
    id: String,
    hydrate: HydrateStrategy,
    height: Option<u32>,
    title: Option<String>,
    code_override: Option<String>,
    fallback_blocks: Vec<Block>,
    scope_key: String,
) -> Element {
    let mut state = use_docs_state();
    let tab_group = format!("demo-mode-{id}-{scope_key}");
    let snap = state.read();
    let theme = snap.theme();
    let mode = snap.active_tab(&tab_group, "preview");
    let entry = snap.playgrounds.get(&id).cloned();
    drop(snap);

    let is_preview = mode != "code";
    let display_title = title
        .or_else(|| entry.as_ref().map(|e| e.meta.title.clone()))
        .unwrap_or_else(|| format!("Live Demo: {id}"));
    let source_code = code_override
        .or_else(|| entry.as_ref().map(|e| e.meta.source_code.clone()))
        .or_else(|| {
            fallback_blocks.iter().find_map(|b| match b {
                Block::CodeBlock(cb) => Some(cb.code.clone()),
                _ => None,
            })
        })
        .unwrap_or_default();
    let source_lang = entry
        .as_ref()
        .map(|e| e.meta.language.clone())
        .unwrap_or_else(|| "rust".to_string());
    let min_height = height
        .or_else(|| entry.as_ref().and_then(|e| e.meta.default_height))
        .unwrap_or(140);
    let hydrate_str = hydrate.as_str();

    let prev = tab_colors(&theme, is_preview, false);
    let code = tab_colors(&theme, !is_preview, false);

    let group_for_preview = tab_group.clone();
    let group_for_code = tab_group.clone();
    let highlighted = if !is_preview {
        highlight_code(Some(&source_lang), &source_code, &[])
    } else {
        Vec::new()
    };

    rsx! {
        view {
            test_id: "docs-demo-{id}",
            accessibility_role: "region",
            accessibility_label: "{display_title}",
            nve_table: crate::render::html_data_attrs(vec![
                ("nve-island".to_owned(), id.clone()),
                ("nve-hydrate".to_owned(), hydrate_str.to_owned()),
            ]),
            style: "width: 100%; flex-direction: column; border-radius: 10px; border-width: 1px; border-color: {theme.border.default.hex()}; background-color: {theme.surface.base.hex()};",

            // Header with title, hydration strategy badge, and Preview | Code tabs
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 12px; padding-vertical: 8px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",

                view {
                    style: "flex-direction: row; align-items: center; gap: 8px;",
                    text {
                        style: "font-size: 13px; font-weight: 700; color: {theme.text.primary.hex()};",
                        "{display_title}"
                    }
                    view {
                        style: "padding-horizontal: 6px; padding-vertical: 1px; border-radius: 4px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                        text {
                            style: "font-size: 10px; font-weight: 600; color: {theme.status.info.text.hex()};",
                            "{hydrate_str}"
                        }
                    }
                }

                view {
                    accessibility_role: "tablist",
                    style: "flex-direction: row; gap: 4px;",
                    pressable {
                        test_id: "docs-demo-tab-preview-{id}",
                        accessibility_role: "tab",
                        aria_selected: if is_preview { "true" } else { "false" },
                        style: "padding-horizontal: 9px; padding-vertical: 4px; border-radius: 6px; background-color: {prev.bg.hex()}; border-width: 1px; border-color: {prev.border.hex()};",
                        onpress: move |_| {
                            state.write().set_active_tab(group_for_preview.clone(), "preview");
                        },
                        text {
                            style: "font-size: 12px; font-weight: 600; color: {prev.fg.hex()};",
                            "Preview"
                        }
                    }
                    pressable {
                        test_id: "docs-demo-tab-code-{id}",
                        accessibility_role: "tab",
                        aria_selected: if !is_preview { "true" } else { "false" },
                        style: "padding-horizontal: 9px; padding-vertical: 4px; border-radius: 6px; background-color: {code.bg.hex()}; border-width: 1px; border-color: {code.border.hex()};",
                        onpress: move |_| {
                            state.write().set_active_tab(group_for_code.clone(), "code");
                        },
                        text {
                            style: "font-size: 12px; font-weight: 600; color: {code.fg.hex()};",
                            "Code"
                        }
                    }
                }
            }

            // Live Playground preview OR syntax-highlighted source code
            if is_preview {
                view {
                    test_id: "docs-demo-preview-{id}",
                    style: "width: 100%; min-height: {min_height}px; padding: 16px; background-color: {theme.surface.canvas.hex()};",
                    if let Some(registered) = entry {
                        PlaygroundHost {
                            key: "playground-{id}",
                            render: PlaygroundRenderProp(registered.render),
                        }
                    } else {
                        text {
                            style: "font-size: 13px; color: {theme.text.secondary.hex()};",
                            "Interactive demo '{id}' is not registered in PlaygroundRegistry."
                        }
                    }
                }
            } else {
                view {
                    test_id: "docs-demo-code-{id}",
                    style: "width: 100%; padding: 10px;",
                    HighlightedCodeView {
                        code: source_code,
                        lang: Some(source_lang),
                        title: Some(format!("{id}.rs")),
                        lines: highlighted,
                        show_line_numbers: true,
                        block_key: format!("demo-code-{id}"),
                    }
                }
            }
        }
    }
}
