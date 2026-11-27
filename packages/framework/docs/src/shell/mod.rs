//! 3-column responsive Fumadocs × Linear application shell (`Navbar`, `Sidebar`, `Toc`, `SearchModal`, `Footer`).

pub mod footer;
pub mod navbar;
pub mod search_modal;
pub mod sidebar;
pub mod toc;

pub use footer::{Breadcrumbs, PageFooter};
pub use navbar::DocsNavbar;
pub use search_modal::SearchModal;
pub use sidebar::{DocsSidebar, route_test_id_slug};
pub use toc::DocsToc;

use nve_abi::EdgeInsets;
use nve_ui::ColorHex;
use nve_dioxus::prelude::*;
use nve_ui::ThemeProvider;

use crate::render::DocsArticle;
use crate::state::use_docs_state;

/// Full 3-column responsive Fumadocs × Linear shell wrapping the active compiled page.
#[component]
pub fn DocsShell() -> Element {
    let mut state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let ui_theme = theme;
    let viewport = snap.viewport;
    let mobile_drawer_open = snap.mobile_drawer_open;
    let viewport_mode = viewport.mode_label();
    let site_title = snap.config.site_title.clone();
    let active_route = snap.active_route.as_str().to_string();
    let custom_page = snap.custom_pages.get(&active_route).cloned();
    let page = if custom_page.is_none() {
        snap.current_page().cloned()
    } else {
        None
    };
    let page_toc_enabled = page.as_ref().is_none_or(|p| p.frontmatter.toc);
    let color_scheme_attr = if theme.is_dark() { "dark" } else { "light" };
    let theme_mode_attr = snap.theme_mode.as_str();
    drop(snap);

    let article_container_style = TypedStyle::new()
        .width_pct(100.0)
        .max_width_px(820.0)
        .flex_col()
        .padding_xy(40.0, 32.0)
        .gap(20.0)
        .at(
            MediaQuery::max_width(959),
            ResponsiveStylePatch::new().padding_insets(EdgeInsets::symmetric(20.0, 16.0)),
        );

    rsx! {
        ThemeProvider {
            theme: ui_theme,
            view {
                test_id: "docs-root",
                accessibility_label: "{site_title}",
                nve_table: crate::render::html_data_attrs(vec![
                    ("theme".to_owned(), color_scheme_attr.to_owned()),
                    ("theme-mode".to_owned(), theme_mode_attr.to_owned()),
                    ("route".to_owned(), active_route.clone()),
                    ("viewport-mode".to_owned(), viewport_mode.to_owned()),
                ]),
                style: "width: 100%; height: 100%; flex-direction: column; background-color: {theme.surface.canvas.hex()}; color: {theme.text.primary.hex()};",
                onkeydown: move |ev: KeyEvent| {
                    let data = ev.data();
                    let is_k = data.key.eq_ignore_ascii_case("k");
                    if is_k && (data.meta || data.ctrl) {
                        let currently_open = state.read().search_open;
                        if currently_open {
                            state.write().close_search();
                        } else {
                            state.write().open_search();
                        }
                    } else if data.key == "Escape" {
                        let mut s = state.write();
                        if s.search_open {
                            s.close_search();
                        } else if s.mobile_drawer_open {
                            s.set_mobile_drawer(false);
                        }
                    }
                },

                // Sticky top navbar
                DocsNavbar {}

                // Body row: Full-bleed custom page OR 3-column Fumadocs layout
                view {
                    test_id: "docs-body",
                    style: "flex-grow: 1; flex-shrink: 1; width: 100%; flex-direction: row; background-color: {theme.surface.canvas.hex()};",

                    if let Some(render_custom) = custom_page {
                        scrollview {
                            test_id: "docs-custom-scroll",
                            accessibility_role: "main",
                            style: "flex-grow: 1; flex-shrink: 1; width: 100%; height: 100%; flex-direction: column; align-items: center; background-color: {theme.surface.canvas.hex()};",
                            {render_custom()}
                        }
                    } else {
                        DocsSidebar { mobile_overlay: false }

                        // Center content scroll column
                        scrollview {
                            test_id: "docs-main-scroll",
                            accessibility_role: "main",
                            style: "flex-grow: 1; flex-shrink: 1; min-width: 0px; height: 100%; flex-direction: column; background-color: {theme.surface.canvas.hex()};",
                            view {
                                test_id: "docs-article-container",
                                nve_style: article_container_style,

                                Breadcrumbs {}

                                if let Some(compiled_page) = page {
                                    DocsArticle { page: compiled_page }
                                } else {
                                    view {
                                        test_id: "docs-empty-page",
                                        style: "padding: 24px; border-width: 1px; border-color: {theme.border.default.hex()}; border-radius: 10px; background-color: {theme.surface.base.hex()};",
                                        text {
                                            style: "font-size: 16px; color: {theme.text.secondary.hex()};",
                                            "No documentation pages found for this route."
                                        }
                                    }
                                }

                                PageFooter {}
                            }
                        }

                        if page_toc_enabled {
                            DocsToc {}
                        }
                    }
                }

                // Mobile slide-over drawer overlay (< 960pt, hidden cleanly via CSS `@media` on desktop)
                if mobile_drawer_open {
                    DocsSidebar { mobile_overlay: true }
                }

                // Global ⌘K search dialog
                SearchModal {}
            }
        }
    }
}
