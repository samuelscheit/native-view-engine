//! Left navigation sidebar (`accessibility_role: "navigation"`, `260px` desktop / mobile slide-over drawer).

use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;
use nve_mdx::NavNode;

use crate::route::DocsRoute;
use crate::state::{navigate_to, use_docs_state};

/// Convert a normalized route into a deterministic `test_id` slug (e.g. `"/guides/intro"` -> `"guides-intro"`).
pub fn route_test_id_slug(route: &str) -> String {
    let trimmed = route.trim().trim_matches('/');
    if trimmed.is_empty() {
        return "root".to_string();
    }
    trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Left sidebar rendered inline at `260px` on desktop (`>= 960pt`) or as a slide-over overlay on mobile.
#[component]
pub fn DocsSidebar(mobile_overlay: bool) -> Element {
    let mut state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let nav_items = snap.site.nav.items.clone();
    let sections = snap.config.sections.clone();
    let active_route = snap.active_route.as_str().to_string();
    let base_path = snap.config.base_path.clone();
    drop(snap);

    let desktop_show_on = (!mobile_overlay).then(|| ShowOn(MediaQuery::min_width(960)));

    let panel = rsx! {
        view {
            test_id: if mobile_overlay { "docs-sidebar-drawer" } else { "docs-sidebar-desktop" },
            nve_show_on: desktop_show_on,
            accessibility_role: "navigation",
            accessibility_label: "Documentation sidebar",
            style: "width: 260px; height: 100%; flex-shrink: 0; flex-direction: column; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",

            if mobile_overlay {
                view {
                    style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 14px; padding-vertical: 12px; border-width: 1px; border-color: {theme.border};",
                    text {
                        style: "font-size: 13px; font-weight: 700; color: {theme.text};",
                        "Navigation"
                    }
                    pressable {
                        test_id: "docs-sidebar-close",
                        accessibility_role: "button",
                        accessibility_label: "Close navigation drawer",
                        style: "padding-horizontal: 8px; padding-vertical: 4px; border-radius: 6px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                        onpress: move |_| {
                            state.write().set_mobile_drawer(false);
                        },
                        text {
                            style: "font-size: 12px; color: {theme.muted};",
                            "Close"
                        }
                    }
                }
            }

            scrollview {
                test_id: "docs-sidebar-scroll",
                style: "flex-grow: 1; width: 100%; flex-direction: column; padding: 14px; gap: 6px;",

                // Show section tabs inside mobile drawer
                if mobile_overlay && !sections.is_empty() {
                    view {
                        style: "flex-direction: column; gap: 4px; padding-vertical: 6px; border-width: 1px; border-color: {theme.border}; border-radius: 8px; padding-horizontal: 8px; background-color: {theme.surface_subtle};",
                        for sec in sections {
                            {
                                let sec_route = DocsRoute::new(&sec.default_route);
                                let sec_href = sec_route.href(&base_path);
                                let is_active = sec.matches_route(&active_route);
                                let fg = if is_active { theme.accent } else { theme.text };
                                rsx! {
                                    pressable {
                                        key: "mobile-sec-{sec.id}",
                                        test_id: "docs-mobile-section-{sec.id}",
                                        accessibility_role: "link",
                                        "href": "{sec_href}",
                                        style: "flex-direction: row; justify-content: space-between; align-items: center; padding-vertical: 5px; padding-horizontal: 6px; border-radius: 6px;",
                                        onpress: move |_| {
                                            navigate_to(state, sec_route.clone());
                                        },
                                        text {
                                            style: "font-size: 13px; font-weight: 600; color: {fg};",
                                            "{sec.label}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                for (idx, node) in nav_items.into_iter().enumerate() {
                    SidebarNodeView {
                        key: "nav-root-{idx}",
                        node: node,
                        depth: 0,
                    }
                }
            }
        }
    };

    if mobile_overlay {
        rsx! {
            view {
                test_id: "docs-sidebar-overlay",
                nve_show_on: ShowOn(MediaQuery::max_width(959)),
                style: "width: 100%; height: 100%; flex-direction: row; z-index: 400; background-color: rgba(0, 0, 0, 0.55);",
                {panel}
                pressable {
                    test_id: "docs-sidebar-backdrop",
                    accessibility_role: "presentation",
                    accessibility_label: "Dismiss navigation drawer",
                    style: "flex-grow: 1; height: 100%; background-color: transparent;",
                    onpress: move |_| {
                        state.write().set_mobile_drawer(false);
                    },
                }
            }
        }
    } else {
        panel
    }
}

#[component]
fn SidebarNodeView(node: NavNode, depth: usize) -> Element {
    let mut state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let active_route = snap.active_route.as_str().to_string();
    let config = snap.config.clone();

    let indent_px = (depth as u32).saturating_mul(12);

    match node {
        NavNode::Separator { title } => {
            drop(snap);
            rsx! {
                view {
                    style: "width: 100%; padding-vertical: 6px; gap: 4px;",
                    view {
                        style: "width: 100%; height: 1px; background-color: {theme.border};",
                    }
                    if let Some(lbl) = title {
                        text {
                            style: "font-size: 11px; font-weight: 700; color: {theme.muted}; padding-horizontal: 6px;",
                            "{lbl}"
                        }
                    }
                }
            }
        }
        NavNode::Page {
            title,
            route,
            badge,
            ..
        } => {
            let target = config.docs_route_for_content(&route);
            let is_active = target.as_str() == active_route;
            let href = config.resolve_href(target.as_str());
            let tid = format!("docs-nav-{}", route_test_id_slug(&route));
            drop(snap);

            let bg = if is_active {
                theme.accent_surface
            } else {
                "transparent"
            };
            let fg = if is_active { theme.accent } else { theme.muted };
            let weight = if is_active { "600" } else { "400" };
            let rail_color = if is_active {
                theme.accent
            } else {
                "transparent"
            };

            rsx! {
                pressable {
                    test_id: "{tid}",
                    accessibility_role: "link",
                    aria_selected: if is_active { Some("true".to_string()) } else { None },
                    "href": "{href}",
                    style: "width: 100%; flex-direction: row; align-items: center; justify-content: space-between; padding-vertical: 6px; padding-horizontal: 8px; gap: 8px; border-radius: 6px; background-color: {bg}; margin: 0px;",
                    onpress: move |_| {
                        navigate_to(state, target.clone());
                    },
                    view {
                        style: "flex-direction: row; align-items: center; gap: 8px; flex-grow: 1; padding-horizontal: {indent_px}px;",
                        view {
                            style: "width: 2px; height: 16px; border-radius: 2px; background-color: {rail_color};",
                        }
                        text {
                            style: "font-size: 13px; font-weight: {weight}; color: {fg};",
                            "{title}"
                        }
                    }
                    if let Some(badge_text) = badge {
                        view {
                            style: "padding-horizontal: 6px; padding-vertical: 1px; border-radius: 999px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                            text {
                                style: "font-size: 10px; font-weight: 600; color: {theme.accent_alt};",
                                "{badge_text}"
                            }
                        }
                    }
                }
            }
        }
        NavNode::ExternalLink {
            title, href, badge, ..
        } => {
            drop(snap);
            rsx! {
                pressable {
                    accessibility_role: "link",
                    "href": "{href}",
                    style: "width: 100%; flex-direction: row; align-items: center; justify-content: space-between; padding-vertical: 6px; padding-horizontal: 10px; border-radius: 6px;",
                    text {
                        style: "font-size: 13px; color: {theme.muted};",
                        "{title} ↗"
                    }
                    if let Some(badge_text) = badge {
                        view {
                            style: "padding-horizontal: 6px; padding-vertical: 1px; border-radius: 999px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                            text {
                                style: "font-size: 10px; font-weight: 600; color: {theme.accent_alt};",
                                "{badge_text}"
                            }
                        }
                    }
                }
            }
        }
        NavNode::Folder {
            title,
            slug,
            index_route,
            badge,
            children,
            ..
        } => {
            let is_collapsed = snap.is_folder_collapsed(&slug);
            let mapped_index_route = index_route
                .as_deref()
                .map(|r| config.docs_route_for_content(r));
            let is_index_active = mapped_index_route
                .as_ref()
                .is_some_and(|r| r.as_str() == active_route);
            let index_href = mapped_index_route
                .as_ref()
                .map(|r| config.resolve_href(r.as_str()));
            drop(snap);

            let chevron = if is_collapsed { "▸" } else { "▾" };
            let expanded_str = if is_collapsed { "false" } else { "true" };
            let header_fg = if is_index_active {
                theme.accent
            } else {
                theme.text
            };
            let folder_tid = format!("docs-sidebar-folder-{slug}");
            let slug_for_toggle = slug.clone();

            rsx! {
                view {
                    style: "width: 100%; flex-direction: column; gap: 2px; padding-vertical: 4px;",
                    pressable {
                        test_id: "{folder_tid}",
                        accessibility_role: "button",
                        aria_expanded: "{expanded_str}",
                        "href": index_href.unwrap_or_default(),
                        style: "width: 100%; flex-direction: row; align-items: center; justify-content: space-between; padding-vertical: 6px; padding-horizontal: 8px; border-radius: 6px;",
                        onpress: move |_| {
                            if let Some(ref route) = mapped_index_route {
                                navigate_to(state, route.clone());
                            } else {
                                state.write().toggle_folder(&slug_for_toggle);
                            }
                        },
                        view {
                            style: "flex-direction: row; align-items: center; gap: 6px;",
                            text {
                                style: "font-size: 11px; color: {theme.muted};",
                                "{chevron}"
                            }
                            text {
                                style: "font-size: 12px; font-weight: 700; color: {header_fg};",
                                "{title}"
                            }
                        }
                        if let Some(badge_text) = badge {
                            view {
                                style: "padding-horizontal: 6px; padding-vertical: 1px; border-radius: 999px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                                text {
                                    style: "font-size: 10px; font-weight: 600; color: {theme.accent_alt};",
                                    "{badge_text}"
                                }
                            }
                        }
                    }

                    if !is_collapsed {
                        view {
                            style: "width: 100%; flex-direction: column; gap: 2px; padding-horizontal: 4px;",
                            for (cidx, child) in children.into_iter().enumerate() {
                                SidebarNodeView {
                                    key: "{slug}-child-{cidx}",
                                    node: child,
                                    depth: depth + 1,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
