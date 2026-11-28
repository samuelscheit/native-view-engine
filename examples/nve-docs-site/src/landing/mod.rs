//! Linear × Vercel × Zed dark-first Landing Page (`/`) and Examples Gallery (`/examples/`)
//! rendered 100% through NVE SSR + progressive WASM hydration.

pub mod benchmarks_chart;
pub mod benchmarks_data;
pub mod bento;
pub mod examples;
pub mod hero;
pub mod progression;

use std::sync::{Arc, OnceLock};
use nve_ui::Separator;

use nve_dioxus::prelude::*;
use nve_docs::{CustomPageRegistry, DocsRoute, navigate_to, use_docs_state};

use crate::site::GITHUB_REPO_URL;

pub use examples::ExamplesGalleryPage;

/// Build and cache the custom full-bleed page registry (`/` Landing + `/examples` Gallery).
pub fn nve_site_custom_pages() -> Arc<CustomPageRegistry> {
    static PAGES: OnceLock<Arc<CustomPageRegistry>> = OnceLock::new();
    Arc::clone(PAGES.get_or_init(|| {
        let mut reg = CustomPageRegistry::new();
        reg.register("/", || rsx! { LandingPage {} });
        reg.register("/examples", || rsx! { ExamplesGalleryPage {} });
        Arc::new(reg)
    }))
}

/// Public Landing Page rendered at `/` inside the unified `nve-docs` shell.
#[component]
pub fn LandingPage() -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    drop(snap);

    let band_style = TypedStyle::new()
        .width_pct(100.0)
        .flex_col()
        .align_items(FlexAlign::Center)
        .padding_x(32.0)
        .at(MediaQuery::max_width(959), RowPatch::new().padding_xy(16.0, 0.0));

    rsx! {
        view {
            test_id: "landing-page",
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_col()
                .align_items(FlexAlign::Center)
                .bg(theme.surface.canvas)
                .color(theme.text.primary),

            // Subtle top ambient gradient layer for the Hero section
            view {
                test_id: "landing-hero-glow",
                nve_style: band_style.clone(),
                hero::LandingHero {}
            }

            // Bento Architecture & Verified Benchmark Grid + Dev & Benchmark Progression + Live
            // Showcase + Footer. Same gutter-outside / 1180px-content structure as the hero glow
            // above, so every section shares the hero's left edge.
            view {
                nve_style: band_style,
                view {
                    nve_style: TypedStyle::new().width_pct(100.0).max_width_px(1180.0).flex_col().gap(56.0).padding_bottom(64.0),
                    bento::BentoArchitectureGrid {}
                    progression::DevAndBenchProgression {}
                    bento::LiveExamplesStrip {}
                    SiteFooter {}
                }
            }
        }
    }
}

/// Compact pill badge used across Landing and Examples sections.
#[component]
pub(crate) fn PillBadge(
    label: String,
    #[props(default = false)] accent: bool,
    #[props(default = false)] secondary: bool,
) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();

    let (bg, border, fg) = if accent {
        (theme.status.accent.subtle, theme.status.accent.border, theme.status.accent.text)
    } else if secondary {
        (theme.surface.subtle, theme.border.default, theme.status.info.text)
    } else {
        (theme.surface.subtle, theme.border.default, theme.text.secondary)
    };

    rsx! {
        view {
            nve_style: TypedStyle::new().flex_shrink(0.0).padding_xy(9.0, 3.0).radius(999.0).bg(bg).border(1.0, border),
            text {
                nve_style: TypedStyle::new().font_size(11.0).font_weight(600).color(fg),
                "{label}"
            }
        }
    }
}

/// Clean multi-column footer shared by `/` and `/examples/`.
#[component]
pub(crate) fn SiteFooter() -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let config = snap.config.clone();
    drop(snap);

    let footer_top_row = TypedStyle::new()
        .width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween).gap(28.0)
        .at(MediaQuery::max_width(959), RowPatch::new().flex_col());
    let footer_cols_row = TypedStyle::new()
        .flex_grow(1.0).flex_row().justify_content(FlexJustify::SpaceBetween).gap(20.0).flex_wrap(FlexWrap::Wrap)
        .at(MediaQuery::max_width(959), RowPatch::new().flex_col());

    type FooterLink = (&'static str, &'static str, bool);
    let columns: [(&'static str, &[FooterLink]); 4] = [
        (
            "Getting Started",
            &[
                ("Introduction", "/docs", false),
                ("Quickstart", "/docs/getting-started/quickstart", false),
                ("Why NVE?", "/docs/getting-started/why-nve", false),
                ("Live Examples", "/examples", false),
            ],
        ),
        (
            "Architecture",
            &[
                ("Modular C ABI", "/docs/architecture/modular-c-abi", false),
                (
                    "CommandBuffer & Events",
                    "/docs/architecture/command-buffer-and-events",
                    false,
                ),
                ("Scene ABI", "/docs/architecture/scene-abi", false),
                ("Hot Reload & Pairing", "/docs/architecture/hot-reload", false),
            ],
        ),
        (
            "Hosts (All 10)",
            &[
                ("Platform Matrix", "/docs/hosts/overview", false),
                ("Apple AppKit & UIKit", "/docs/hosts/apple-macos-ios", false),
                ("Windows RNW & D2D", "/docs/hosts/windows", false),
                ("Web WASM & SSR", "/docs/hosts/web-wasm", false),
            ],
        ),
        (
            "Reference",
            &[
                ("CLI Reference", "/docs/reference/cli", false),
                ("Benchmarks & Profiling", "/docs/reference/benchmarks", false),
                ("Live NVE Studio", "/examples/studio/", true),
                ("GitHub Repository ↗", GITHUB_REPO_URL, true),
            ],
        ),
    ];

    rsx! {
        view {
            test_id: "landing-footer",
            accessibility_role: "contentinfo",
            nve_style: TypedStyle::new()
                .width_pct(100.0).flex_col().gap(28.0)
                .border(1.0, theme.border.default).radius(14.0).padding(28.0).bg(theme.surface.base),

            view {
                nve_style: footer_top_row,

                // Brand summary column
                view {
                    nve_style: TypedStyle::new().flex_col().gap(10.0).max_width_px(280.0),
                    view {
                        nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(8.0),
                        view {
                            nve_style: TypedStyle::new().padding_xy(7.0, 3.0).radius(6.0).bg(theme.status.accent.subtle).border(1.0, theme.status.accent.border),
                            text {
                                nve_style: TypedStyle::new().font_size(11.0).font_weight(700).color(theme.text.link),
                                "NVE"
                            }
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(15.0).font_weight(700).color(theme.text.primary),
                            "Native View Engine"
                        }
                    }
                    text {
                        nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary).line_height_ratio(1.5),
                        "Binary Rust + C++20 multi-backend UI runtime driving real native views, GPU Skia canvases, and Web WASM SSR from one binary ABI."
                    }
                }

                // 4 link columns
                view {
                    nve_style: footer_cols_row,
                    for (heading, links) in columns {
                        view {
                            key: "footer-col-{heading}",
                            nve_style: TypedStyle::new().flex_col().gap(10.0).min_width_px(136.0),
                            text {
                                nve_style: TypedStyle::new().font_size(12.0).font_weight(700).color(theme.text.primary),
                                "{heading}"
                            }
                            view {
                                nve_style: TypedStyle::new().flex_col().gap(7.0),
                                for (label, raw_target, hard_nav) in links {
                                    {
                                        let is_ext = raw_target.starts_with("https://");
                                        let href = if is_ext {
                                            (*raw_target).to_string()
                                        } else if *hard_nav {
                                            resolve_example_app_href(&config.base_path, raw_target)
                                        } else {
                                            config.resolve_href(raw_target)
                                        };
                                        let spa_target = (!is_ext && !*hard_nav)
                                            .then(|| DocsRoute::new(raw_target));
                                        let is_hard = *hard_nav && !is_ext;
                                        rsx! {
                                            pressable {
                                                key: "footer-link-{label}",
                                                accessibility_role: "link",
                                                "href": "{href}",
                                                nve_table: nve_dioxus::table_attr_fn(move |tx, node| {
                                                    if is_hard {
                                                        tx.edit::<nve_web_html::HtmlDataAttrs>(node).entries(Some(
                                                            nve_web_html::DataAttrList::from_pairs(vec![(
                                                                "data-nve-hard-nav".to_owned(),
                                                                "true".to_owned(),
                                                            )]),
                                                        ));
                                                    }
                                                }),
                                                onpress: move |_| {
                                                    if let Some(ref rt) = spa_target {
                                                        navigate_to(state, rt.clone());
                                                    }
                                                },
                                                text {
                                                    nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary),
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
            }

            // Bottom hairline + build note (a divided row inside the footer card, not a nested card)
            view {
                nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(16.0),
                Separator {}
                view {
                    nve_style: TypedStyle::new().width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::Center).flex_wrap(FlexWrap::Wrap).gap(8.0),
                    text {
                        nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                        "Rendered 100% with Native View Engine · Semantic HTML5 SSR + Zero-Flicker WASM Hydration"
                    }
                    text {
                        nve_style: TypedStyle::new().font_size(12.0).font_family("monospace").color(theme.text.link),
                        "nve_get_interface(1..=5) · 0 JSON"
                    }
                }
            }
        }
    }
}

/// Build a trailing-slash href for standalone lazy-loaded example apps (`/examples/studio/`, `/examples/kitchen-sink/`).
pub(crate) fn resolve_example_app_href(base_path: &str, app_path: &str) -> String {
    let clean_base = base_path.trim().trim_matches('/');
    let clean_app = app_path.trim().trim_matches('/');
    if clean_base.is_empty() {
        format!("/{clean_app}/")
    } else {
        format!("/{clean_base}/{clean_app}/")
    }
}
