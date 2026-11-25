//! Notion-class Block-Doc Editor view (`ActiveView::Docs`).
//!
//! Supports document switching, typed blocks (`H1`, `H2`, `Paragraph`, `Checklist`, `Code`,
//! `Callout`), interactive checklist toggles, inline `TextInput` block editing (`onchange` +
//! `onsubmit`), block creation, kind cycling, up/down reordering, deletion, and live Markdown
//! export (`WorkspaceDoc::to_markdown`), with every mutation tracked in Time-Travel history.

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;

use crate::state::{DocBlockKind, WorkspaceSnapshot, WorkspaceState};
use crate::theme::{
    card_border, control_border, row_border, LayoutMetrics, BORDER_DEFAULT, CARD_ALT_BG, CARD_BG,
    CONTROL_BG, PANEL_BG, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use crate::ui::mutate_state;

fn block_card_colors(
    kind: DocBlockKind,
    accent_hex: &'static str,
    accent_tint: &'static str,
) -> (&'static str, &'static str, &'static str) {
    match kind {
        DocBlockKind::Heading1 => (CARD_BG, TEXT_PRIMARY, BORDER_DEFAULT),
        DocBlockKind::Heading2 => (CARD_BG, accent_hex, BORDER_DEFAULT),
        DocBlockKind::Paragraph => (CARD_BG, TEXT_SECONDARY, BORDER_DEFAULT),
        DocBlockKind::Checklist { checked: true } => ("#071f17", "#86efac", "#22c55e"),
        DocBlockKind::Checklist { checked: false } => (CARD_BG, TEXT_PRIMARY, BORDER_DEFAULT),
        DocBlockKind::Code => ("#090f1d", "#7dd3fc", "#1e3a5f"),
        DocBlockKind::Callout => (accent_tint, TEXT_PRIMARY, accent_hex),
    }
}

fn block_font_size(kind: DocBlockKind) -> u32 {
    match kind {
        DocBlockKind::Heading1 => 14,
        DocBlockKind::Heading2 => 12,
        DocBlockKind::Code => 10,
        _ => 11,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_docs_view(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    current: &WorkspaceSnapshot,
    accent_hex: &'static str,
    accent_tint: &'static str,
    xray: bool,
    m: LayoutMetrics,
    is_mobile: bool,
) -> Element {
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);

    let active_doc = current
        .docs
        .iter()
        .find(|d| d.id == current.active_doc_id)
        .or_else(|| current.docs.first());

    let (done_checks, total_checks) = active_doc
        .map(|d| d.checklist_progress())
        .unwrap_or((0, 0));
    let block_count = active_doc.map(|d| d.blocks.len()).unwrap_or(0);
    let meta_summary = format!(
        "{block_count} blocks | {done_checks}/{total_checks} checks | updated {}",
        active_doc.map(|d| d.updated_at.as_str()).unwrap_or("-")
    );
    let md_preview_text = if current.doc_markdown_preview_open {
        active_doc.map(|d| d.to_markdown()).unwrap_or_default()
    } else {
        String::new()
    };

    let (md_bw, md_bc) = control_border(xray, current.doc_markdown_preview_open, accent_hex);
    let md_bg = if current.doc_markdown_preview_open {
        accent_tint
    } else {
        CONTROL_BG
    };
    let md_fg = if current.doc_markdown_preview_open {
        accent_hex
    } else {
        TEXT_SECONDARY
    };
    let md_btn_lbl = if current.doc_markdown_preview_open {
        "Hide MD"
    } else {
        "Export MD"
    };

    let s_new_doc = Arc::clone(store);
    let s_export_md = Arc::clone(store);
    let s_new_kind = Arc::clone(store);
    let s_new_input = Arc::clone(store);
    let s_new_submit = Arc::clone(store);
    let s_new_btn = Arc::clone(store);

    let composer_kind_lbl = current.doc_new_block_kind.label();
    let tab_bar_dir = if is_mobile { "column" } else { "row" };

    rsx! {
        // 1. Document Switcher + Action Header
        view {
            test_id: "docs-header",
            style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 6px; padding: 8px; background-color: {PANEL_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: {tab_bar_dir}; justify-content: space-between; align-items: center; gap: 6px; border-width: {row_bw}px; border-color: {row_bc};",
                view {
                    style: "flex-direction: row; align-items: center; gap: 5px; flex: 1;",
                    for doc in current.docs.iter() {
                        {
                            let doc_id = doc.id;
                            let is_sel = active_doc.is_some_and(|a| a.id == doc_id);
                            let (bw, bc) = control_border(xray, is_sel, accent_hex);
                            let bg = if is_sel { accent_tint } else { CONTROL_BG };
                            let fg = if is_sel { accent_hex } else { TEXT_SECONDARY };
                            let tab_tid = format!("doc-tab-{doc_id}");
                            let short_title = if doc.title.len() > 22 && is_mobile {
                                format!("{}...", &doc.title[..19])
                            } else if doc.title.len() > 26 {
                                format!("{}...", &doc.title[..23])
                            } else {
                                doc.title.clone()
                            };
                            let s_sel = Arc::clone(store);
                            rsx! {
                                pressable {
                                    key: "doc-tab-{doc_id}",
                                    test_id: "{tab_tid}",
                                    style: "height: 24px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 8px; background-color: {bg}; border-radius: {m.pill_radius}px; border-width: {bw}px; border-color: {bc};",
                                    onpress: move |_| mutate_state(&s_sel, version, |st| st.select_doc(doc_id)),
                                    text { style: "color: {fg}; font-size: 10px;", "{short_title}" }
                                }
                            }
                        }
                    }
                }
                view {
                    style: "flex-direction: row; align-items: center; gap: 5px; flex-shrink: 0;",
                    pressable {
                        test_id: "doc-new-btn",
                        style: "height: 24px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 7px; background-color: {accent_tint}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: {accent_hex};",
                        onpress: move |_| mutate_state(&s_new_doc, version, |st| {
                            let _ = st.create_new_doc();
                        }),
                        text { style: "color: {accent_hex}; font-size: 10px;", "+ Doc" }
                    }
                    pressable {
                        test_id: "doc-export-md",
                        style: "height: 24px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 7px; background-color: {md_bg}; border-radius: {m.pill_radius}px; border-width: {md_bw}px; border-color: {md_bc};",
                        onpress: move |_| mutate_state(&s_export_md, version, |st| st.toggle_doc_markdown_preview()),
                        text { style: "color: {md_fg}; font-size: 10px;", "{md_btn_lbl}" }
                    }
                }
            }
            text { style: "color: {TEXT_MUTED}; font-size: 10px;", "{meta_summary}" }
        }

        // 2. Live Exported Markdown Preview Panel (when toggled ON)
        if current.doc_markdown_preview_open {
            view {
                test_id: "doc-markdown-preview",
                style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 4px; padding: 8px; background-color: #090f1d; border-radius: {m.button_radius}px; border-width: 1px; border-color: {accent_hex};",
                text { style: "color: {accent_hex}; font-size: 9px;", "MARKDOWN EXPORT" }
                text { style: "color: #cbd5e1; font-size: 10px;", "{md_preview_text}" }
            }
        }

        // 3. Interactive Block List
        scrollview {
            test_id: "docs-blocks-list",
            style: "width: 100%; flex: 1; flex-direction: column; gap: {m.list_gap}px; border-width: {row_bw}px; border-color: {row_bc};",
            if let Some(doc) = active_doc {
                for (b_idx, block) in doc.blocks.iter().enumerate() {
                    {
                        let bid = block.id;
                        let total_blocks = doc.blocks.len();
                        let can_up = b_idx > 0;
                        let can_down = b_idx + 1 < total_blocks;
                        let is_editing = current.doc_editing_block_id == Some(bid);
                        let (bg, fg, def_bc) = block_card_colors(block.kind, accent_hex, accent_tint);
                        let bc = if xray { card_bc } else { def_bc };
                        let bw = if xray { card_bw } else { 1 };
                        let font_sz = block_font_size(block.kind);
                        let kind_lbl = block.kind.label();
                        let block_tid = format!("doc-block-{bid}");
                        let kind_tid = format!("doc-block-kind-{bid}");
                        let up_tid = format!("doc-block-up-{bid}");
                        let down_tid = format!("doc-block-down-{bid}");
                        let edit_tid = format!("doc-block-edit-{bid}");
                        let del_tid = format!("doc-block-del-{bid}");
                        let check_tid = format!("doc-check-{bid}");
                        let input_tid = format!("doc-block-input-{bid}");
                        let save_tid = format!("doc-block-save-{bid}");
                        let cancel_tid = format!("doc-block-cancel-{bid}");
                        let up_op = if can_up { "1.0" } else { "0.35" };
                        let down_op = if can_down { "1.0" } else { "0.35" };

                        let s_kind = Arc::clone(store);
                        let s_up = Arc::clone(store);
                        let s_down = Arc::clone(store);
                        let s_edit = Arc::clone(store);
                        let s_del = Arc::clone(store);
                        let s_check = Arc::clone(store);
                        let s_inp = Arc::clone(store);
                        let s_sub = Arc::clone(store);
                        let s_save = Arc::clone(store);
                        let s_cancel = Arc::clone(store);

                        rsx! {
                            view {
                                key: "doc-blk-{bid}",
                                test_id: "{block_tid}",
                                style: "width: 100%; flex-shrink: 0; flex-direction: column; padding: {m.card_pad}px; gap: {m.card_gap}px; background-color: {bg}; border-radius: {m.card_radius}px; border-width: {bw}px; border-color: {bc};",

                                // Top micro-toolbar: Kind pill + Reorder + Edit + Delete
                                view {
                                    style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; gap: 4px;",
                                    pressable {
                                        test_id: "{kind_tid}",
                                        style: "height: 18px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 6px; background-color: {CONTROL_BG}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                                        onpress: move |_| mutate_state(&s_kind, version, |st| {
                                            let _ = st.cycle_doc_block_kind(bid);
                                        }),
                                        text { style: "color: {accent_hex}; font-size: 9px;", "{kind_lbl}" }
                                    }
                                    view {
                                        style: "flex-direction: row; align-items: center; gap: 4px; flex-shrink: 0;",
                                        pressable {
                                            test_id: "{up_tid}",
                                            style: "width: 22px; height: 18px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: 4px; border-width: 1px; border-color: {BORDER_DEFAULT}; opacity: {up_op};",
                                            onpress: move |_| mutate_state(&s_up, version, |st| {
                                                let _ = st.move_doc_block(bid, -1);
                                            }),
                                            text { style: "color: {TEXT_SECONDARY}; font-size: 9px;", "^" }
                                        }
                                        pressable {
                                            test_id: "{down_tid}",
                                            style: "width: 22px; height: 18px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: 4px; border-width: 1px; border-color: {BORDER_DEFAULT}; opacity: {down_op};",
                                            onpress: move |_| mutate_state(&s_down, version, |st| {
                                                let _ = st.move_doc_block(bid, 1);
                                            }),
                                            text { style: "color: {TEXT_SECONDARY}; font-size: 9px;", "v" }
                                        }
                                        pressable {
                                            test_id: "{edit_tid}",
                                            style: "height: 18px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 6px; background-color: {CONTROL_BG}; border-radius: 4px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                                            onpress: move |_| mutate_state(&s_edit, version, |st| {
                                                let _ = st.begin_edit_doc_block(bid);
                                            }),
                                            text { style: "color: {TEXT_SECONDARY}; font-size: 9px;", "Edit" }
                                        }
                                        pressable {
                                            test_id: "{del_tid}",
                                            style: "width: 22px; height: 18px; flex-shrink: 0; justify-content: center; align-items: center; background-color: #450a0a; border-radius: 4px; border-width: 1px; border-color: #ef4444;",
                                            onpress: move |_| mutate_state(&s_del, version, |st| {
                                                let _ = st.delete_doc_block(bid);
                                            }),
                                            text { style: "color: #fca5a5; font-size: 9px;", "x" }
                                        }
                                    }
                                }

                                // Block Content or Inline TextInput Editor
                                if is_editing {
                                    view {
                                        style: "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 5px;",
                                        textinput {
                                            test_id: "{input_tid}",
                                            placeholder: "Edit block text...",
                                            value: "{current.doc_edit_draft}",
                                            multiline: false,
                                            accessibility_label: "Edit document block text",
                                            style: "flex: 1; height: 26px; padding-horizontal: 8px; background-color: {CARD_ALT_BG}; color: {TEXT_PRIMARY}; font-size: 11px; border-radius: {m.button_radius}px; border-width: 1px; border-color: {accent_hex};",
                                            onchange: move |ev| {
                                                let txt = ev.data().value.clone();
                                                mutate_state(&s_inp, version, |st| st.set_doc_edit_draft(txt));
                                            },
                                            onsubmit: move |_| {
                                                mutate_state(&s_sub, version, |st| {
                                                    let _ = st.commit_doc_block_edit();
                                                });
                                            },
                                        }
                                        pressable {
                                            test_id: "{save_tid}",
                                            style: "width: 42px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {accent_hex}; border-radius: {m.button_radius}px;",
                                            onpress: move |_| mutate_state(&s_save, version, |st| {
                                                let _ = st.commit_doc_block_edit();
                                            }),
                                            text { style: "color: #0b1120; font-size: 10px;", "Save" }
                                        }
                                        pressable {
                                            test_id: "{cancel_tid}",
                                            style: "width: 44px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.button_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                                            onpress: move |_| mutate_state(&s_cancel, version, |st| st.cancel_edit_doc_block()),
                                            text { style: "color: {TEXT_SECONDARY}; font-size: 9px;", "Cancel" }
                                        }
                                    }
                                } else if let DocBlockKind::Checklist { checked } = block.kind {
                                    {
                                        let chk_lbl = if checked { "[x]" } else { "[ ]" };
                                        let chk_bg = if checked { "#052e16" } else { CONTROL_BG };
                                        let chk_fg = if checked { "#86efac" } else { TEXT_SECONDARY };
                                        let chk_bc = if checked { "#22c55e" } else { BORDER_DEFAULT };
                                        rsx! {
                                            view {
                                                style: "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 8px;",
                                                pressable {
                                                    test_id: "{check_tid}",
                                                    accessibility_label: "Toggle checklist item",
                                                    style: "width: 28px; height: 22px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {chk_bg}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: {chk_bc};",
                                                    onpress: move |_| mutate_state(&s_check, version, |st| {
                                                        let _ = st.toggle_doc_checklist(bid);
                                                    }),
                                                    text { style: "color: {chk_fg}; font-size: 10px;", "{chk_lbl}" }
                                                }
                                                text { style: "flex: 1; color: {fg}; font-size: {font_sz}px;", "{block.text}" }
                                            }
                                        }
                                    }
                                } else {
                                    text { style: "color: {fg}; font-size: {font_sz}px;", "{block.text}" }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 4. Add-Block Composer Bar (real TextInput + Kind cycler)
        view {
            test_id: "docs-composer",
            style: "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 5px; padding: 8px; background-color: {PANEL_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
            pressable {
                test_id: "doc-new-block-kind",
                style: "width: 50px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.button_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                onpress: move |_| mutate_state(&s_new_kind, version, |st| st.cycle_doc_new_block_kind()),
                text { style: "color: {accent_hex}; font-size: 9px;", "{composer_kind_lbl}" }
            }
            textinput {
                test_id: "doc-new-block-input",
                placeholder: "Add block text (Enter to append)...",
                value: "{current.doc_new_block_draft}",
                multiline: false,
                accessibility_label: "New document block text",
                style: "flex: 1; height: 26px; padding-horizontal: 8px; background-color: {CARD_ALT_BG}; color: {TEXT_PRIMARY}; font-size: 11px; border-radius: {m.button_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                onchange: move |ev| {
                    let txt = ev.data().value.clone();
                    mutate_state(&s_new_input, version, |st| st.set_doc_new_block_draft(txt));
                },
                onsubmit: move |_| {
                    mutate_state(&s_new_submit, version, |st| {
                        let _ = st.submit_new_doc_block_draft();
                    });
                },
            }
            pressable {
                test_id: "doc-new-block-submit",
                style: "width: 52px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {accent_hex}; border-radius: {m.button_radius}px;",
                onpress: move |_| mutate_state(&s_new_btn, version, |st| {
                    let _ = st.submit_new_doc_block_draft();
                }),
                text { style: "color: #0b1120; font-size: 10px;", "+ Block" }
            }
        }
    }
}
