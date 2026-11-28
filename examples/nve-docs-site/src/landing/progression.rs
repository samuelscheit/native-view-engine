//! Development Progression Timeline (Waves 1 → 14 from `docs/CHANGELOG.md`) paired with the
//! interactive Purple-Garden-style `NVE_DRAW1` Benchmark Progression Dashboard.

use nve_dioxus::prelude::*;
use nve_docs::use_docs_state;

use crate::landing::PillBadge;
use crate::landing::benchmarks_chart::BenchmarkProgressionDashboard;

struct WaveMilestone {
    wave: &'static str,
    commit: &'static str,
    hosts: &'static str,
    title: &'static str,
    summary: &'static str,
}

const WAVE_MILESTONES: [WaveMilestone; 8] = [
    WaveMilestone {
        wave: "Waves 1–2",
        commit: "63b381e",
        hosts: "1 → 4 hosts",
        title: "Bootstrap & Modular C ABI",
        summary: "Cargo workspace, single-symbol nve_get_interface(1..=4), safe NveApp + export_nve_app!, composable Taffy/Yoga layout, and Criterion/C++20 fuzz suites.",
    },
    WaveMilestone {
        wave: "Waves 3–5",
        commit: "7430122",
        hosts: "4 → 9 hosts",
        title: "Direct FFI NveBinValue & 10-Host Expansion",
        summary: "Replaced JSON crossings with zero-copy NveBinValue, shipped Qt 6, GTK4, Windows RNW/D2D, iOS, Android Compose, Flutter RenderBox, and flagship Studio.",
    },
    WaveMilestone {
        wave: "Waves 6–8",
        commit: "d131b04",
        hosts: "10 hosts",
        title: "Hybrid Codegen, Skia GPU & Frame-0 Boot",
        summary: "6-target Nitro-style Hybrid FFI codegen, Metal/Vulkan/D3D12 Skia swapchains, NVD1 UDP/TCP/WS delta transport, and Frame-0 NVE2 + atomic StateFile cold start.",
    },
    WaveMilestone {
        wave: "Wave 9",
        commit: "cf1f3a7",
        hosts: "10 hosts",
        title: "Zero-Copy Scene View & Tier-1 RSX Hot-Patch",
        summary: "O(changed) dirty-index scene reconciliation across all platforms, < 10 ms NVP1 RSX live hot-patcher, and catch_unwind panic boundaries.",
    },
    WaveMilestone {
        wave: "Waves 10–11",
        commit: "2d1ceae",
        hosts: "10 hosts",
        title: "Universal NVE_DRAW1, Pairing & Perf Wave",
        summary: "Binary NVE_DRAW1 canvas across hosts, NVT1/NVD1 thin-client pairing, CommandBuffer -51.9%, GestureArbitrator -73.8%, and WASM release -47.6%.",
    },
    WaveMilestone {
        wave: "Wave 12",
        commit: "b3ecf8f",
        hosts: "10 hosts",
        title: "Binary NVC1 CRDTs & Windows 11 Live Parity",
        summary: "Local-first NVC1 binary CRDTs (HLC, LwwRegister, PnCounter, OrSet, RgaList), WinSock2 pairing client, and full live Windows 11 MSVC/D3D12/RNW verification.",
    },
    WaveMilestone {
        wave: "Wave 13",
        commit: "0aeee16",
        hosts: "10 hosts",
        title: "NVE_IFACE_HOST=5 SDK, nve-ui & nve-router",
        summary: "Binary NVH1 Tier-1/2/3 native capabilities, 15 accessible nve-ui components + 100k Fenwick VirtualList, type-safe nve-router, and 7-host multi-window.",
    },
    WaveMilestone {
        wave: "Wave 14",
        commit: "36a6e5a",
        hosts: "10/10 parity",
        title: "Option 5 SSR (-77.2%), nve-docs & Zero-Copy",
        summary: "Content-hashed Hybrid CSS + lossless hydration, nve-mdx + NVS2 search, reusable nve-docs, Swift/Flutter pairing parity, and zero-copy TypedStyle & HotAppInstance.",
    },
];

/// Complete Development Progression Timeline + Interactive Benchmark Progression Dashboard on `/`.
#[component]
pub fn DevAndBenchProgression() -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    drop(snap);

    let milestone_rows: Vec<&[WaveMilestone]> = WAVE_MILESTONES.chunks(4).collect();
    let mobile_col_row = TypedStyle::new()
        .width_pct(100.0).flex_row().gap(12.0)
        .at(MediaQuery::max_width(959), RowPatch::new().flex_col());

    rsx! {
        view {
            test_id: "landing-progression-section",
            nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(28.0),

            // Section Header + Growth Telemetry Pills
            view {
                nve_style: TypedStyle::new().width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::FlexEnd).flex_wrap(FlexWrap::Wrap).gap(14.0),
                view {
                    nve_style: TypedStyle::new().flex_col().gap(6.0).max_width_px(720.0),
                    text {
                        nve_style: TypedStyle::new().font_size(12.0).font_weight(700).color(theme.text.link),
                        "DEVELOPMENT & BENCHMARK PROGRESSION"
                    }
                    view {
                        accessibility_role: "heading",
                        "aria-level": "2",
                        text {
                            nve_style: TypedStyle::new().font_size(28.0).font_weight(800).color(theme.text.primary),
                            "14 engineering waves, continuously measured."
                        }
                    }
                    text {
                        nve_style: TypedStyle::new().font_size(14.0).color(theme.text.secondary).line_height_ratio(1.5),
                        "Every latency and payload breakthrough below is tracked directly from benchmarks/baseline.json and rendered natively via binary NVE_DRAW1 vector canvases."
                    }
                }

                view {
                    nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(8.0).align_items(FlexAlign::Center),
                    PillBadge { label: "1 → 10 Platform Hosts".to_string(), accent: true }
                    PillBadge { label: "411+ Rust & 8 Host Suites".to_string(), secondary: true }
                    PillBadge { label: "Waves 1 → 14".to_string() }
                }
            }

            // 1. Development Progression Timeline (explicit 4x2 grid on desktop, 1-column on mobile)
            view {
                test_id: "dev-progression-timeline",
                nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(12.0),
                for (ridx, row) in milestone_rows.into_iter().enumerate() {
                    view {
                        key: "wave-row-{ridx}",
                        nve_style: mobile_col_row.clone(),
                        for m in row {
                            view {
                                key: "wave-{m.wave}",
                                nve_style: TypedStyle::new()
                                    .flex_1().min_width_px(0.0).flex_col().justify_content(FlexJustify::SpaceBetween)
                                    .gap(10.0).padding(16.0).radius(12.0).bg(theme.surface.base).border(1.0, theme.border.default),
                                view {
                                    nve_style: TypedStyle::new().flex_col().gap(7.0),
                                    view {
                                        nve_style: TypedStyle::new().width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::Center).gap(6.0),
                                        text {
                                            nve_style: TypedStyle::new().font_size(11.0).font_weight(700).color(theme.text.link),
                                            "{m.wave}"
                                        }
                                        text {
                                            nve_style: TypedStyle::new().font_size(11.0).font_family("monospace").color(theme.text.secondary),
                                            "{m.commit}"
                                        }
                                    }
                                    text {
                                        nve_style: TypedStyle::new().font_size(14.0).font_weight(700).color(theme.text.primary),
                                        "{m.title}"
                                    }
                                    text {
                                        nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary).line_height_ratio(1.45),
                                        "{m.summary}"
                                    }
                                }
                                view {
                                    nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).justify_content(FlexJustify::SpaceBetween).padding_top(6.0),
                                    PillBadge { label: m.hosts.to_string() }
                                }
                            }
                        }
                    }
                }
            }

            // 2. Purple-Garden-Style Interactive Benchmark Progression Dashboard (100% NVE_DRAW1)
            BenchmarkProgressionDashboard {}
        }
    }
}
