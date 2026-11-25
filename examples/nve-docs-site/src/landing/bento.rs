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

    let mobile_col = TypedStyle::new()
        .at(MediaQuery::max_width(959), ResponsiveStylePatch::new().flex_col());
    let bench_href = config.resolve_href("/docs/reference/benchmarks");

    rsx! {
        view {
            test_id: "landing-bento",
            style: "width: 100%; flex-direction: column; gap: 20px;",

            // Section header
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: flex-end; flex-wrap: wrap; gap: 12px;",
                view {
                    style: "flex-direction: column; gap: 6px; max-width: 760px;",
                    text {
                        style: "font-size: 12px; font-weight: 700; color: {theme.accent};",
                        "ARCHITECTURE & MEASURED BASELINES"
                    }
                    view {
                        accessibility_role: "heading",
                        "aria-level": "2",
                        text {
                            style: "font-size: 28px; font-weight: 800; color: {theme.text};",
                            "Zero-VM performance, measured on real silicon."
                        }
                    }
                }
                pressable {
                    test_id: "landing-bento-bench-link",
                    accessibility_role: "link",
                    "href": "{bench_href}",
                    style: "padding-horizontal: 12px; padding-vertical: 7px; border-radius: 8px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                    onpress: move |_| {
                        navigate_to(state, DocsRoute::new("/docs/reference/benchmarks"));
                    },
                    text {
                        style: "font-size: 13px; font-weight: 600; color: {theme.accent};",
                        "Full Criterion & C++20 Benchmarks →"
                    }
                }
            }

            // Row 1: 3 metric cards (Modular C ABI | Binary IR & SceneView | 100k Fenwick VirtualList)
            view {
                nve_style: mobile_col.clone(),
                style: "width: 100%; flex-direction: row; gap: 16px;",

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
                    title: "CommandBuffer + 64B NveRenderNode",
                    body: "Stack-packed 1,000-node frame encode (18.05 µs streaming decode at ~4.7 GiB/s) paired with 64-byte cache-line NveRenderNode slabs (24.82 ns dirty read).",
                    tags: vec!["NVE1 Frame", "64-byte aligned", "0 JSON"],
                }

                BentoMetricCard {
                    test_id: "bento-vlist",
                    eyebrow: "VIRTUALIZATION & SSR",
                    metric: "100,000 rows",
                    title: "O(log N) Fenwick VirtualList @ 60fps",
                    body: "Instant prefix-sum jump & filter across 100k items, 451 µs C++20 1,000-row layout, 3.62 µs hit-test, and 12.45 µs semantic HTML5 SSR.",
                    tags: vec!["Fenwick Tree", "Taffy / Yoga", "12.45 µs SSR"],
                }
            }

            // Row 2: Wide 10-Host Matrix card + Live Pairing & Local-First CRDT card
            view {
                nve_style: mobile_col,
                style: "width: 100%; flex-direction: row; gap: 16px;",

                // 10 Host Targets Matrix
                view {
                    test_id: "bento-hosts",
                    style: "flex-grow: 3; flex-shrink: 1; flex-basis: 0%; min-width: 0; flex-direction: column; justify-content: space-between; gap: 16px; padding: 22px; border-radius: 14px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                    view {
                        style: "flex-direction: column; gap: 8px;",
                        view {
                            style: "flex-direction: row; justify-content: space-between; align-items: center; gap: 8px;",
                            text {
                                style: "font-size: 11px; font-weight: 700; color: {theme.accent};",
                                "10 NATIVE, GPU & WEB HOSTS"
                            }
                            PillBadge { label: "One Binary ABI".to_string(), accent: true }
                        }
                        text {
                            style: "font-size: 19px; font-weight: 700; color: {theme.text};",
                            "Real platform views everywhere — from one Rust core."
                        }
                        text {
                            style: "font-size: 13px; color: {theme.muted}; line-height: 1.5;",
                            "Native accessibility, IME, scrolling physics, and GPU swapchains across desktop, mobile, embedded, and zero-flicker Web SSR."
                        }
                    }

                    // 10 Host Matrix Pills
                    view {
                        test_id: "bento-host-pills",
                        style: "width: 100%; flex-direction: row; flex-wrap: wrap; gap: 8px;",
                        for host in HOST_TARGETS {
                            view {
                                key: "host-pill-{host}",
                                style: "flex-direction: row; align-items: center; gap: 6px; padding-horizontal: 10px; padding-vertical: 6px; border-radius: 8px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                                view {
                                    style: "width: 6px; height: 6px; border-radius: 999px; background-color: {theme.accent};",
                                }
                                text {
                                    style: "font-size: 12px; font-weight: 600; color: {theme.text};",
                                    "{host}"
                                }
                            }
                        }
                    }
                }

                // Live Pairing + Local-First NVC1 CRDT + MmapSnapshot
                view {
                    test_id: "bento-pairing",
                    style: "flex-grow: 2; flex-shrink: 1; flex-basis: 0%; min-width: 0; flex-direction: column; justify-content: space-between; gap: 16px; padding: 22px; border-radius: 14px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                    view {
                        style: "flex-direction: column; gap: 8px;",
                        text {
                            style: "font-size: 11px; font-weight: 700; color: {theme.accent_alt};",
                            "LIVE PAIRING & LOCAL-FIRST SYNC"
                        }
                        text {
                            style: "font-size: 28px; font-weight: 800; color: {theme.text};",
                            "5.18 KB/s"
                        }
                        text {
                            style: "font-size: 15px; font-weight: 700; color: {theme.text};",
                            "NVT1/NVD1 Streaming + NVC1 CRDTs"
                        }
                        text {
                            style: "font-size: 13px; color: {theme.muted}; line-height: 1.5;",
                            "Stream 60fps binary deltas to zero-cdylib thin clients over UDP/WS, restore Frame 0 from MmapSnapshot in 82–136 µs, and converge offline state via zero-JSON NVC1 CRDTs."
                        }
                    }
                    view {
                        style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                        PillBadge { label: "NVT1 / NVD1".to_string(), secondary: true }
                        PillBadge { label: "NVC1 CRDT".to_string(), secondary: true }
                        PillBadge { label: "82–136 µs Cold Start".to_string() }
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
            style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0; flex-direction: column; justify-content: space-between; gap: 16px; padding: 22px; border-radius: 14px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
            view {
                style: "flex-direction: column; gap: 8px;",
                text {
                    style: "font-size: 11px; font-weight: 700; color: {theme.accent};",
                    "{eyebrow}"
                }
                text {
                    style: "font-size: 30px; font-weight: 800; color: {theme.text};",
                    "{metric}"
                }
                text {
                    style: "font-size: 15px; font-weight: 700; color: {theme.text};",
                    "{title}"
                }
                text {
                    style: "font-size: 13px; color: {theme.muted}; line-height: 1.5;",
                    "{body}"
                }
            }
            view {
                style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
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

    let mobile_col = TypedStyle::new()
        .at(MediaQuery::max_width(959), ResponsiveStylePatch::new().flex_col());
    let examples_href = config.resolve_href("/examples");
    let studio_href = resolve_example_app_href(&config.base_path, "/examples/studio");
    let counter_href = resolve_example_app_href(&config.base_path, "/examples/counter");

    let playground_links: [(&str, &str); 4] = [
        ("UI + 100k List", "/docs/guides/ui-components"),
        ("Router Sandbox", "/docs/guides/routing"),
        ("NVE_DRAW1 Canvas", "/docs/guides/canvas-draw1"),
        ("NVC1 CRDT Sync", "/docs/guides/local-first-crdt"),
    ];

    rsx! {
        view {
            test_id: "landing-examples-strip",
            style: "width: 100%; flex-direction: column; gap: 20px;",

            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: flex-end; flex-wrap: wrap; gap: 12px;",
                view {
                    style: "flex-direction: column; gap: 6px;",
                    text {
                        style: "font-size: 12px; font-weight: 700; color: {theme.accent};",
                        "LIVE INTERACTIVE DEMOS"
                    }
                    view {
                        accessibility_role: "heading",
                        "aria-level": "2",
                        text {
                            style: "font-size: 28px; font-weight: 800; color: {theme.text};",
                            "Launch full NVE apps — lazy-loaded on demand."
                        }
                    }
                }
                pressable {
                    test_id: "landing-examples-all-link",
                    accessibility_role: "link",
                    "href": "{examples_href}",
                    style: "padding-horizontal: 12px; padding-vertical: 7px; border-radius: 8px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                    onpress: move |_| {
                        navigate_to(state, DocsRoute::new("/examples"));
                    },
                    text {
                        style: "font-size: 13px; font-weight: 600; color: {theme.accent};",
                        "Explore Examples Gallery →"
                    }
                }
            }

            view {
                nve_style: mobile_col,
                style: "width: 100%; flex-direction: row; gap: 16px;",

                // Card 1: Aether Workspace + NVE Studio
                view {
                    test_id: "showcase-card-studio",
                    style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0; flex-direction: column; justify-content: space-between; gap: 18px; padding: 22px; border-radius: 14px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                    view {
                        style: "flex-direction: column; gap: 10px;",
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                            PillBadge { label: "WASM App".to_string(), accent: true }
                            PillBadge { label: "100k Fenwick".to_string(), secondary: true }
                            PillBadge { label: "NVC1 CRDT".to_string() }
                        }
                        text {
                            style: "font-size: 18px; font-weight: 700; color: {theme.text};",
                            "Aether Workspace + NVE Studio"
                        }
                        text {
                            style: "font-size: 13px; color: {theme.muted}; line-height: 1.5;",
                            "Flagship Linear × Notion workspace with live X-Ray layout inspection, time-travel CommandBuffer scrubbing, and NVE_DRAW1 telemetry canvas."
                        }
                    }
                    pressable {
                        test_id: "showcase-launch-studio",
                        accessibility_role: "link",
                        "href": "{studio_href}",
                        "data-nve-hard-nav": "true",
                        style: "width: 100%; justify-content: center; align-items: center; padding-vertical: 10px; padding-horizontal: 14px; border-radius: 8px; background-color: {theme.accent}; border-width: 1px; border-color: {theme.accent};",
                        text {
                            style: "font-size: 13px; font-weight: 700; color: {theme.accent_fg};",
                            "Launch Studio WASM →"
                        }
                    }
                }

                // Card 2: Interactive Counter
                view {
                    test_id: "showcase-card-counter",
                    style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0; flex-direction: column; justify-content: space-between; gap: 18px; padding: 22px; border-radius: 14px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                    view {
                        style: "flex-direction: column; gap: 10px;",
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                            PillBadge { label: "WASM App".to_string(), accent: true }
                            PillBadge { label: "Minimal Bundle".to_string() }
                            PillBadge { label: "Dioxus Signals".to_string() }
                        }
                        text {
                            style: "font-size: 18px; font-weight: 700; color: {theme.text};",
                            "Interactive Counter"
                        }
                        text {
                            style: "font-size: 13px; color: {theme.muted}; line-height: 1.5;",
                            "Minimal end-to-end NveApp proving fine-grained signal diffs, direct binary EventRing dispatch, and zero-JSON DOM patching."
                        }
                    }
                    pressable {
                        test_id: "showcase-launch-counter",
                        accessibility_role: "link",
                        "href": "{counter_href}",
                        "data-nve-hard-nav": "true",
                        style: "width: 100%; justify-content: center; align-items: center; padding-vertical: 10px; padding-horizontal: 14px; border-radius: 8px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                        text {
                            style: "font-size: 13px; font-weight: 700; color: {theme.text};",
                            "Launch Counter WASM →"
                        }
                    }
                }

                // Card 3: 5 Embedded Docs Playgrounds
                view {
                    test_id: "showcase-card-playgrounds",
                    style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0; flex-direction: column; justify-content: space-between; gap: 18px; padding: 22px; border-radius: 14px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                    view {
                        style: "flex-direction: column; gap: 10px;",
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                            PillBadge { label: "5 Live Playgrounds".to_string(), secondary: true }
                            PillBadge { label: "SSR + Hydrate".to_string() }
                        }
                        text {
                            style: "font-size: 18px; font-weight: 700; color: {theme.text};",
                            "Embedded Docs Playgrounds"
                        }
                        text {
                            style: "font-size: 13px; color: {theme.muted}; line-height: 1.5;",
                            "Hydrated directly inside documentation articles with zero mismatches — test components, routing, vector draw lists, and CRDT convergence."
                        }
                    }
                    view {
                        style: "width: 100%; flex-direction: row; flex-wrap: wrap; gap: 6px;",
                        for (label, route) in playground_links {
                            {
                                let href = config.resolve_href(route);
                                let target = DocsRoute::new(route);
                                rsx! {
                                    pressable {
                                        key: "showcase-pg-{route}",
                                        accessibility_role: "link",
                                        "href": "{href}",
                                        style: "padding-horizontal: 9px; padding-vertical: 5px; border-radius: 6px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                                        onpress: move |_| {
                                            navigate_to(state, target.clone());
                                        },
                                        text {
                                            style: "font-size: 12px; font-weight: 600; color: {theme.accent};",
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
