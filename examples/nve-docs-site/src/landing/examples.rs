//! Examples Gallery Page (`/examples/`) showcasing standalone lazy-loaded WASM apps
//! (`/examples/studio/`, `/examples/kitchen-sink/`) and all 5 live interactive docs playgrounds.

use nve_dioxus::prelude::*;
use nve_docs::{DocsRoute, navigate_to, use_docs_state};

use crate::landing::{PillBadge, SiteFooter, resolve_example_app_href};

struct PlaygroundCardSpec {
    id: &'static str,
    tab_label: &'static str,
    title: &'static str,
    guide_route: &'static str,
    summary: &'static str,
    badges: &'static [&'static str],
}

const PLAYGROUND_SPECS: [PlaygroundCardSpec; 5] = [
    PlaygroundCardSpec {
        id: "ui-playground",
        tab_label: "UI Kitchen Sink",
        title: "nve-ui Component Kitchen Sink",
        guide_route: "/docs/guides/ui-components",
        summary: "Interactive Button variants, Switch, Checkbox, Input, Tabs, Badge, and Toast notifications driven by UiTheme tokens.",
        badges: &["nve-ui", "WAI-ARIA", "Zero CSS"],
    },
    PlaygroundCardSpec {
        id: "virtual-list-100k",
        tab_label: "100k VirtualList",
        title: "100,000-Row Fenwick VirtualList",
        guide_route: "/docs/guides/ui-components",
        summary: "O(log N) prefix-sum virtualized scrolling with instant row jump (#75,000) and live severity filtering across 100k items.",
        badges: &["100k Fenwick", "O(log N)", "60fps"],
    },
    PlaygroundCardSpec {
        id: "router-sandbox",
        tab_label: "Router Sandbox",
        title: "nve-router Stack, Tabs & Guard Sandbox",
        guide_route: "/docs/guides/routing",
        summary: "Test push/pop history stacks, declarative auth redirect guards, and modal overlays shared across Web History & native.",
        badges: &["nve-router", "Auth Guards", "Modals"],
    },
    PlaygroundCardSpec {
        id: "canvas-draw1",
        tab_label: "NVE_DRAW1 Canvas",
        title: "NVE_DRAW1 Vector Canvas",
        guide_route: "/docs/guides/canvas-draw1",
        summary: "Switch sparkline/bar/radial scenes, color palettes, and stroke widths compiled directly to binary NVE_DRAW1 draw op buffers.",
        badges: &["NVE_DRAW1", "Zero JSON", "Vector"],
    },
    PlaygroundCardSpec {
        id: "crdt-simulator",
        tab_label: "NVC1 CRDT Sync",
        title: "Two-Peer Local-First NVC1 CRDT Simulator",
        guide_route: "/docs/guides/local-first-crdt",
        summary: "Edit Peer A and Peer B offline while partitioned, then merge binary NVC1 deltas across LwwRegister, PnCounter, and OrSet.",
        badges: &["NVC1 CRDT", "Local-First", "Binary Delta"],
    },
];

/// Full-bleed Examples Gallery rendered at `/examples/`.
#[component]
pub fn ExamplesGalleryPage() -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let config = snap.config.clone();
    let playgrounds = snap.playgrounds.clone();
    drop(snap);

    let mut active_pg_id = use_signal(|| "ui-playground");
    let selected_id = active_pg_id();

    let outer_resp = TypedStyle::new().at(
        MediaQuery::max_width(959),
        ResponsiveStylePatch::new().padding_xy(16.0, 40.0),
    );
    let mobile_col = TypedStyle::new()
        .at(MediaQuery::max_width(959), ResponsiveStylePatch::new().flex_col());
    let h1_resp = TypedStyle::new().at(
        MediaQuery::max_width(959),
        ResponsiveStylePatch::new().font_size(30.0),
    );

    let studio_href = resolve_example_app_href(&config.base_path, "/examples/studio");
    let kitchen_sink_href = resolve_example_app_href(&config.base_path, "/examples/kitchen-sink");
    let selected_spec = PLAYGROUND_SPECS
        .iter()
        .find(|p| p.id == selected_id)
        .unwrap_or(&PLAYGROUND_SPECS[0]);
    let selected_guide_href = config.resolve_href(selected_spec.guide_route);
    let selected_guide_route = DocsRoute::new(selected_spec.guide_route);
    let selected_entry = playgrounds.get(selected_spec.id).cloned();

    rsx! {
        view {
            test_id: "examples-gallery-page",
            style: "width: 100%; flex-direction: column; align-items: center; background-color: {theme.canvas}; color: {theme.text};",

            view {
                nve_style: outer_resp,
                style: "width: 100%; max-width: 1180px; flex-direction: column; padding-horizontal: 32px; padding-vertical: 40px; gap: 44px;",

                // Gallery Header
                view {
                    test_id: "examples-hero",
                    style: "width: 100%; flex-direction: column; gap: 14px; max-width: 780px;",
                    view {
                        style: "flex-direction: row; align-items: center; gap: 8px; flex-wrap: wrap;",
                        PillBadge { label: "Lazy-Loaded WASM Apps".to_string(), accent: true }
                        PillBadge { label: "5 Live Embedded Playgrounds".to_string(), secondary: true }
                    }
                    view {
                        test_id: "examples-hero-h1",
                        accessibility_role: "heading",
                        "aria-level": "1",
                        text {
                            nve_style: h1_resp,
                            style: "font-size: 40px; font-weight: 800; color: {theme.text};",
                            "Interactive Examples & Live Playgrounds"
                        }
                    }
                    text {
                        style: "font-size: 16px; color: {theme.muted}; line-height: 1.55;",
                        "Run full NVE applications compiled to WebAssembly on demand — keeping the landing page and documentation ultra-fast — or experiment directly inside the live embedded playgrounds below."
                    }
                }

                // Section 1: Standalone Lazy-Loaded WASM Apps
                view {
                    test_id: "examples-wasm-apps",
                    style: "width: 100%; flex-direction: column; gap: 16px;",
                    view {
                        accessibility_role: "heading",
                        "aria-level": "2",
                        text {
                            style: "font-size: 20px; font-weight: 700; color: {theme.text};",
                            "Standalone Live WASM Applications"
                        }
                    }

                    view {
                        nve_style: mobile_col,
                        style: "width: 100%; flex-direction: row; gap: 18px;",

                        // Studio App Card
                        view {
                            test_id: "example-card-studio",
                            style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0; flex-direction: column; justify-content: space-between; gap: 20px; padding: 24px; border-radius: 14px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                            view {
                                style: "flex-direction: column; gap: 12px;",
                                view {
                                    style: "flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px;",
                                    text {
                                        style: "font-size: 11px; font-weight: 700; color: {theme.accent};",
                                        "FLAGSHIP WORKBENCH"
                                    }
                                    text {
                                        style: "font-size: 11px; font-family: monospace; color: {theme.muted};",
                                        "/examples/studio/"
                                    }
                                }
                                text {
                                    style: "font-size: 22px; font-weight: 800; color: {theme.text};",
                                    "Aether Workspace + NVE Studio"
                                }
                                text {
                                    style: "font-size: 14px; color: {theme.muted}; line-height: 1.55;",
                                    "Full multi-pane issue tracker, docs workbench, live X-Ray layout & FFI inspector, 100,000-row virtualized table, binary NVC1 CRDT sync, and NVE_DRAW1 telemetry canvas running in WebAssembly."
                                }
                                view {
                                    style: "flex-direction: row; flex-wrap: wrap; gap: 6px; padding-top: 2px;",
                                    PillBadge { label: "WASM".to_string(), accent: true }
                                    PillBadge { label: "Native Desktop".to_string() }
                                    PillBadge { label: "100k Fenwick".to_string(), secondary: true }
                                    PillBadge { label: "NVC1 CRDT".to_string(), secondary: true }
                                    PillBadge { label: "NVE_DRAW1".to_string() }
                                }
                            }
                            view {
                                style: "flex-direction: row; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 10px;",
                                pressable {
                                    test_id: "example-launch-studio",
                                    accessibility_role: "link",
                                    "href": "{studio_href}",
                                    "data-nve-hard-nav": "true",
                                    style: "padding-horizontal: 18px; padding-vertical: 10px; border-radius: 9px; background-color: {theme.accent}; border-width: 1px; border-color: {theme.accent};",
                                    text {
                                        style: "font-size: 14px; font-weight: 700; color: {theme.accent_fg};",
                                        "Launch Live Studio →"
                                    }
                                }
                                text {
                                    style: "font-size: 12px; font-family: monospace; color: {theme.muted};",
                                    "Lazy-loaded · Portal Bar"
                                }
                            }
                        }

                        // Kitchen Sink Gallery Card
                        view {
                            test_id: "example-card-kitchen-sink",
                            style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0; flex-direction: column; justify-content: space-between; gap: 20px; padding: 24px; border-radius: 14px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                            view {
                                style: "flex-direction: column; gap: 12px;",
                                view {
                                    style: "flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px;",
                                    text {
                                        style: "font-size: 11px; font-weight: 700; color: {theme.accent_alt};",
                                        "COMPONENT GALLERY"
                                    }
                                    text {
                                        style: "font-size: 11px; font-family: monospace; color: {theme.muted};",
                                        "/examples/kitchen-sink/"
                                    }
                                }
                                text {
                                    style: "font-size: 22px; font-weight: 800; color: {theme.text};",
                                    "NVE Kitchen Sink Gallery"
                                }
                                text {
                                    style: "font-size: 14px; color: {theme.muted}; line-height: 1.55;",
                                    "Interactive showcase of every nve-ui component and engine primitive: Dark/Light themes, UiDensity, live nve-i18n locales & RTL, overlays, 1k VirtualList, NVE_DRAW1 Canvas, and UiErrorBoundary recovery."
                                }
                                view {
                                    style: "flex-direction: row; flex-wrap: wrap; gap: 6px; padding-top: 2px;",
                                    PillBadge { label: "All nve-ui".to_string(), accent: true }
                                    PillBadge { label: "nve-i18n + RTL".to_string() }
                                    PillBadge { label: "Zero JSON".to_string(), secondary: true }
                                    PillBadge { label: "ErrorBoundary".to_string() }
                                }
                            }
                            view {
                                style: "flex-direction: row; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 10px;",
                                pressable {
                                    test_id: "example-launch-kitchen-sink",
                                    accessibility_role: "link",
                                    "href": "{kitchen_sink_href}",
                                    "data-nve-hard-nav": "true",
                                    style: "padding-horizontal: 18px; padding-vertical: 10px; border-radius: 9px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                                    text {
                                        style: "font-size: 14px; font-weight: 700; color: {theme.text};",
                                        "Launch Kitchen Sink →"
                                    }
                                }
                                text {
                                    style: "font-size: 12px; font-family: monospace; color: {theme.muted};",
                                    "Lazy-loaded · Portal Bar"
                                }
                            }
                        }
                    }
                }

                // Section 2: Interactive Live Playground Stage (all 5 playgrounds right here!)
                view {
                    test_id: "examples-playgrounds-section",
                    style: "width: 100%; flex-direction: column; gap: 16px;",

                    view {
                        style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: flex-end; flex-wrap: wrap; gap: 10px;",
                        view {
                            style: "flex-direction: column; gap: 4px;",
                            view {
                                accessibility_role: "heading",
                                "aria-level": "2",
                                text {
                                    style: "font-size: 20px; font-weight: 700; color: {theme.text};",
                                    "Live Embedded Playgrounds"
                                }
                            }
                            text {
                                style: "font-size: 13px; color: {theme.muted};",
                                "Select any subsystem below to interact immediately in-page, or open its full technical guide."
                            }
                        }
                        pressable {
                            test_id: "examples-open-guide-link",
                            accessibility_role: "link",
                            "href": "{selected_guide_href}",
                            style: "padding-horizontal: 12px; padding-vertical: 7px; border-radius: 8px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                            onpress: move |_| {
                                navigate_to(state, selected_guide_route.clone());
                            },
                            text {
                                style: "font-size: 13px; font-weight: 600; color: {theme.accent};",
                                "Open Full Guide →"
                            }
                        }
                    }

                    // Playground switcher tabs
                    view {
                        accessibility_role: "tablist",
                        style: "width: 100%; flex-direction: row; flex-wrap: wrap; gap: 8px;",
                        for spec in &PLAYGROUND_SPECS {
                            {
                                let is_sel = spec.id == selected_id;
                                let bg = if is_sel { theme.accent_surface } else { theme.surface };
                                let fg = if is_sel { theme.accent } else { theme.muted };
                                let bc = if is_sel { theme.accent } else { theme.border };
                                let id_val = spec.id;
                                let sel_attr = if is_sel { "true" } else { "false" };
                                rsx! {
                                    pressable {
                                        key: "gallery-pg-tab-{spec.id}",
                                        test_id: "examples-pg-tab-{spec.id}",
                                        accessibility_role: "tab",
                                        aria_selected: "{sel_attr}",
                                        style: "flex-shrink: 0; padding-horizontal: 14px; padding-vertical: 8px; border-radius: 8px; background-color: {bg}; border-width: 1px; border-color: {bc};",
                                        onpress: move |_| {
                                            active_pg_id.set(id_val);
                                        },
                                        text {
                                            style: "font-size: 13px; font-weight: 600; color: {fg};",
                                            "{spec.tab_label}"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Active live playground preview card
                    view {
                        test_id: "examples-live-stage",
                        style: "width: 100%; flex-direction: column; border-radius: 14px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border}; overflow: hidden;",

                        view {
                            style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px; padding-horizontal: 18px; padding-vertical: 12px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                            view {
                                style: "flex-direction: column; gap: 2px;",
                                text {
                                    style: "font-size: 14px; font-weight: 700; color: {theme.text};",
                                    "{selected_spec.title}"
                                }
                                text {
                                    style: "font-size: 12px; color: {theme.muted};",
                                    "{selected_spec.summary}"
                                }
                            }
                            view {
                                style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                                for b in selected_spec.badges {
                                    PillBadge { key: "stage-badge-{b}", label: (*b).to_string() }
                                }
                            }
                        }

                        view {
                            style: "width: 100%; padding: 20px; background-color: {theme.canvas};",
                            if let Some(entry) = selected_entry {
                                {(entry.render)()}
                            }
                        }
                    }
                }

                SiteFooter {}
            }
        }
    }
}
