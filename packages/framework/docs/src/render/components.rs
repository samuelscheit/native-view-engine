//! Declarative MDX `<Component ... />` block and inline renderers (`Callout`, `Tabs`, `Steps`,
//! `Cards`, `Card`, `LiveDemo`, `Badge`, `Kbd`, `ApiTable`, `CodeGroup`).

use dioxus_core_macro::Props;
use nve_ui::ColorHex;
use nve_dioxus::prelude::*;
use nve_mdx::{
    Block, CalloutKind, CardItem, ComponentNode, HydrateStrategy, PropValue, StepItem, TabItem,
};

use crate::render::blocks::BlocksView;
use crate::render::directives::{CalloutView, CardsGridView, StepsView, TabsDirectiveView};
use crate::render::live_demo::LiveDemoView;
use crate::state::use_docs_state;

/// Renders a block-level declarative `<Component ... />` MDX tag.
#[component]
pub fn ComponentBlockView(comp: ComponentNode, scope_key: String) -> Element {
    match comp.name.as_str() {
        "Callout" => {
            let kind = prop_str(&comp, "kind")
                .map(CalloutKind::parse)
                .unwrap_or(CalloutKind::Info);
            let title = prop_str(&comp, "title").map(str::to_string);
            rsx! {
                CalloutView {
                    kind: kind,
                    title: title,
                    blocks: comp.children,
                    scope_key: scope_key,
                }
            }
        }
        "Tabs" => {
            let group = prop_str(&comp, "id")
                .or_else(|| prop_str(&comp, "group"))
                .map(str::to_string)
                .unwrap_or_else(|| scope_key.clone());
            let default_label = prop_str(&comp, "default").map(str::to_string);
            let items: Vec<TabItem> = comp
                .children
                .into_iter()
                .filter_map(|child| match child {
                    Block::Component(tab) if tab.name == "Tab" => {
                        let label = prop_str(&tab, "label")
                            .or_else(|| prop_str(&tab, "value"))
                            .unwrap_or("Tab")
                            .to_string();
                        Some(TabItem {
                            label,
                            blocks: tab.children,
                            line: tab.pos.line,
                        })
                    }
                    _ => None,
                })
                .collect();
            rsx! {
                TabsDirectiveView {
                    items: items,
                    group_key: group,
                    default_label: default_label,
                }
            }
        }
        "Steps" => {
            let start = prop_int(&comp, "start")
                .and_then(|v| usize::try_from(v).ok())
                .unwrap_or(1);
            let items: Vec<StepItem> = comp
                .children
                .into_iter()
                .filter_map(|child| match child {
                    Block::Component(step) if step.name == "Step" => Some(StepItem {
                        title: prop_str(&step, "title").unwrap_or("Step").to_string(),
                        blocks: step.children,
                        line: step.pos.line,
                    }),
                    _ => None,
                })
                .collect();
            rsx! {
                StepsView {
                    items: items,
                    start: start,
                    scope_key: scope_key,
                }
            }
        }
        "Cards" => {
            let items: Vec<CardItem> = comp
                .children
                .into_iter()
                .filter_map(|child| match child {
                    Block::Component(card) if card.name == "Card" => Some(CardItem {
                        title: prop_str(&card, "title").unwrap_or("Card").to_string(),
                        href: prop_str(&card, "href").map(str::to_string),
                        description: prop_str(&card, "description").map(str::to_string),
                        icon: prop_str(&card, "icon").map(str::to_string),
                        blocks: card.children,
                        line: card.pos.line,
                    }),
                    _ => None,
                })
                .collect();
            rsx! {
                CardsGridView {
                    items: items,
                    scope_key: scope_key,
                }
            }
        }
        "Card" => {
            let item = CardItem {
                title: prop_str(&comp, "title").unwrap_or("Card").to_string(),
                href: prop_str(&comp, "href").map(str::to_string),
                description: prop_str(&comp, "description").map(str::to_string),
                icon: prop_str(&comp, "icon").map(str::to_string),
                blocks: comp.children,
                line: comp.pos.line,
            };
            rsx! {
                CardsGridView {
                    items: vec![item],
                    scope_key: scope_key,
                }
            }
        }
        "LiveDemo" | "Demo" => {
            let id = prop_str(&comp, "id").unwrap_or("demo").to_string();
            let title = prop_str(&comp, "title").map(str::to_string);
            let hydrate = prop_str(&comp, "hydrate")
                .map(HydrateStrategy::parse)
                .unwrap_or(HydrateStrategy::Visible);
            let height = prop_int(&comp, "height").and_then(|v| u32::try_from(v).ok());
            let code_override = prop_str(&comp, "code").map(str::to_string);
            rsx! {
                LiveDemoView {
                    id: id,
                    hydrate: hydrate,
                    height: height,
                    title: title,
                    code_override: code_override,
                    fallback_blocks: comp.children,
                    scope_key: scope_key,
                }
            }
        }
        "CodeGroup" => {
            let default_label = prop_str(&comp, "default").map(str::to_string);
            let items: Vec<TabItem> = comp
                .children
                .into_iter()
                .enumerate()
                .map(|(idx, child)| match child {
                    Block::CodeBlock(cb) => {
                        let label = cb
                            .title
                            .clone()
                            .or_else(|| cb.lang.clone())
                            .unwrap_or_else(|| format!("Tab {}", idx + 1));
                        let line = cb.line;
                        TabItem {
                            label,
                            blocks: vec![Block::CodeBlock(cb)],
                            line,
                        }
                    }
                    other => TabItem {
                        label: format!("Tab {}", idx + 1),
                        blocks: vec![other],
                        line: comp.pos.line,
                    },
                })
                .collect();
            rsx! {
                TabsDirectiveView {
                    items: items,
                    group_key: format!("{scope_key}-codegroup"),
                    default_label: default_label,
                }
            }
        }
        "ApiTable" => {
            let state = use_docs_state();
            let theme = state.read().theme();
            let title = prop_str(&comp, "title").map(str::to_string);
            let caption = prop_str(&comp, "caption").map(str::to_string);
            rsx! {
                view {
                    test_id: "docs-api-table-{scope_key}",
                    style: "width: 100%; min-width: 0px; flex-direction: column; gap: 10px; padding: 14px; border-radius: 10px; border-width: 1px; border-color: {theme.border.default.hex()}; background-color: {theme.surface.base.hex()};",
                    if let Some(t) = title {
                        text {
                            style: "font-size: 14px; font-weight: 700; color: {theme.text.primary.hex()};",
                            "{t}"
                        }
                    }
                    if let Some(c) = caption {
                        text {
                            style: "font-size: 12px; color: {theme.text.secondary.hex()};",
                            "{c}"
                        }
                    }
                    BlocksView {
                        blocks: comp.children,
                        scope_prefix: format!("{scope_key}-api"),
                    }
                }
            }
        }
        "Badge" | "Kbd" => rsx! {
            InlineComponentView { comp: comp }
        },
        _ => {
            let state = use_docs_state();
            let theme = state.read().theme();
            let tag_label = format!("<{}>", comp.name);
            rsx! {
                view {
                    style: "width: 100%; flex-direction: column; gap: 8px; padding: 12px; border-radius: 8px; border-width: 1px; border-color: {theme.border.default.hex()}; background-color: {theme.surface.base.hex()};",
                    text {
                        style: "font-size: 12px; font-weight: 600; color: {theme.text.secondary.hex()};",
                        "{tag_label}"
                    }
                    BlocksView {
                        blocks: comp.children,
                        scope_prefix: format!("{scope_key}-comp"),
                    }
                }
            }
        }
    }
}

/// Renders inline MDX components (`<Badge />`, `<Kbd />`).
#[component]
pub fn InlineComponentView(comp: ComponentNode) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();

    match comp.name.as_str() {
        "Badge" => {
            let label = prop_str(&comp, "label").unwrap_or("Badge").to_string();
            let tone = prop_str(&comp, "tone")
                .or_else(|| prop_str(&comp, "variant"))
                .unwrap_or("accent");
            let st = theme.status;
            let tinted = |t: nve_ui::Tone| (t.subtle.hex(), t.border.hex(), t.text.hex());
            let (bg, border, fg) = match tone {
                "success" | "tip" => tinted(st.success),
                "warning" => tinted(st.warning),
                "danger" | "error" => tinted(st.danger),
                "muted" | "neutral" => (theme.surface.subtle.hex(), theme.border.default.hex(), theme.text.secondary.hex()),
                _ => tinted(st.accent),
            };
            rsx! {
                view {
                    style: "display: inline-flex; align-items: center; vertical-align: baseline; padding-horizontal: 7px; padding-vertical: 2px; border-radius: 999px; background-color: {bg}; border-width: 1px; border-color: {border};",
                    text {
                        style: "display: inline; font-size: 11px; font-weight: 600; color: {fg};",
                        "{label}"
                    }
                }
            }
        }
        "Kbd" => {
            let keys = prop_str(&comp, "keys").unwrap_or("").to_string();
            rsx! {
                view {
                    nve_table: crate::render::html_semantic(nve_web_html::SemanticElement::Kbd),
                    style: "display: inline-flex; align-items: center; vertical-align: baseline; padding-horizontal: 6px; padding-vertical: 1px; border-radius: 4px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                    text {
                        style: "display: inline; font-size: 11px; font-weight: 600; font-family: monospace; color: {theme.text.primary.hex()};",
                        "{keys}"
                    }
                }
            }
        }
        _ => {
            let tag_label = format!("<{} />", comp.name);
            rsx! {
                text {
                    style: "display: inline; font-size: 13px; color: {theme.text.secondary.hex()};",
                    "{tag_label}"
                }
            }
        }
    }
}

fn prop_str<'a>(comp: &'a ComponentNode, name: &str) -> Option<&'a str> {
    match comp.prop(name)? {
        PropValue::String(s) => Some(s.as_str()),
        _ => None,
    }
}

fn prop_int(comp: &ComponentNode, name: &str) -> Option<i64> {
    match comp.prop(name)? {
        PropValue::Int(v) => Some(*v),
        _ => None,
    }
}
