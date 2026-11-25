//! Inline `nve-mdx` AST renderer (text, emphasis, strong, strikethrough, inline code, links, images, components).

use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;
use nve_mdx::Inline;

use crate::render::components::InlineComponentView;
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

/// Renders a slice of [`Inline`] AST nodes in a wrapping horizontal row.
#[component]
pub fn InlinesView(
    inlines: Vec<Inline>,
    #[props(default = 15)] font_size: u32,
    #[props(default)] bold: bool,
    #[props(default)] color_override: Option<String>,
) -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let base_path = snap.config.base_path.clone();
    drop(snap);

    let text_color = color_override.unwrap_or_else(|| theme.text.to_string());
    let base_weight = if bold { "700" } else { "400" };

    if inlines.len() == 1
        && matches!(
            inlines[0],
            Inline::Text(_) | Inline::Strong(_) | Inline::Emphasis(_) | Inline::Strikethrough(_)
        )
    {
        let single = inlines.into_iter().next().unwrap();
        return rsx! {
            InlineNodeView {
                inline: single,
                font_size: font_size,
                base_weight: base_weight.to_string(),
                text_color: text_color,
                base_path: base_path,
            }
        };
    }

    rsx! {
        view {
            test_id: "docs-inlines",
            style: "flex-direction: row; align-items: center; gap: 2px; flex-wrap: wrap;",
            for (idx, inline) in inlines.into_iter().enumerate() {
                InlineNodeView {
                    key: "inl-{idx}",
                    inline: inline,
                    font_size: font_size,
                    base_weight: base_weight.to_string(),
                    text_color: text_color.clone(),
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
    text_color: String,
    base_path: String,
) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();

    match inline {
        Inline::Text(content) => rsx! {
            text {
                style: "font-size: {font_size}px; font-weight: {base_weight}; color: {text_color};",
                "{content}"
            }
        },
        Inline::Strong(children) => {
            let txt = inlines_plain_text(&children);
            rsx! {
                text {
                    style: "font-size: {font_size}px; font-weight: 700; color: {text_color};",
                    "{txt}"
                }
            }
        }
        Inline::Emphasis(children) => {
            let txt = inlines_plain_text(&children);
            rsx! {
                text {
                    style: "font-size: {font_size}px; font-weight: {base_weight}; font-style: italic; color: {text_color};",
                    "{txt}"
                }
            }
        }
        Inline::Strikethrough(children) => {
            let txt = inlines_plain_text(&children);
            rsx! {
                text {
                    style: "font-size: {font_size}px; text-decoration: line-through; color: {theme.muted};",
                    "{txt}"
                }
            }
        }
        Inline::Code(code) => {
            let code_size = font_size.saturating_sub(1).max(11);
            rsx! {
                view {
                    test_id: "docs-inline-code",
                    accessibility_role: "code",
                    style: "flex-shrink: 0; padding-horizontal: 5px; padding-vertical: 1px; border-radius: 4px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                    text {
                        style: "font-size: {code_size}px; font-family: monospace; color: {theme.accent};",
                        "{code}"
                    }
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
            let internal_target = DocsRoute::parse_link_target(&url);
            let href = match &internal_target {
                Some((Some(route), Some(anchor))) => format!("{}#{anchor}", route.href(&base_path)),
                Some((Some(route), None)) => route.href(&base_path),
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
                    style: "flex-shrink: 0; padding-horizontal: 1px;",
                    onpress: move |_| {
                        if let Some((route, anchor)) = internal_target.clone() {
                            navigate_to_target(state, route, anchor);
                        }
                    },
                    text {
                        style: "font-size: {font_size}px; font-weight: 500; color: {theme.accent}; text-decoration: underline;",
                        "{display}"
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
                    onpress: move |_| {
                        if let Some((route, anchor)) = internal_target.clone() {
                            navigate_to_target(state, route, anchor);
                        }
                    },
                    text {
                        style: "font-size: {font_size}px; color: {theme.accent}; text-decoration: underline;",
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
                    style: "max-width: 100%; border-radius: 8px; border-width: 1px; border-color: {theme.border};",
                }
            }
        }
        Inline::HardBreak => rsx! {
            view { style: "width: 100%; height: 4px;" }
        },
        Inline::InlineComponent(comp) => rsx! {
            InlineComponentView { comp: comp }
        },
    }
}
