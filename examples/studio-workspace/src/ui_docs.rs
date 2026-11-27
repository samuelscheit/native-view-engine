//! Notion-class Block-Doc Editor view (`ActiveView::Docs`) built on shared `nve-ui` components
//! (`TabsRoot`, `TabList`, `TabTrigger`, `Card`, `Button`, `Input`, `Separator`).
//!
//! Supports document switching, typed blocks (`H1`, `H2`, `Paragraph`, `Checklist`, `Code`,
//! `Callout`), interactive checklist toggles, inline `Input` block editing, block creation,
//! kind cycling, up/down reordering, deletion, and live Markdown export.
//!
//! [`DocsView`] subscribes to the `docs` domain (plus `nav` for the clipboard notice); each
//! [`DocBlockCard`] is memoized on its block.

use nve_dioxus::prelude::*;
use nve_ui::{
    Button, ButtonSize, ButtonVariant, Card, CardVariant, ColorHex, Input, Separator, TabList,
    TabTrigger, TabsRoot, Theme,
};

use nve_abi::ColorRgba;

use crate::state::{DocBlock, DocBlockKind};
use crate::theme::badge_min_w;
use crate::ui::store::use_studio;
use crate::ui::style::use_studio_style;

/// `(background, text, border)` of a block card.
const fn block_colors(t: &Theme, kind: DocBlockKind) -> (ColorRgba, ColorRgba, ColorRgba) {
    match kind {
        DocBlockKind::Heading1 => (t.surface.raised, t.text.primary, t.border.default),
        DocBlockKind::Heading2 => (t.surface.raised, t.text.link, t.border.default),
        DocBlockKind::Paragraph => (t.surface.raised, t.text.secondary, t.border.default),
        DocBlockKind::Checklist { checked: true } => (t.status.success.subtle, t.status.success.text, t.status.success.border),
        DocBlockKind::Checklist { checked: false } => (t.surface.raised, t.text.primary, t.border.default),
        DocBlockKind::Code => (t.surface.sunken, t.syntax.plain, t.border.default),
        DocBlockKind::Callout => (t.status.accent.subtle, t.text.primary, t.status.accent.border),
    }
}

const fn block_font_size(kind: DocBlockKind) -> u32 {
    match kind {
        DocBlockKind::Heading1 => 14,
        DocBlockKind::Heading2 => 12,
        DocBlockKind::Code => 10,
        _ => 11,
    }
}

fn short_title(title: &str, compact: bool) -> String {
    let max = if compact { 20 } else { 26 };
    if title.chars().count() > max {
        format!("{}...", title.chars().take(max - 3).collect::<String>())
    } else {
        title.to_owned()
    }
}

#[component]
pub(crate) fn DocsView(compact: bool) -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let notice = store.nav.read().clipboard_notice.clone();
    let docs = store.docs.read();
    let active = docs.docs.iter().find(|d| d.id == docs.active_doc_id).or_else(|| docs.docs.first());
    let active_id = active.map(|d| d.id);
    let tabs: Vec<(u32, String)> = docs.docs.iter().map(|d| (d.id, short_title(&d.title, compact))).collect();
    let blocks: Vec<DocBlock> = active.map(|d| d.blocks.clone()).unwrap_or_default();
    let (done_checks, total_checks) = active.map(|d| d.checklist_progress()).unwrap_or((0, 0));
    let base_meta = format!(
        "{} blocks | {done_checks}/{total_checks} checks | updated {}",
        blocks.len(),
        active.map_or("-", |d| d.updated_at.as_str())
    );
    let md_open = docs.doc_markdown_preview_open;
    let md_preview = if md_open { active.map(|d| d.to_markdown()).unwrap_or_default() } else { String::new() };
    let editing = docs.doc_editing_block_id;
    let edit_draft = docs.doc_edit_draft.clone();
    let new_kind_lbl = docs.doc_new_block_kind.label();
    let new_draft = docs.doc_new_block_draft.clone();
    drop(docs);

    let meta_summary = match notice {
        Some(notice) => format!("{base_meta} | {notice}"),
        None => base_meta,
    };
    let md_btn_lbl = if md_open { "Hide MD" } else { "Export MD" };
    let pill = |label: &str, active: bool| {
        s.compact_control(
            &format!("min-width: {}px; height: 24px; min-height: 24px;", badge_min_w(label)),
            4,
            s.m.pill_radius,
            Some(active),
        )
    };
    let panel_card = |extra: &str| {
        format!(
            "width: 100%; flex-shrink: 0; padding: 8px; {extra} background-color: {}; border-radius: {}px; {}",
            s.t.surface.base.hex(),
            s.m.button_radius,
            s.card()
        )
    };
    let tab_bar_dir = if compact { "column" } else { "row" };
    let block_count = blocks.len();

    rsx! {
        Card {
            variant: CardVariant::Outlined,
            test_id: Some("docs-header".into()),
            padding: Some(8.0),
            gap: Some(6.0),
            style: Some(panel_card("gap: 6px;")),
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: {tab_bar_dir}; justify-content: space-between; align-items: center; gap: 6px; {s.row()}",
                view {
                    style: "flex: 1; width: 100%;",
                    TabsRoot {
                        value: active_id.map(|id| id.to_string()),
                        id_prefix: Some("docs-tabs".into()),
                        style: Some("width: 100%; gap: 0px;".into()),
                        on_change: move |next: String| {
                            if let Ok(doc_id) = next.parse::<u32>() {
                                store.update(|st| st.select_doc(doc_id));
                            }
                        },
                        TabList {
                            accessibility_label: Some("Workspace documents".into()),
                            style: Some("flex-direction: row; align-items: center; gap: 5px; padding: 0px; background-color: transparent; border-width: 0px; border-color: transparent; border-radius: 0px;".into()),
                            for (doc_id, title) in tabs {
                                TabTrigger {
                                    key: "doc-tab-{doc_id}",
                                    value: doc_id.to_string(),
                                    test_id: Some(format!("doc-tab-{doc_id}")),
                                    style: Some(pill(&title, active_id == Some(doc_id))),
                                    "{title}"
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
                        style: Some(pill("+ Doc", true)),
                        on_press: move |_| store.update(|st| {
                            let _ = st.create_new_doc();
                        }),
                        "+ Doc"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("doc-export-md".into()),
                        style: Some(pill(md_btn_lbl, md_open)),
                        on_press: move |_| store.update(|st| st.toggle_doc_markdown_preview()),
                        "{md_btn_lbl}"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("doc-copy-md".into()),
                        accessibility_label: Some("Copy active document Markdown to clipboard".into()),
                        style: Some(pill("Copy MD", false)),
                        on_press: move |_| {
                            let services = store.services();
                            store.update(|st| {
                                let _ = st.copy_active_doc_markdown(&services);
                            });
                        },
                        "Copy MD"
                    }
                }
            }
            Separator {}
            text { style: "color: {s.t.text.tertiary.hex()}; font-size: 10px;", "{meta_summary}" }
        }

        if md_open {
            Card {
                variant: CardVariant::Outlined,
                test_id: Some("doc-markdown-preview".into()),
                padding: Some(8.0),
                gap: Some(4.0),
                style: Some(format!(
                    "width: 100%; flex-shrink: 0; padding: 8px; gap: 4px; background-color: {}; border-radius: {}px; {}",
                    s.t.surface.sunken.hex(),
                    s.m.button_radius,
                    s.control(true)
                )),
                text { style: "color: {s.t.text.link.hex()}; font-size: 9px;", "MARKDOWN EXPORT" }
                text { style: "color: {s.t.text.secondary.hex()}; font-size: 10px;", "{md_preview}" }
            }
        }

        scrollview {
            test_id: "docs-blocks-list",
            style: "width: 100%; flex: 1; flex-direction: column; gap: {s.m.list_gap}px; {s.row()}",
            for (idx, block) in blocks.into_iter().enumerate() {
                DocBlockCard {
                    key: "doc-blk-{block.id}",
                    can_up: idx > 0,
                    can_down: idx + 1 < block_count,
                    edit_draft: (editing == Some(block.id)).then(|| edit_draft.clone()),
                    block,
                }
            }
        }

        Card {
            variant: CardVariant::Outlined,
            test_id: Some("docs-composer".into()),
            padding: Some(8.0),
            gap: Some(5.0),
            style: Some(panel_card("flex-direction: row; align-items: center; gap: 5px;")),
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("doc-new-block-kind".into()),
                style: Some(s.compact_control("width: 54px; height: 34px; min-height: 34px;", 2, s.m.button_radius, Some(false))),
                on_press: move |_| store.update(|st| st.cycle_doc_new_block_kind()),
                "{new_kind_lbl}"
            }
            view {
                style: "flex: 1;",
                Input {
                    test_id: Some("doc-new-block-input".into()),
                    placeholder: Some("Add block text (Enter to append)...".into()),
                    value: new_draft,
                    accessibility_label: Some("New document block text".into()),
                    on_change: move |txt: String| store.update(|st| st.set_doc_new_block_draft(txt)),
                    on_submit: move |_| store.update(|st| {
                        let _ = st.submit_new_doc_block_draft();
                    }),
                }
            }
            Button {
                variant: ButtonVariant::Primary,
                size: ButtonSize::Sm,
                test_id: Some("doc-new-block-submit".into()),
                style: Some(s.compact_control("width: 58px; height: 34px; min-height: 34px;", 2, s.m.button_radius, None)),
                on_press: move |_| store.update(|st| {
                    let _ = st.submit_new_doc_block_draft();
                }),
                "+ Block"
            }
        }
    }
}

/// One document block with its micro-toolbar; `edit_draft` is `Some` while it is being edited.
#[component]
fn DocBlockCard(block: DocBlock, can_up: bool, can_down: bool, edit_draft: Option<String>) -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let bid = block.id;
    let (bg, fg, border) = block_colors(&s.t, block.kind);
    let border = if s.xray() { s.card() } else { crate::ui::style::Outline { width: 1, color: border } };
    let font_sz = block_font_size(block.kind);
    let kind_lbl = block.kind.label();
    let card_style = format!(
        "width: 100%; flex-shrink: 0; padding: {}px; gap: {}px; background-color: {}; border-radius: {}px; {border}",
        s.m.card_pad,
        s.m.card_gap,
        bg.hex(),
        s.m.card_radius
    );
    let radius_sm = s.t.radius.sm as u32;
    let tool = |dims: &str, enabled: bool| {
        let style = s.compact_control(dims, 0, radius_sm, Some(false));
        if enabled { style } else { format!("{style} opacity: 0.35;") }
    };
    let field_btn = |w: u32, paint: Option<bool>| {
        s.compact_control(&format!("width: {w}px; height: 34px; min-height: 34px;"), 2, s.m.button_radius, paint)
    };
    let fg = fg.hex();

    rsx! {
        Card {
            variant: CardVariant::Elevated,
            test_id: Some(format!("doc-block-{bid}")),
            padding: Some(s.m.card_pad as f32),
            gap: Some(s.m.card_gap as f32),
            style: Some(card_style),
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; gap: 4px;",
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some(format!("doc-block-kind-{bid}")),
                    style: Some(s.compact_control(
                        &format!("min-width: {}px; height: 18px; min-height: 18px;", badge_min_w(kind_lbl)),
                        4,
                        s.m.pill_radius,
                        Some(false),
                    )),
                    on_press: move |_| store.update(|st| {
                        let _ = st.cycle_doc_block_kind(bid);
                    }),
                    "{kind_lbl}"
                }
                view {
                    style: "flex-direction: row; align-items: center; gap: 4px; flex-shrink: 0;",
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-block-up-{bid}")),
                        style: Some(tool("width: 22px; height: 18px; min-height: 18px;", can_up)),
                        on_press: move |_| store.update(|st| {
                            let _ = st.move_doc_block(bid, -1);
                        }),
                        "^"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-block-down-{bid}")),
                        style: Some(tool("width: 22px; height: 18px; min-height: 18px;", can_down)),
                        on_press: move |_| store.update(|st| {
                            let _ = st.move_doc_block(bid, 1);
                        }),
                        "v"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-block-edit-{bid}")),
                        style: Some(tool("min-width: 34px; height: 18px; min-height: 18px;", true)),
                        on_press: move |_| store.update(|st| {
                            let _ = st.begin_edit_doc_block(bid);
                        }),
                        "Edit"
                    }
                    Button {
                        variant: ButtonVariant::Destructive,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-block-del-{bid}")),
                        style: Some(s.compact_control("width: 22px; height: 18px; min-height: 18px;", 0, radius_sm, None)),
                        on_press: move |_| store.update(|st| {
                            let _ = st.delete_doc_block(bid);
                        }),
                        "x"
                    }
                }
            }

            if let Some(draft) = edit_draft {
                view {
                    style: "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 5px;",
                    view {
                        style: "flex: 1;",
                        Input {
                            test_id: Some(format!("doc-block-input-{bid}")),
                            placeholder: Some("Edit block text...".into()),
                            value: draft,
                            accessibility_label: Some("Edit document block text".into()),
                            on_change: move |txt: String| store.update(|st| st.set_doc_edit_draft(txt)),
                            on_submit: move |_| store.update(|st| {
                                let _ = st.commit_doc_block_edit();
                            }),
                        }
                    }
                    Button {
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-block-save-{bid}")),
                        style: Some(field_btn(46, None)),
                        on_press: move |_| store.update(|st| {
                            let _ = st.commit_doc_block_edit();
                        }),
                        "Save"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-block-cancel-{bid}")),
                        style: Some(field_btn(50, Some(false))),
                        on_press: move |_| store.update(|st| st.cancel_edit_doc_block()),
                        "Cancel"
                    }
                }
            } else if let DocBlockKind::Checklist { checked } = block.kind {
                view {
                    style: "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 8px;",
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-check-{bid}")),
                        accessibility_label: Some("Toggle checklist item".into()),
                        style: Some(format!(
                            "{} background-color: {}; border-width: 1px; border-color: {};",
                            s.compact_control("width: 30px; height: 22px; min-height: 22px;", 0, s.m.pill_radius, None),
                            if checked { s.t.status.success.subtle.hex() } else { s.t.surface.subtle.hex() },
                            if checked { s.t.status.success.border.hex() } else { s.t.border.default.hex() }
                        )),
                        on_press: move |_| store.update(|st| {
                            let _ = st.toggle_doc_checklist(bid);
                        }),
                        if checked { "[x]" } else { "[ ]" }
                    }
                    text { style: "flex: 1; color: {fg}; font-size: {font_sz}px;", "{block.text}" }
                }
            } else {
                text { style: "color: {fg}; font-size: {font_sz}px;", "{block.text}" }
            }
        }
    }
}
