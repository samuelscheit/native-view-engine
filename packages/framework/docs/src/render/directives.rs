//! Fenced `:::` directives (`callout`, `tabs`, `steps`, `cards`, `demo`, and custom directives).

use dioxus_core_macro::Props;
use crate::theme::callout_tone;
use nve_ui::accent_indicator;
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
                    nve_style: TypedStyle::new()
                        .width_pct(100.0)
                        .flex_col()
                        .gap(8.0)
                        .padding(14.0)
                        .radius(8.0)
                        .border(1.0, theme.border.default)
                        .bg(theme.surface.base),
                    text {
                        nve_style: TypedStyle::new().font_size(12.0).font_weight(700).color(theme.text.secondary),
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
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_row()
                .gap(12.0)
                .padding(14.0)
                .radius(10.0)
                .bg(tone.colors.subtle)
                .border(1.0, tone.colors.border),
            view {
                nve_style: TypedStyle::new()
                    .width_px(3.0)
                    .flex_shrink(0.0)
                    .radius(2.0)
                    .bg(tone.colors.border),
            }
            view {
                nve_style: TypedStyle::new().flex_grow(1.0).min_width_px(0.0).flex_col().gap(8.0),
                view {
                    nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(8.0),
                    view {
                        nve_style: TypedStyle::new()
                            .padding_xy(6.0, 1.0)
                            .radius(4.0)
                            .border(1.0, tone.colors.border),
                        text {
                            nve_style: TypedStyle::new().font_size(11.0).font_weight(700).color(tone.colors.text),
                            "{tone.icon}"
                        }
                    }
                    text {
                        nve_style: TypedStyle::new().font_size(13.0).font_weight(700).color(tone.colors.text),
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
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .min_width_px(0.0)
                .flex_col()
                .radius(10.0)
                .border(1.0, theme.border.default)
                .bg(theme.surface.base),

            view {
                accessibility_role: "tablist",
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_row()
                    .flex_wrap(FlexWrap::Wrap)
                    .align_items(FlexAlign::Center)
                    .gap(4.0)
                    .padding(6.0)
                    .bg(theme.surface.subtle)
                    .border(1.0, theme.border.default),
                for (idx, item) in items.iter().enumerate() {
                    {
                        let is_selected = item.label == active_label;
                        let bg = if is_selected { theme.status.accent.subtle } else { ColorRgba::TRANSPARENT };
                        let fg = if is_selected { theme.text.link } else { theme.text.secondary };
                        let border_c = if is_selected { accent_indicator(&theme) } else { ColorRgba::TRANSPARENT };
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
                                nve_style: TypedStyle::new()
                                    .padding_xy(10.0, 6.0)
                                    .radius(6.0)
                                    .bg(bg)
                                    .border(1.0, border_c),
                                onpress: move |_| {
                                    state.write().set_active_tab(group_for_press.clone(), label_for_press.clone());
                                },
                                text {
                                    nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(fg),
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
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .min_width_px(0.0)
                    .flex_col()
                    .padding(14.0)
                    .gap(10.0),
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
            nve_style: TypedStyle::new().width_pct(100.0).min_width_px(0.0).flex_col().gap(14.0),
            for (idx, step) in items.into_iter().enumerate() {
                {
                    let step_no = start + idx;
                    rsx! {
                        view {
                            key: "{scope_key}-step-{idx}",
                            nve_style: TypedStyle::new()
                                .width_pct(100.0)
                                .min_width_px(0.0)
                                .flex_row()
                                .align_items(FlexAlign::FlexStart)
                                .gap(12.0),
                            view {
                                nve_style: TypedStyle::new()
                                    .width_px(26.0)
                                    .height_px(26.0)
                                    .radius(999.0)
                                    .justify_content(FlexJustify::Center)
                                    .align_items(FlexAlign::Center)
                                    .bg(theme.status.accent.subtle)
                                    .border(1.0, theme.status.accent.border)
                                    .flex_shrink(0.0),
                                text {
                                    nve_style: TypedStyle::new().font_size(12.0).font_weight(700).color(theme.text.link),
                                    "{step_no}"
                                }
                            }
                            view {
                                nve_style: TypedStyle::new()
                                    .flex_grow(1.0)
                                    .min_width_px(0.0)
                                    .flex_col()
                                    .gap(8.0)
                                    .padding_y(2.0),
                                text {
                                    nve_style: TypedStyle::new().font_size(15.0).font_weight(700).color(theme.text.primary),
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

    let dir = if is_mobile { FlexDirection::Column } else { FlexDirection::Row };

    rsx! {
        view {
            test_id: "docs-cards-{scope_key}",
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_direction(dir)
                .gap(12.0)
                .flex_wrap(FlexWrap::Wrap),
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
                            nve_style: TypedStyle::new()
                                .flex_grow(1.0)
                                .min_width_px(220.0)
                                .flex_col()
                                .gap(6.0)
                                .padding(14.0)
                                .radius(10.0)
                                .bg(theme.surface.base)
                                .border(1.0, theme.border.default),
                            onpress: move |_| {
                                if let Some((route, anchor)) = target.clone() {
                                    navigate_to_target(state, route, anchor);
                                }
                            },
                            view {
                                nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(8.0),
                                if let Some(ref icon_txt) = card.icon {
                                    text {
                                        nve_style: TypedStyle::new().font_size(14.0).color(theme.text.link),
                                        "{icon_txt}"
                                    }
                                }
                                text {
                                    nve_style: TypedStyle::new().font_size(15.0).font_weight(700).color(theme.text.primary),
                                    "{card.title}"
                                }
                            }
                            if let Some(ref desc) = card.description {
                                text {
                                    nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary),
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
