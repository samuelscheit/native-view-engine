//! Fenced `:::` directives (`callout`, `tabs`, `steps`, `cards`, `demo`, and custom directives).

use dioxus_core_macro::Props;
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
                    style: "width: 100%; flex-direction: column; gap: 8px; padding: 14px; border-radius: 8px; border-width: 1px; border-color: {theme.border}; background-color: {theme.surface};",
                    text {
                        style: "font-size: 12px; font-weight: 700; color: {theme.muted};",
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
    let tone = theme.callout_tone(kind);
    let heading = title.unwrap_or_else(|| tone.badge_text.to_string());

    rsx! {
        view {
            test_id: "docs-callout-{scope_key}",
            accessibility_role: "note",
            "data-callout-kind": "{kind.as_str()}",
            style: "width: 100%; flex-direction: row; gap: 12px; padding: 14px; border-radius: 10px; background-color: {tone.bg}; border-width: 1px; border-color: {tone.border};",
            view {
                style: "width: 3px; border-radius: 2px; background-color: {tone.border};",
            }
            view {
                style: "flex-grow: 1; flex-direction: column; gap: 8px;",
                view {
                    style: "flex-direction: row; align-items: center; gap: 8px;",
                    view {
                        style: "padding-horizontal: 6px; padding-vertical: 1px; border-radius: 4px; border-width: 1px; border-color: {tone.border};",
                        text {
                            style: "font-size: 11px; font-weight: 700; color: {tone.title};",
                            "{tone.icon}"
                        }
                    }
                    text {
                        style: "font-size: 13px; font-weight: 700; color: {tone.title};",
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
            style: "width: 100%; flex-direction: column; border-radius: 10px; border-width: 1px; border-color: {theme.border}; background-color: {theme.surface};",

            view {
                accessibility_role: "tablist",
                style: "width: 100%; flex-direction: row; align-items: center; gap: 4px; padding: 6px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                for (idx, item) in items.iter().enumerate() {
                    {
                        let is_selected = item.label == active_label;
                        let bg = if is_selected { theme.accent_surface } else { "transparent" };
                        let fg = if is_selected { theme.accent } else { theme.muted };
                        let border_c = if is_selected { theme.accent } else { "transparent" };
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
                                "data-tab-label": "{label_tid}",
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
                style: "width: 100%; flex-direction: column; padding: 14px; gap: 10px;",
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
            style: "width: 100%; flex-direction: column; gap: 14px;",
            for (idx, step) in items.into_iter().enumerate() {
                {
                    let step_no = start + idx;
                    rsx! {
                        view {
                            key: "{scope_key}-step-{idx}",
                            style: "width: 100%; flex-direction: row; align-items: flex-start; gap: 12px;",
                            view {
                                style: "width: 26px; height: 26px; border-radius: 999px; justify-content: center; align-items: center; background-color: {theme.accent_surface}; border-width: 1px; border-color: {theme.accent}; flex-shrink: 0;",
                                text {
                                    style: "font-size: 12px; font-weight: 700; color: {theme.accent};",
                                    "{step_no}"
                                }
                            }
                            view {
                                style: "flex-grow: 1; flex-direction: column; gap: 8px; padding-vertical: 2px;",
                                text {
                                    style: "font-size: 15px; font-weight: 700; color: {theme.text};",
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
    let base_path = snap.config.base_path.clone();
    drop(snap);

    let dir = if is_mobile { "column" } else { "row" };

    rsx! {
        view {
            test_id: "docs-cards-{scope_key}",
            style: "width: 100%; flex-direction: {dir}; gap: 12px; flex-wrap: wrap;",
            for (idx, card) in items.into_iter().enumerate() {
                {
                    let target = card.href.as_deref().and_then(DocsRoute::parse_link_target);
                    let resolved_href = card.href.as_deref().map(|h| match DocsRoute::parse_link_target(h) {
                        Some((Some(r), Some(a))) => format!("{}#{a}", r.href(&base_path)),
                        Some((Some(r), None)) => r.href(&base_path),
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
                            style: "flex-grow: 1; min-width: 220px; flex-direction: column; gap: 6px; padding: 14px; border-radius: 10px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                            onpress: move |_| {
                                if let Some((route, anchor)) = target.clone() {
                                    navigate_to_target(state, route, anchor);
                                }
                            },
                            view {
                                style: "flex-direction: row; align-items: center; gap: 8px;",
                                if let Some(ref icon_txt) = card.icon {
                                    text {
                                        style: "font-size: 14px; color: {theme.accent};",
                                        "{icon_txt}"
                                    }
                                }
                                text {
                                    style: "font-size: 15px; font-weight: 700; color: {theme.text};",
                                    "{card.title}"
                                }
                            }
                            if let Some(ref desc) = card.description {
                                text {
                                    style: "font-size: 13px; color: {theme.muted};",
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
