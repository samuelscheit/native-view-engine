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

/// Shared flex sizing for table header and body cells.
///
/// `nve-abi::LayoutProps` has `width`, `min_width`, `flex_grow`, and `flex_shrink` (with
/// `flex-basis: auto`), so setting `width: 0px; min-width: 0px;` gives every cell a `0px`
/// flex base size across CSS Flexbox (§9.2), Taffy/Skia, and Yoga — preventing columns with
/// different text lengths from misaligning horizontally between header and body rows.
const TABLE_CELL_FLEX_STYLE: &str =
    "flex-grow: 1; flex-shrink: 1; width: 0px; min-width: 0px;";

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
                                    style: "{TABLE_CELL_FLEX_STYLE} padding-horizontal: 12px; padding-vertical: 9px; align-items: {af}; text-align: {ta};",
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
                                            style: "{TABLE_CELL_FLEX_STYLE} padding-horizontal: 12px; padding-vertical: 8px; align-items: {af}; text-align: {ta};",
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nve_abi::{Dimension, RawPropValue};
    use nve_backend_headless::HeadlessBackend;
    use nve_backend_web::WebBackend;
    use nve_dioxus::dioxus_elements::backend::{RenderBackend, Viewport};
    use nve_host_services::{HostServiceProvider, MemoryHostServices};
    use nve_mdx::{CompileOptions, SiteSource, compile_site_with_validation};

    use crate::{
        DocsSiteConfig, DocsViewport, PlaygroundRegistry, create_docs_surface_with_viewport,
        default_component_registry,
    };

    #[test]
    fn table_view_aligns_header_and_body_columns_with_zero_base_and_min_width() {
        let mut src = SiteSource::new();
        src.insert("_meta.toml", "index = \"Benchmarks\"\n");
        src.insert(
            "index.mdx",
            r#"---
title: "Benchmarks"
description: "Table column alignment verification."
---

| Benchmark | Median | Mean | Notes |
| :--- | :---: | ---: | :--- |
| Encode 10k nodes with deeply nested flexbox children | 1.2ms | 1.3ms | X |
| A | 999.999ms | 1000.001ms | Very long explanatory benchmark note spanning multiple words |
"#,
        );
        let registry = default_component_registry();
        let (site, diags) =
            compile_site_with_validation(&src, &CompileOptions::default(), &registry);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:#?}");

        for (vw, vh) in [(1280.0_f32, 800.0_f32), (390.0_f32, 844.0_f32)] {
            let services: Arc<dyn HostServiceProvider> = Arc::new(MemoryHostServices::new());
            let mut surface = create_docs_surface_with_viewport(
                1,
                "/",
                DocsViewport::new(vw, vh),
                DocsSiteConfig::new("Test Docs"),
                Arc::new(site.clone()),
                Arc::new(PlaygroundRegistry::new()),
                services,
            );
            let (_, _, frame_bytes) = surface.render_frame(1);

            // 1. Verify typed LayoutProps + Taffy computed column rects in HeadlessBackend.
            let mut headless = HeadlessBackend::new();
            headless.set_viewport(
                1,
                Viewport {
                    width: vw,
                    height: vh,
                    scale_factor: 1.0,
                },
            );
            headless
                .submit_frame(&frame_bytes)
                .expect("headless submit_frame");

            let table_id = headless
                .nodes()
                .values()
                .find_map(|n| match n.raw_props.get("testID") {
                    Some(RawPropValue::String(s)) if s.starts_with("docs-table-") => Some(n.id),
                    _ => None,
                })
                .expect("rendered docs-table node");

            let rows = &headless.node(table_id).expect("table node").children;
            assert_eq!(rows.len(), 3, "expected 1 header row + 2 body rows");

            let header_cells = &headless.node(rows[0]).expect("header row").children;
            assert_eq!(header_cells.len(), 4);

            for &row_id in rows {
                let cells = &headless.node(row_id).expect("row node").children;
                assert_eq!(cells.len(), 4);
                for (col_idx, &cell_id) in cells.iter().enumerate() {
                    let cell = headless.node(cell_id).expect("cell node");
                    assert_eq!(cell.layout.flex_grow, 1.0);
                    assert_eq!(cell.layout.flex_shrink, 1.0);
                    assert_eq!(cell.layout.width, Dimension::Points(0.0));
                    assert_eq!(cell.layout.min_width, Dimension::Points(0.0));

                    let header_rect = headless
                        .layout_of(header_cells[col_idx])
                        .expect("header rect");
                    let cell_rect = headless.layout_of(cell_id).expect("cell rect");
                    assert!(
                        (header_rect.x - cell_rect.x).abs() < 0.01,
                        "viewport {vw}x{vh} col {col_idx} x mismatch: header={} body={}",
                        header_rect.x,
                        cell_rect.x
                    );
                    assert!(
                        (header_rect.width - cell_rect.width).abs() < 0.01,
                        "viewport {vw}x{vh} col {col_idx} width mismatch: header={} body={}",
                        header_rect.width,
                        cell_rect.width
                    );
                }
            }

            // 2. Verify WebBackend emits consistent `width:0;min-width:0;flex-grow:1;` CSS.
            let mut web = WebBackend::new();
            web.submit_frame(&frame_bytes).expect("web submit_frame");
            let ssr = web.render_ssr();
            assert!(
                ssr.style_css.contains("flex-grow:1;width:0;min-width:0;"),
                "expected zero-basis flex cell rule in SSR CSS:\n{}",
                ssr.style_css
            );
            let html = web.render_html();
            assert!(html.contains("role=\"columnheader\""));
            assert!(html.contains("role=\"cell\""));
            assert!(html.contains("flex-grow:1;flex-shrink:1;"));
            assert!(html.contains("width:0px;"));
            assert!(html.contains("min-width:0px;"));
        }
    }
}
