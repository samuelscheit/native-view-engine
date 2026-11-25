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
            style: "flex-direction: row; align-items: center; gap: 6px;",
            for (idx, (label, route)) in trail.into_iter().enumerate() {
                {
                    let is_last = idx == last_idx;
                    let fg = if is_last { theme.text } else { theme.muted };
                    let weight = if is_last { "600" } else { "400" };
                    let href = if route.is_empty() {
                        String::new()
                    } else {
                        DocsRoute::new(&route).href(&base_path)
                    };
                    let has_route = !route.is_empty() && !is_last;

                    rsx! {
                        view {
                            key: "crumb-{idx}",
                            style: "flex-direction: row; align-items: center; gap: 6px;",
                            if idx > 0 {
                                text {
                                    style: "font-size: 12px; color: {theme.muted};",
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
                                        style: "font-size: 12px; font-weight: {weight}; color: {fg};",
                                        "{label}"
                                    }
                                }
                            } else {
                                text {
                                    style: "font-size: 12px; font-weight: {weight}; color: {fg};",
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
            style: "width: 100%; flex-direction: column; gap: 16px; padding-vertical: 20px; border-width: 1px; border-color: {theme.border}; border-radius: 10px; padding-horizontal: 16px; background-color: {theme.surface};",

            if let Some(url) = edit_url {
                pressable {
                    test_id: "docs-edit-link",
                    accessibility_role: "link",
                    accessibility_label: "Edit this page on GitHub",
                    "href": "{url}",
                    style: "flex-direction: row; align-items: center; gap: 6px;",
                    text {
                        style: "font-size: 12px; font-weight: 500; color: {theme.accent};",
                        "Edit this page on GitHub ↗"
                    }
                }
            }

            if prev_next.prev.is_some() || prev_next.next.is_some() {
                view {
                    test_id: "docs-prev-next",
                    style: "width: 100%; flex-direction: row; justify-content: space-between; gap: 12px;",

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
                                    style: "flex-grow: 1; flex-direction: column; gap: 4px; padding: 12px; border-radius: 8px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                                    onpress: move |_| {
                                        navigate_to(state, prev_route.clone());
                                    },
                                    text {
                                        style: "font-size: 11px; font-weight: 600; color: {theme.muted};",
                                        "← Previous"
                                    }
                                    text {
                                        style: "font-size: 14px; font-weight: 600; color: {theme.text};",
                                        "{prev.title}"
                                    }
                                }
                            }
                        }
                    } else {
                        view { style: "flex-grow: 1;" }
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
                                    style: "flex-grow: 1; flex-direction: column; align-items: flex-end; gap: 4px; padding: 12px; border-radius: 8px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                                    onpress: move |_| {
                                        navigate_to(state, next_route.clone());
                                    },
                                    text {
                                        style: "font-size: 11px; font-weight: 600; color: {theme.muted};",
                                        "Next →"
                                    }
                                    text {
                                        style: "font-size: 14px; font-weight: 600; color: {theme.text};",
                                        "{next.title}"
                                    }
                                }
                            }
                        }
                    } else {
                        view { style: "flex-grow: 1;" }
                    }
                }
            }
        }
    }
}
