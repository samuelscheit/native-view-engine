//! Shared deterministic fixtures for `nve-bench` Criterion benchmarks and WASM bundle
//! size reporting.
//!
//! Keeps command buffers, event batches, Dioxus surfaces, modular FFI modules, and
//! `hosts/web-shell/pkg` size metrics DRY across every benchmark group.

pub mod arena_mmap;
pub mod wasm_bundle_size;

pub use wasm_bundle_size::{
    default_brotli_budget, measure_bundle_dir, measure_web_shell_bundles,
    measure_workspace_web_shell_bundles, AssetSizeBreakdown, BudgetStatus, WasmBundleMetrics,
    WasmBundleReport, DEFAULT_BROTLI_BUDGET_COUNTER, DEFAULT_BROTLI_BUDGET_DEV,
    DEFAULT_BROTLI_BUDGET_STUDIO, DEFAULT_WEB_SHELL_BUNDLES,
};

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask,
    EventPayload, FlexAlign, FlexDirection, FlexJustify, FrameId, LayoutProps, NativeEvent,
    NativeEventKind, NveGestureDisposition, NveIntrinsicSize, NveSizeConstraints, RawPropValue,
    StyleProps, SurfaceId, ROOT_NODE_ID,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{NveApp, Viewport};

pub const DEFAULT_VIEWPORT: Viewport = Viewport {
    width: 393.0,
    height: 852.0,
    scale_factor: 2.0,
};

/// Builds a flat row-list `CommandBuffer` with `row_count` rows (2 nodes per row: View + Text).
/// When `total_nodes` is desired directly, pass `total_nodes / 2`.
pub fn build_row_tree_cmd(row_count: usize, frame_id: FrameId) -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, frame_id);
    let mut root_children = Vec::with_capacity(row_count);
    let mut next_id = ROOT_NODE_ID + 1;

    let row_layout = LayoutProps {
        width: Dimension::Percent(100.0),
        height: Dimension::Points(48.0),
        flex_direction: FlexDirection::Row,
        justify_content: FlexJustify::SpaceBetween,
        align_items: FlexAlign::Center,
        padding: EdgeInsets::symmetric(8.0, 16.0),
        gap: 12.0,
        ..Default::default()
    };
    let row_style = StyleProps {
        background: ColorRgba::rgb(30, 41, 59),
        text_color: ColorRgba::WHITE,
        border_color: ColorRgba::rgb(51, 65, 85),
        border_width: 1.0,
        border_radius: 8.0,
        opacity: 1.0,
        font_size: 16.0,
        z_index: 0,
    };

    for i in 0..row_count {
        let row_id = next_id;
        let text_id = next_id + 1;
        next_id += 2;

        b.create_node(row_id, ComponentKind::View, None);
        b.set_layout(row_id, &row_layout);
        b.set_style(row_id, &row_style);
        b.set_event_mask(row_id, EventMask::PRESS);

        b.create_node(text_id, ComponentKind::Text, None);
        b.set_text(text_id, &format!("Row #{i} — NVE Bench"));
        b.set_style(
            text_id,
            &StyleProps {
                text_color: ColorRgba::WHITE,
                font_size: 14.0,
                ..Default::default()
            },
        );
        b.set_children(row_id, &[text_id]);
        root_children.push(row_id);
    }

    b.set_children(ROOT_NODE_ID, &root_children);
    b.commit(true);
    b.into_bytes()
}

/// Builds a `CommandBuffer` containing `node_count` created nodes with varied opcodes
/// (layout, style, text, raw props, event mask, children, commit).
pub fn build_node_tree_cmd(node_count: usize, frame_id: FrameId) -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, frame_id);
    let mut root_children = Vec::with_capacity(node_count);

    let layout = LayoutProps {
        width: Dimension::Percent(100.0),
        height: Dimension::Points(32.0),
        flex_direction: FlexDirection::Row,
        padding: EdgeInsets::all(8.0),
        gap: 6.0,
        ..Default::default()
    };
    let style = StyleProps {
        background: ColorRgba::rgb(15, 23, 42),
        text_color: ColorRgba::rgb(226, 232, 240),
        border_radius: 6.0,
        font_size: 14.0,
        ..Default::default()
    };

    for i in 0..node_count {
        let nid = ROOT_NODE_ID + 1 + i as u32;
        if i % 2 == 0 {
            b.create_node(nid, ComponentKind::View, None);
            b.set_layout(nid, &layout);
            b.set_style(nid, &style);
            b.set_event_mask(nid, EventMask::PRESS);
        } else {
            b.create_node(nid, ComponentKind::Text, None);
            b.set_text(nid, &format!("Node #{i}"));
            b.set_raw_prop(nid, "index", &RawPropValue::I64(i as i64));
        }
        root_children.push(nid);
    }

    b.set_children(ROOT_NODE_ID, &root_children);
    b.commit(true);
    b.into_bytes()
}

/// Builds a styled multi-element card scene suitable for Skia display-list + raster benchmarks.
pub fn build_card_scene_cmd(frame_id: FrameId) -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, frame_id);

    // Outer background container
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Percent(100.0),
            padding: EdgeInsets::all(24.0),
            gap: 16.0,
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(15, 23, 42),
            ..Default::default()
        },
    );

    // Card surface
    b.create_node(3, ComponentKind::View, None);
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            padding: EdgeInsets::all(20.0),
            gap: 10.0,
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
    );
    b.set_style(
        3,
        &StyleProps {
            background: ColorRgba::rgb(30, 41, 59),
            border_color: ColorRgba::rgb(56, 189, 248),
            border_width: 2.0,
            border_radius: 16.0,
            ..Default::default()
        },
    );

    // Header label
    b.create_node(4, ComponentKind::Text, None);
    b.set_style(
        4,
        &StyleProps {
            text_color: ColorRgba::rgb(56, 189, 248),
            font_size: 14.0,
            ..Default::default()
        },
    );
    b.set_text(4, "NATIVE VIEW ENGINE");

    // Headline
    b.create_node(5, ComponentKind::Text, None);
    b.set_style(
        5,
        &StyleProps {
            text_color: ColorRgba::WHITE,
            font_size: 24.0,
            ..Default::default()
        },
    );
    b.set_text(5, "Card Benchmark Scene");

    // Action button
    b.create_node(6, ComponentKind::Pressable, None);
    b.set_layout(
        6,
        &LayoutProps {
            width: Dimension::Points(160.0),
            height: Dimension::Points(48.0),
            justify_content: FlexJustify::Center,
            align_items: FlexAlign::Center,
            ..Default::default()
        },
    );
    b.set_style(
        6,
        &StyleProps {
            background: ColorRgba::rgb(37, 99, 235),
            border_radius: 12.0,
            opacity: 0.92,
            ..Default::default()
        },
    );
    b.set_event_mask(6, EventMask::PRESS);

    b.create_node(7, ComponentKind::Text, None);
    b.set_style(
        7,
        &StyleProps {
            text_color: ColorRgba::WHITE,
            font_size: 16.0,
            ..Default::default()
        },
    );
    b.set_text(7, "Action");

    b.set_children(6, &[7]);
    b.set_children(3, &[4, 5, 6]);
    b.set_children(2, &[3]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    b.into_bytes()
}

/// Builds an incremental patch updating `updates` text nodes inside a `row_count` row tree.
pub fn build_row_patch_cmd(frame_id: FrameId, row_count: usize, updates: usize) -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, frame_id);
    if row_count > 0 {
        let count = updates.min(row_count);
        for i in 0..count {
            let idx = ((frame_id as usize * 7 + i) % row_count) as u32;
            let text_id = ROOT_NODE_ID + 1 + idx * 2 + 1;
            b.set_text(text_id, &format!("Row #{idx} — Frame {frame_id}"));
        }
    }
    b.commit(true);
    b.into_bytes()
}

/// Builds a mixed batch of `NativeEvent` payloads for EventRing benchmarks.
pub fn build_event_batch(count: usize) -> Vec<NativeEvent> {
    (0..count)
        .map(|i| {
            let node_id = (i as u32 % 100) + 2;
            let ts = (i as u64 + 1) * 16_000;
            match i % 4 {
                0 => NativeEvent {
                    surface_id: 1,
                    node_id,
                    timestamp_us: ts,
                    kind: NativeEventKind::Press,
                    payload: EventPayload::Point {
                        x: (i % 390) as f32 + 0.5,
                        y: (i % 800) as f32 + 0.25,
                    },
                },
                1 => NativeEvent {
                    surface_id: 1,
                    node_id,
                    timestamp_us: ts,
                    kind: NativeEventKind::Scroll,
                    payload: EventPayload::Scroll {
                        x: 0.0,
                        y: i as f32 * 12.0,
                        content_width: 393.0,
                        content_height: 4800.0,
                    },
                },
                2 => NativeEvent {
                    surface_id: 1,
                    node_id,
                    timestamp_us: ts,
                    kind: NativeEventKind::TextChange,
                    payload: EventPayload::Text(format!("query-{i}")),
                },
                _ => NativeEvent {
                    surface_id: 1,
                    node_id,
                    timestamp_us: ts,
                    kind: NativeEventKind::Custom,
                    payload: EventPayload::Custom {
                        name: "onGesture".into(),
                        data: vec![1, 2, 3, 4],
                    },
                },
            }
        })
        .collect()
}

/// Interactive Dioxus component used for initial render, incremental diff, and hot-patch benchmarks.
#[component]
pub fn BenchCounterApp() -> Element {
    let mut count = use_signal(|| 0i32);

    rsx! {
        view {
            style: "width: 100%; height: 100%; padding: 20px; gap: 12px; background-color: #0f172a;",
            view {
                style: "width: 100%; padding: 16px; gap: 8px; background-color: #1e293b; border-radius: 12px;",
                text {
                    style: "color: #38bdf8; font-size: 14px;",
                    "BENCHMARK SURFACE"
                }
                text {
                    style: "color: #ffffff; font-size: 24px;",
                    "Count: {count}"
                }
            }
            pressable {
                style: "width: 160px; height: 48px; background-color: #2563eb; border-radius: 10px;",
                onpress: move |_| count += 1,
                text {
                    style: "color: #ffffff; font-size: 16px;",
                    "Increment"
                }
            }
        }
    }
}

pub fn create_bench_dioxus_surface() -> DioxusSurface {
    DioxusSurface::new(1, BenchCounterApp)
}

/// Minimal `NveApp` exported via `export_nve_app!` for zero-cost `nve_get_interface` FFI benchmarks.
pub struct BenchFfiApp {
    surface_id: SurfaceId,
    frame_buf: Vec<u8>,
}

impl NveApp for BenchFfiApp {
    fn create(surface_id: SurfaceId) -> Self {
        let mut b = CommandBufferBuilder::new(surface_id, 1);
        b.commit(true);
        Self {
            surface_id,
            frame_buf: b.into_bytes(),
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let _ = (self.surface_id, frame_id);
        &self.frame_buf
    }

    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        !encoded_events.is_empty()
    }

    fn measure(&mut self, constraints: NveSizeConstraints) -> NveIntrinsicSize {
        NveIntrinsicSize {
            width: constraints.max_w.clamp(constraints.min_w, 400.0),
            height: constraints.max_h.clamp(constraints.min_h, 240.0),
        }
    }

    fn hit_test(&mut self, x: f32, y: f32, _pointer_phase: u8) -> NveGestureDisposition {
        if (0.0..=400.0).contains(&x) && (0.0..=240.0).contains(&y) {
            NveGestureDisposition::Claim
        } else {
            NveGestureDisposition::PassThrough
        }
    }
}

nve_engine::export_nve_app!(BenchFfiApp, symbol = nve_bench_get_interface);

unsafe extern "C" {
    pub fn nve_bench_get_interface(interface_id: u32) -> *const std::ffi::c_void;
}
