//! Shared deterministic fixtures for `nve-bench` Criterion benchmarks and WASM bundle
//! size reporting.
//!
//! Keeps `NVE2` frames, `NodeStore` fixtures, event batches, Dioxus surfaces, modular FFI
//! modules, and `hosts/web-shell/pkg` size metrics DRY across every benchmark group.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod wasm_bundle_size;

pub use wasm_bundle_size::{
    default_brotli_budget, measure_bundle_dir, measure_web_shell_bundles,
    measure_workspace_web_shell_bundles, AssetSizeBreakdown, BudgetStatus, WasmBundleMetrics,
    WasmBundleReport, DEFAULT_BROTLI_BUDGET_DEV, DEFAULT_BROTLI_BUDGET_KITCHEN_SINK,
    DEFAULT_BROTLI_BUDGET_STUDIO, DEFAULT_WEB_SHELL_BUNDLES,
};

use nve_abi::{
    EventBatchBuf, EventFlags, EventId, EventSet, FrameId, InteractionEvent, InterfaceId, Key64,
    NodeId, NveGestureDisposition, NveIntrinsicSize, NveSizeConstraints, SurfaceId, TimestampUs,
    ValidatedEvents, ROOT_NODE_ID,
};
use nve_tables::tables::{
    register_core_tables, AlignItems, FlexDirection, Interaction, JustifyContent, Layout,
    TextContent, Typography, Visual,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{NveApp, Viewport};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeStore, Rgba};

pub const DEFAULT_VIEWPORT: Viewport = Viewport {
    width: 393.0,
    height: 852.0,
    scale_factor: 2.0,
};

/// Stateful row-list `NodeStore` + `WireEncoder` fixture for initial + incremental `NVE2` benches.
pub struct RowFeedFixture {
    store: NodeStore,
    encoder: WireEncoder,
    changes: ChangeSet,
    row_count: usize,
    initialized: bool,
}

impl RowFeedFixture {
    pub fn new(row_count: usize) -> Self {
        register_core_tables();
        Self {
            store: NodeStore::new(),
            encoder: WireEncoder::default(),
            changes: ChangeSet::default(),
            row_count,
            initialized: false,
        }
    }

    pub fn encode_initial(&mut self, out: &mut Vec<u8>) {
        out.clear();
        self.store = NodeStore::new();
        self.encoder = WireEncoder::default();
        let mut root_children = Vec::with_capacity(self.row_count);
        let mut next_id = ROOT_NODE_ID.raw() + 1;

        {
            let mut tx = self.store.tx();
            for i in 0..self.row_count {
                let row_id = NodeId::new(next_id, 0);
                let text_id = NodeId::new(next_id + 1, 0);
                next_id += 2;

                tx.create_at(row_id);
                tx.set(row_id, Layout::WIDTH, Length::percent(100.0));
                tx.set(row_id, Layout::HEIGHT, Length::points(48.0));
                tx.set(row_id, Layout::FLEX_DIRECTION, FlexDirection::Row);
                tx.set(row_id, Layout::JUSTIFY_CONTENT, JustifyContent::SpaceBetween);
                tx.set(row_id, Layout::ALIGN_ITEMS, AlignItems::Center);
                tx.set(row_id, Layout::PADDING_TOP, Length::points(8.0));
                tx.set(row_id, Layout::PADDING_BOTTOM, Length::points(8.0));
                tx.set(row_id, Layout::PADDING_LEFT, Length::points(16.0));
                tx.set(row_id, Layout::PADDING_RIGHT, Length::points(16.0));
                tx.set(row_id, Layout::ROW_GAP, Length::points(12.0));
                tx.set(row_id, Layout::COLUMN_GAP, Length::points(12.0));
                tx.set(row_id, Visual::BACKGROUND, Rgba::rgb(30, 41, 59));
                tx.set(row_id, Visual::BORDER_COLOR, Rgba::rgb(51, 65, 85));
                tx.set(row_id, Visual::BORDER_WIDTH, 1.0f32);
                tx.set(row_id, Visual::BORDER_RADIUS, 8.0f32);
                tx.edit::<Interaction>(row_id).listeners(EventSet::one(InteractionEvent::Press));

                tx.create_at(text_id);
                tx.set_str(text_id, TextContent::TEXT, &format!("Row #{i} — NVE Bench"));
                tx.set(text_id, Typography::COLOR, Rgba::WHITE);
                tx.set(text_id, Typography::FONT_SIZE, 14.0f32);

                tx.set_children(row_id, &[text_id]);
                root_children.push(row_id);
            }
            tx.set_children(NodeId::ROOT, &root_children);
            tx.commit_into(&mut self.changes);
        }

        self.encoder.encode(&self.store, &self.changes, out);
        self.initialized = true;
    }

    pub fn encode_patch(&mut self, frame_id: FrameId, updates: usize, out: &mut Vec<u8>) {
        if !self.initialized {
            self.encode_initial(out);
            return;
        }
        out.clear();
        if self.row_count > 0 {
            let count = updates.min(self.row_count);
            let mut tx = self.store.tx();
            for i in 0..count {
                let idx = ((frame_id.raw() as usize * 7 + i) % self.row_count) as u32;
                let text_id = NodeId::new(ROOT_NODE_ID.raw() + 1 + idx * 2 + 1, 0);
                tx.set_str(
                    text_id,
                    TextContent::TEXT,
                    &format!("Row #{idx} — Frame {frame_id}"),
                );
            }
            tx.commit_into(&mut self.changes);
        } else {
            self.store.tx().commit_into(&mut self.changes);
        }
        self.encoder.encode(&self.store, &self.changes, out);
    }
}

/// Builds a flat row-list `NVE2` frame with `row_count` rows (2 nodes per row: container + text).
pub fn build_row_tree_cmd(row_count: usize, _frame_id: FrameId) -> Vec<u8> {
    let mut feed = RowFeedFixture::new(row_count);
    let mut out = Vec::with_capacity(row_count.saturating_mul(160).max(4096));
    feed.encode_initial(&mut out);
    out
}

/// Builds an `NVE2` frame containing `node_count` created nodes across core word tables.
pub fn build_node_tree_cmd(node_count: usize, _frame_id: FrameId) -> Vec<u8> {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut root_children = Vec::with_capacity(node_count);

    {
        let mut tx = store.tx();
        for i in 0..node_count {
            let nid = NodeId::new(ROOT_NODE_ID.raw() + 1 + i as u32, 0);
            tx.create_at(nid);
            if i % 2 == 0 {
                tx.set(nid, Layout::WIDTH, Length::percent(100.0));
                tx.set(nid, Layout::HEIGHT, Length::points(32.0));
                tx.set(nid, Layout::FLEX_DIRECTION, FlexDirection::Row);
                tx.set(nid, Layout::PADDING_TOP, Length::points(8.0));
                tx.set(nid, Layout::PADDING_BOTTOM, Length::points(8.0));
                tx.set(nid, Layout::PADDING_LEFT, Length::points(8.0));
                tx.set(nid, Layout::PADDING_RIGHT, Length::points(8.0));
                tx.set(nid, Layout::ROW_GAP, Length::points(6.0));
                tx.set(nid, Layout::COLUMN_GAP, Length::points(6.0));
                tx.set(nid, Visual::BACKGROUND, Rgba::rgb(15, 23, 42));
                tx.set(nid, Visual::BORDER_RADIUS, 6.0f32);
                tx.edit::<Interaction>(nid).listeners(EventSet::one(InteractionEvent::Press));
            } else {
                tx.set_str(nid, TextContent::TEXT, &format!("Node #{i}"));
                tx.set(nid, Typography::COLOR, Rgba::rgb(226, 232, 240));
                tx.set(nid, Typography::FONT_SIZE, 14.0f32);
            }
            root_children.push(nid);
        }
        tx.set_children(NodeId::ROOT, &root_children);
        tx.commit_into(&mut changes);
    }

    let mut enc = WireEncoder::default();
    let mut out = Vec::with_capacity(node_count.saturating_mul(96).max(1024));
    enc.encode(&store, &changes, &mut out);
    out
}

/// Builds a styled multi-element card scene `NVE2` frame suitable for Skia display-list + raster benchmarks.
pub fn build_card_scene_cmd(_frame_id: FrameId) -> Vec<u8> {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();

    {
        let mut tx = store.tx();
        let outer = NodeId::new(2, 0);
        let card = NodeId::new(3, 0);
        let eyebrow = NodeId::new(4, 0);
        let headline = NodeId::new(5, 0);
        let button = NodeId::new(6, 0);
        let btn_label = NodeId::new(7, 0);

        tx.create_at(outer);
        tx.set(outer, Layout::WIDTH, Length::percent(100.0));
        tx.set(outer, Layout::HEIGHT, Length::percent(100.0));
        tx.set(outer, Layout::PADDING_TOP, Length::points(24.0));
        tx.set(outer, Layout::PADDING_BOTTOM, Length::points(24.0));
        tx.set(outer, Layout::PADDING_LEFT, Length::points(24.0));
        tx.set(outer, Layout::PADDING_RIGHT, Length::points(24.0));
        tx.set(outer, Layout::ROW_GAP, Length::points(16.0));
        tx.set(outer, Layout::FLEX_DIRECTION, FlexDirection::Column);
        tx.set(outer, Visual::BACKGROUND, Rgba::rgb(15, 23, 42));

        tx.create_at(card);
        tx.set(card, Layout::WIDTH, Length::percent(100.0));
        tx.set(card, Layout::PADDING_TOP, Length::points(20.0));
        tx.set(card, Layout::PADDING_BOTTOM, Length::points(20.0));
        tx.set(card, Layout::PADDING_LEFT, Length::points(20.0));
        tx.set(card, Layout::PADDING_RIGHT, Length::points(20.0));
        tx.set(card, Layout::ROW_GAP, Length::points(10.0));
        tx.set(card, Layout::FLEX_DIRECTION, FlexDirection::Column);
        tx.set(card, Visual::BACKGROUND, Rgba::rgb(30, 41, 59));
        tx.set(card, Visual::BORDER_COLOR, Rgba::rgb(56, 189, 248));
        tx.set(card, Visual::BORDER_WIDTH, 2.0f32);
        tx.set(card, Visual::BORDER_RADIUS, 16.0f32);

        tx.create_at(eyebrow);
        tx.set(eyebrow, Typography::COLOR, Rgba::rgb(56, 189, 248));
        tx.set(eyebrow, Typography::FONT_SIZE, 14.0f32);
        tx.set_str(eyebrow, TextContent::TEXT, "NATIVE VIEW ENGINE");

        tx.create_at(headline);
        tx.set(headline, Typography::COLOR, Rgba::WHITE);
        tx.set(headline, Typography::FONT_SIZE, 24.0f32);
        tx.set_str(headline, TextContent::TEXT, "Card Benchmark Scene");

        tx.create_at(button);
        tx.set(button, Layout::WIDTH, Length::points(160.0));
        tx.set(button, Layout::HEIGHT, Length::points(48.0));
        tx.set(button, Layout::JUSTIFY_CONTENT, JustifyContent::Center);
        tx.set(button, Layout::ALIGN_ITEMS, AlignItems::Center);
        tx.set(button, Visual::BACKGROUND, Rgba::rgb(37, 99, 235));
        tx.set(button, Visual::BORDER_RADIUS, 12.0f32);
        tx.set(button, Visual::OPACITY, 0.92f32);
        tx.edit::<Interaction>(button).listeners(EventSet::one(InteractionEvent::Press));

        tx.create_at(btn_label);
        tx.set(btn_label, Typography::COLOR, Rgba::WHITE);
        tx.set(btn_label, Typography::FONT_SIZE, 16.0f32);
        tx.set_str(btn_label, TextContent::TEXT, "Action");

        tx.set_children(button, &[btn_label]);
        tx.set_children(card, &[eyebrow, headline, button]);
        tx.set_children(outer, &[card]);
        tx.set_children(NodeId::ROOT, &[outer]);
        tx.commit_into(&mut changes);
    }

    let mut enc = WireEncoder::default();
    let mut out = Vec::with_capacity(2048);
    enc.encode(&store, &changes, &mut out);
    out
}

/// Builds a mixed batch of 64-byte [`nve_abi::NveEvent`] payloads for EventBatch benchmarks.
pub fn build_event_batch(count: usize) -> EventBatchBuf {
    let mut batch = EventBatchBuf::with_capacity(count, count * 16);
    let gesture_table = Key64::of("nve.gesture");
    for i in 0..count {
        let node_id = NodeId::new((i as u32 % 100) + 2, 0);
        let ts = TimestampUs((i as u64 + 1) * 16_000);
        match i % 4 {
            0 => batch.push_press(
                SurfaceId::ROOT,
                node_id,
                ts,
                (i % 390) as f32 + 0.5,
                (i % 800) as f32 + 0.25,
            ),
            1 => batch.push_scroll(SurfaceId::ROOT, node_id, ts, 0.0, i as f32 * 12.0, 393.0, 4800.0),
            2 => batch.push_text_change(SurfaceId::ROOT, node_id, ts, &format!("query-{i}")),
            _ => batch.push_table_event(
                gesture_table,
                SurfaceId::ROOT,
                node_id,
                ts,
                EventId(1),
                EventFlags::EMPTY,
                [0; 6],
                &[1, 2, 3, 4],
            ),
        }
    }
    batch
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
    DioxusSurface::new(SurfaceId::ROOT, BenchCounterApp)
}

/// Minimal `NveApp` exported via `export_nve_app!` for zero-cost `nve_get_interface` FFI benchmarks.
pub struct BenchFfiApp {
    surface_id: SurfaceId,
    frame_buf: Vec<u8>,
}

impl NveApp for BenchFfiApp {
    fn create(surface_id: SurfaceId) -> Self {
        register_core_tables();
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        store.tx().commit_into(&mut changes);
        let mut enc = WireEncoder::default();
        let mut frame_buf = Vec::with_capacity(64);
        enc.encode(&store, &changes, &mut frame_buf);
        Self {
            surface_id,
            frame_buf,
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let _ = (self.surface_id, frame_id);
        &self.frame_buf
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        !events.is_empty()
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
    pub fn nve_bench_get_interface(interface_id: InterfaceId) -> *const std::ffi::c_void;
}
