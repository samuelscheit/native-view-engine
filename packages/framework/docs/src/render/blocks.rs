//! Block-level `nve-mdx` AST renderer (headings, paragraphs, lists/checklists, GFM tables, blockquotes, rules).

use dioxus_core_macro::Props;
use nve_ui::ColorHex;
use nve_dioxus::prelude::*;
use nve_mdx::{Block, ListItem, TableAlign};

use crate::render::code_block::CodeBlockView;
use crate::render::components::ComponentBlockView;
use crate::render::directives::DirectiveBlockView;
use nve_web_html::SemanticElement;

use crate::render::html_semantic;
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
                            style: "width: 100%; height: 1px; background-color: {theme.border.default.hex()};",
                        }
                    }
                }
            }
        }
        Block::Paragraph { content, .. } => rsx! {
            view {
                style: "width: 100%; min-width: 0px;",
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
                style: "width: 100%; min-width: 0px; flex-direction: row; gap: 12px; padding-vertical: 10px; padding-horizontal: 14px; border-radius: 6px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                view {
                    style: "width: 3px; flex-shrink: 0; border-radius: 2px; background-color: {theme.status.info.text.hex()};",
                }
                view {
                    style: "flex-grow: 1; min-width: 0px; flex-direction: column; gap: 8px;",
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
                style: "width: 100%; height: 1px; background-color: {theme.border.default.hex()}; margin: 6px;",
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
                        Some(true) => theme.status.success.text.hex(),
                        Some(false) => theme.text.secondary.hex(),
                        None => theme.status.accent.text.hex(),
                    };
                    let checked_attr = item.checked.map(|c| if c { "true" } else { "false" }.to_string());

                    rsx! {
                        view {
                            key: "{scope_prefix}-li-{idx}",
                            accessibility_role: "listitem",
                            aria_checked: checked_attr,
                            style: "width: 100%; flex-direction: row; align-items: flex-start; gap: 10px;",
                            text {
                                style: "font-size: 14px; font-weight: 600; line-height: 1.65; color: {marker_color}; min-width: 18px;",
                                "{marker}"
                            }
                            view {
                                style: "flex-grow: 1; min-width: 0px; flex-direction: column; gap: 6px;",
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

/// Shared flex sizing for table header and body cells on native Taffy/Yoga backends.
///
/// On Web, `th.nve, td.nve` in `NVE_BASE_RESET_CSS` uses `display: table-cell; width: auto;`
/// (higher specificity than generated `.s*` classes), so browsers allocate proportional column
/// widths via CSS table layout while Taffy/Skia and Yoga still get an exact `0px` flex base size.
const TABLE_CELL_FLEX_STYLE: &str =
    "flex-grow: 1; flex-shrink: 1; width: 0px; min-width: 0px; overflow-wrap: break-word;";

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

    let col_count = headers.len().max(rows.first().map_or(0, Vec::len));
    let min_table_w = if col_count >= 3 {
        (col_count as u32).saturating_mul(135).min(680)
    } else {
        0
    };
    let table_style = if min_table_w > 0 {
        format!("width: 100%; min-width: {min_table_w}px; flex-direction: column;")
    } else {
        "width: 100%; flex-direction: column;".to_string()
    };

    rsx! {
        view {
            test_id: "docs-table-wrap-{scope_key}",
            style: "width: 100%; min-width: 0px; overflow-x: auto; border-radius: 8px; border-width: 1px; border-color: {theme.border.default.hex()}; background-color: {theme.surface.base.hex()};",
            view {
                test_id: "docs-table-{scope_key}",
                nve_table: html_semantic(SemanticElement::Table),
                accessibility_role: "table",
                style: "{table_style}",

                // Table header
                if !headers.is_empty() {
                    view {
                        nve_table: html_semantic(SemanticElement::Thead),
                        style: "width: 100%; flex-direction: column;",
                        view {
                            nve_table: html_semantic(SemanticElement::Tr),
                            accessibility_role: "row",
                            style: "width: 100%; flex-direction: row; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                            for (col_idx, cell) in headers.into_iter().enumerate() {
                                {
                                    let ta = align_css(col_idx);
                                    let af = align_flex(col_idx);
                                    rsx! {
                                        view {
                                            key: "{scope_key}-th-{col_idx}",
                                            nve_table: html_semantic(SemanticElement::Th),
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
                }

                // Table body
                if !rows.is_empty() {
                    view {
                        nve_table: html_semantic(SemanticElement::Tbody),
                        style: "width: 100%; flex-direction: column;",
                        for (row_idx, row) in rows.into_iter().enumerate() {
                            {
                                let row_bg = if row_idx % 2 == 1 {
                                    theme.surface.canvas.hex()
                                } else {
                                    theme.surface.base.hex()
                                };
                                rsx! {
                                    view {
                                        key: "{scope_key}-tr-{row_idx}",
                                        nve_table: html_semantic(SemanticElement::Tr),
                                        accessibility_role: "row",
                                        style: "width: 100%; flex-direction: row; background-color: {row_bg}; border-width: 1px; border-color: {theme.border.default.hex()};",
                                        for (col_idx, cell) in row.into_iter().enumerate() {
                                            {
                                                let ta = align_css(col_idx);
                                                let af = align_flex(col_idx);
                                                rsx! {
                                                    view {
                                                        key: "{scope_key}-td-{row_idx}-{col_idx}",
                                                        nve_table: html_semantic(SemanticElement::Td),
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
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nve_backend_headless::HeadlessBackend;
    use nve_backend_web::WebBackend;
    use nve_tables::tables::{DebugId, Layout};
    use nve_dioxus::dioxus_elements::backend::{StoreBackend, Viewport};
    use nve_store::{Length, NodeId};
    use nve_abi::{FrameId, SurfaceId};
    use nve_web_html_web::register_web_html_systems;
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
                SurfaceId::ROOT,
                "/",
                DocsViewport::new(vw, vh),
                DocsSiteConfig::new("Test Docs"),
                Arc::new(site.clone()),
                Arc::new(PlaygroundRegistry::new()),
                services,
            );
            // 1. Verify typed Layout word table + TaffyStoreLayout column rects in HeadlessBackend.
            let mut headless = HeadlessBackend::new();
            headless.set_viewport(
                SurfaceId::ROOT,
                Viewport {
                    width: vw,
                    height: vh,
                    scale_factor: 1.0,
                },
            );
            surface
                .commit_to(FrameId(1), &mut headless)
                .expect("headless commit_to");

            let store = surface.store();
            let mut stack = vec![NodeId::ROOT];
            let mut table_id = None;
            while let Some(nid) = stack.pop() {
                if let Some(dbg) = store.try_row::<DebugId>(nid) {
                    let tid = dbg.test_id();
                    if tid.starts_with("docs-table-") && !tid.starts_with("docs-table-wrap-") {
                        table_id = Some(nid);
                        break;
                    }
                }
                stack.extend_from_slice(store.children(nid));
            }
            let table_id = table_id.expect("rendered docs-table node");

            let sections = store.children(table_id);
            assert_eq!(sections.len(), 2, "expected thead + tbody sections");
            let rows: Vec<NodeId> = sections
                .iter()
                .flat_map(|&sec_id| store.children(sec_id).iter().copied())
                .collect();
            assert_eq!(rows.len(), 3, "expected 1 header row + 2 body rows");

            let header_cells = store.children(rows[0]);
            assert_eq!(header_cells.len(), 4);

            for &row_id in &rows {
                let cells = store.children(row_id);
                assert_eq!(cells.len(), 4);
                for (col_idx, &cell_id) in cells.iter().enumerate() {
                    let layout = store.try_row::<Layout>(cell_id).expect("cell layout");
                    assert_eq!(layout.flex_grow(), 1.0);
                    assert_eq!(layout.flex_shrink(), 1.0);
                    assert_eq!(layout.width(), Length::points(0.0));
                    assert_eq!(layout.min_width(), Length::points(0.0));

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

            // 2. Verify WebBackend + nve-web-html-web emits semantic table tags, overflow-x wrapper, and cell wrapping.
            let mut web = WebBackend::new();
            register_web_html_systems(web.table_registry_mut());
            web.commit(surface.store(), surface.last_changes())
                .expect("web commit");
            let ssr = web.render_ssr(surface.store());
            assert!(
                ssr.style_css.contains("overflow-wrap:break-word;"),
                "expected overflow-wrap:break-word in SSR CSS:\n{}",
                ssr.style_css
            );
            assert!(
                ssr.style_css.contains("overflow-x:auto;"),
                "expected overflow-x:auto wrapper in SSR CSS:\n{}",
                ssr.style_css
            );
            assert!(ssr.html.contains("<table "), "expected semantic <table>");
            assert!(ssr.html.contains("<thead "), "expected semantic <thead>");
            assert!(ssr.html.contains("<tbody "), "expected semantic <tbody>");
            assert!(ssr.html.contains("<tr "), "expected semantic <tr>");
            assert!(ssr.html.contains("<th "), "expected semantic <th>");
            assert!(ssr.html.contains("<td "), "expected semantic <td>");
            let html = web.render_html(surface.store());
            assert!(html.contains("role=\"columnheader\""));
            assert!(html.contains("role=\"cell\""));
        }
    }
}
