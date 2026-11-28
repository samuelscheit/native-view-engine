//! Notion-class Block-Doc Editor view (`ActiveView::Docs`) built on shared `nve-ui` components
//! (`TabsRoot`, `TabList`, `TabTrigger`, `Card`, `Button`, `Input`, `Separator`).
//!
//! Supports document switching, typed blocks (`H1`, `H2`, `Paragraph`, `Checklist`, `Code`,
//! `Callout`), interactive checklist toggles, inline `Input` block editing, block creation,
//! kind cycling, up/down reordering, deletion, and live Markdown export.
//!
//! [`DocsView`] subscribes to the `docs` domain (plus `nav` for the clipboard notice); each
//! [`DocBlockCard`] is memoized on its block.

use nve_abi::ColorRgba;
use nve_dioxus::prelude::*;
use nve_ui::{
    Button, ButtonSize, ButtonVariant, Card, CardVariant, Input, Separator, TabList, TabTrigger,
    TabsRoot, Theme,
};

use crate::state::{DocBlock, DocBlockKind};
use crate::theme::badge_min_w;
use crate::ui::store::use_studio;
use crate::ui::style::{
    Outline, StudioStyleExt, fixed_dims, min_w_dims, row_center, text_style, use_studio_style,
};

/// `(background, text, border)` of a block card.
const fn block_colors(t: &Theme, kind: DocBlockKind) -> (ColorRgba, ColorRgba, ColorRgba) {
    match kind {
        DocBlockKind::Heading1 => (t.surface.raised, t.text.primary, t.border.default),
        DocBlockKind::Heading2 => (t.surface.raised, t.text.link, t.border.default),
        DocBlockKind::Paragraph => (t.surface.raised, t.text.secondary, t.border.default),
        DocBlockKind::Checklist { checked: true } => (
            t.status.success.subtle,
            t.status.success.text,
            t.status.success.border,
        ),
        DocBlockKind::Checklist { checked: false } => {
            (t.surface.raised, t.text.primary, t.border.default)
        }
        DocBlockKind::Code => (t.surface.sunken, t.syntax.plain, t.border.default),
        DocBlockKind::Callout => (
            t.status.accent.subtle,
            t.text.primary,
            t.status.accent.border,
        ),
    }
}

const fn block_font_size(kind: DocBlockKind) -> f32 {
    match kind {
        DocBlockKind::Heading1 => 14.0,
        DocBlockKind::Heading2 => 12.0,
        DocBlockKind::Code => 10.0,
        _ => 11.0,
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
    nve_dioxus::render_probe::record_render("DocsView");
    let store = use_studio();
    let s = use_studio_style();
    let notice = store.route.read().clipboard_notice.clone();
    let docs = store.docs.read();
    let active = docs
        .docs
        .iter()
        .find(|d| d.id == docs.active_doc_id)
        .or_else(|| docs.docs.first());
    let active_id = active.map(|d| d.id);
    let tabs: Vec<(u32, String)> = docs
        .docs
        .iter()
        .map(|d| (d.id, short_title(&d.title, compact)))
        .collect();
    let blocks: Vec<DocBlock> = active.map(|d| d.blocks.clone()).unwrap_or_default();
    let (done_checks, total_checks) = active.map(|d| d.checklist_progress()).unwrap_or((0, 0));
    let base_meta = format!(
        "{} blocks | {done_checks}/{total_checks} checks | updated {}",
        blocks.len(),
        active.map_or("-", |d| d.updated_at.as_str())
    );
    let md_open = docs.doc_markdown_preview_open;
    let md_preview = if md_open {
        active.map(|d| d.to_markdown()).unwrap_or_default()
    } else {
        String::new()
    };
    let editing = docs.doc_editing_block_id;
    drop(docs);

    let meta_summary = match notice {
        Some(notice) => format!("{base_meta} | {notice}"),
        None => base_meta,
    };
    let md_btn_lbl = if md_open { "Hide MD" } else { "Export MD" };
    let pill = |label: &str, active: bool| {
        s.compact_control(
            min_w_dims(badge_min_w(label) as f32, 24.0),
            4,
            s.m.pill_radius,
            Some(active),
        )
    };
    let header_card = s.card_surface(8, 6, s.t.surface.base, s.m.button_radius);
    let composer_card = s
        .card_surface(8, 5, s.t.surface.base, s.m.button_radius)
        .flex_row()
        .align_items(FlexAlign::Center);
    let tab_bar_dir = if compact {
        FlexDirection::Column
    } else {
        FlexDirection::Row
    };
    let block_count = blocks.len();

    rsx! {
        Card {
            variant: CardVariant::Outlined,
            test_id: Some("docs-header".into()),
            padding: Some(8.0),
            gap: Some(6.0),
            nve_style: Some(header_card),
            view {
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_shrink(0.0)
                    .flex_direction(tab_bar_dir)
                    .justify_content(FlexJustify::SpaceBetween)
                    .align_items(FlexAlign::Center)
                    .gap(6.0)
                    .outline(s.row()),
                view {
                    nve_style: TypedStyle::new().flex_1().width_pct(100.0),
                    TabsRoot {
                        value: active_id.map(|id| id.to_string()),
                        id_prefix: Some("docs-tabs".into()),
                        nve_style: Some(TypedStyle::new().width_pct(100.0).gap(0.0)),
                        on_change: move |next: String| {
                            if let Ok(doc_id) = next.parse::<u32>() {
                                store.update(|st| st.select_doc(doc_id));
                            }
                        },
                        TabList {
                            accessibility_label: Some("Workspace documents".into()),
                            nve_style: Some(
                                row_center(5.0)
                                    .padding(0.0)
                                    .bg(ColorRgba::TRANSPARENT)
                                    .border(0.0, ColorRgba::TRANSPARENT)
                                    .radius(0.0),
                            ),
                            for (doc_id, title) in tabs {
                                TabTrigger {
                                    key: "doc-tab-{doc_id}",
                                    value: doc_id.to_string(),
                                    test_id: Some(format!("doc-tab-{doc_id}")),
                                    nve_style: Some(pill(&title, active_id == Some(doc_id))),
                                    "{title}"
                                }
                            }
                        }
                    }
                }
                view {
                    nve_style: row_center(5.0).flex_shrink(0.0),
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("doc-new-btn".into()),
                        nve_style: Some(pill("+ Doc", true)),
                        on_press: move |_| store.update(|st| {
                            let _ = st.create_new_doc();
                        }),
                        "+ Doc"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("doc-export-md".into()),
                        nve_style: Some(pill(md_btn_lbl, md_open)),
                        on_press: move |_| store.update(|st| st.toggle_doc_markdown_preview()),
                        "{md_btn_lbl}"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("doc-copy-md".into()),
                        accessibility_label: Some("Copy active document Markdown to clipboard".into()),
                        nve_style: Some(pill("Copy MD", false)),
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
            text { nve_style: text_style(s.t.text.tertiary, 10.0), "{meta_summary}" }
        }

        if md_open {
            Card {
                variant: CardVariant::Outlined,
                test_id: Some("doc-markdown-preview".into()),
                padding: Some(8.0),
                gap: Some(4.0),
                nve_style: Some(
                    TypedStyle::new()
                        .width_pct(100.0)
                        .flex_shrink(0.0)
                        .padding(8.0)
                        .gap(4.0)
                        .bg(s.t.surface.sunken)
                        .radius(s.m.button_radius as f32)
                        .outline(s.control(true)),
                ),
                text { nve_style: text_style(s.t.text.link, 9.0), "MARKDOWN EXPORT" }
                text { nve_style: text_style(s.t.text.secondary, 10.0), "{md_preview}" }
            }
        }

        scrollview {
            test_id: "docs-blocks-list",
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_1()
                .flex_col()
                .gap(s.m.list_gap as f32)
                .outline(s.row()),
            for (idx, block) in blocks.into_iter().enumerate() {
                DocBlockCard {
                    key: "doc-blk-{block.id}",
                    can_up: idx > 0,
                    can_down: idx + 1 < block_count,
                    editing: editing == Some(block.id),
                    block,
                }
            }
        }

        DocComposer { card_style: composer_card }
    }
}

/// New-block composer: the only docs part subscribed to `doc_drafts` (besides the open editor).
#[component]
fn DocComposer(card_style: TypedStyle) -> Element {
    nve_dioxus::render_probe::record_render("DocComposer");
    let store = use_studio();
    let s = use_studio_style();
    let drafts = store.doc_drafts.read();
    let (new_kind_lbl, new_draft) = (
        drafts.doc_new_block_kind.label(),
        drafts.doc_new_block_draft.clone(),
    );
    drop(drafts);
    rsx! {
        Card {
            variant: CardVariant::Outlined,
            test_id: Some("docs-composer".into()),
            padding: Some(8.0),
            gap: Some(5.0),
            nve_style: Some(card_style),
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("doc-new-block-kind".into()),
                nve_style: Some(s.compact_control(fixed_dims(54.0, 34.0), 2, s.m.button_radius, Some(false))),
                on_press: move |_| store.update(|st| st.cycle_doc_new_block_kind()),
                "{new_kind_lbl}"
            }
            view {
                nve_style: TypedStyle::new().flex_1(),
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
                nve_style: Some(s.compact_control(fixed_dims(58.0, 34.0), 2, s.m.button_radius, None)),
                on_press: move |_| store.update(|st| {
                    let _ = st.submit_new_doc_block_draft();
                }),
                "+ Block"
            }
        }
    }
}

/// In-place editor of block `bid`: the only docs part subscribed to `doc_edit_draft`.
#[component]
fn DocBlockEditor(bid: u32) -> Element {
    nve_dioxus::render_probe::record_render("DocBlockEditor");
    let store = use_studio();
    let s = use_studio_style();
    let draft = store.doc_edit_draft.read().text.clone();
    let field_btn = |w: f32, paint: Option<bool>| {
        s.compact_control(fixed_dims(w, 34.0), 2, s.m.button_radius, paint)
    };
    rsx! {
        view {
            nve_style: row_center(5.0).width_pct(100.0).flex_shrink(0.0),
            view {
                nve_style: TypedStyle::new().flex_1(),
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
                nve_style: Some(field_btn(46.0, None)),
                on_press: move |_| store.update(|st| {
                    let _ = st.commit_doc_block_edit();
                }),
                "Save"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some(format!("doc-block-cancel-{bid}")),
                nve_style: Some(field_btn(50.0, Some(false))),
                on_press: move |_| store.update(|st| st.cancel_edit_doc_block()),
                "Cancel"
            }
        }
    }
}

/// One document block with its micro-toolbar; `editing` while its in-place editor is open.
#[component]
fn DocBlockCard(block: DocBlock, can_up: bool, can_down: bool, editing: bool) -> Element {
    nve_dioxus::render_probe::record_render("DocBlockCard");
    let store = use_studio();
    let s = use_studio_style();
    let bid = block.id;
    let (bg, fg, border) = block_colors(&s.t, block.kind);
    let border = if s.xray() {
        s.card()
    } else {
        Outline {
            width: 1,
            color: border,
        }
    };
    let font_sz = block_font_size(block.kind);
    let kind_lbl = block.kind.label();
    let card_style = TypedStyle::new()
        .width_pct(100.0)
        .flex_shrink(0.0)
        .padding(s.m.card_pad as f32)
        .gap(s.m.card_gap as f32)
        .bg(bg)
        .radius(s.m.card_radius as f32)
        .outline(border);
    let radius_sm = s.t.radius.sm as u32;
    let tool = |dims: TypedStyle, enabled: bool| {
        let style = s.compact_control(dims, 0, radius_sm, Some(false));
        if enabled { style } else { style.opacity(0.35) }
    };

    rsx! {
        Card {
            variant: CardVariant::Elevated,
            test_id: Some(format!("doc-block-{bid}")),
            padding: Some(s.m.card_pad as f32),
            gap: Some(s.m.card_gap as f32),
            nve_style: Some(card_style),
            view {
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_shrink(0.0)
                    .flex_row()
                    .justify_content(FlexJustify::SpaceBetween)
                    .align_items(FlexAlign::Center)
                    .gap(4.0),
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some(format!("doc-block-kind-{bid}")),
                    nve_style: Some(s.compact_control(
                        min_w_dims(badge_min_w(kind_lbl) as f32, 18.0),
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
                    nve_style: row_center(4.0).flex_shrink(0.0),
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-block-up-{bid}")),
                        nve_style: Some(tool(fixed_dims(22.0, 18.0), can_up)),
                        on_press: move |_| store.update(|st| {
                            let _ = st.move_doc_block(bid, -1);
                        }),
                        "^"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-block-down-{bid}")),
                        nve_style: Some(tool(fixed_dims(22.0, 18.0), can_down)),
                        on_press: move |_| store.update(|st| {
                            let _ = st.move_doc_block(bid, 1);
                        }),
                        "v"
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-block-edit-{bid}")),
                        nve_style: Some(tool(min_w_dims(34.0, 18.0), true)),
                        on_press: move |_| store.update(|st| {
                            let _ = st.begin_edit_doc_block(bid);
                        }),
                        "Edit"
                    }
                    Button {
                        variant: ButtonVariant::Destructive,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-block-del-{bid}")),
                        nve_style: Some(s.compact_control(fixed_dims(22.0, 18.0), 0, radius_sm, None)),
                        on_press: move |_| store.update(|st| {
                            let _ = st.delete_doc_block(bid);
                        }),
                        "x"
                    }
                }
            }

            if editing {
                DocBlockEditor { bid }
            } else if let DocBlockKind::Checklist { checked } = block.kind {
                view {
                    nve_style: row_center(8.0).width_pct(100.0).flex_shrink(0.0),
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some(format!("doc-check-{bid}")),
                        accessibility_label: Some("Toggle checklist item".into()),
                        nve_style: Some(
                            s.compact_control(fixed_dims(30.0, 22.0), 0, s.m.pill_radius, None)
                                .bg(if checked {
                                    s.t.status.success.subtle
                                } else {
                                    s.t.surface.subtle
                                })
                                .border(
                                    1.0,
                                    if checked {
                                        s.t.status.success.border
                                    } else {
                                        s.t.border.default
                                    },
                                ),
                        ),
                        on_press: move |_| store.update(|st| {
                            let _ = st.toggle_doc_checklist(bid);
                        }),
                        if checked { "[x]" } else { "[ ]" }
                    }
                    text { nve_style: text_style(fg, font_sz).flex_1(), "{block.text}" }
                }
            } else {
                text { nve_style: text_style(fg, font_sz), "{block.text}" }
            }
        }
    }
}
