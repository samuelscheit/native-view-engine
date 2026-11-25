//! Linear × Vercel × Zed dark-first Landing Page (`/`) and Examples Gallery (`/examples/`)
//! rendered 100% through NVE SSR + progressive WASM hydration.

pub mod benchmarks_chart;
pub mod benchmarks_data;
pub mod bento;
pub mod examples;
pub mod hero;
pub mod progression;

use std::sync::{Arc, OnceLock};

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
    let is_mobile = snap.viewport.is_mobile();
    drop(snap);

    let outer_pad_h = if is_mobile { "16px" } else { "32px" };

    rsx! {
        view {
            test_id: "landing-page",
            style: "width: 100%; flex-direction: column; align-items: center; background-color: {theme.canvas}; color: {theme.text};",

            // Subtle top ambient gradient layer for the Hero section
            view {
                test_id: "landing-hero-glow",
                style: "width: 100%; flex-direction: column; align-items: center; padding-horizontal: {outer_pad_h};",
                hero::LandingHero {}
            }

            // Bento Architecture & Verified Benchmark Grid + Dev & Benchmark Progression + Live Showcase + Footer
            view {
                style: "width: 100%; max-width: 1180px; flex-direction: column; padding-horizontal: {outer_pad_h}; gap: 56px; padding-bottom: 64px;",
                bento::BentoArchitectureGrid {}
                progression::DevAndBenchProgression {}
                bento::LiveExamplesStrip {}
                SiteFooter {}
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
        (theme.accent_surface, theme.accent, theme.accent)
    } else if secondary {
        (theme.surface_subtle, theme.border, theme.accent_alt)
    } else {
        (theme.surface_subtle, theme.border, theme.muted)
    };

    rsx! {
        view {
            style: "flex-shrink: 0; padding-horizontal: 9px; padding-vertical: 3px; border-radius: 999px; background-color: {bg}; border-width: 1px; border-color: {border};",
            text {
                style: "font-size: 11px; font-weight: 600; color: {fg};",
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
    let is_mobile = snap.viewport.is_mobile();
    let config = snap.config.clone();
    drop(snap);

    let cols_dir = if is_mobile { "column" } else { "row" };

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
                ("64-Byte Scene ABI", "/docs/architecture/scene-abi", false),
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
            style: "width: 100%; flex-direction: column; gap: 28px; padding-top: 32px; padding-bottom: 16px; border-top-width: 1px; border-width: 1px; border-color: {theme.border}; border-radius: 14px; padding: 28px; background-color: {theme.surface};",

            view {
                style: "width: 100%; flex-direction: {cols_dir}; justify-content: space-between; gap: 28px;",

                // Brand summary column
                view {
                    style: "flex-direction: column; gap: 10px; max-width: 280px;",
                    view {
                        style: "flex-direction: row; align-items: center; gap: 8px;",
                        view {
                            style: "padding-horizontal: 7px; padding-vertical: 3px; border-radius: 6px; background-color: {theme.accent_surface}; border-width: 1px; border-color: {theme.accent};",
                            text {
                                style: "font-size: 11px; font-weight: 700; color: {theme.accent};",
                                "NVE"
                            }
                        }
                        text {
                            style: "font-size: 15px; font-weight: 700; color: {theme.text};",
                            "Native View Engine"
                        }
                    }
                    text {
                        style: "font-size: 13px; color: {theme.muted}; line-height: 1.5;",
                        "Zero-JSON Rust + C++20 multi-backend UI runtime driving real native views, GPU Skia canvases, and Web WASM SSR from one binary ABI."
                    }
                }

                // 4 link columns
                view {
                    style: "flex-grow: 1; flex-direction: {cols_dir}; justify-content: space-between; gap: 20px; flex-wrap: wrap;",
                    for (heading, links) in columns {
                        view {
                            key: "footer-col-{heading}",
                            style: "flex-direction: column; gap: 10px; min-width: 136px;",
                            text {
                                style: "font-size: 12px; font-weight: 700; color: {theme.text};",
                                "{heading}"
                            }
                            view {
                                style: "flex-direction: column; gap: 7px;",
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
                                        let hard_attr = (*hard_nav && !is_ext)
                                            .then(|| "true".to_string());
                                        rsx! {
                                            pressable {
                                                key: "footer-link-{label}",
                                                accessibility_role: "link",
                                                "href": "{href}",
                                                "data-nve-hard-nav": hard_attr,
                                                onpress: move |_| {
                                                    if let Some(ref rt) = spa_target {
                                                        navigate_to(state, rt.clone());
                                                    }
                                                },
                                                text {
                                                    style: "font-size: 13px; color: {theme.muted};",
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

            // Bottom hairline copyright / build note
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px; padding-top: 16px; border-width: 1px; border-color: {theme.border}; border-radius: 8px; padding-horizontal: 14px; padding-vertical: 10px; background-color: {theme.canvas};",
                text {
                    style: "font-size: 12px; color: {theme.muted};",
                    "Rendered 100% with Native View Engine · Semantic HTML5 SSR + Zero-Flicker WASM Hydration"
                }
                text {
                    style: "font-size: 12px; font-family: monospace; color: {theme.accent};",
                    "nve_get_interface(1..=5) · 0 JSON"
                }
            }
        }
    }
}

/// Build a trailing-slash href for standalone lazy-loaded example apps (`/examples/studio/`, `/examples/counter/`).
pub(crate) fn resolve_example_app_href(base_path: &str, app_path: &str) -> String {
    let clean_base = base_path.trim().trim_matches('/');
    let clean_app = app_path.trim().trim_matches('/');
    if clean_base.is_empty() {
        format!("/{clean_app}/")
    } else {
        format!("/{clean_base}/{clean_app}/")
    }
}
