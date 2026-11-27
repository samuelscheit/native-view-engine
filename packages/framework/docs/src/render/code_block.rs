//! Syntax-highlighted code blocks using `nve_mdx::highlight_code`, line numbers, highlighted-line accent, and clipboard copy.

use dioxus_core_macro::Props;
use crate::theme::syntax_color;
use nve_ui::{accent_indicator, ColorHex, Hex};
use nve_dioxus::prelude::*;
use nve_mdx::{CodeBlock, HighlightLine, highlight_code};

use crate::state::use_docs_state;

/// Renders a compiled [`nve_mdx::CodeBlock`] with syntax tokens, optional title bar, line highlights, and Copy button.
#[component]
pub fn CodeBlockView(block: CodeBlock, block_key: String) -> Element {
    let lines = if block.highlighted.is_empty() && !block.code.is_empty() {
        highlight_code(block.lang.as_deref(), &block.code, &block.highlight_lines)
    } else {
        block.highlighted.clone()
    };

    rsx! {
        HighlightedCodeView {
            code: block.code,
            lang: block.lang,
            title: block.title,
            lines: lines,
            show_line_numbers: block.show_line_numbers,
            block_key: block_key,
        }
    }
}

/// Reusable highlighted code-block component shared by fenced blocks, `<CodeGroup>`, and `LiveDemo` Code tabs.
#[component]
pub fn HighlightedCodeView(
    code: String,
    #[props(default)] lang: Option<String>,
    #[props(default)] title: Option<String>,
    lines: Vec<HighlightLine>,
    #[props(default = false)] show_line_numbers: bool,
    block_key: String,
) -> Element {
    let mut state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let is_copied = snap.is_code_copied(&block_key);
    drop(snap);

    let copy_label = if is_copied { "Copied" } else { "Copy" };
    let copy_fg = if is_copied {
        theme.status.success.text.hex()
    } else {
        theme.text.secondary.hex()
    };
    let copy_border = if is_copied {
        theme.status.success.text.hex()
    } else {
        theme.border.default.hex()
    };
    let copy_tid = format!("docs-code-copy-{block_key}");
    let frame_tid = format!("docs-code-block-{block_key}");
    let code_for_copy = code.clone();
    let key_for_copy = block_key.clone();
    let lang_badge = lang
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);

    let display_line_numbers = show_line_numbers || lines.iter().any(|l| l.highlighted);

    rsx! {
        view {
            test_id: "{frame_tid}",
            accessibility_role: "codeblock",
            accessibility_label: title.clone().or_else(|| lang_badge.clone()).unwrap_or_else(|| "Code block".to_string()),
            style: "width: 100%; min-width: 0px; overflow: hidden; flex-direction: column; border-radius: 10px; border-width: 1px; border-color: {theme.border.default.hex()}; background-color: {theme.surface.sunken.hex()};",

            // Top bar: optional filename title, language pill, Copy button
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 12px; padding-vertical: 8px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",

                view {
                    style: "flex-direction: row; align-items: center; gap: 8px;",
                    if let Some(ref fname) = title {
                        text {
                            style: "font-size: 12px; font-weight: 600; color: {theme.text.primary.hex()};",
                            "{fname}"
                        }
                    }
                    if let Some(ref lang_label) = lang_badge {
                        view {
                            style: "padding-horizontal: 6px; padding-vertical: 2px; border-radius: 4px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                            text {
                                style: "font-size: 11px; font-weight: 600; color: {theme.status.info.text.hex()};",
                                "{lang_label}"
                            }
                        }
                    }
                }

                pressable {
                    test_id: "{copy_tid}",
                    accessibility_role: "button",
                    accessibility_label: "Copy code to clipboard",
                    nve_table: nve_dioxus::table_attr_fn(|tx, node| {
                        tx.edit::<nve_web_html::HtmlDataAttrs>(node).entries(Some(
                            nve_web_html::DataAttrList::from_pairs(vec![(
                                "data-code-copy".to_owned(),
                                "true".to_owned(),
                            )]),
                        ));
                    }),
                    style: "padding-horizontal: 9px; padding-vertical: 4px; border-radius: 6px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {copy_border};",
                    onpress: move |_| {
                        let _ = state.write().copy_code(&key_for_copy, &code_for_copy);
                    },
                    text {
                        style: "font-size: 11px; font-weight: 600; color: {copy_fg};",
                        "{copy_label}"
                    }
                }
            }

            // Highlighted code lines
            scrollview {
                accessibility_role: "code",
                style: "width: 100%; min-width: 0px; flex-direction: column; padding-vertical: 10px;",
                if lines.is_empty() {
                    view {
                        style: "padding-horizontal: 14px;",
                        text {
                            style: "font-size: 13px; font-family: monospace; white-space: pre; color: {theme.syntax.plain.hex()};",
                            "{code}"
                        }
                    }
                } else {
                    for line in lines {
                        {
                            let line_bg = if line.highlighted {
                                theme.status.accent.subtle.hex()
                            } else {
                                Hex::TRANSPARENT
                            };
                            let marker_color = if line.highlighted {
                                accent_indicator(&theme).hex()
                            } else {
                                Hex::TRANSPARENT
                            };
                            let line_no_str = line.line_no.to_string();
                            let is_highlighted = line.highlighted;

                            rsx! {
                                view {
                                    key: "{block_key}-ln-{line.line_no}",
                                    nve_table: nve_dioxus::table_attr_fn_with_clear(
                                        move |tx, node| {
                                            if is_highlighted {
                                                tx.edit::<nve_web_html::HtmlDataAttrs>(node)
                                                    .entries(Some(
                                                        nve_web_html::DataAttrList::from_pairs(
                                                            vec![(
                                                                "data-line-highlighted".to_owned(),
                                                                "true".to_owned(),
                                                            )],
                                                        ),
                                                    ));
                                            } else if tx
                                                .store()
                                                .has::<nve_web_html::HtmlDataAttrs>(node)
                                            {
                                                tx.detach::<nve_web_html::HtmlDataAttrs>(node);
                                            }
                                        },
                                        |tx, node| {
                                            tx.detach::<nve_web_html::HtmlDataAttrs>(node);
                                        },
                                    ),
                                    style: "width: 100%; flex-direction: row; align-items: center; padding-horizontal: 10px; padding-vertical: 2px; gap: 8px; background-color: {line_bg};",

                                    view {
                                        style: "width: 2px; height: 16px; flex-shrink: 0; background-color: {marker_color}; border-radius: 1px;",
                                    }

                                    if display_line_numbers {
                                        text {
                                            style: "min-width: 26px; flex-shrink: 0; font-size: 12px; font-family: monospace; color: {theme.text.secondary.hex()}; text-align: right;",
                                            "{line_no_str}"
                                        }
                                    }

                                    view {
                                        test_id: "docs-code-tokens",
                                        style: "display: block; flex-direction: row; flex-grow: 1; white-space: pre; line-height: 1.55;",
                                        if line.tokens.is_empty() {
                                            text {
                                                style: "display: inline; font-size: 13px; font-family: monospace; white-space: pre; color: {theme.syntax.plain.hex()};",
                                                " "
                                            }
                                        } else {
                                            for (tok_idx, span) in line.tokens.into_iter().enumerate() {
                                                {
                                                    let tok_color = syntax_color(&theme.syntax, span.kind).hex();
                                                    let tok_kind: &'static str = span.kind.as_str();
                                                    rsx! {
                                                        text {
                                                            key: "{block_key}-ln-{line.line_no}-tok-{tok_idx}",
                                                            nve_table: nve_dioxus::table_attr_fn_with_clear(
                                                                move |tx, node| {
                                                                    tx.edit::<nve_web_html::HtmlDataAttrs>(node)
                                                                        .entries(Some(
                                                                            nve_web_html::DataAttrList::from_pairs(
                                                                                vec![(
                                                                                    "data-token".to_owned(),
                                                                                    tok_kind.to_owned(),
                                                                                )],
                                                                            ),
                                                                        ));
                                                                },
                                                                |tx, node| {
                                                                    tx.detach::<nve_web_html::HtmlDataAttrs>(node);
                                                                },
                                                            ),
                                                            style: "display: inline; font-size: 13px; font-family: monospace; white-space: pre; color: {tok_color};",
                                                            "{span.text}"
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
