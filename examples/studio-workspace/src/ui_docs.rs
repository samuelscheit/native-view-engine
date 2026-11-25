//! Notion-class Block-Doc Editor view (`ActiveView::Docs`) built on shared `nve-ui` components
//! (`TabsRoot`, `TabList`, `TabTrigger`, `Card`, `Button`, `Input`, `Separator`).
//!
//! Supports document switching, typed blocks (`H1`, `H2`, `Paragraph`, `Checklist`, `Code`,
//! `Callout`), interactive checklist toggles, inline `Input` block editing, block creation,
//! kind cycling, up/down reordering, deletion, and live Markdown export.

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;
use nve_ui::{
    Button, ButtonSize, ButtonVariant, Card, CardVariant, Input, Separator, TabList, TabTrigger,
    TabsRoot,
};

use crate::sdk::StudioHostServices;
use crate::state::{DocBlockKind, WorkspaceSnapshot, WorkspaceState};
use crate::theme::{
    badge_min_w, card_border, control_border, row_border, LayoutMetrics, BORDER_DEFAULT, CARD_BG,
    CONTROL_BG, PANEL_BG, STRETCH_COL, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
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
    host_services: &Arc<StudioHostServices>,
    version: Signal<u64>,
    current: &WorkspaceSnapshot,
    clipboard_notice: Option<&str>,
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
    let active_doc_val = active_doc.map(|d| d.id.to_string()).unwrap_or_default();

    let (done_checks, total_checks) = active_doc
        .map(|d| d.checklist_progress())
        .unwrap_or((0, 0));
    let block_count = active_doc.map(|d| d.blocks.len()).unwrap_or(0);
    let base_meta = format!(
        "{block_count} blocks | {done_checks}/{total_checks} checks | updated {}",
        active_doc.map(|d| d.updated_at.as_str()).unwrap_or("-")
    );
    let meta_summary = match clipboard_notice {
        Some(notice) => format!("{base_meta} | {notice}"),
        None => base_meta,
    };
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
    let md_btn_lbl = if current.doc_markdown_preview_open {
        "Hide MD"
    } else {
        "Export MD"
    };

    let s_sel_tab = Arc::clone(store);
    let s_new_doc = Arc::clone(store);
    let s_export_md = Arc::clone(store);
    let s_copy_md = Arc::clone(store);
    let hs_copy_md = Arc::clone(host_services);
    let s_new_kind = Arc::clone(store);
    let s_new_input = Arc::clone(store);
    let s_new_submit = Arc::clone(store);
    let s_new_btn = Arc::clone(store);

    let composer_kind_lbl = current.doc_new_block_kind.label();
    let tab_bar_dir = if is_mobile { "column" } else { "row" };

    let header_card_style = format!(
        "width: 100%; flex-shrink: 0; padding: 8px; gap: 6px; background-color: {PANEL_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );
    let new_doc_btn_style = format!(
        "min-width: {}px; height: 24px; min-height: 24px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        badge_min_w("+ Doc"),
        m.pill_radius
    );
    let md_btn_style = format!(
        "min-width: {}px; height: 24px; min-height: 24px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {md_bg}; border-radius: {}px; border-width: {md_bw}px; border-color: {md_bc};",
        badge_min_w(md_btn_lbl),
        m.pill_radius
    );
    let copy_md_btn_style = format!(
        "min-width: {}px; height: 24px; min-height: 24px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        badge_min_w("Copy MD"),
        m.pill_radius
    );
    let md_card_style = format!(
        "width: 100%; flex-shrink: 0; padding: 8px; gap: 4px; background-color: #090f1d; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        m.button_radius
    );
    let composer_card_style = format!(
        "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 5px; padding: 8px; background-color: {PANEL_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );
    let kind_cycle_style = format!(
        "width: 54px; height: 34px; min-height: 34px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: {}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
        m.button_radius
    );
    let add_block_btn_style = format!(
        "width: 58px; height: 34px; min-height: 34px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; border-radius: {}px;",
        m.button_radius
    );

    rsx! {
        // 1. Document Switcher Tabs + Action Header (`nve-ui` Card + TabsRoot/TabList/TabTrigger + Button)
        Card {
            variant: CardVariant::Outlined,
            test_id: Some("docs-header".into()),
            padding: Some(8.0),
            gap: Some(6.0),
            style: Some(header_card_style),
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: {tab_bar_dir}; justify-content: space-between; align-items: center; gap: 6px; border-width: {row_bw}px; border-color: {row_bc};",
                view {
                    style: "flex: 1; width: 100%;",
                    TabsRoot {
                        value: Some(active_doc_val),
                        id_prefix: Some("docs-tabs".into()),
                        style: Some("width: 100%; gap: 0px;".into()),
                        on_change: move |next: String| {
                            if let Ok(doc_id) = next.parse::<u32>() {
                                mutate_state(&s_sel_tab, version, |st| st.select_doc(doc_id));
                            }
                        },
                        TabList {
                            accessibility_label: Some("Workspace documents".into()),
                            style: Some("flex-direction: row; align-items: center; gap: 5px; padding: 0px; background-color: transparent; border-width: 0px; border-color: transparent; border-radius: 0px;".into()),
                            for doc in current.docs.iter() {
                                {
                                    let doc_id = doc.id;
                                    let is_sel = active_doc.is_some_and(|a| a.id == doc_id);
                                    let (bw, bc) = control_border(xray, is_sel, accent_hex);
                                    let bg = if is_sel { accent_tint } else { CONTROL_BG };
                                    let tab_tid = format!("doc-tab-{doc_id}");
                                    let short_title = if doc.title.len() > 20 && is_mobile {
                                        format!("{}...", &doc.title[..17])
                                    } else if doc.title.len() > 26 {
                                        format!("{}...", &doc.title[..23])
                                    } else {
                                        doc.title.clone()
                                    };
                                    let trig_style = format!(
                                        "min-width: {}px; height: 24px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                                        badge_min_w(&short_title),
                                        m.pill_radius
                                    );
                                    rsx! {
                                        TabTrigger {
                                            key: "doc-tab-{doc_id}",
                                            value: doc_id.to_string(),
                                            test_id: Some(tab_tid),
                                            style: Some(trig_style),
                                            "{short_title}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                view {
                    style: "flex-direction: row; align-items: center; gap: 5px; flex-shrink: 0;",
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("doc-new-btn".into()),
                        style: Some(new_doc_btn_style),
                        on_press: move |_| mutate_state(&s_new_doc, version, |st| {
                            let _ = st.create_new_doc();
                        }),
                        "+ Doc"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("doc-export-md".into()),
                        style: Some(md_btn_style),
                        on_press: move |_| mutate_state(&s_export_md, version, |st| st.toggle_doc_markdown_preview()),
                        "{md_btn_lbl}"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("doc-copy-md".into()),
                        accessibility_label: Some("Copy active document Markdown to clipboard".into()),
                        style: Some(copy_md_btn_style),
                        on_press: move |_| mutate_state(&s_copy_md, version, |st| {
                            let _ = st.copy_active_doc_markdown(&hs_copy_md);
                        }),
                        "Copy MD"
                    }
                }
            }
            Separator {}
            text { style: "color: {TEXT_MUTED}; font-size: 10px;", "{meta_summary}" }
        }

        // 2. Live Exported Markdown Preview Panel (when toggled ON)
        if current.doc_markdown_preview_open {
            Card {
                variant: CardVariant::Outlined,
                test_id: Some("doc-markdown-preview".into()),
                padding: Some(8.0),
                gap: Some(4.0),
                style: Some(md_card_style),
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

                        let blk_card_style = format!(
                            "width: 100%; flex-shrink: 0; padding: {}px; gap: {}px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                            m.card_pad, m.card_gap, m.card_radius
                        );
                        let kind_btn_style = format!(
                            "min-width: {}px; height: 18px; min-height: 18px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: {}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                            badge_min_w(kind_lbl),
                            m.pill_radius
                        );
                        let up_btn_style = format!(
                            "width: 22px; height: 18px; min-height: 18px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 0px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: 4px; border-width: 1px; border-color: {BORDER_DEFAULT}; opacity: {up_op};"
                        );
                        let down_btn_style = format!(
                            "width: 22px; height: 18px; min-height: 18px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 0px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: 4px; border-width: 1px; border-color: {BORDER_DEFAULT}; opacity: {down_op};"
                        );
                        let edit_btn_style = format!(
                            "min-width: 34px; height: 18px; min-height: 18px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: 4px; border-width: 1px; border-color: {BORDER_DEFAULT};"
                        );
                        let del_btn_style = format!("width: 22px; height: 18px; min-height: 18px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 0px; padding-vertical: 0px; background-color: #450a0a; border-radius: 4px; border-width: 1px; border-color: #ef4444;");
                        let save_btn_style = format!(
                            "width: 46px; height: 34px; min-height: 34px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; border-radius: {}px;",
                            m.button_radius
                        );
                        let cancel_btn_style = format!(
                            "width: 50px; height: 34px; min-height: 34px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: {}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                            m.button_radius
                        );

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
                            Card {
                                key: "doc-blk-{bid}",
                                variant: CardVariant::Elevated,
                                test_id: Some(block_tid),
                                padding: Some(m.card_pad as f32),
                                gap: Some(m.card_gap as f32),
                                style: Some(blk_card_style),

                                // Top micro-toolbar: Kind pill + Reorder + Edit + Delete
                                view {
                                    style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; gap: 4px;",
                                    Button {
                                        variant: ButtonVariant::Secondary,
                                        size: ButtonSize::Sm,
                                        test_id: Some(kind_tid),
                                        style: Some(kind_btn_style),
                                        on_press: move |_| mutate_state(&s_kind, version, |st| {
                                            let _ = st.cycle_doc_block_kind(bid);
                                        }),
                                        "{kind_lbl}"
                                    }
                                    view {
                                        style: "flex-direction: row; align-items: center; gap: 4px; flex-shrink: 0;",
                                        Button {
                                            variant: ButtonVariant::Secondary,
                                            size: ButtonSize::Sm,
                                            test_id: Some(up_tid),
                                            style: Some(up_btn_style),
                                            on_press: move |_| mutate_state(&s_up, version, |st| {
                                                let _ = st.move_doc_block(bid, -1);
                                            }),
                                            "^"
                                        }
                                        Button {
                                            variant: ButtonVariant::Secondary,
                                            size: ButtonSize::Sm,
                                            test_id: Some(down_tid),
                                            style: Some(down_btn_style),
                                            on_press: move |_| mutate_state(&s_down, version, |st| {
                                                let _ = st.move_doc_block(bid, 1);
                                            }),
                                            "v"
                                        }
                                        Button {
                                            variant: ButtonVariant::Secondary,
                                            size: ButtonSize::Sm,
                                            test_id: Some(edit_tid),
                                            style: Some(edit_btn_style),
                                            on_press: move |_| mutate_state(&s_edit, version, |st| {
                                                let _ = st.begin_edit_doc_block(bid);
                                            }),
                                            "Edit"
                                        }
                                        Button {
                                            variant: ButtonVariant::Destructive,
                                            size: ButtonSize::Sm,
                                            test_id: Some(del_tid),
                                            style: Some(del_btn_style),
                                            on_press: move |_| mutate_state(&s_del, version, |st| {
                                                let _ = st.delete_doc_block(bid);
                                            }),
                                            "x"
                                        }
                                    }
                                }

                                // Block Content or Inline Input Editor
                                if is_editing {
                                    view {
                                        style: "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 5px;",
                                        view {
                                            style: "flex: 1;",
                                            Input {
                                                test_id: Some(input_tid),
                                                placeholder: Some("Edit block text...".into()),
                                                value: current.doc_edit_draft.clone(),
                                                accessibility_label: Some("Edit document block text".into()),
                                                on_change: move |txt: String| {
                                                    mutate_state(&s_inp, version, |st| st.set_doc_edit_draft(txt));
                                                },
                                                on_submit: move |_| {
                                                    mutate_state(&s_sub, version, |st| {
                                                        let _ = st.commit_doc_block_edit();
                                                    });
                                                },
                                            }
                                        }
                                        Button {
                                            variant: ButtonVariant::Primary,
                                            size: ButtonSize::Sm,
                                            test_id: Some(save_tid),
                                            style: Some(save_btn_style),
                                            on_press: move |_| mutate_state(&s_save, version, |st| {
                                                let _ = st.commit_doc_block_edit();
                                            }),
                                            "Save"
                                        }
                                        Button {
                                            variant: ButtonVariant::Secondary,
                                            size: ButtonSize::Sm,
                                            test_id: Some(cancel_tid),
                                            style: Some(cancel_btn_style),
                                            on_press: move |_| mutate_state(&s_cancel, version, |st| st.cancel_edit_doc_block()),
                                            "Cancel"
                                        }
                                    }
                                } else if let DocBlockKind::Checklist { checked } = block.kind {
                                    {
                                        let chk_lbl = if checked { "[x]" } else { "[ ]" };
                                        let chk_bg = if checked { "#052e16" } else { CONTROL_BG };
                                        let chk_bc = if checked { "#22c55e" } else { BORDER_DEFAULT };
                                        let chk_btn_style = format!(
                                            "width: 30px; height: 22px; min-height: 22px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 0px; padding-vertical: 0px; background-color: {chk_bg}; border-radius: {}px; border-width: 1px; border-color: {chk_bc};",
                                            m.pill_radius
                                        );
                                        rsx! {
                                            view {
                                                style: "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 8px;",
                                                Button {
                                                    variant: ButtonVariant::Secondary,
                                                    size: ButtonSize::Sm,
                                                    test_id: Some(check_tid),
                                                    accessibility_label: Some("Toggle checklist item".into()),
                                                    style: Some(chk_btn_style),
                                                    on_press: move |_| mutate_state(&s_check, version, |st| {
                                                        let _ = st.toggle_doc_checklist(bid);
                                                    }),
                                                    "{chk_lbl}"
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

        // 4. Add-Block Composer Bar (`nve-ui` Card + Button + Input)
        Card {
            variant: CardVariant::Outlined,
            test_id: Some("docs-composer".into()),
            padding: Some(8.0),
            gap: Some(5.0),
            style: Some(composer_card_style),
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("doc-new-block-kind".into()),
                style: Some(kind_cycle_style),
                on_press: move |_| mutate_state(&s_new_kind, version, |st| st.cycle_doc_new_block_kind()),
                "{composer_kind_lbl}"
            }
            view {
                style: "flex: 1;",
                Input {
                    test_id: Some("doc-new-block-input".into()),
                    placeholder: Some("Add block text (Enter to append)...".into()),
                    value: current.doc_new_block_draft.clone(),
                    accessibility_label: Some("New document block text".into()),
                    on_change: move |txt: String| {
                        mutate_state(&s_new_input, version, |st| st.set_doc_new_block_draft(txt));
                    },
                    on_submit: move |_| {
                        mutate_state(&s_new_submit, version, |st| {
                            let _ = st.submit_new_doc_block_draft();
                        });
                    },
                }
            }
            Button {
                variant: ButtonVariant::Primary,
                size: ButtonSize::Sm,
                test_id: Some("doc-new-block-submit".into()),
                style: Some(add_block_btn_style),
                on_press: move |_| mutate_state(&s_new_btn, version, |st| {
                    let _ = st.submit_new_doc_block_draft();
                }),
                "+ Block"
            }
        }
    }
}
