//! Examples Gallery Page (`/examples/`) showcasing standalone lazy-loaded WASM apps
//! (`/examples/studio/`, `/examples/kitchen-sink/`) and all 5 live interactive docs playgrounds.

use nve_dioxus::prelude::*;
use nve_ui::accent_indicator;
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
        summary: "Interactive Button variants, Switch, Checkbox, Input, Tabs, Badge, and Toast notifications driven by nve_ui::Theme tokens.",
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
        badges: &["NVE_DRAW1", "Binary ABI", "Vector"],
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

    let outer_style = TypedStyle::new()
        .width_pct(100.0)
        .max_width_px(1180.0)
        .flex_col()
        .padding_xy(32.0, 40.0)
        .gap(44.0)
        .at(
            MediaQuery::max_width(959),
            RowPatch::new().padding_xy(16.0, 40.0),
        );
    let mobile_col_row = TypedStyle::new()
        .width_pct(100.0)
        .flex_row()
        .gap(18.0)
        .at(MediaQuery::max_width(959), RowPatch::new().flex_col());
    let h1_style = TypedStyle::new()
        .font_size(40.0)
        .font_weight(800)
        .color(theme.text.primary)
        .at(
            MediaQuery::max_width(959),
            RowPatch::new().font_size(30.0),
        );
    let app_card_style = TypedStyle::new()
        .flex_1().min_width_px(0.0).flex_col().justify_content(FlexJustify::SpaceBetween)
        .gap(20.0).padding(24.0).radius(14.0).bg(theme.surface.base).border(1.0, theme.border.default);

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
            nve_style: TypedStyle::new()
                .width_pct(100.0).flex_col().align_items(FlexAlign::Center)
                .bg(theme.surface.canvas).color(theme.text.primary),

            view {
                nve_style: outer_style,

                // Gallery Header
                view {
                    test_id: "examples-hero",
                    nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(14.0).max_width_px(780.0),
                    view {
                        nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(8.0).flex_wrap(FlexWrap::Wrap),
                        PillBadge { label: "Lazy-Loaded WASM Apps".to_string(), accent: true }
                        PillBadge { label: "5 Live Embedded Playgrounds".to_string(), secondary: true }
                    }
                    view {
                        test_id: "examples-hero-h1",
                        accessibility_role: "heading",
                        "aria-level": "1",
                        text {
                            nve_style: h1_style,
                            "Interactive Examples & Live Playgrounds"
                        }
                    }
                    text {
                        nve_style: TypedStyle::new().font_size(16.0).color(theme.text.secondary).line_height_ratio(1.55),
                        "Run full NVE applications compiled to WebAssembly on demand — keeping the landing page and documentation ultra-fast — or experiment directly inside the live embedded playgrounds below."
                    }
                }

                // Section 1: Standalone Lazy-Loaded WASM Apps
                view {
                    test_id: "examples-wasm-apps",
                    nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(16.0),
                    view {
                        accessibility_role: "heading",
                        "aria-level": "2",
                        text {
                            nve_style: TypedStyle::new().font_size(20.0).font_weight(700).color(theme.text.primary),
                            "Standalone Live WASM Applications"
                        }
                    }

                    view {
                        nve_style: mobile_col_row,

                        // Studio App Card
                        view {
                            test_id: "example-card-studio",
                            nve_style: app_card_style.clone(),
                            view {
                                nve_style: TypedStyle::new().flex_col().gap(12.0),
                                view {
                                    nve_style: TypedStyle::new().flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::Center).flex_wrap(FlexWrap::Wrap).gap(8.0),
                                    text {
                                        nve_style: TypedStyle::new().font_size(11.0).font_weight(700).color(theme.text.link),
                                        "FLAGSHIP WORKBENCH"
                                    }
                                    text {
                                        nve_style: TypedStyle::new().font_size(11.0).font_family("monospace").color(theme.text.secondary),
                                        "/examples/studio/"
                                    }
                                }
                                text {
                                    nve_style: TypedStyle::new().font_size(22.0).font_weight(800).color(theme.text.primary),
                                    "Aether Workspace + NVE Studio"
                                }
                                text {
                                    nve_style: TypedStyle::new().font_size(14.0).color(theme.text.secondary).line_height_ratio(1.55),
                                    "Full multi-pane issue tracker, docs workbench, live X-Ray layout & FFI inspector, 100,000-row virtualized table, binary NVC1 CRDT sync, and NVE_DRAW1 telemetry canvas running in WebAssembly."
                                }
                                view {
                                    nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(6.0).padding_top(2.0),
                                    PillBadge { label: "WASM".to_string(), accent: true }
                                    PillBadge { label: "Native Desktop".to_string() }
                                    PillBadge { label: "100k Fenwick".to_string(), secondary: true }
                                    PillBadge { label: "NVC1 CRDT".to_string(), secondary: true }
                                    PillBadge { label: "NVE_DRAW1".to_string() }
                                }
                            }
                            view {
                                nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).justify_content(FlexJustify::SpaceBetween).flex_wrap(FlexWrap::Wrap).gap(10.0),
                                pressable {
                                    test_id: "example-launch-studio",
                                    accessibility_role: "link",
                                    "href": "{studio_href}",
                                    nve_table: nve_dioxus::table_attr_fn(|tx, node| {
                                        tx.edit::<nve_web_html::HtmlDataAttrs>(node).entries(Some(
                                            nve_web_html::DataAttrList::from_pairs(vec![(
                                                "data-nve-hard-nav".to_owned(),
                                                "true".to_owned(),
                                            )]),
                                        ));
                                    }),
                                    nve_style: TypedStyle::new().padding_xy(18.0, 10.0).radius(9.0).bg(theme.status.accent.solid).border(1.0, theme.status.accent.solid),
                                    text {
                                        nve_style: TypedStyle::new().font_size(14.0).font_weight(700).color(theme.text.on_accent),
                                        "Launch Live Studio →"
                                    }
                                }
                                text {
                                    nve_style: TypedStyle::new().font_size(12.0).font_family("monospace").color(theme.text.secondary),
                                    "Lazy-loaded · Portal Bar"
                                }
                            }
                        }

                        // Kitchen Sink Gallery Card
                        view {
                            test_id: "example-card-kitchen-sink",
                            nve_style: app_card_style,
                            view {
                                nve_style: TypedStyle::new().flex_col().gap(12.0),
                                view {
                                    nve_style: TypedStyle::new().flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::Center).flex_wrap(FlexWrap::Wrap).gap(8.0),
                                    text {
                                        nve_style: TypedStyle::new().font_size(11.0).font_weight(700).color(theme.status.info.text),
                                        "COMPONENT GALLERY"
                                    }
                                    text {
                                        nve_style: TypedStyle::new().font_size(11.0).font_family("monospace").color(theme.text.secondary),
                                        "/examples/kitchen-sink/"
                                    }
                                }
                                text {
                                    nve_style: TypedStyle::new().font_size(22.0).font_weight(800).color(theme.text.primary),
                                    "NVE Kitchen Sink Gallery"
                                }
                                text {
                                    nve_style: TypedStyle::new().font_size(14.0).color(theme.text.secondary).line_height_ratio(1.55),
                                    "Interactive showcase of every nve-ui component and engine primitive: Dark/Light themes, UiDensity, live nve-i18n locales & RTL, overlays, 1k VirtualList, NVE_DRAW1 Canvas, and UiErrorBoundary recovery."
                                }
                                view {
                                    nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(6.0).padding_top(2.0),
                                    PillBadge { label: "All nve-ui".to_string(), accent: true }
                                    PillBadge { label: "nve-i18n + RTL".to_string() }
                                    PillBadge { label: "Binary ABI".to_string(), secondary: true }
                                    PillBadge { label: "ErrorBoundary".to_string() }
                                }
                            }
                            view {
                                nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).justify_content(FlexJustify::SpaceBetween).flex_wrap(FlexWrap::Wrap).gap(10.0),
                                pressable {
                                    test_id: "example-launch-kitchen-sink",
                                    accessibility_role: "link",
                                    "href": "{kitchen_sink_href}",
                                    nve_table: nve_dioxus::table_attr_fn(|tx, node| {
                                        tx.edit::<nve_web_html::HtmlDataAttrs>(node).entries(Some(
                                            nve_web_html::DataAttrList::from_pairs(vec![(
                                                "data-nve-hard-nav".to_owned(),
                                                "true".to_owned(),
                                            )]),
                                        ));
                                    }),
                                    nve_style: TypedStyle::new().padding_xy(18.0, 10.0).radius(9.0).bg(theme.surface.subtle).border(1.0, theme.border.default),
                                    text {
                                        nve_style: TypedStyle::new().font_size(14.0).font_weight(700).color(theme.text.primary),
                                        "Launch Kitchen Sink →"
                                    }
                                }
                                text {
                                    nve_style: TypedStyle::new().font_size(12.0).font_family("monospace").color(theme.text.secondary),
                                    "Lazy-loaded · Portal Bar"
                                }
                            }
                        }
                    }
                }

                // Section 2: Interactive Live Playground Stage (all 5 playgrounds right here!)
                view {
                    test_id: "examples-playgrounds-section",
                    nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(16.0),

                    view {
                        nve_style: TypedStyle::new().width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::FlexEnd).flex_wrap(FlexWrap::Wrap).gap(10.0),
                        view {
                            nve_style: TypedStyle::new().flex_col().gap(4.0),
                            view {
                                accessibility_role: "heading",
                                "aria-level": "2",
                                text {
                                    nve_style: TypedStyle::new().font_size(20.0).font_weight(700).color(theme.text.primary),
                                    "Live Embedded Playgrounds"
                                }
                            }
                            text {
                                nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary),
                                "Select any subsystem below to interact immediately in-page, or open its full technical guide."
                            }
                        }
                        pressable {
                            test_id: "examples-open-guide-link",
                            accessibility_role: "link",
                            "href": "{selected_guide_href}",
                            nve_style: TypedStyle::new().padding_xy(12.0, 7.0).radius(8.0).bg(theme.surface.base).border(1.0, theme.border.default),
                            onpress: move |_| {
                                navigate_to(state, selected_guide_route.clone());
                            },
                            text {
                                nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(theme.text.link),
                                "Open Full Guide →"
                            }
                        }
                    }

                    // Playground switcher tabs
                    view {
                        accessibility_role: "tablist",
                        nve_style: TypedStyle::new().width_pct(100.0).flex_row().flex_wrap(FlexWrap::Wrap).gap(8.0),
                        for spec in &PLAYGROUND_SPECS {
                            {
                                let is_sel = spec.id == selected_id;
                                let bg = if is_sel { theme.status.accent.subtle } else { theme.surface.base };
                                let fg = if is_sel { theme.text.link } else { theme.text.secondary };
                                let bc = if is_sel { accent_indicator(&theme) } else { theme.border.default };
                                let id_val = spec.id;
                                let sel_attr = if is_sel { "true" } else { "false" };
                                rsx! {
                                    pressable {
                                        key: "gallery-pg-tab-{spec.id}",
                                        test_id: "examples-pg-tab-{spec.id}",
                                        accessibility_role: "tab",
                                        aria_selected: "{sel_attr}",
                                        nve_style: TypedStyle::new().flex_shrink(0.0).padding_xy(14.0, 8.0).radius(8.0).bg(bg).border(1.0, bc),
                                        onpress: move |_| {
                                            active_pg_id.set(id_val);
                                        },
                                        text {
                                            nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(fg),
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
                        nve_style: TypedStyle::new()
                            .width_pct(100.0).flex_col().radius(14.0)
                            .bg(theme.surface.base).border(1.0, theme.border.default).overflow_mode(Overflow::Hidden),

                        view {
                            nve_style: TypedStyle::new()
                                .width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween)
                                .align_items(FlexAlign::Center).flex_wrap(FlexWrap::Wrap).gap(8.0)
                                .padding_xy(18.0, 12.0).bg(theme.surface.subtle).border(1.0, theme.border.default),
                            view {
                                nve_style: TypedStyle::new().flex_col().gap(2.0),
                                text {
                                    nve_style: TypedStyle::new().font_size(14.0).font_weight(700).color(theme.text.primary),
                                    "{selected_spec.title}"
                                }
                                text {
                                    nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                                    "{selected_spec.summary}"
                                }
                            }
                            view {
                                nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                                for b in selected_spec.badges {
                                    PillBadge { key: "stage-badge-{b}", label: (*b).to_string() }
                                }
                            }
                        }

                        view {
                            nve_style: TypedStyle::new().width_pct(100.0).padding(20.0).bg(theme.surface.canvas),
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
