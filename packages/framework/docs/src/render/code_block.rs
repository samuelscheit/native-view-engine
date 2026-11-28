//! Syntax-highlighted code blocks using `nve_mdx::highlight_code`, line numbers, highlighted-line accent, and clipboard copy.

use dioxus_core_macro::Props;
use crate::theme::syntax_color;
use nve_ui::{accent_indicator, Separator};
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
        theme.status.success.text
    } else {
        theme.text.secondary
    };
    let copy_border = if is_copied {
        theme.status.success.text
    } else {
        theme.border.default
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
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .min_width_px(0.0)
                .overflow_mode(Overflow::Hidden)
                .flex_col()
                .radius(10.0)
                .border(1.0, theme.border.default)
                .bg(theme.surface.sunken),

            // Top bar: optional filename title, language pill, Copy button; one hairline below
            // (the frame already draws the outer border).
            view {
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_row()
                    .justify_content(FlexJustify::SpaceBetween)
                    .align_items(FlexAlign::Center)
                    .padding_xy(12.0, 8.0)
                    .bg(theme.surface.base),

                view {
                    nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(8.0),
                    if let Some(ref fname) = title {
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(theme.text.primary),
                            "{fname}"
                        }
                    }
                    if let Some(ref lang_label) = lang_badge {
                        view {
                            nve_style: TypedStyle::new()
                                .padding_xy(6.0, 2.0)
                                .radius(4.0)
                                .bg(theme.surface.subtle)
                                .border(1.0, theme.border.default),
                            text {
                                nve_style: TypedStyle::new().font_size(11.0).font_weight(600).color(theme.status.info.text),
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
                    nve_style: TypedStyle::new()
                        .padding_xy(9.0, 4.0)
                        .radius(6.0)
                        .bg(theme.surface.subtle)
                        .border(1.0, copy_border),
                    onpress: move |_| {
                        let _ = state.write().copy_code(&key_for_copy, &code_for_copy);
                    },
                    text {
                        nve_style: TypedStyle::new().font_size(11.0).font_weight(600).color(copy_fg),
                        "{copy_label}"
                    }
                }
            }

            Separator {}

            // Highlighted code lines
            scrollview {
                accessibility_role: "code",
                nve_style: TypedStyle::new().width_pct(100.0).min_width_px(0.0).flex_col().padding_y(10.0),
                if lines.is_empty() {
                    view {
                        nve_style: TypedStyle::new().padding_x(14.0),
                        text {
                            nve_style: TypedStyle::new()
                                .font_size(13.0)
                                .font_family("monospace")
                                .white_space(WhiteSpace::Pre)
                                .color(theme.syntax.plain),
                            "{code}"
                        }
                    }
                } else {
                    for line in lines {
                        {
                            let line_bg = if line.highlighted {
                                theme.status.accent.subtle
                            } else {
                                ColorRgba::TRANSPARENT
                            };
                            let marker_color = if line.highlighted {
                                accent_indicator(&theme)
                            } else {
                                ColorRgba::TRANSPARENT
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
                                    nve_style: TypedStyle::new()
                                        .width_pct(100.0)
                                        .flex_row()
                                        .align_items(FlexAlign::Center)
                                        .padding_xy(10.0, 2.0)
                                        .gap(8.0)
                                        .bg(line_bg),

                                    view {
                                        nve_style: TypedStyle::new()
                                            .width_px(2.0)
                                            .height_px(16.0)
                                            .flex_shrink(0.0)
                                            .bg(marker_color)
                                            .radius(1.0),
                                    }

                                    if display_line_numbers {
                                        text {
                                            nve_style: TypedStyle::new()
                                                .min_width_px(26.0)
                                                .flex_shrink(0.0)
                                                .font_size(12.0)
                                                .font_family("monospace")
                                                .color(theme.text.secondary)
                                                .text_align(TextAlign::Right),
                                            "{line_no_str}"
                                        }
                                    }

                                    view {
                                        test_id: "docs-code-tokens",
                                        nve_style: TypedStyle::new()
                                            .display(Display::Block)
                                            .flex_row()
                                            .flex_grow(1.0)
                                            .white_space(WhiteSpace::Pre)
                                            .line_height_ratio(1.55),
                                        if line.tokens.is_empty() {
                                            text {
                                                nve_style: TypedStyle::new()
                                                    .display(Display::Inline)
                                                    .font_size(13.0)
                                                    .font_family("monospace")
                                                    .white_space(WhiteSpace::Pre)
                                                    .color(theme.syntax.plain),
                                                " "
                                            }
                                        } else {
                                            for (tok_idx, span) in line.tokens.into_iter().enumerate() {
                                                {
                                                    let tok_color = syntax_color(&theme.syntax, span.kind);
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
                                                            nve_style: TypedStyle::new()
                                                                .display(Display::Inline)
                                                                .font_size(13.0)
                                                                .font_family("monospace")
                                                                .white_space(WhiteSpace::Pre)
                                                                .color(tok_color),
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
