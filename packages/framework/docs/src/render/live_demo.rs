//! Interactive `:::demo{id="..."}` / `<LiveDemo id="..." />` playground frame with `Preview | Code` tabs
//! and `data-nve-island` / `data-nve-hydrate` progressive hydration attributes.

use std::sync::Arc;
use nve_ui::{Separator, tab_colors};

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
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_col()
                .radius(10.0)
                .border(1.0, theme.border.default)
                .bg(theme.surface.base),

            // Header with title, hydration strategy badge, and Preview | Code tabs. It wraps on
            // narrow viewports (tabs move below the title) and sits on the frame's own surface,
            // divided from the body by one hairline instead of a second nested border box.
            view {
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_row()
                    .flex_wrap(FlexWrap::Wrap)
                    .justify_content(FlexJustify::SpaceBetween)
                    .align_items(FlexAlign::Center)
                    .gap(8.0)
                    .padding_xy(12.0, 8.0),

                view {
                    nve_style: TypedStyle::new()
                        .flex_row()
                        .flex_wrap(FlexWrap::Wrap)
                        .flex_shrink(1.0)
                        .min_width_px(0.0)
                        .align_items(FlexAlign::Center)
                        .gap(8.0),
                    text {
                        nve_style: TypedStyle::new().font_size(13.0).font_weight(700).color(theme.text.primary),
                        "{display_title}"
                    }
                    view {
                        nve_style: TypedStyle::new()
                            .padding_xy(6.0, 1.0)
                            .radius(4.0)
                            .bg(theme.surface.base)
                            .border(1.0, theme.border.default),
                        text {
                            nve_style: TypedStyle::new().font_size(10.0).font_weight(600).color(theme.status.info.text),
                            "{hydrate_str}"
                        }
                    }
                }

                view {
                    accessibility_role: "tablist",
                    nve_style: TypedStyle::new().flex_row().flex_shrink(0.0).gap(4.0),
                    pressable {
                        test_id: "docs-demo-tab-preview-{id}",
                        accessibility_role: "tab",
                        aria_selected: if is_preview { "true" } else { "false" },
                        nve_style: TypedStyle::new()
                            .padding_xy(9.0, 4.0)
                            .radius(6.0)
                            .bg(prev.bg)
                            .border(1.0, prev.border),
                        onpress: move |_| {
                            state.write().set_active_tab(group_for_preview.clone(), "preview");
                        },
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(prev.fg),
                            "Preview"
                        }
                    }
                    pressable {
                        test_id: "docs-demo-tab-code-{id}",
                        accessibility_role: "tab",
                        aria_selected: if !is_preview { "true" } else { "false" },
                        nve_style: TypedStyle::new()
                            .padding_xy(9.0, 4.0)
                            .radius(6.0)
                            .bg(code.bg)
                            .border(1.0, code.border),
                        onpress: move |_| {
                            state.write().set_active_tab(group_for_code.clone(), "code");
                        },
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(code.fg),
                            "Code"
                        }
                    }
                }
            }

            Separator {}

            // Live Playground preview OR syntax-highlighted source code
            if is_preview {
                view {
                    test_id: "docs-demo-preview-{id}",
                    nve_style: TypedStyle::new()
                        .width_pct(100.0)
                        .min_height_px(min_height as f32)
                        .padding(16.0)
                        .bg(theme.surface.canvas),
                    if let Some(registered) = entry {
                        PlaygroundHost {
                            key: "playground-{id}",
                            render: PlaygroundRenderProp(registered.render),
                        }
                    } else {
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary),
                            "Interactive demo '{id}' is not registered in PlaygroundRegistry."
                        }
                    }
                }
            } else {
                view {
                    test_id: "docs-demo-code-{id}",
                    nve_style: TypedStyle::new().width_pct(100.0).padding(10.0),
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
