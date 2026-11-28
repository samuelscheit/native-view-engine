//! Page breadcrumbs, "Edit this page on GitHub" link, and Prev/Next pagination cards.

use nve_dioxus::prelude::*;

use crate::route::DocsRoute;
use crate::state::{navigate_to, use_docs_state};

/// Breadcrumb trail rendered above the article heading.
#[component]
pub fn Breadcrumbs() -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let base_path = snap.config.base_path.clone();
    let trail = snap.breadcrumbs();
    drop(snap);

    if trail.is_empty() {
        return rsx! {};
    }

    let last_idx = trail.len().saturating_sub(1);

    rsx! {
        view {
            test_id: "docs-breadcrumbs",
            accessibility_role: "navigation",
            accessibility_label: "Breadcrumb",
            nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(6.0),
            for (idx, (label, route)) in trail.into_iter().enumerate() {
                {
                    let is_last = idx == last_idx;
                    let fg = if is_last { theme.text.primary } else { theme.text.secondary };
                    let weight = if is_last { 600 } else { 400 };
                    let href = if route.is_empty() {
                        String::new()
                    } else {
                        DocsRoute::new(&route).href(&base_path)
                    };
                    let has_route = !route.is_empty() && !is_last;

                    rsx! {
                        view {
                            key: "crumb-{idx}",
                            nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(6.0),
                            if idx > 0 {
                                text {
                                    nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                                    "/"
                                }
                            }
                            if has_route {
                                pressable {
                                    test_id: "docs-breadcrumb-{idx}",
                                    accessibility_role: "link",
                                    "href": "{href}",
                                    onpress: move |_| {
                                        navigate_to(state, DocsRoute::new(&route));
                                    },
                                    text {
                                        nve_style: TypedStyle::new().font_size(12.0).font_weight(weight).color(fg),
                                        "{label}"
                                    }
                                }
                            } else {
                                text {
                                    nve_style: TypedStyle::new().font_size(12.0).font_weight(weight).color(fg),
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Page footer displaying the optional GitHub edit link and Prev/Next pagination cards.
#[component]
pub fn PageFooter() -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let prev_next = snap.prev_next();
    let config = snap.config.clone();
    let edit_url = snap
        .current_page()
        .and_then(|page| snap.config.edit_url_for(&page.source_path));
    drop(snap);

    rsx! {
        view {
            test_id: "docs-page-footer",
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_col()
                .gap(16.0)
                .padding_xy(16.0, 20.0)
                .border(1.0, theme.border.default)
                .radius(10.0)
                .bg(theme.surface.base),

            if let Some(url) = edit_url {
                pressable {
                    test_id: "docs-edit-link",
                    accessibility_role: "link",
                    accessibility_label: "Edit this page on GitHub",
                    "href": "{url}",
                    nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(6.0),
                    text {
                        nve_style: TypedStyle::new().font_size(12.0).font_weight(500).color(theme.text.link),
                        "Edit this page on GitHub ↗"
                    }
                }
            }

            if prev_next.prev.is_some() || prev_next.next.is_some() {
                view {
                    test_id: "docs-prev-next",
                    nve_style: TypedStyle::new()
                        .width_pct(100.0)
                        .flex_row()
                        .justify_content(FlexJustify::SpaceBetween)
                        .gap(12.0),

                    if let Some(prev) = prev_next.prev {
                        {
                            let prev_route = config.docs_route_for_content(&prev.route);
                            let prev_href = config.resolve_href(prev_route.as_str());
                            rsx! {
                                pressable {
                                    test_id: "docs-prev-link",
                                    accessibility_role: "link",
                                    accessibility_label: "Previous page: {prev.title}",
                                    "href": "{prev_href}",
                                    nve_style: TypedStyle::new()
                                        .flex_grow(1.0)
                                        .min_width_px(0.0)
                                        .flex_col()
                                        .gap(4.0)
                                        .padding(12.0)
                                        .radius(8.0)
                                        .bg(theme.surface.subtle)
                                        .border(1.0, theme.border.default),
                                    onpress: move |_| {
                                        navigate_to(state, prev_route.clone());
                                    },
                                    text {
                                        nve_style: TypedStyle::new().font_size(11.0).font_weight(600).color(theme.text.secondary),
                                        "← Previous"
                                    }
                                    text {
                                        nve_style: TypedStyle::new().font_size(14.0).font_weight(600).color(theme.text.primary),
                                        "{prev.title}"
                                    }
                                }
                            }
                        }
                    } else {
                        view { nve_style: TypedStyle::new().flex_grow(1.0) }
                    }

                    if let Some(next) = prev_next.next {
                        {
                            let next_route = config.docs_route_for_content(&next.route);
                            let next_href = config.resolve_href(next_route.as_str());
                            rsx! {
                                pressable {
                                    test_id: "docs-next-link",
                                    accessibility_role: "link",
                                    accessibility_label: "Next page: {next.title}",
                                    "href": "{next_href}",
                                    nve_style: TypedStyle::new()
                                        .flex_grow(1.0)
                                        .min_width_px(0.0)
                                        .flex_col()
                                        .align_items(FlexAlign::FlexEnd)
                                        .gap(4.0)
                                        .padding(12.0)
                                        .radius(8.0)
                                        .bg(theme.surface.subtle)
                                        .border(1.0, theme.border.default),
                                    onpress: move |_| {
                                        navigate_to(state, next_route.clone());
                                    },
                                    text {
                                        nve_style: TypedStyle::new().font_size(11.0).font_weight(600).color(theme.text.secondary),
                                        "Next →"
                                    }
                                    text {
                                        nve_style: TypedStyle::new().font_size(14.0).font_weight(600).color(theme.text.primary),
                                        "{next.title}"
                                    }
                                }
                            }
                        }
                    } else {
                        view { nve_style: TypedStyle::new().flex_grow(1.0) }
                    }
                }
            }
        }
    }
}
