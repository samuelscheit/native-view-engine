//! Bento Architecture & Verified Benchmark Grid + Interactive Live Examples Showcase Strip.

use nve_dioxus::prelude::*;
use nve_docs::{DocsRoute, navigate_to, use_docs_state};

use crate::landing::{PillBadge, resolve_example_app_href};

const HOST_TARGETS: [&str; 10] = [
    "macOS AppKit",
    "iOS UIKit",
    "Windows RNW/D2D",
    "Android Compose",
    "Skia Metal/Vulkan/D3D12",
    "Qt 6",
    "GTK4",
    "SwiftUI",
    "Flutter",
    "Web WASM SSR",
];

/// Bento grid of verified NVE architectural & Criterion/C++20 benchmark numbers.
#[component]
pub fn BentoArchitectureGrid() -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let config = snap.config.clone();
    drop(snap);

    let mobile_col_row = TypedStyle::new()
        .width_pct(100.0)
        .flex_row()
        .gap(16.0)
        .at(MediaQuery::max_width(959), RowPatch::new().flex_col());
    let bench_href = config.resolve_href("/docs/reference/benchmarks");
    let card_surface = |grow: f32| {
        TypedStyle::new()
            .flex_grow(grow).flex_shrink(1.0).flex_basis_pct(0.0).min_width_px(0.0)
            .flex_col().justify_content(FlexJustify::SpaceBetween).gap(16.0).padding(22.0)
            .radius(14.0).bg(theme.surface.base).border(1.0, theme.border.default)
    };

    rsx! {
        view {
            test_id: "landing-bento",
            nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(20.0),

            // Section header
            view {
                nve_style: TypedStyle::new().width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::FlexEnd).flex_wrap(FlexWrap::Wrap).gap(12.0),
                view {
                    nve_style: TypedStyle::new().flex_col().gap(6.0).max_width_px(760.0),
                    text {
                        nve_style: TypedStyle::new().font_size(12.0).font_weight(700).color(theme.text.link),
                        "ARCHITECTURE & MEASURED BASELINES"
                    }
                    view {
                        accessibility_role: "heading",
                        "aria-level": "2",
                        text {
                            nve_style: TypedStyle::new().font_size(28.0).font_weight(800).color(theme.text.primary),
                            "Zero-VM performance, measured on real silicon."
                        }
                    }
                }
                pressable {
                    test_id: "landing-bento-bench-link",
                    accessibility_role: "link",
                    "href": "{bench_href}",
                    nve_style: TypedStyle::new().padding_xy(12.0, 7.0).radius(8.0).bg(theme.surface.base).border(1.0, theme.border.default),
                    onpress: move |_| {
                        navigate_to(state, DocsRoute::new("/docs/reference/benchmarks"));
                    },
                    text {
                        nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(theme.text.link),
                        "Full Criterion Benchmarks →"
                    }
                }
            }

            // Row 1: 3 metric cards (Modular C ABI | Binary IR & SceneView | 100k Fenwick VirtualList)
            view {
                nve_style: mobile_col_row.clone(),

                BentoMetricCard {
                    test_id: "bento-abi",
                    eyebrow: "MODULAR C ABI",
                    metric: "0.98 ns",
                    title: "nve_get_interface vtable dispatch",
                    body: "Single exported C symbol resolves SURFACE, LAYOUT, COMPONENT, DEV, and HOST tables with catch_unwind panic isolation and 1.33 ns render-slice dispatch.",
                    tags: vec!["nve_get_interface(1..=5)", "catch_unwind", "Zero JS VM"],
                }

                BentoMetricCard {
                    test_id: "bento-ir",
                    eyebrow: "BINARY IR & SCENE SLAB",
                    metric: "21.17 µs",
                    title: "CommandBuffer + 32B NveNodeRow",
                    body: "Stack-packed 1,000-node frame encode (18.05 µs streaming decode at ~4.7 GiB/s) paired with a zero-copy NveSceneView over 32-byte NveNodeRow slabs (17.09 ns direct FFI value read).",
                    tags: vec!["NVE2 Frame", "32-byte rows", "0 JSON"],
                }

                BentoMetricCard {
                    test_id: "bento-vlist",
                    eyebrow: "VIRTUALIZATION & SSR",
                    metric: "100,000 rows",
                    title: "O(log N) Fenwick VirtualList @ 60fps",
                    body: "Instant prefix-sum jump & filter across 100k items, 1.77 ms Taffy 1,000-row layout, 3.62 µs hit-test, and 12.45 µs semantic HTML5 SSR.",
                    tags: vec!["Fenwick Tree", "Taffy / Yoga", "12.45 µs SSR"],
                }
            }

            // Row 2: Wide 10-Host Matrix card + Live Pairing & Local-First CRDT card
            view {
                nve_style: mobile_col_row,

                // 10 Host Targets Matrix
                view {
                    test_id: "bento-hosts",
                    nve_style: card_surface(3.0),
                    view {
                        nve_style: TypedStyle::new().flex_col().gap(8.0),
                        view {
                            nve_style: TypedStyle::new().flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::Center).gap(8.0),
                            text {
                                nve_style: TypedStyle::new().font_size(11.0).font_weight(700).color(theme.text.link),
                                "10 NATIVE, GPU & WEB HOSTS"
                            }
                            PillBadge { label: "One Binary ABI".to_string(), accent: true }
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(19.0).font_weight(700).color(theme.text.primary),
                            "Real platform views everywhere — from one Rust core."
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary).line_height_ratio(1.5),
                            "Native accessibility, IME, scrolling physics, and GPU swapchains across desktop, mobile, embedded, and zero-flicker Web SSR."
                        }
                    }

                    // 10 Host Matrix Pills
                    view {
                        test_id: "bento-host-pills",
                        nve_style: TypedStyle::new().width_pct(100.0).flex_row().flex_wrap(FlexWrap::Wrap).gap(8.0),
                        for host in HOST_TARGETS {
                            view {
                                key: "host-pill-{host}",
                                nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(6.0).padding_xy(10.0, 6.0).radius(8.0).bg(theme.surface.subtle).border(1.0, theme.border.default),
                                view {
                                    nve_style: TypedStyle::new().width_px(6.0).height_px(6.0).radius(999.0).bg(theme.status.accent.text),
                                }
                                text {
                                    nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(theme.text.primary),
                                    "{host}"
                                }
                            }
                        }
                    }
                }

                // Live Pairing + Local-First NveCrdtDelta + Frame-0 NVE2 Boot
                view {
                    test_id: "bento-pairing",
                    nve_style: card_surface(2.0),
                    view {
                        nve_style: TypedStyle::new().flex_col().gap(8.0),
                        text {
                            nve_style: TypedStyle::new().font_size(11.0).font_weight(700).color(theme.status.info.text),
                            "LIVE PAIRING & LOCAL-FIRST SYNC"
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(28.0).font_weight(800).color(theme.text.primary),
                            "5.18 KB/s"
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(15.0).font_weight(700).color(theme.text.primary),
                            "NveWireFrame Streaming + Binary CRDTs"
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary).line_height_ratio(1.5),
                            "Stream 60fps binary deltas to zero-cdylib thin clients over QUIC/WS, boot Frame 0 from NVE2 keyframes + Taffy layout in < 2 ms, and converge offline state via binary NveCrdtDelta CRDTs."
                        }
                    }
                    view {
                        nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                        PillBadge { label: "NveWireFrame".to_string(), secondary: true }
                        PillBadge { label: "NveCrdtDelta".to_string(), secondary: true }
                        PillBadge { label: "< 2 ms Frame-0 Boot".to_string() }
                    }
                }
            }
        }
    }
}

#[component]
fn BentoMetricCard(
    test_id: &'static str,
    eyebrow: &'static str,
    metric: &'static str,
    title: &'static str,
    body: &'static str,
    tags: Vec<&'static str>,
) -> Element {
    let state = use_docs_state();
    let theme = state.read().theme();

    rsx! {
        view {
            test_id: "{test_id}",
            nve_style: TypedStyle::new()
                .flex_1().min_width_px(0.0).flex_col().justify_content(FlexJustify::SpaceBetween)
                .gap(16.0).padding(22.0).radius(14.0).bg(theme.surface.base).border(1.0, theme.border.default),
            view {
                nve_style: TypedStyle::new().flex_col().gap(8.0),
                text {
                    nve_style: TypedStyle::new().font_size(11.0).font_weight(700).color(theme.text.link),
                    "{eyebrow}"
                }
                text {
                    nve_style: TypedStyle::new().font_size(30.0).font_weight(800).color(theme.text.primary),
                    "{metric}"
                }
                text {
                    nve_style: TypedStyle::new().font_size(15.0).font_weight(700).color(theme.text.primary),
                    "{title}"
                }
                text {
                    nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary).line_height_ratio(1.5),
                    "{body}"
                }
            }
            view {
                nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                for tag in tags {
                    PillBadge { key: "{test_id}-{tag}", label: tag.to_string() }
                }
            }
        }
    }
}

/// Interactive Live Examples Showcase Strip on the Landing Page.
#[component]
pub fn LiveExamplesStrip() -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let config = snap.config.clone();
    drop(snap);

    let mobile_col_row = TypedStyle::new()
        .width_pct(100.0)
        .flex_row()
        .gap(16.0)
        .at(MediaQuery::max_width(959), RowPatch::new().flex_col());
    let examples_href = config.resolve_href("/examples");
    let studio_href = resolve_example_app_href(&config.base_path, "/examples/studio");
    let kitchen_sink_href = resolve_example_app_href(&config.base_path, "/examples/kitchen-sink");

    let playground_links: [(&str, &str); 4] = [
        ("UI + 100k List", "/docs/guides/ui-components"),
        ("Router Sandbox", "/docs/guides/routing"),
        ("NVE_DRAW1 Canvas", "/docs/guides/canvas-draw1"),
        ("CRDT Sync", "/docs/guides/local-first-crdt"),
    ];
    let showcase_card = TypedStyle::new()
        .flex_1().min_width_px(0.0).flex_col().justify_content(FlexJustify::SpaceBetween)
        .gap(18.0).padding(22.0).radius(14.0).bg(theme.surface.base).border(1.0, theme.border.default);

    rsx! {
        view {
            test_id: "landing-examples-strip",
            nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(20.0),

            view {
                nve_style: TypedStyle::new().width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::FlexEnd).flex_wrap(FlexWrap::Wrap).gap(12.0),
                view {
                    nve_style: TypedStyle::new().flex_col().gap(6.0),
                    text {
                        nve_style: TypedStyle::new().font_size(12.0).font_weight(700).color(theme.text.link),
                        "LIVE INTERACTIVE DEMOS"
                    }
                    view {
                        accessibility_role: "heading",
                        "aria-level": "2",
                        text {
                            nve_style: TypedStyle::new().font_size(28.0).font_weight(800).color(theme.text.primary),
                            "Launch full NVE apps — lazy-loaded on demand."
                        }
                    }
                }
                pressable {
                    test_id: "landing-examples-all-link",
                    accessibility_role: "link",
                    "href": "{examples_href}",
                    nve_style: TypedStyle::new().padding_xy(12.0, 7.0).radius(8.0).bg(theme.surface.base).border(1.0, theme.border.default),
                    onpress: move |_| {
                        navigate_to(state, DocsRoute::new("/examples"));
                    },
                    text {
                        nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(theme.text.link),
                        "Explore Examples Gallery →"
                    }
                }
            }

            view {
                nve_style: mobile_col_row,

                // Card 1: Aether Workspace + NVE Studio
                view {
                    test_id: "showcase-card-studio",
                    nve_style: showcase_card.clone(),
                    view {
                        nve_style: TypedStyle::new().flex_col().gap(10.0),
                        view {
                            nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                            PillBadge { label: "WASM App".to_string(), accent: true }
                            PillBadge { label: "100k Fenwick".to_string(), secondary: true }
                            PillBadge { label: "NveCrdtDelta".to_string() }
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(18.0).font_weight(700).color(theme.text.primary),
                            "Aether Workspace + NVE Studio"
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary).line_height_ratio(1.5),
                            "Flagship Linear × Notion workspace with live X-Ray layout inspection, time-travel CommandBuffer scrubbing, and NVE_DRAW1 telemetry canvas."
                        }
                    }
                    pressable {
                        test_id: "showcase-launch-studio",
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
                        nve_style: TypedStyle::new()
                            .width_pct(100.0).justify_content(FlexJustify::Center).align_items(FlexAlign::Center)
                            .padding_xy(14.0, 10.0).radius(8.0)
                            .bg(theme.status.accent.solid).border(1.0, theme.status.accent.solid),
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).font_weight(700).color(theme.text.on_accent),
                            "Launch Studio WASM →"
                        }
                    }
                }

                // Card 2: Kitchen Sink Component Gallery
                view {
                    test_id: "showcase-card-kitchen-sink",
                    nve_style: showcase_card.clone(),
                    view {
                        nve_style: TypedStyle::new().flex_col().gap(10.0),
                        view {
                            nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                            PillBadge { label: "WASM App".to_string(), accent: true }
                            PillBadge { label: "All nve-ui".to_string() }
                            PillBadge { label: "nve-i18n".to_string() }
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(18.0).font_weight(700).color(theme.text.primary),
                            "NVE Kitchen Sink Gallery"
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary).line_height_ratio(1.5),
                            "Interactive showcase of every nve-ui component, theme/density/locale switching, 1k VirtualList, Canvas draw lists, and ErrorBoundary recovery."
                        }
                    }
                    pressable {
                        test_id: "showcase-launch-kitchen-sink",
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
                        nve_style: TypedStyle::new()
                            .width_pct(100.0).justify_content(FlexJustify::Center).align_items(FlexAlign::Center)
                            .padding_xy(14.0, 10.0).radius(8.0)
                            .bg(theme.surface.subtle).border(1.0, theme.border.default),
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).font_weight(700).color(theme.text.primary),
                            "Launch Kitchen Sink WASM →"
                        }
                    }
                }

                // Card 3: 5 Embedded Docs Playgrounds
                view {
                    test_id: "showcase-card-playgrounds",
                    nve_style: showcase_card,
                    view {
                        nve_style: TypedStyle::new().flex_col().gap(10.0),
                        view {
                            nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                            PillBadge { label: "5 Live Playgrounds".to_string(), secondary: true }
                            PillBadge { label: "SSR + Hydrate".to_string() }
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(18.0).font_weight(700).color(theme.text.primary),
                            "Embedded Docs Playgrounds"
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary).line_height_ratio(1.5),
                            "Hydrated directly inside documentation articles with zero mismatches — test components, routing, vector draw lists, and CRDT convergence."
                        }
                    }
                    view {
                        nve_style: TypedStyle::new().width_pct(100.0).flex_row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                        for (label, route) in playground_links {
                            {
                                let href = config.resolve_href(route);
                                let target = DocsRoute::new(route);
                                rsx! {
                                    pressable {
                                        key: "showcase-pg-{route}",
                                        accessibility_role: "link",
                                        "href": "{href}",
                                        nve_style: TypedStyle::new().padding_xy(9.0, 5.0).radius(6.0).bg(theme.surface.subtle).border(1.0, theme.border.default),
                                        onpress: move |_| {
                                            navigate_to(state, target.clone());
                                        },
                                        text {
                                            nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(theme.text.link),
                                            "{label} →"
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
}
