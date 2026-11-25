//! Sticky top navbar (`accessibility_role: "banner"`) with brand, sections, `⌘K` search, theme toggle, and mobile drawer toggle.

use nve_dioxus::prelude::*;

use crate::route::DocsRoute;
use crate::state::{navigate_to, use_docs_state};

/// Sticky top navigation bar rendered across all viewports.
#[component]
pub fn DocsNavbar() -> Element {
    let mut state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let is_mobile = snap.viewport.is_mobile();
    let drawer_open = snap.mobile_drawer_open;
    let active_route = snap.active_route.as_str().to_string();
    let site_title = snap.config.site_title.clone();
    let brand_mark = snap.config.brand_mark.clone();
    let home_href = snap.config.resolve_href("/");
    let github_url = snap.config.github_repo_url.clone();
    let active_section_id = snap
        .config
        .active_section_for(&active_route)
        .map(|s| s.id.clone());
    let sections: Vec<_> = snap
        .config
        .sections
        .iter()
        .map(|s| {
            let is_active = active_section_id.as_deref() == Some(s.id.as_str());
            let href = snap.config.resolve_href(&s.default_route);
            (
                s.id.clone(),
                s.label.clone(),
                s.default_route.clone(),
                href,
                s.badge.clone(),
                is_active,
            )
        })
        .collect();
    drop(snap);

    let theme_label = if theme.is_dark { "Light" } else { "Dark" };
    let theme_hint = if theme.is_dark {
        "Switch to light theme"
    } else {
        "Switch to dark theme"
    };
    let menu_label = if drawer_open { "Close" } else { "Menu" };
    let menu_expanded = if drawer_open { "true" } else { "false" };

    rsx! {
        view {
            test_id: "docs-navbar",
            accessibility_role: "banner",
            accessibility_label: "{site_title} navigation",
            style: "width: 100%; height: 56px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 16px; gap: 12px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border}; z-index: 50;",

            // Left cluster: mobile drawer toggle + brand mark + section tabs
            view {
                style: "flex-direction: row; align-items: center; gap: 12px;",

                if is_mobile {
                    pressable {
                        test_id: "docs-mobile-menu-toggle",
                        accessibility_role: "button",
                        accessibility_label: "Toggle navigation sidebar",
                        aria_expanded: "{menu_expanded}",
                        aria_controls: "docs-sidebar-drawer",
                        style: "padding-horizontal: 10px; padding-vertical: 6px; border-radius: 6px; border-width: 1px; border-color: {theme.border}; background-color: {theme.surface_subtle};",
                        onpress: move |_| {
                            state.write().toggle_mobile_drawer();
                        },
                        text {
                            style: "font-size: 13px; font-weight: 600; color: {theme.text};",
                            "{menu_label}"
                        }
                    }
                }

                // Brand link
                pressable {
                    test_id: "docs-brand-link",
                    accessibility_role: "link",
                    accessibility_label: "{site_title} home",
                    "href": "{home_href}",
                    style: "flex-direction: row; align-items: center; gap: 8px; padding-vertical: 4px;",
                    onpress: move |_| {
                        navigate_to(state, DocsRoute::root());
                    },
                    view {
                        style: "padding-horizontal: 7px; padding-vertical: 3px; border-radius: 6px; background-color: {theme.accent_surface}; border-width: 1px; border-color: {theme.accent};",
                        text {
                            style: "font-size: 11px; font-weight: 700; color: {theme.accent};",
                            "{brand_mark}"
                        }
                    }
                    if !is_mobile {
                        text {
                            style: "font-size: 15px; font-weight: 700; color: {theme.text};",
                            "{site_title}"
                        }
                    }
                }

                // Desktop section tabs
                if !is_mobile && !sections.is_empty() {
                    view {
                        test_id: "docs-section-tabs",
                        accessibility_role: "navigation",
                        accessibility_label: "Documentation sections",
                        style: "flex-direction: row; align-items: center; gap: 4px; margin: 0px;",
                        for (sec_id, sec_label, sec_route, sec_href, sec_badge, is_active) in sections {
                            {
                                let bg = if is_active { theme.accent_surface } else { "transparent" };
                                let fg = if is_active { theme.accent } else { theme.muted };
                                let border_c = if is_active { theme.accent } else { "transparent" };
                                let tid = format!("docs-section-{sec_id}");
                                let target_route = DocsRoute::new(&sec_route);
                                rsx! {
                                    pressable {
                                        key: "{sec_id}",
                                        test_id: "{tid}",
                                        accessibility_role: "link",
                                        "href": "{sec_href}",
                                        style: "flex-direction: row; align-items: center; gap: 6px; padding-horizontal: 10px; padding-vertical: 6px; border-radius: 6px; background-color: {bg}; border-width: 1px; border-color: {border_c};",
                                        onpress: move |_| {
                                            navigate_to(state, target_route.clone());
                                        },
                                        text {
                                            style: "font-size: 13px; font-weight: 500; color: {fg};",
                                            "{sec_label}"
                                        }
                                        if let Some(badge_txt) = sec_badge {
                                            view {
                                                style: "padding-horizontal: 5px; padding-vertical: 1px; border-radius: 999px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                                                text {
                                                    style: "font-size: 10px; font-weight: 600; color: {theme.accent_alt};",
                                                    "{badge_txt}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Right cluster: ⌘K search trigger pill, Dark/Light toggle, GitHub link
            view {
                style: "flex-direction: row; align-items: center; gap: 8px;",

                pressable {
                    test_id: "docs-search-trigger",
                    accessibility_role: "button",
                    accessibility_label: "Search documentation (Command K)",
                    style: "flex-direction: row; align-items: center; justify-content: space-between; gap: 14px; padding-horizontal: 10px; padding-vertical: 6px; border-radius: 8px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                    onpress: move |_| {
                        state.write().open_search();
                    },
                    text {
                        style: "font-size: 13px; color: {theme.muted};",
                        if is_mobile { "Search" } else { "Search docs..." }
                    }
                    if !is_mobile {
                        view {
                            style: "padding-horizontal: 6px; padding-vertical: 2px; border-radius: 4px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                            text {
                                style: "font-size: 11px; font-weight: 600; color: {theme.muted};",
                                "⌘K"
                            }
                        }
                    }
                }

                pressable {
                    test_id: "docs-theme-toggle",
                    accessibility_role: "button",
                    accessibility_label: "{theme_hint}",
                    style: "padding-horizontal: 10px; padding-vertical: 6px; border-radius: 8px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                    onpress: move |_| {
                        state.write().toggle_theme();
                    },
                    text {
                        style: "font-size: 12px; font-weight: 600; color: {theme.text};",
                        "{theme_label}"
                    }
                }

                if !is_mobile && let Some(repo_url) = github_url {
                    pressable {
                        test_id: "docs-github-link",
                        accessibility_role: "link",
                        accessibility_label: "GitHub repository",
                        "href": "{repo_url}",
                        style: "padding-horizontal: 10px; padding-vertical: 6px; border-radius: 8px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                        text {
                            style: "font-size: 12px; font-weight: 600; color: {theme.muted};",
                            "GitHub"
                        }
                    }
                }
            }
        }
    }
}
