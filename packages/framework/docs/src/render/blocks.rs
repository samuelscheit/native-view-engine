//! Block-level `nve-mdx` AST renderer (headings, paragraphs, lists/checklists, GFM tables, blockquotes, rules).

use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;
use nve_mdx::{Block, ListItem, TableAlign};

use crate::render::code_block::CodeBlockView;
use crate::render::components::ComponentBlockView;
use crate::render::directives::DirectiveBlockView;
use crate::render::inlines::InlinesView;
use crate::state::use_docs_state;

/// Renders an ordered list of [`Block`] AST nodes with stable scope keys.
#[component]
pub fn BlocksView(blocks: Vec<Block>, scope_prefix: String) -> Element {
    rsx! {
        view {
            style: "width: 100%; flex-direction: column; gap: 14px;",
            for (idx, block) in blocks.into_iter().enumerate() {
                BlockView {
                    key: "{scope_prefix}-blk-{idx}",
                    block: block,
                    scope_key: format!("{scope_prefix}-{idx}"),
                }
            }
        }
    }
}

#[component]
fn BlockView(block: Block, scope_key: String) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();

    match block {
        Block::Heading {
            level,
            id,
            content,
            text: heading_text,
            ..
        } => {
            let clamped = level.clamp(1, 6);
            let font_size: u32 = match clamped {
                1 => 30,
                2 => 23,
                3 => 19,
                4 => 16,
                5 => 14,
                _ => 13,
            };
            let level_str = clamped.to_string();
            let tid = format!("docs-heading-{id}");
            let top_pad = if clamped <= 2 { "10px" } else { "4px" };

            rsx! {
                view {
                    test_id: "{tid}",
                    accessibility_role: "heading",
                    accessibility_label: "{heading_text}",
                    "aria-level": "{level_str}",
                    "id": "{id}",
                    style: "width: 100%; flex-direction: column; gap: 6px; padding-vertical: {top_pad};",
                    InlinesView {
                        inlines: content,
                        font_size: font_size,
                        bold: true,
                    }
                    if clamped == 2 {
                        view {
                            style: "width: 100%; height: 1px; background-color: {theme.border};",
                        }
                    }
                }
            }
        }
        Block::Paragraph { content, .. } => rsx! {
            view {
                style: "width: 100%;",
                InlinesView {
                    inlines: content,
                    font_size: 15,
                }
            }
        },
        Block::CodeBlock(cb) => rsx! {
            CodeBlockView {
                block: cb,
                block_key: scope_key,
            }
        },
        Block::List {
            ordered,
            start,
            items,
            ..
        } => rsx! {
            ListView {
                ordered: ordered,
                start: start,
                items: items,
                scope_prefix: scope_key,
            }
        },
        Block::Table {
            alignments,
            headers,
            rows,
            ..
        } => rsx! {
            TableView {
                alignments: alignments,
                headers: headers,
                rows: rows,
                scope_key: scope_key,
            }
        },
        Block::Blockquote { blocks, .. } => rsx! {
            view {
                accessibility_role: "blockquote",
                style: "width: 100%; flex-direction: row; gap: 12px; padding-vertical: 10px; padding-horizontal: 14px; border-radius: 6px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                view {
                    style: "width: 3px; border-radius: 2px; background-color: {theme.accent_alt};",
                }
                view {
                    style: "flex-grow: 1; flex-direction: column; gap: 8px;",
                    BlocksView {
                        blocks: blocks,
                        scope_prefix: format!("{scope_key}-bq"),
                    }
                }
            }
        },
        Block::ThematicBreak { .. } => rsx! {
            view {
                accessibility_role: "separator",
                style: "width: 100%; height: 1px; background-color: {theme.border}; margin: 6px;",
            }
        },
        Block::Directive(dir) => rsx! {
            DirectiveBlockView {
                directive: dir,
                scope_key: scope_key,
            }
        },
        Block::Component(comp) => rsx! {
            ComponentBlockView {
                comp: comp,
                scope_key: scope_key,
            }
        },
    }
}

#[component]
fn ListView(ordered: bool, start: usize, items: Vec<ListItem>, scope_prefix: String) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();

    rsx! {
        view {
            accessibility_role: "list",
            style: "width: 100%; flex-direction: column; gap: 8px; padding-horizontal: 4px;",
            for (idx, item) in items.into_iter().enumerate() {
                {
                    let marker = match item.checked {
                        Some(true) => "☑".to_string(),
                        Some(false) => "☐".to_string(),
                        None if ordered => format!("{}.", start + idx),
                        None => "•".to_string(),
                    };
                    let marker_color = match item.checked {
                        Some(true) => theme.success,
                        Some(false) => theme.muted,
                        None => theme.accent,
                    };
                    let checked_attr = item.checked.map(|c| if c { "true" } else { "false" }.to_string());

                    rsx! {
                        view {
                            key: "{scope_prefix}-li-{idx}",
                            accessibility_role: "listitem",
                            aria_checked: checked_attr,
                            style: "width: 100%; flex-direction: row; align-items: flex-start; gap: 10px;",
                            text {
                                style: "font-size: 14px; font-weight: 600; color: {marker_color}; min-width: 18px;",
                                "{marker}"
                            }
                            view {
                                style: "flex-grow: 1; flex-direction: column; gap: 6px;",
                                BlocksView {
                                    blocks: item.blocks,
                                    scope_prefix: format!("{scope_prefix}-li-{idx}"),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TableView(
    alignments: Vec<TableAlign>,
    headers: Vec<Vec<nve_mdx::Inline>>,
    rows: Vec<Vec<Vec<nve_mdx::Inline>>>,
    scope_key: String,
) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();

    let align_css = |col: usize| -> &'static str {
        match alignments.get(col).copied().unwrap_or(TableAlign::None) {
            TableAlign::Center => "center",
            TableAlign::Right => "right",
            TableAlign::Left | TableAlign::None => "left",
        }
    };
    let align_flex = |col: usize| -> &'static str {
        match alignments.get(col).copied().unwrap_or(TableAlign::None) {
            TableAlign::Center => "center",
            TableAlign::Right => "flex-end",
            TableAlign::Left | TableAlign::None => "flex-start",
        }
    };

    rsx! {
        view {
            test_id: "docs-table-{scope_key}",
            accessibility_role: "table",
            style: "width: 100%; flex-direction: column; border-radius: 8px; border-width: 1px; border-color: {theme.border}; background-color: {theme.surface};",

            // Table header row
            if !headers.is_empty() {
                view {
                    accessibility_role: "row",
                    style: "width: 100%; flex-direction: row; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                    for (col_idx, cell) in headers.into_iter().enumerate() {
                        {
                            let ta = align_css(col_idx);
                            let af = align_flex(col_idx);
                            rsx! {
                                view {
                                    key: "{scope_key}-th-{col_idx}",
                                    accessibility_role: "columnheader",
                                    style: "flex-grow: 1; flex-shrink: 1; padding-horizontal: 12px; padding-vertical: 9px; align-items: {af}; text-align: {ta};",
                                    InlinesView {
                                        inlines: cell,
                                        font_size: 13,
                                        bold: true,
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Table body rows
            for (row_idx, row) in rows.into_iter().enumerate() {
                {
                    let row_bg = if row_idx % 2 == 1 {
                        theme.canvas
                    } else {
                        theme.surface
                    };
                    rsx! {
                        view {
                            key: "{scope_key}-tr-{row_idx}",
                            accessibility_role: "row",
                            style: "width: 100%; flex-direction: row; background-color: {row_bg}; border-width: 1px; border-color: {theme.border};",
                            for (col_idx, cell) in row.into_iter().enumerate() {
                                {
                                    let ta = align_css(col_idx);
                                    let af = align_flex(col_idx);
                                    rsx! {
                                        view {
                                            key: "{scope_key}-td-{row_idx}-{col_idx}",
                                            accessibility_role: "cell",
                                            style: "flex-grow: 1; flex-shrink: 1; padding-horizontal: 12px; padding-vertical: 8px; align-items: {af}; text-align: {ta};",
                                            InlinesView {
                                                inlines: cell,
                                                font_size: 13,
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
