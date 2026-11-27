//! Fenced `:::` directives (`callout`, `tabs`, `steps`, `cards`, `demo`, and custom directives).

use dioxus_core_macro::Props;
use crate::theme::callout_tone;
use nve_ui::{ColorHex, Hex};
use nve_dioxus::prelude::*;
use nve_mdx::{Block, CalloutKind, CardItem, DirectiveBlock, StepItem, TabItem};

use crate::render::blocks::BlocksView;
use crate::render::live_demo::LiveDemoView;
use crate::route::DocsRoute;
use crate::state::{navigate_to_target, use_docs_state};

/// Renders any fenced `:::` [`DirectiveBlock`].
#[component]
pub fn DirectiveBlockView(directive: DirectiveBlock, scope_key: String) -> Element {
    match directive {
        DirectiveBlock::Callout {
            kind,
            title,
            blocks,
            ..
        } => rsx! {
            CalloutView {
                kind: kind,
                title: title,
                blocks: blocks,
                scope_key: scope_key,
            }
        },
        DirectiveBlock::Tabs { items, .. } => rsx! {
            TabsDirectiveView {
                items: items,
                group_key: scope_key,
                default_label: None,
            }
        },
        DirectiveBlock::Steps { items, .. } => rsx! {
            StepsView {
                items: items,
                start: 1,
                scope_key: scope_key,
            }
        },
        DirectiveBlock::Cards { items, .. } => rsx! {
            CardsGridView {
                items: items,
                scope_key: scope_key,
            }
        },
        DirectiveBlock::Demo {
            id,
            hydrate,
            height,
            title,
            blocks,
            ..
        } => rsx! {
            LiveDemoView {
                id: id,
                hydrate: hydrate,
                height: height,
                title: title,
                code_override: None,
                fallback_blocks: blocks,
                scope_key: scope_key,
            }
        },
        DirectiveBlock::Custom { name, blocks, .. } => {
            let state = use_docs_state();
            let theme = state.read().theme();
            rsx! {
                view {
                    style: "width: 100%; flex-direction: column; gap: 8px; padding: 14px; border-radius: 8px; border-width: 1px; border-color: {theme.border.default.hex()}; background-color: {theme.surface.base.hex()};",
                    text {
                        style: "font-size: 12px; font-weight: 700; color: {theme.text.secondary.hex()};",
                        "{name}"
                    }
                    BlocksView {
                        blocks: blocks,
                        scope_prefix: format!("{scope_key}-custom"),
                    }
                }
            }
        }
    }
}

/// Shared callout component used by both `:::callout` directives and `<Callout>` MDX tags.
#[component]
pub fn CalloutView(
    kind: CalloutKind,
    title: Option<String>,
    blocks: Vec<Block>,
    scope_key: String,
) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();
    let tone = callout_tone(&theme, kind);
    let heading = title.unwrap_or_else(|| tone.label.to_string());

    rsx! {
        view {
            test_id: "docs-callout-{scope_key}",
            accessibility_role: "note",
            nve_table: {
                let kind_str = kind.as_str().to_owned();
                nve_dioxus::table_attr_fn(move |tx, node| {
                    tx.edit::<nve_web_html::HtmlDataAttrs>(node).entries(Some(
                        nve_web_html::DataAttrList::from_pairs(vec![(
                            "data-callout-kind".to_owned(),
                            kind_str.clone(),
                        )]),
                    ));
                })
            },
            style: "width: 100%; flex-direction: row; gap: 12px; padding: 14px; border-radius: 10px; background-color: {tone.colors.subtle.hex()}; border-width: 1px; border-color: {tone.colors.border.hex()};",
            view {
                style: "width: 3px; flex-shrink: 0; border-radius: 2px; background-color: {tone.colors.border.hex()};",
            }
            view {
                style: "flex-grow: 1; min-width: 0px; flex-direction: column; gap: 8px;",
                view {
                    style: "flex-direction: row; align-items: center; gap: 8px;",
                    view {
                        style: "padding-horizontal: 6px; padding-vertical: 1px; border-radius: 4px; border-width: 1px; border-color: {tone.colors.border.hex()};",
                        text {
                            style: "font-size: 11px; font-weight: 700; color: {tone.colors.text.hex()};",
                            "{tone.icon}"
                        }
                    }
                    text {
                        style: "font-size: 13px; font-weight: 700; color: {tone.colors.text.hex()};",
                        "{heading}"
                    }
                }
                BlocksView {
                    blocks: blocks,
                    scope_prefix: format!("{scope_key}-callout"),
                }
            }
        }
    }
}

/// Interactive tabbed panel shared by `:::tabs`, `<Tabs>`, and `<CodeGroup>`.
#[component]
pub fn TabsDirectiveView(
    items: Vec<TabItem>,
    group_key: String,
    default_label: Option<String>,
) -> Element {
    let mut state = use_docs_state();
    if items.is_empty() {
        return rsx! {};
    }

    let fallback_label = default_label
        .filter(|d| items.iter().any(|it| it.label == *d))
        .unwrap_or_else(|| items[0].label.clone());
    let snap = state.read();
    let theme = snap.theme();
    let active_label = snap.active_tab(&group_key, &fallback_label);
    drop(snap);

    let active_blocks = items
        .iter()
        .find(|it| it.label == active_label)
        .or_else(|| items.first())
        .map(|it| it.blocks.clone())
        .unwrap_or_default();

    let clean_key = sanitize_tid(&group_key);

    rsx! {
        view {
            test_id: "docs-tabs-{clean_key}",
            style: "width: 100%; min-width: 0px; flex-direction: column; border-radius: 10px; border-width: 1px; border-color: {theme.border.default.hex()}; background-color: {theme.surface.base.hex()};",

            view {
                accessibility_role: "tablist",
                style: "width: 100%; flex-direction: row; flex-wrap: wrap; align-items: center; gap: 4px; padding: 6px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                for (idx, item) in items.iter().enumerate() {
                    {
                        let is_selected = item.label == active_label;
                        let bg = if is_selected { theme.status.accent.subtle.hex() } else { Hex::TRANSPARENT };
                        let fg = if is_selected { theme.text.link.hex() } else { theme.text.secondary.hex() };
                        let border_c = if is_selected { theme.status.accent.solid.hex() } else { Hex::TRANSPARENT };
                        let sel_attr = if is_selected { "true" } else { "false" };
                        let label = item.label.clone();
                        let label_for_press = label.clone();
                        let group_for_press = group_key.clone();
                        let trigger_tid = format!("docs-tab-{clean_key}-{idx}");
                        let label_tid = format!("docs-tab-label-{}", sanitize_tid(&label));

                        rsx! {
                            pressable {
                                key: "{clean_key}-tab-{idx}",
                                test_id: "{trigger_tid}",
                                accessibility_role: "tab",
                                aria_selected: "{sel_attr}",
                                nve_table: {
                                    let lbl = label_tid.clone();
                                    nve_dioxus::table_attr_fn(move |tx, node| {
                                        tx.edit::<nve_web_html::HtmlDataAttrs>(node).entries(Some(
                                            nve_web_html::DataAttrList::from_pairs(vec![(
                                                "data-tab-label".to_owned(),
                                                lbl.clone(),
                                            )]),
                                        ));
                                    })
                                },
                                style: "padding-horizontal: 10px; padding-vertical: 6px; border-radius: 6px; background-color: {bg}; border-width: 1px; border-color: {border_c};",
                                onpress: move |_| {
                                    state.write().set_active_tab(group_for_press.clone(), label_for_press.clone());
                                },
                                text {
                                    style: "font-size: 13px; font-weight: 600; color: {fg};",
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }

            view {
                test_id: "docs-tab-panel-{clean_key}",
                accessibility_role: "tabpanel",
                style: "width: 100%; min-width: 0px; flex-direction: column; padding: 14px; gap: 10px;",
                BlocksView {
                    blocks: active_blocks,
                    scope_prefix: format!("{clean_key}-panel-{active_label}"),
                }
            }
        }
    }
}

/// Numbered vertical steps shared by `:::steps` and `<Steps>`.
#[component]
pub fn StepsView(items: Vec<StepItem>, start: usize, scope_key: String) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();

    rsx! {
        view {
            test_id: "docs-steps-{scope_key}",
            style: "width: 100%; min-width: 0px; flex-direction: column; gap: 14px;",
            for (idx, step) in items.into_iter().enumerate() {
                {
                    let step_no = start + idx;
                    rsx! {
                        view {
                            key: "{scope_key}-step-{idx}",
                            style: "width: 100%; min-width: 0px; flex-direction: row; align-items: flex-start; gap: 12px;",
                            view {
                                style: "width: 26px; height: 26px; border-radius: 999px; justify-content: center; align-items: center; background-color: {theme.status.accent.subtle.hex()}; border-width: 1px; border-color: {theme.status.accent.border.hex()}; flex-shrink: 0;",
                                text {
                                    style: "font-size: 12px; font-weight: 700; color: {theme.text.link.hex()};",
                                    "{step_no}"
                                }
                            }
                            view {
                                style: "flex-grow: 1; min-width: 0px; flex-direction: column; gap: 8px; padding-vertical: 2px;",
                                text {
                                    style: "font-size: 15px; font-weight: 700; color: {theme.text.primary.hex()};",
                                    "{step.title}"
                                }
                                BlocksView {
                                    blocks: step.blocks,
                                    scope_prefix: format!("{scope_key}-step-{idx}"),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Responsive cards grid shared by `:::cards`, `<Cards>`, and `<Card>`.
#[component]
pub fn CardsGridView(items: Vec<CardItem>, scope_key: String) -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let is_mobile = snap.viewport.is_mobile();
    let config = snap.config.clone();
    drop(snap);

    let dir = if is_mobile { "column" } else { "row" };

    rsx! {
        view {
            test_id: "docs-cards-{scope_key}",
            style: "width: 100%; flex-direction: {dir}; gap: 12px; flex-wrap: wrap;",
            for (idx, card) in items.into_iter().enumerate() {
                {
                    let target = card.href.as_deref().and_then(|h| {
                        DocsRoute::parse_link_target(h).map(|(r, a)| {
                            (r.map(|rt| config.docs_route_for_content(rt.as_str())), a)
                        })
                    });
                    let resolved_href = card.href.as_deref().map(|h| match &target {
                        Some((Some(r), Some(a))) => {
                            format!("{}#{a}", config.resolve_href(r.as_str()))
                        }
                        Some((Some(r), None)) => config.resolve_href(r.as_str()),
                        Some((None, Some(a))) => format!("#{a}"),
                        _ => h.to_string(),
                    });
                    let has_href = resolved_href.is_some();

                    rsx! {
                        pressable {
                            key: "{scope_key}-card-{idx}",
                            test_id: "docs-card-{scope_key}-{idx}",
                            accessibility_role: if has_href { "link" } else { "region" },
                            "href": resolved_href,
                            style: "flex-grow: 1; min-width: 220px; flex-direction: column; gap: 6px; padding: 14px; border-radius: 10px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                            onpress: move |_| {
                                if let Some((route, anchor)) = target.clone() {
                                    navigate_to_target(state, route, anchor);
                                }
                            },
                            view {
                                style: "flex-direction: row; align-items: center; gap: 8px;",
                                if let Some(ref icon_txt) = card.icon {
                                    text {
                                        style: "font-size: 14px; color: {theme.text.link.hex()};",
                                        "{icon_txt}"
                                    }
                                }
                                text {
                                    style: "font-size: 15px; font-weight: 700; color: {theme.text.primary.hex()};",
                                    "{card.title}"
                                }
                            }
                            if let Some(ref desc) = card.description {
                                text {
                                    style: "font-size: 13px; color: {theme.text.secondary.hex()};",
                                    "{desc}"
                                }
                            }
                            if !card.blocks.is_empty() {
                                BlocksView {
                                    blocks: card.blocks,
                                    scope_prefix: format!("{scope_key}-card-{idx}"),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn sanitize_tid(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}
