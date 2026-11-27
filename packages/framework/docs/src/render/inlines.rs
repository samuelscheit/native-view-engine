//! Inline `nve-mdx` AST renderer (text, emphasis, strong, strikethrough, inline code, links, images, components).

use dioxus_core_macro::Props;
use nve_abi::ColorRgba;
use nve_ui::ColorHex;
use nve_dioxus::prelude::*;
use nve_mdx::Inline;

use nve_web_html::SemanticElement;

use crate::render::components::InlineComponentView;
use crate::render::html_semantic;
use crate::route::DocsRoute;
use crate::state::{navigate_to_target, use_docs_state};

/// Flatten inline nodes to plain text (used for accessible labels and link hints).
pub fn inlines_plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(s) | Inline::Code(s) => out.push_str(s),
            Inline::Strong(c) | Inline::Emphasis(c) | Inline::Strikethrough(c) => {
                out.push_str(&inlines_plain_text(c));
            }
            Inline::Link { text, .. } => out.push_str(&inlines_plain_text(text)),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::Autolink { url, .. } => out.push_str(url),
            Inline::HardBreak => out.push(' '),
            Inline::InlineComponent(comp) => out.push_str(&comp.name),
        }
    }
    out
}

/// Renders a slice of [`Inline`] AST nodes as true inline rich text on Web (and wrapping row on native).
#[component]
pub fn InlinesView(
    inlines: Vec<Inline>,
    #[props(default = 15)] font_size: u32,
    #[props(default)] bold: bool,
    #[props(default)] color_override: Option<ColorRgba>,
    #[props(default)] line_height: Option<String>,
) -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let base_path = snap.config.base_path.clone();
    drop(snap);

    let text_color = color_override.unwrap_or(theme.text.primary);
    let base_weight = if bold { "700" } else { "400" };
    let lh = line_height.unwrap_or_else(|| {
        if font_size >= 18 {
            "1.25".to_string()
        } else if font_size <= 13 {
            "1.45".to_string()
        } else {
            "1.65".to_string()
        }
    });

    rsx! {
        view {
            test_id: "docs-inlines",
            style: "display: block; width: 100%; min-width: 0px; flex-direction: row; align-items: center; flex-wrap: wrap; line-height: {lh}; overflow-wrap: break-word;",
            for (idx, inline) in inlines.into_iter().enumerate() {
                InlineNodeView {
                    key: "inl-{idx}",
                    inline: inline,
                    font_size: font_size,
                    base_weight: base_weight.to_string(),
                    text_color,
                    base_path: base_path.clone(),
                }
            }
        }
    }
}

#[component]
fn InlineNodeView(
    inline: Inline,
    font_size: u32,
    base_weight: String,
    text_color: ColorRgba,
    base_path: String,
) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();

    match inline {
        Inline::Text(content) => rsx! {
            text {
                style: "display: inline; font-size: {font_size}px; font-weight: {base_weight}; color: {text_color.hex()};",
                "{content}"
            }
        },
        Inline::Strong(children) => rsx! {
            text {
                nve_table: html_semantic(SemanticElement::Strong),
                style: "display: inline; font-size: {font_size}px; font-weight: 700; color: {text_color.hex()};",
                for (idx, child) in children.into_iter().enumerate() {
                    InlineNodeView {
                        key: "strong-{idx}",
                        inline: child,
                        font_size: font_size,
                        base_weight: "700".to_string(),
                        text_color,
                        base_path: base_path.clone(),
                    }
                }
            }
        },
        Inline::Emphasis(children) => rsx! {
            text {
                nve_table: html_semantic(SemanticElement::Em),
                style: "display: inline; font-size: {font_size}px; font-weight: {base_weight}; font-style: italic; color: {text_color.hex()};",
                for (idx, child) in children.into_iter().enumerate() {
                    InlineNodeView {
                        key: "em-{idx}",
                        inline: child,
                        font_size: font_size,
                        base_weight: base_weight.clone(),
                        text_color,
                        base_path: base_path.clone(),
                    }
                }
            }
        },
        Inline::Strikethrough(children) => rsx! {
            text {
                nve_table: html_semantic(SemanticElement::Strike),
                style: "display: inline; font-size: {font_size}px; font-weight: {base_weight}; text-decoration: line-through; color: {theme.text.secondary.hex()};",
                for (idx, child) in children.into_iter().enumerate() {
                    InlineNodeView {
                        key: "del-{idx}",
                        inline: child,
                        font_size: font_size,
                        base_weight: base_weight.clone(),
                        text_color: theme.text.secondary,
                        base_path: base_path.clone(),
                    }
                }
            }
        },
        Inline::Code(code) => {
            let code_size = font_size.saturating_sub(1).max(11);
            rsx! {
                text {
                    test_id: "docs-inline-code",
                    accessibility_role: "code",
                    style: "display: inline; font-size: {code_size}px; font-family: monospace; color: {theme.text.link.hex()}; padding-horizontal: 4px; padding-vertical: 1px; border-radius: 4px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                    "{code}"
                }
            }
        }
        Inline::Link {
            text: link_children,
            url,
            title,
            ..
        } => {
            let label = inlines_plain_text(&link_children);
            let display = if label.is_empty() { url.clone() } else { label };
            let has_children = !link_children.is_empty();
            let config = state.read().config.clone();
            let internal_target = DocsRoute::parse_link_target(&url).map(|(route, anchor)| {
                let mapped = route.map(|r| config.docs_route_for_content(r.as_str()));
                (mapped, anchor)
            });
            let href = match &internal_target {
                Some((Some(route), Some(anchor))) => {
                    format!("{}#{anchor}", config.resolve_href(route.as_str()))
                }
                Some((Some(route), None)) => config.resolve_href(route.as_str()),
                Some((None, Some(anchor))) => format!("#{anchor}"),
                _ => url.clone(),
            };
            let tid = internal_target.as_ref().and_then(|(route, _)| {
                route
                    .as_ref()
                    .map(|r| format!("docs-inline-link-{}", crate::shell::route_test_id_slug(r.as_str())))
            });

            rsx! {
                pressable {
                    test_id: tid,
                    accessibility_role: "link",
                    accessibility_label: title.unwrap_or_else(|| display.clone()),
                    "href": "{href}",
                    style: "display: inline; font-size: {font_size}px; font-weight: 500; color: {theme.text.link.hex()}; text-decoration: underline;",
                    onpress: move |_| {
                        if let Some((route, anchor)) = internal_target.clone() {
                            navigate_to_target(state, route, anchor);
                        }
                    },
                    if has_children {
                        for (idx, child) in link_children.into_iter().enumerate() {
                            InlineNodeView {
                                key: "link-{idx}",
                                inline: child,
                                font_size: font_size,
                                base_weight: "500".to_string(),
                                text_color: theme.text.link,
                                base_path: base_path.clone(),
                            }
                        }
                    } else {
                        text {
                            style: "display: inline; font-size: {font_size}px; font-weight: 500; color: {theme.text.link.hex()};",
                            "{display}"
                        }
                    }
                }
            }
        }
        Inline::Autolink { url, .. } => {
            let internal_target = DocsRoute::parse_link_target(&url);
            let href = url.clone();
            rsx! {
                pressable {
                    accessibility_role: "link",
                    "href": "{href}",
                    style: "display: inline; font-size: {font_size}px; color: {theme.text.link.hex()}; text-decoration: underline;",
                    onpress: move |_| {
                        if let Some((route, anchor)) = internal_target.clone() {
                            navigate_to_target(state, route, anchor);
                        }
                    },
                    text {
                        style: "display: inline; font-size: {font_size}px; color: {theme.text.link.hex()};",
                        "{url}"
                    }
                }
            }
        }
        Inline::Image { alt, url, title, .. } => {
            let label = if !alt.is_empty() {
                alt.clone()
            } else {
                title.unwrap_or_else(|| "Embedded image".to_string())
            };
            rsx! {
                image {
                    src: "{url}",
                    alt: "{alt}",
                    accessibility_label: "{label}",
                    resize_mode: "contain",
                    style: "max-width: 100%; border-radius: 8px; border-width: 1px; border-color: {theme.border.default.hex()};",
                }
            }
        }
        Inline::HardBreak => rsx! {
            view {
                nve_table: html_semantic(SemanticElement::Br),
                style: "width: 100%; height: 4px;",
            }
        },
        Inline::InlineComponent(comp) => rsx! {
            InlineComponentView { comp: comp }
        },
    }
}
