//! Left navigation sidebar (`accessibility_role: "navigation"`, `260px` desktop / mobile slide-over drawer).

use dioxus_core_macro::Props;
use nve_ui::accent_indicator;
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
            nve_style: TypedStyle::new()
                .width_px(260.0)
                .height_pct(100.0)
                .flex_shrink(0.0)
                .flex_col()
                .bg(theme.surface.base)
                .border(1.0, theme.border.default),

            if mobile_overlay {
                view {
                    nve_style: TypedStyle::new()
                        .width_pct(100.0)
                        .flex_row()
                        .justify_content(FlexJustify::SpaceBetween)
                        .align_items(FlexAlign::Center)
                        .padding_xy(14.0, 12.0)
                        .border(1.0, theme.border.default),
                    text {
                        nve_style: TypedStyle::new().font_size(13.0).font_weight(700).color(theme.text.primary),
                        "Navigation"
                    }
                    pressable {
                        test_id: "docs-sidebar-close",
                        accessibility_role: "button",
                        accessibility_label: "Close navigation drawer",
                        nve_style: TypedStyle::new()
                            .padding_xy(8.0, 4.0)
                            .radius(6.0)
                            .bg(theme.surface.subtle)
                            .border(1.0, theme.border.default),
                        onpress: move |_| {
                            state.write().set_mobile_drawer(false);
                        },
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                            "Close"
                        }
                    }
                }
            }

            scrollview {
                test_id: "docs-sidebar-scroll",
                nve_style: TypedStyle::new().flex_grow(1.0).width_pct(100.0).flex_col().padding(14.0).gap(6.0),

                // Show section tabs inside mobile drawer
                if mobile_overlay && !sections.is_empty() {
                    view {
                        nve_style: TypedStyle::new()
                            .flex_col()
                            .gap(4.0)
                            .padding_xy(8.0, 6.0)
                            .border(1.0, theme.border.default)
                            .radius(8.0)
                            .bg(theme.surface.subtle),
                        for sec in sections {
                            {
                                let sec_route = DocsRoute::new(&sec.default_route);
                                let sec_href = sec_route.href(&base_path);
                                let is_active = sec.matches_route(&active_route);
                                let fg = if is_active { theme.text.link } else { theme.text.primary };
                                rsx! {
                                    pressable {
                                        key: "mobile-sec-{sec.id}",
                                        test_id: "docs-mobile-section-{sec.id}",
                                        accessibility_role: "link",
                                        "href": "{sec_href}",
                                        nve_style: TypedStyle::new()
                                            .flex_row()
                                            .justify_content(FlexJustify::SpaceBetween)
                                            .align_items(FlexAlign::Center)
                                            .padding_xy(6.0, 5.0)
                                            .radius(6.0),
                                        onpress: move |_| {
                                            navigate_to(state, sec_route.clone());
                                        },
                                        text {
                                            nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(fg),
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
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .height_pct(100.0)
                    .flex_row()
                    .z_index(400)
                    .bg(theme.scrim),
                {panel}
                pressable {
                    test_id: "docs-sidebar-backdrop",
                    accessibility_role: "presentation",
                    accessibility_label: "Dismiss navigation drawer",
                    nve_style: TypedStyle::new()
                        .flex_grow(1.0)
                        .height_pct(100.0)
                        .bg(ColorRgba::TRANSPARENT),
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

    let indent_px = (depth as u32).saturating_mul(12) as f32;

    match node {
        NavNode::Separator { title } => {
            drop(snap);
            rsx! {
                view {
                    nve_style: TypedStyle::new().width_pct(100.0).padding_y(6.0).gap(4.0),
                    view {
                        nve_style: TypedStyle::new().width_pct(100.0).height_px(1.0).bg(theme.border.default),
                    }
                    if let Some(lbl) = title {
                        text {
                            nve_style: TypedStyle::new()
                                .font_size(11.0)
                                .font_weight(700)
                                .color(theme.text.secondary)
                                .padding_x(6.0),
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
                theme.status.accent.subtle
            } else {
                ColorRgba::TRANSPARENT
            };
            let fg = if is_active { theme.text.link } else { theme.text.secondary };
            let weight = if is_active { 600 } else { 400 };
            let rail_color = if is_active {
                accent_indicator(&theme)
            } else {
                ColorRgba::TRANSPARENT
            };

            rsx! {
                pressable {
                    test_id: "{tid}",
                    accessibility_role: "link",
                    aria_selected: if is_active { Some("true".to_string()) } else { None },
                    "href": "{href}",
                    nve_style: TypedStyle::new()
                        .width_pct(100.0)
                        .flex_row()
                        .align_items(FlexAlign::Center)
                        .justify_content(FlexJustify::SpaceBetween)
                        .padding_xy(8.0, 6.0)
                        .gap(8.0)
                        .radius(6.0)
                        .bg(bg)
                        .margin(0.0),
                    onpress: move |_| {
                        navigate_to(state, target.clone());
                    },
                    view {
                        nve_style: TypedStyle::new()
                            .flex_row()
                            .align_items(FlexAlign::Center)
                            .gap(8.0)
                            .flex_grow(1.0)
                            .min_width_px(0.0)
                            .padding_x(indent_px),
                        view {
                            nve_style: TypedStyle::new().width_px(2.0).height_px(16.0).radius(2.0).bg(rail_color),
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).font_weight(weight).color(fg),
                            "{title}"
                        }
                    }
                    if let Some(badge_text) = badge {
                        view {
                            nve_style: TypedStyle::new()
                                .padding_xy(6.0, 1.0)
                                .radius(999.0)
                                .bg(theme.surface.subtle)
                                .border(1.0, theme.border.default),
                            text {
                                nve_style: TypedStyle::new().font_size(10.0).font_weight(600).color(theme.status.info.text),
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
                    nve_style: TypedStyle::new()
                        .width_pct(100.0)
                        .flex_row()
                        .align_items(FlexAlign::Center)
                        .justify_content(FlexJustify::SpaceBetween)
                        .padding_xy(10.0, 6.0)
                        .radius(6.0),
                    text {
                        nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary),
                        "{title} ↗"
                    }
                    if let Some(badge_text) = badge {
                        view {
                            nve_style: TypedStyle::new()
                                .padding_xy(6.0, 1.0)
                                .radius(999.0)
                                .bg(theme.surface.subtle)
                                .border(1.0, theme.border.default),
                            text {
                                nve_style: TypedStyle::new().font_size(10.0).font_weight(600).color(theme.status.info.text),
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
                theme.text.link
            } else {
                theme.text.primary
            };
            let folder_tid = format!("docs-sidebar-folder-{slug}");
            let slug_for_toggle = slug.clone();

            rsx! {
                view {
                    nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(2.0).padding_y(4.0),
                    pressable {
                        test_id: "{folder_tid}",
                        accessibility_role: "button",
                        aria_expanded: "{expanded_str}",
                        "href": index_href.unwrap_or_default(),
                        nve_style: TypedStyle::new()
                            .width_pct(100.0)
                            .flex_row()
                            .align_items(FlexAlign::Center)
                            .justify_content(FlexJustify::SpaceBetween)
                            .padding_xy(8.0, 6.0)
                            .radius(6.0),
                        onpress: move |_| {
                            if let Some(ref route) = mapped_index_route {
                                navigate_to(state, route.clone());
                            } else {
                                state.write().toggle_folder(&slug_for_toggle);
                            }
                        },
                        view {
                            nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(6.0),
                            text {
                                nve_style: TypedStyle::new().font_size(11.0).color(theme.text.secondary),
                                "{chevron}"
                            }
                            text {
                                nve_style: TypedStyle::new().font_size(12.0).font_weight(700).color(header_fg),
                                "{title}"
                            }
                        }
                        if let Some(badge_text) = badge {
                            view {
                                nve_style: TypedStyle::new()
                                    .padding_xy(6.0, 1.0)
                                    .radius(999.0)
                                    .bg(theme.surface.subtle)
                                    .border(1.0, theme.border.default),
                                text {
                                    nve_style: TypedStyle::new().font_size(10.0).font_weight(600).color(theme.status.info.text),
                                    "{badge_text}"
                                }
                            }
                        }
                    }

                    if !is_collapsed {
                        view {
                            nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(2.0).padding_x(4.0),
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
