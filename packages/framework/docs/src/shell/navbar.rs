//! Sticky top navbar (`accessibility_role: "banner"`) with brand, sections, `⌘K` search, theme toggle, and mobile drawer toggle.

use nve_dioxus::prelude::*;
use nve_ui::{accent_indicator, tab_colors, ThemeMode};

use crate::route::DocsRoute;
use crate::state::{navigate_to, use_docs_state};

/// One segment of the System / Light / Dark theme control (`role="radio"`); the selected
/// segment is raised and outlined with the accent indicator (WCAG 1.4.11 boundary).
#[component]
fn ThemeModeOption(mode: ThemeMode, selected: bool) -> Element {
    let mut state = use_docs_state();
    let theme = state.read().theme();
    let c = tab_colors(&theme, selected, false);
    let hint = match mode {
        ThemeMode::System => "Follow the system theme",
        ThemeMode::Light => "Light theme",
        ThemeMode::Dark => "Dark theme",
    };
    rsx! {
        pressable {
            test_id: "docs-theme-mode-{mode.as_str()}",
            accessibility_role: "radio",
            accessibility_label: "{hint}",
            aria_checked: if selected { "true" } else { "false" },
            nve_style: TypedStyle::new()
                .padding_xy(8.0, 4.0)
                .radius(6.0)
                .bg(c.bg)
                .border(1.0, c.border),
            onpress: move |_| state.write().set_theme_mode(mode),
            text {
                nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(c.fg),
                "{mode.label()}"
            }
        }
    }
}

/// Sticky top navigation bar rendered across all viewports.
#[component]
pub fn DocsNavbar() -> Element {
    let mut state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let theme_mode = snap.theme_mode();
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

    let menu_label = if drawer_open { "Close" } else { "Menu" };
    let menu_expanded = if drawer_open { "true" } else { "false" };

    rsx! {
        view {
            test_id: "docs-navbar",
            accessibility_role: "banner",
            accessibility_label: "{site_title} navigation",
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .height_px(56.0)
                .flex_shrink(0.0)
                .flex_row()
                .justify_content(FlexJustify::SpaceBetween)
                .align_items(FlexAlign::Center)
                .padding_x(16.0)
                .gap(12.0)
                .bg(theme.surface.base)
                .border(1.0, theme.border.default)
                .z_index(50),

            // Left cluster: mobile drawer toggle + brand mark + section tabs
            view {
                nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(12.0),

                pressable {
                    test_id: "docs-mobile-menu-toggle",
                    nve_show_on: ShowOn(MediaQuery::max_width(959)),
                    accessibility_role: "button",
                    accessibility_label: "Toggle navigation sidebar",
                    aria_expanded: "{menu_expanded}",
                    aria_controls: "docs-sidebar-drawer",
                    nve_style: TypedStyle::new()
                        .padding_xy(10.0, 6.0)
                        .radius(6.0)
                        .border(1.0, theme.border.default)
                        .bg(theme.surface.subtle),
                    onpress: move |_| {
                        state.write().toggle_mobile_drawer();
                    },
                    text {
                        nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(theme.text.primary),
                        "{menu_label}"
                    }
                }

                // Brand link
                pressable {
                    test_id: "docs-brand-link",
                    accessibility_role: "link",
                    accessibility_label: "{site_title} home",
                    "href": "{home_href}",
                    nve_style: TypedStyle::new()
                        .flex_row()
                        .align_items(FlexAlign::Center)
                        .gap(8.0)
                        .padding_y(4.0),
                    onpress: move |_| {
                        navigate_to(state, DocsRoute::root());
                    },
                    view {
                        nve_style: TypedStyle::new()
                            .padding_xy(7.0, 3.0)
                            .radius(6.0)
                            .bg(theme.status.accent.subtle)
                            .border(1.0, theme.status.accent.border),
                        text {
                            nve_style: TypedStyle::new().font_size(11.0).font_weight(700).color(theme.text.link),
                            "{brand_mark}"
                        }
                    }
                    text {
                        test_id: "docs-brand-title",
                        nve_show_on: ShowOn(MediaQuery::min_width(960)),
                        nve_style: TypedStyle::new().font_size(15.0).font_weight(700).color(theme.text.primary),
                        "{site_title}"
                    }
                }

                // Desktop section tabs
                if !sections.is_empty() {
                    view {
                        test_id: "docs-section-tabs",
                        nve_show_on: ShowOn(MediaQuery::min_width(960)),
                        accessibility_role: "navigation",
                        accessibility_label: "Documentation sections",
                        nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(4.0).margin(0.0),
                        for (sec_id, sec_label, sec_route, sec_href, sec_badge, is_active) in sections {
                            {
                                let bg = if is_active { theme.status.accent.subtle } else { ColorRgba::TRANSPARENT };
                                let fg = if is_active { theme.text.link } else { theme.text.secondary };
                                let border_c = if is_active { accent_indicator(&theme) } else { ColorRgba::TRANSPARENT };
                                let tid = format!("docs-section-{sec_id}");
                                let target_route = DocsRoute::new(&sec_route);
                                rsx! {
                                    pressable {
                                        key: "{sec_id}",
                                        test_id: "{tid}",
                                        accessibility_role: "link",
                                        "href": "{sec_href}",
                                        nve_style: TypedStyle::new()
                                            .flex_row()
                                            .align_items(FlexAlign::Center)
                                            .gap(6.0)
                                            .padding_xy(10.0, 6.0)
                                            .radius(6.0)
                                            .bg(bg)
                                            .border(1.0, border_c),
                                        onpress: move |_| {
                                            navigate_to(state, target_route.clone());
                                        },
                                        text {
                                            nve_style: TypedStyle::new().font_size(13.0).font_weight(500).color(fg),
                                            "{sec_label}"
                                        }
                                        if let Some(badge_txt) = sec_badge {
                                            view {
                                                nve_style: TypedStyle::new()
                                                    .padding_xy(5.0, 1.0)
                                                    .radius(999.0)
                                                    .bg(theme.surface.subtle)
                                                    .border(1.0, theme.border.default),
                                                text {
                                                    nve_style: TypedStyle::new().font_size(10.0).font_weight(600).color(theme.status.info.text),
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
                nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(8.0),

                pressable {
                    test_id: "docs-search-trigger",
                    accessibility_role: "button",
                    accessibility_label: "Search documentation (Command K)",
                    nve_style: TypedStyle::new()
                        .flex_row()
                        .align_items(FlexAlign::Center)
                        .justify_content(FlexJustify::SpaceBetween)
                        .gap(14.0)
                        .padding_xy(10.0, 6.0)
                        .radius(8.0)
                        .bg(theme.surface.subtle)
                        .border(1.0, theme.border.default),
                    onpress: move |_| {
                        state.write().open_search();
                    },
                    text {
                        nve_show_on: ShowOn(MediaQuery::min_width(960)),
                        nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary),
                        "Search docs..."
                    }
                    text {
                        nve_show_on: ShowOn(MediaQuery::max_width(959)),
                        nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary),
                        "Search"
                    }
                    view {
                        nve_show_on: ShowOn(MediaQuery::min_width(960)),
                        nve_style: TypedStyle::new()
                            .padding_xy(6.0, 2.0)
                            .radius(4.0)
                            .bg(theme.surface.base)
                            .border(1.0, theme.border.default),
                        text {
                            nve_style: TypedStyle::new().font_size(11.0).font_weight(600).color(theme.text.secondary),
                            "⌘K"
                        }
                    }
                }

                view {
                    test_id: "docs-theme-mode",
                    accessibility_role: "radiogroup",
                    accessibility_label: "Color theme",
                    nve_style: TypedStyle::new()
                        .flex_row()
                        .gap(2.0)
                        .padding(2.0)
                        .radius(8.0)
                        .bg(theme.surface.subtle)
                        .border(1.0, theme.border.default),
                    for mode in ThemeMode::ALL {
                        ThemeModeOption { key: "{mode.as_str()}", mode, selected: mode == theme_mode }
                    }
                }

                if let Some(repo_url) = github_url {
                    pressable {
                        test_id: "docs-github-link",
                        nve_show_on: ShowOn(MediaQuery::min_width(960)),
                        accessibility_role: "link",
                        accessibility_label: "GitHub repository",
                        "href": "{repo_url}",
                        nve_style: TypedStyle::new()
                            .padding_xy(10.0, 6.0)
                            .radius(8.0)
                            .bg(theme.surface.subtle)
                            .border(1.0, theme.border.default),
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(theme.text.secondary),
                            "GitHub"
                        }
                    }
                }
            }
        }
    }
}
