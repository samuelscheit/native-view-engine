//! Verified benchmark baseline data (embedded from `benchmarks/baseline.json` at compile time),
//! wave progression series, and direct binary FFI `NVE_DRAW1` bar/line chart draw-list builders.

use nve_abi::ColorRgba;
use nve_canvas::DrawList;

include!(concat!(
    env!("OUT_DIR"),
    "/embedded_benchmarks.generated.rs"
));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BenchCategory {
    All,
    RustCore,
    CxxBridge,
    WebWasm,
    SyncLayout,
}

impl BenchCategory {
    pub(crate) const ALL: [Self; 5] = [
        Self::All,
        Self::RustCore,
        Self::CxxBridge,
        Self::WebWasm,
        Self::SyncLayout,
    ];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::RustCore => "rust-core",
            Self::CxxBridge => "cxx-bridge",
            Self::WebWasm => "web-wasm",
            Self::SyncLayout => "sync-layout",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::RustCore => "Rust Core",
            Self::CxxBridge => "C++20 Bridge",
            Self::WebWasm => "Web & WASM",
            Self::SyncLayout => "Sync & Layout",
        }
    }

    pub(crate) fn color_idx(self) -> Option<u8> {
        match self {
            Self::All => None,
            Self::RustCore => Some(0),
            Self::CxxBridge => Some(1),
            Self::WebWasm => Some(2),
            Self::SyncLayout => Some(3),
        }
    }
}

pub(crate) fn category_swatch_hex(color_idx: u8, dark: bool) -> &'static str {
    match (color_idx, dark) {
        (0, true) => "#38bdf8",
        (0, false) => "#0284c7",
        (1, true) => "#818cf8",
        (1, false) => "#4f46e5",
        (2, true) => "#34d399",
        (2, false) => "#059669",
        (_, true) => "#fbbf24",
        (_, false) => "#d97706",
    }
}

pub(crate) fn bar_chart_lo_bound(min_v: f64, log_scale: bool) -> f64 {
    if log_scale {
        (min_v * 0.5).max(0.5)
    } else {
        0.0
    }
}

pub(crate) fn line_chart_lo_bound(min_v: f64, log_scale: bool) -> f64 {
    if log_scale {
        (min_v * 0.6).max(0.1)
    } else {
        0.0
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LatestBarSpec {
    pub(crate) short_label: &'static str,
    pub(crate) baseline_key: &'static str,
    pub(crate) category: BenchCategory,
    pub(crate) color_idx: u8,
}

pub(crate) const LATEST_BARS: [LatestBarSpec; 10] = [
    LatestBarSpec {
        short_label: "get_iface",
        baseline_key: "rust::abi/nve_get_interface/lookup_interface",
        category: BenchCategory::RustCore,
        color_idx: 0,
    },
    LatestBarSpec {
        short_label: "Scene dirty",
        baseline_key: "rust::abi/scene/scene_view_read_dirty_3_of_1000",
        category: BenchCategory::RustCore,
        color_idx: 0,
    },
    LatestBarSpec {
        short_label: "ChannelBus",
        baseline_key: "rust::interop/channel_bus/publish_subscribers/1",
        category: BenchCategory::SyncLayout,
        color_idx: 3,
    },
    LatestBarSpec {
        short_label: "Gesture",
        baseline_key: "rust::interop/gesture_arbitration/nested_scroll_sequence",
        category: BenchCategory::RustCore,
        color_idx: 0,
    },
    LatestBarSpec {
        short_label: "C++ Roundtrip",
        baseline_key: "cxx::rn_bridge_render_press_roundtrip",
        category: BenchCategory::CxxBridge,
        color_idx: 1,
    },
    LatestBarSpec {
        short_label: "Dioxus diff",
        baseline_key: "rust::dioxus/incremental_state_update_diff",
        category: BenchCategory::RustCore,
        color_idx: 0,
    },
    LatestBarSpec {
        short_label: "DOM patch 10",
        baseline_key: "rust::backend_web/incremental_dom_patch_10_nodes",
        category: BenchCategory::WebWasm,
        color_idx: 2,
    },
    LatestBarSpec {
        short_label: "C++ Patch 10",
        baseline_key: "cxx::incremental_patch_10_on_1000_nodes",
        category: BenchCategory::CxxBridge,
        color_idx: 1,
    },
    LatestBarSpec {
        short_label: "SSR 100 rows",
        baseline_key: "rust::backend_web/render_html_100_rows",
        category: BenchCategory::WebWasm,
        color_idx: 2,
    },
    LatestBarSpec {
        short_label: "CmdBuf 1k",
        baseline_key: "rust::abi/command_buffer_encode/1000",
        category: BenchCategory::RustCore,
        color_idx: 0,
    },
];

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProgressionPoint {
    pub(crate) label: &'static str,
    pub(crate) value: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProgressionSeries {
    pub(crate) id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) subtitle: &'static str,
    pub(crate) unit: &'static str,
    pub(crate) delta_badge: &'static str,
    pub(crate) category: BenchCategory,
    pub(crate) color_idx: u8,
    pub(crate) points: &'static [ProgressionPoint],
}

pub(crate) const PROGRESSION_SERIES: [ProgressionSeries; 6] = [
    ProgressionSeries {
        id: "cmdbuf-encode",
        title: "CommandBufferBuilder Encode (1,000 nodes)",
        subtitle: "Stack-packed binary IR encoder · commit 2d1ceae",
        unit: "µs",
        delta_badge: "-51.9%",
        category: BenchCategory::RustCore,
        color_idx: 0,
        points: &[
            ProgressionPoint { label: "W2", value: 44.14 },
            ProgressionPoint { label: "W6", value: 36.20 },
            ProgressionPoint { label: "W9", value: 28.90 },
            ProgressionPoint { label: "W11", value: 21.17 },
            ProgressionPoint { label: "W14", value: 21.17 },
        ],
    },
    ProgressionSeries {
        id: "gesture-arb",
        title: "GestureArbitrator Nested Scroll",
        subtitle: "Zero-alloc inline SurfaceChain + FastU32Map · commit 9406d1e",
        unit: "ns",
        delta_badge: "-73.8%",
        category: BenchCategory::RustCore,
        color_idx: 1,
        points: &[
            ProgressionPoint { label: "W6", value: 246.96 },
            ProgressionPoint { label: "W8", value: 184.50 },
            ProgressionPoint { label: "W10", value: 112.20 },
            ProgressionPoint { label: "W11", value: 64.51 },
            ProgressionPoint { label: "W14", value: 64.51 },
        ],
    },
    ProgressionSeries {
        id: "web-ssr-payload",
        title: "Studio Frame 0 Web SSR + CSS Payload",
        subtitle: "Option 5 Hybrid content-hashed deduplicated CSS · commit 36a6e5a",
        unit: "KiB",
        delta_badge: "-77.2%",
        category: BenchCategory::WebWasm,
        color_idx: 2,
        points: &[
            ProgressionPoint { label: "W5", value: 112.3 },
            ProgressionPoint { label: "W9", value: 78.4 },
            ProgressionPoint { label: "W11", value: 51.2 },
            ProgressionPoint { label: "W13", value: 34.8 },
            ProgressionPoint { label: "W14", value: 25.6 },
        ],
    },
    ProgressionSeries {
        id: "wasm-release",
        title: "Web WASM Release Bundle (Raw Size)",
        subtitle: "wasm-release + wasm-opt → 285.8 KiB gzip · commit 9545a54",
        unit: "KiB",
        delta_badge: "-47.6%",
        category: BenchCategory::WebWasm,
        color_idx: 2,
        points: &[
            ProgressionPoint { label: "W5", value: 1500.0 },
            ProgressionPoint { label: "W8", value: 1210.0 },
            ProgressionPoint { label: "W10", value: 965.0 },
            ProgressionPoint { label: "W11", value: 786.0 },
            ProgressionPoint { label: "W14", value: 786.0 },
        ],
    },
    ProgressionSeries {
        id: "cxx-bridge-lat",
        title: "C++20 Fabric Bridge Scaling (100 → 1,000 nodes)",
        subtitle: "Verified baseline.json · direct NveSceneView & decode mount",
        unit: "µs",
        delta_badge: "1.50M ops/s",
        category: BenchCategory::CxxBridge,
        color_idx: 1,
        points: &[
            ProgressionPoint { label: "Press", value: 0.67 },
            ProgressionPoint { label: "Patch10", value: 7.12 },
            ProgressionPoint { label: "Pub10", value: 7.54 },
            ProgressionPoint { label: "Mnt100", value: 18.33 },
            ProgressionPoint { label: "Mnt1k", value: 171.48 },
        ],
    },
    ProgressionSeries {
        id: "layout-sync",
        title: "Layout & Hit-Test Latency Spectrum",
        subtitle: "Verified baseline.json · Taffy vs C++20 heuristic layout",
        unit: "µs",
        delta_badge: "3.62 µs hit",
        category: BenchCategory::SyncLayout,
        color_idx: 3,
        points: &[
            ProgressionPoint { label: "Hit100", value: 0.34 },
            ProgressionPoint { label: "Hit1k", value: 3.62 },
            ProgressionPoint { label: "C++100", value: 44.67 },
            ProgressionPoint { label: "Taf100", value: 168.51 },
            ProgressionPoint { label: "C++1k", value: 450.69 },
        ],
    },
];

pub(crate) fn lookup_baseline(key: &str) -> Option<&'static EmbeddedBaselineEntry> {
    BASELINE_BENCHMARKS.iter().find(|e| e.key == key)
}

pub(crate) fn format_ns(ns: f64) -> String {
    if ns >= 1_000_000.0 {
        format!("{:.2} ms", ns / 1_000_000.0)
    } else if ns >= 1_000.0 {
        format!("{:.2} µs", ns / 1_000.0)
    } else if ns >= 10.0 {
        format!("{:.1} ns", ns)
    } else {
        format!("{:.2} ns", ns)
    }
}

pub(crate) fn format_val(val: f64, unit: &str) -> String {
    if val >= 100.0 {
        format!("{:.0} {unit}", val)
    } else if val >= 10.0 {
        format!("{:.1} {unit}", val)
    } else {
        format!("{:.2} {unit}", val)
    }
}

fn scale_norm(val: f64, min_v: f64, max_v: f64, log_scale: bool) -> f32 {
    if log_scale {
        let lo = (min_v.max(0.1)).log10();
        let hi = (max_v.max(min_v + 0.1)).log10();
        let cur = (val.max(0.1)).log10();
        if (hi - lo).abs() < 1e-6 {
            0.5
        } else {
            ((cur - lo) / (hi - lo)).clamp(0.05, 1.0) as f32
        }
    } else {
        let hi = max_v.max(1e-6);
        (val / hi).clamp(0.03, 1.0) as f32
    }
}

fn series_color(idx: u8, dark: bool) -> (ColorRgba, ColorRgba) {
    match (idx, dark) {
        (0, true) => (ColorRgba::rgb(56, 189, 248), ColorRgba::rgba(56, 189, 248, 38)),
        (0, false) => (ColorRgba::rgb(2, 132, 199), ColorRgba::rgba(2, 132, 199, 32)),
        (1, true) => (ColorRgba::rgb(129, 140, 248), ColorRgba::rgba(129, 140, 248, 38)),
        (1, false) => (ColorRgba::rgb(79, 70, 229), ColorRgba::rgba(79, 70, 229, 30)),
        (2, true) => (ColorRgba::rgb(52, 211, 153), ColorRgba::rgba(52, 211, 153, 36)),
        (2, false) => (ColorRgba::rgb(5, 150, 105), ColorRgba::rgba(5, 150, 105, 30)),
        (_, true) => (ColorRgba::rgb(251, 191, 36), ColorRgba::rgba(251, 191, 36, 36)),
        (_, false) => (ColorRgba::rgb(217, 119, 6), ColorRgba::rgba(217, 119, 6, 30)),
    }
}

pub(crate) fn build_bar_chart_draw_list(
    values: &[(f64, u8)],
    log_scale: bool,
    width: f32,
    height: f32,
    dark: bool,
) -> DrawList {
    let bg = if dark {
        ColorRgba::rgb(12, 12, 15)
    } else {
        ColorRgba::rgb(248, 250, 252)
    };
    let grid = if dark {
        ColorRgba::rgba(148, 163, 184, 34)
    } else {
        ColorRgba::rgba(100, 116, 139, 38)
    };

    let mut dl = DrawList::with_capacity(values.len() * 2 + 6);
    dl.clear(bg);

    let pad_x = 16.0;
    let pad_top = 14.0;
    let pad_bot = 14.0;
    let plot_w = (width - pad_x * 2.0).max(40.0);
    let plot_h = (height - pad_top - pad_bot).max(40.0);

    for step in 0..=3 {
        let y = pad_top + plot_h * (step as f32 / 3.0);
        dl.stroke_line(pad_x, y, width - pad_x, y, 1.0, grid);
    }

    if values.is_empty() {
        return dl.finish();
    }

    let min_v = values
        .iter()
        .map(|(v, _)| *v)
        .fold(f64::INFINITY, f64::min)
        .max(0.5);
    let max_v = values
        .iter()
        .map(|(v, _)| *v)
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let slot_w = plot_w / values.len() as f32;
    let bar_w = (slot_w * 0.56).clamp(14.0, 56.0);

    let lo_bound = bar_chart_lo_bound(min_v, log_scale);
    for (idx, &(val, cidx)) in values.iter().enumerate() {
        let norm = scale_norm(val, lo_bound, max_v, log_scale);
        let bh = (plot_h * norm).max(6.0);
        let x = pad_x + idx as f32 * slot_w + (slot_w - bar_w) * 0.5;
        let y = pad_top + plot_h - bh;
        let (solid, tint) = series_color(cidx, dark);
        dl.fill_rounded_rect(x - 2.0, y - 2.0, bar_w + 4.0, bh + 2.0, 7.0, tint);
        dl.fill_rounded_rect(x, y, bar_w, bh, 6.0, solid);
    }

    dl.finish()
}

pub(crate) fn build_line_chart_draw_list(
    points: &[ProgressionPoint],
    log_scale: bool,
    width: f32,
    height: f32,
    dark: bool,
    color_idx: u8,
) -> DrawList {
    let bg = if dark {
        ColorRgba::rgb(12, 12, 15)
    } else {
        ColorRgba::rgb(248, 250, 252)
    };
    let grid = if dark {
        ColorRgba::rgba(148, 163, 184, 34)
    } else {
        ColorRgba::rgba(100, 116, 139, 38)
    };
    let (stroke, tint) = series_color(color_idx, dark);

    let mut dl = DrawList::with_capacity(points.len() * 3 + 8);
    dl.clear(bg);

    let pad_x = 18.0;
    let pad_top = 16.0;
    let pad_bot = 16.0;
    let plot_w = (width - pad_x * 2.0).max(40.0);
    let plot_h = (height - pad_top - pad_bot).max(40.0);

    for step in 0..=3 {
        let y = pad_top + plot_h * (step as f32 / 3.0);
        dl.stroke_line(pad_x, y, width - pad_x, y, 1.0, grid);
    }

    if points.len() < 2 {
        return dl.finish();
    }

    let min_v = points
        .iter()
        .map(|p| p.value)
        .fold(f64::INFINITY, f64::min);
    let max_v = points.iter().map(|p| p.value).fold(0.0_f64, f64::max);
    let lo_bound = line_chart_lo_bound(min_v, log_scale);

    let mut coords: Vec<(f32, f32)> = Vec::with_capacity(points.len());
    let denom = (points.len() - 1) as f32;
    for (idx, pt) in points.iter().enumerate() {
        let x = pad_x + plot_w * (idx as f32 / denom);
        let norm = scale_norm(pt.value, lo_bound, max_v, log_scale);
        let y = pad_top + plot_h * (1.0 - norm);
        coords.push((x, y));
    }

    for &(x, y) in &coords {
        let h = (pad_top + plot_h - y).max(2.0);
        dl.fill_rounded_rect(x - 6.0, y, 12.0, h, 4.0, tint);
    }

    dl.stroke_polyline(&coords, 2.5, stroke);

    for &(x, y) in &coords {
        dl.fill_circle(x, y, 6.0, tint);
        dl.fill_circle(x, y, 3.6, stroke);
    }

    dl.finish()
}
