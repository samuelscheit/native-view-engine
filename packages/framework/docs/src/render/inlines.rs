//! Inline `nve-mdx` AST renderer (text, emphasis, strong, strikethrough, inline code, links, images, components).

use dioxus_core_macro::Props;
use nve_abi::ColorRgba;
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
    #[props(default)] line_height: Option<f32>,
) -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let base_path = snap.config.base_path.clone();
    drop(snap);

    let text_color = color_override.unwrap_or(theme.text.primary);
    let base_weight: u16 = if bold { 700 } else { 400 };
    let lh = line_height.unwrap_or(if font_size >= 18 {
        1.25
    } else if font_size <= 13 {
        1.45
    } else {
        1.65
    });

    rsx! {
        view {
            test_id: "docs-inlines",
            nve_style: TypedStyle::new()
                .display(Display::Block)
                .width_pct(100.0)
                .min_width_px(0.0)
                .flex_row()
                .align_items(FlexAlign::Center)
                .flex_wrap(FlexWrap::Wrap)
                .line_height_ratio(lh)
                .overflow_wrap(OverflowWrap::BreakWord),
            for (idx, inline) in inlines.into_iter().enumerate() {
                InlineNodeView {
                    key: "inl-{idx}",
                    inline: inline,
                    font_size: font_size,
                    base_weight,
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
    base_weight: u16,
    text_color: ColorRgba,
    base_path: String,
) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();
    let fs = font_size as f32;

    match inline {
        Inline::Text(content) => rsx! {
            text {
                nve_style: TypedStyle::new()
                    .display(Display::Inline)
                    .font_size(fs)
                    .font_weight(base_weight)
                    .color(text_color),
                "{content}"
            }
        },
        Inline::Strong(children) => rsx! {
            text {
                nve_table: html_semantic(SemanticElement::Strong),
                nve_style: TypedStyle::new()
                    .display(Display::Inline)
                    .font_size(fs)
                    .font_weight(700)
                    .color(text_color),
                for (idx, child) in children.into_iter().enumerate() {
                    InlineNodeView {
                        key: "strong-{idx}",
                        inline: child,
                        font_size: font_size,
                        base_weight: 700,
                        text_color,
                        base_path: base_path.clone(),
                    }
                }
            }
        },
        Inline::Emphasis(children) => rsx! {
            text {
                nve_table: html_semantic(SemanticElement::Em),
                nve_style: TypedStyle::new()
                    .display(Display::Inline)
                    .font_size(fs)
                    .font_weight(base_weight)
                    .font_style(FontStyle::Italic)
                    .color(text_color),
                for (idx, child) in children.into_iter().enumerate() {
                    InlineNodeView {
                        key: "em-{idx}",
                        inline: child,
                        font_size: font_size,
                        base_weight,
                        text_color,
                        base_path: base_path.clone(),
                    }
                }
            }
        },
        Inline::Strikethrough(children) => rsx! {
            text {
                nve_table: html_semantic(SemanticElement::Strike),
                nve_style: TypedStyle::new()
                    .display(Display::Inline)
                    .font_size(fs)
                    .font_weight(base_weight)
                    .text_decoration(TextDecoration::LINE_THROUGH)
                    .color(theme.text.secondary),
                for (idx, child) in children.into_iter().enumerate() {
                    InlineNodeView {
                        key: "del-{idx}",
                        inline: child,
                        font_size: font_size,
                        base_weight,
                        text_color: theme.text.secondary,
                        base_path: base_path.clone(),
                    }
                }
            }
        },
        Inline::Code(code) => {
            let code_size = font_size.saturating_sub(1).max(11) as f32;
            rsx! {
                text {
                    test_id: "docs-inline-code",
                    accessibility_role: "code",
                    nve_style: TypedStyle::new()
                        .display(Display::Inline)
                        .font_size(code_size)
                        .font_family("monospace")
                        .color(theme.text.link)
                        .padding_xy(4.0, 1.0)
                        .radius(4.0)
                        .bg(theme.surface.subtle)
                        .border(1.0, theme.border.default),
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
                    nve_style: TypedStyle::new()
                        .display(Display::Inline)
                        .font_size(fs)
                        .font_weight(500)
                        .color(theme.text.link)
                        .text_decoration(TextDecoration::UNDERLINE),
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
                                base_weight: 500,
                                text_color: theme.text.link,
                                base_path: base_path.clone(),
                            }
                        }
                    } else {
                        text {
                            nve_style: TypedStyle::new()
                                .display(Display::Inline)
                                .font_size(fs)
                                .font_weight(500)
                                .color(theme.text.link),
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
                    nve_style: TypedStyle::new()
                        .display(Display::Inline)
                        .font_size(fs)
                        .color(theme.text.link)
                        .text_decoration(TextDecoration::UNDERLINE),
                    onpress: move |_| {
                        if let Some((route, anchor)) = internal_target.clone() {
                            navigate_to_target(state, route, anchor);
                        }
                    },
                    text {
                        nve_style: TypedStyle::new()
                            .display(Display::Inline)
                            .font_size(fs)
                            .color(theme.text.link),
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
                    nve_style: TypedStyle::new()
                        .max_width_pct(100.0)
                        .radius(8.0)
                        .border(1.0, theme.border.default),
                }
            }
        }
        Inline::HardBreak => rsx! {
            view {
                nve_table: html_semantic(SemanticElement::Br),
                nve_style: TypedStyle::new().width_pct(100.0).height_px(4.0),
            }
        },
        Inline::InlineComponent(comp) => rsx! {
            InlineComponentView { comp: comp }
        },
    }
}
