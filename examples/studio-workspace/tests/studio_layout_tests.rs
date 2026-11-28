mod common;

use std::sync::Arc;
use std::time::Instant;

use common::{
    MultiBackendHarness, data_attr_in_store, find_in_store, live_node_ids, register_studio_tables,
};
use nve_abi::{EventBatchBuf, FrameId, NodeId, SurfaceId, TimestampUs};
use nve_backend_headless::HeadlessBackend;
use nve_engine::{
    Frame0File, Frame0Meta, LayoutRect, NveApp, StateFile, StoreBackend, StoreWireReplica,
    Viewport,
};
use nve_layout::{DynLayoutEngine, StoreLayoutEngine, TaffyStoreLayout, TextMeasurer};
use nve_layout_yoga::YogaLayoutEngine;
use nve_store::{ChangeSet, NodeStore};
use nve_studio_workspace::{
    AccentTheme, ActiveView, CornerRadiusMode, DensityMode, LayoutEngineMode, StudioWorkspaceModule,
};

#[test]
fn frame0_nve2_and_state_file_boot_restore_and_responsive_viewport() {
    let snap_dir =
        std::env::temp_dir().join(format!("nve-studio-frame0-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&snap_dir);
    std::fs::create_dir_all(&snap_dir).expect("create temp frame0 dir");
    let frame0_path = snap_dir.join("frame0.nve2");
    let state_path = snap_dir.join("state.nve");

    let vp = Viewport {
        width: 960.0,
        height: 640.0,
        scale_factor: 1.0,
    };

    register_studio_tables();
    let mut app = common::headless_module(SurfaceId::ROOT);
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(SurfaceId::ROOT, vp);
    let mut host = StoreWireReplica::new(backend, nve_tables::tables::core_realization_order());
    let f1 = app.render(FrameId(1)).to_vec();
    host.submit_nve2_frame(&f1)
        .expect("submit initial studio frame");

    let press_test_id = |host: &mut StoreWireReplica<HeadlessBackend>,
                         app: &mut StudioWorkspaceModule,
                         tid: &str,
                         fid: u64| {
        let nid =
            find_in_store(host.store(), tid).unwrap_or_else(|| panic!("missing testID='{tid}'"));
        let rect = host.backend().layout_of(nid).expect("node layout");
        let mut ev = EventBatchBuf::new();
        ev.push_press(
            SurfaceId::ROOT,
            nid,
            TimestampUs(fid * 16_000),
            rect.x + rect.width * 0.5,
            rect.y + rect.height * 0.5,
        );
        assert!(app.dispatch_events(ev.as_validated()), "press '{tid}' not handled");
        let frame = app.render(FrameId(fid)).to_vec();
        host.submit_nve2_frame(&frame)
            .expect("submit frame after press");
    };

    press_test_id(&mut host, &mut app, "btn-new-task", 2);
    press_test_id(&mut host, &mut app, "accent-violet", 3);
    press_test_id(&mut host, &mut app, "radius-pill", 4);
    press_test_id(&mut host, &mut app, "toggle-xray", 5);
    press_test_id(&mut host, &mut app, "nav-engineering", 6);
    press_test_id(&mut host, &mut app, "reply-ship", 7);

    let expected_state = app.state_snapshot();
    assert_eq!(
        expected_state.current.active_view,
        ActiveView::ChannelEngineering
    );
    assert_eq!(expected_state.current.studio.accent, AccentTheme::Violet);
    assert_eq!(expected_state.current.studio.radius, CornerRadiusMode::Pill);
    assert!(expected_state.current.studio.xray);

    let saved_state_bytes = app.snapshot_state().to_vec();
    assert!(!saved_state_bytes.is_empty());

    let meta = Frame0Meta {
        table_schema_hash: nve_tables::tables::core_tables_schema_hash(),
        app_build_id: 1,
        env_hash: 0,
    };
    let footer = Frame0File::save(&frame0_path, host.store(), meta).expect("save frame0.nve2");
    assert!(footer.node_count > 10);

    let mut state_file = StateFile::open(&state_path, 1).expect("open state.nve");
    state_file.save(&saved_state_bytes).expect("save state.nve");
    drop(state_file);

    let mut cold_backend = HeadlessBackend::new();
    cold_backend.set_viewport(SurfaceId::ROOT, vp);
    let mut cold_host =
        StoreWireReplica::new(cold_backend, nve_tables::tables::core_realization_order());

    let t0 = Instant::now();
    let (boot_footer, _) = Frame0File::boot_into_replica(&frame0_path, meta, &mut cold_host)
        .expect("cold Frame-0 NVE2 boot");
    let t_frame0 = t0.elapsed();
    assert_eq!(boot_footer.node_count, footer.node_count);

    let frame0_tree = cold_host.backend().render_ascii_tree(cold_host.store());
    assert!(frame0_tree.contains("# engineering"));
    assert!(frame0_tree.contains("Ship It - all checks green!"));
    assert!(frame0_tree.contains("X-Ray HUD: ON"));
    assert!(frame0_tree.contains("Snapshot 7 / 7"));

    // Boot the identical frame0.nve2 at a wider viewport: live Taffy layout reflects the new width.
    let wide_vp = Viewport {
        width: 1440.0,
        height: 900.0,
        scale_factor: 2.0,
    };
    let mut wide_backend = HeadlessBackend::new();
    wide_backend.set_viewport(SurfaceId::ROOT, wide_vp);
    let mut wide_host = StoreWireReplica::new(
        wide_backend,
        nve_tables::tables::core_realization_order(),
    );
    Frame0File::boot_into_replica(&frame0_path, meta, &mut wide_host)
        .expect("wide viewport Frame-0 boot");
    let root_wide = wide_host
        .backend()
        .layout_of(NodeId::ROOT)
        .expect("root layout");
    assert!((root_wide.width - 1440.0).abs() <= 1.0);

    let reopened_state = StateFile::open(&state_path, 1).expect("reopen state.nve");
    let restored_bytes = reopened_state
        .load()
        .expect("load state.nve")
        .expect("state payload present");
    let mut cold_app = common::headless_module(SurfaceId::ROOT);
    let t_state_start = Instant::now();
    cold_app.restore_state(&restored_bytes);
    let t_state = t_state_start.elapsed();
    assert_eq!(cold_app.state_snapshot(), expected_state);

    eprintln!(
        "[studio-workspace frame0] nodes={} | Frame0 boot={} us | state restore={} us",
        boot_footer.node_count,
        t_frame0.as_micros(),
        t_state.as_micros(),
    );

    assert!(
        t_frame0.as_millis() < 50,
        "cold Frame-0 NVE2 boot unexpectedly slow: {t_frame0:?}"
    );

    let _ = std::fs::remove_dir_all(&snap_dir);
}

#[test]
fn stream_100k_virtualized_list_sub_2ms_jumps_and_fenwick_expand() {
    let mut h = MultiBackendHarness::new(960.0, 640.0);

    h.press("nav-stream");
    assert!(h.ascii().contains("100k Stream"));
    assert!(h.ascii().contains("STR-00001"));
    h.assert_no_horizontal_overflow("960x640-stream-top");

    let read_stream_prop = |h: &MultiBackendHarness, key: &str| -> u32 {
        let vp_id = h
            .find_by_test_id("stream-viewport")
            .expect("stream-viewport");
        let val = h
            .data_attr(vp_id, key)
            .unwrap_or_else(|| panic!("missing prop {key}"));
        val.parse::<u32>().expect("numeric virtual prop")
    };

    let mounted_top = read_stream_prop(&h, "virtual-rendered-count");
    let total_h_initial = read_stream_prop(&h, "virtual-total-height");
    assert!(
        (8..=28).contains(&mounted_top),
        "expected O(viewport) ~15-24 mounted rows, got {mounted_top}"
    );
    assert!(
        total_h_initial > 3_500_000,
        "expected 100,000-row virtual document height > 3.5M px, got {total_h_initial}"
    );

    let mut frontend_frame_us = Vec::new();
    let mut e2e_frame_us = Vec::new();
    for tid in [
        "stream-jump-50k",
        "stream-jump-end",
        "stream-jump-top",
        "stream-jump-50k",
        "stream-jump-end",
        "stream-jump-50k",
    ] {
        let nid = h.find_by_test_id(tid).unwrap();
        let rect = h.headless.layout_of(nid).unwrap();
        let cx = rect.x + rect.width * 0.5;
        let cy = rect.y + rect.height * 0.5;

        let t0 = Instant::now();
        let mut batch = EventBatchBuf::new();
        batch.push_press(
            SurfaceId::ROOT,
            nid,
            TimestampUs(h.next_frame * 16_000),
            cx,
            cy,
        );
        let handled = h.surface.dispatch_events(batch.as_validated());
        assert!(handled);
        let t_disp = t0.elapsed().as_micros();
        let fid = h.next_frame;
        h.next_frame += 1;
        let rec_us = h.surface.reconcile(FrameId(fid));
        let fe_us = t0.elapsed().as_micros();
        let store = h.surface.store();
        let changes = h.surface.last_changes();
        let stats = h.headless.commit(store, changes).unwrap();
        let total_us = t0.elapsed().as_micros();
        eprintln!(
            "  [{tid}] fe={fe_us}us total={total_us}us | disp={t_disp}us rec={rec_us}us | lay={}us diff={}us | muts={}",
            stats.layout_us, stats.diff_us, stats.mutation_count,
        );
        frontend_frame_us.push(fe_us);
        e2e_frame_us.push(total_us);
        h.skia.commit(store, changes).unwrap();
        h.web.commit(store, changes).unwrap();
    }

    let tree_50k = h.ascii();
    assert!(
        tree_50k.contains("STR-50001"),
        "expected row #50,000 (STR-50001) visible:\n{tree_50k}"
    );
    assert!(!tree_50k.contains("STR-00001"));
    let mounted_50k = read_stream_prop(&h, "virtual-rendered-count");
    assert!(
        (8..=28).contains(&mounted_50k),
        "expected ~20 mounted rows at #50,000, got {mounted_50k}"
    );

    h.press("stream-jump-end");
    let tree_end = h.ascii();
    assert!(
        tree_end.contains("STR-100000"),
        "expected final row #99,999 (STR-100000) visible:\n{tree_end}"
    );
    let mounted_end = read_stream_prop(&h, "virtual-rendered-count");
    assert!(
        (8..=28).contains(&mounted_end),
        "expected ~20 mounted rows at #99,999, got {mounted_end}"
    );
    h.assert_no_horizontal_overflow("960x640-stream-end");

    let h_before_expand = read_stream_prop(&h, "virtual-total-height");
    h.press("stream-row-99999");
    let h_after_expand = read_stream_prop(&h, "virtual-total-height");
    assert!(
        h_after_expand > h_before_expand,
        "expanding row #99,999 must increase Fenwick total height ({h_before_expand} -> {h_after_expand})"
    );

    h.press("stream-toggle-heights");
    assert!(h.ascii().contains("O(1) FixedH"));
    h.press("stream-toggle-heights");
    assert!(h.ascii().contains("Fenwick VarH"));

    frontend_frame_us.sort_unstable();
    e2e_frame_us.sort_unstable();
    let fe_median_us = frontend_frame_us[frontend_frame_us.len() / 2];
    let e2e_median_us = e2e_frame_us[e2e_frame_us.len() / 2];

    let v_idx = nve_studio_workspace::stream_row_heights(
        nve_studio_workspace::DEFAULT_STREAM_ROW_COUNT,
        DensityMode::Normal,
        true,
        &[50_000, 99_999],
    );
    let t_slice = Instant::now();
    let mut acc_rows = 0usize;
    for step in 0..1_000usize {
        let target = (step * 97) % nve_studio_workspace::DEFAULT_STREAM_ROW_COUNT;
        let off = v_idx.offset_of(target) as f64;
        let w = v_idx.slice(off, 480.0, 2);
        acc_rows += w.visible_count;
    }
    let slice_1k_us = t_slice.elapsed().as_micros();
    assert!(acc_rows > 10_000);

    eprintln!(
        "[100k stream] mounted={mounted_50k} rows | frontend jump median={fe_median_us}us (all={frontend_frame_us:?}) | e2e debug median={e2e_median_us}us (all={e2e_frame_us:?}) | 1000x Fenwick slices={slice_1k_us}us ({:.2}us/slice)",
        slice_1k_us as f64 / 1000.0
    );
    assert!(
        fe_median_us < 2_000,
        "expected 100k-row virtualized incremental frame time < 2ms (2000us) in debug, got {fe_median_us}us ({frontend_frame_us:?})"
    );
    assert!(
        e2e_median_us < 8_330,
        "expected 100k-row end-to-end jump (incl. debug layout) within 8.33ms 120fps budget, got {e2e_median_us}us"
    );
}

struct HostMirrorLayoutEngine {
    inner: TaffyStoreLayout,
}

impl HostMirrorLayoutEngine {
    fn new() -> Self {
        Self {
            inner: TaffyStoreLayout::new(),
        }
    }
}

impl StoreLayoutEngine for HostMirrorLayoutEngine {
    fn name(&self) -> &'static str {
        "yoga-host-mirror"
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.inner.set_text_measurer(measurer);
    }

    fn sync_changes(&mut self, store: &NodeStore, changes: &ChangeSet) {
        self.inner.sync_changes(store, changes);
    }

    fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) -> bool {
        self.inner.compute_layout(store, root, viewport)
    }

    fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        self.inner.layout_of(id)
    }

    fn reset(&mut self) {
        self.inner.reset();
    }
}

fn read_root_layout_engine_prop(store: &NodeStore) -> String {
    let root_id = find_in_store(store, "workspace-root").expect("workspace-root");
    data_attr_in_store(store, root_id, "layout-engine")
        .expect("workspace-root must emit 'layout-engine' data attr")
}

#[test]
fn live_taffy_yoga_layout_engine_switcher_and_mid_session_parity() {
    register_studio_tables();
    let vp = Viewport {
        width: 960.0,
        height: 640.0,
        scale_factor: 1.0,
    };

    let mut app = common::headless_module(SurfaceId::ROOT);
    let mut dyn_backend: HeadlessBackend<DynLayoutEngine> =
        HeadlessBackend::with_layout(Box::new(TaffyStoreLayout::new()));
    dyn_backend.set_viewport(SurfaceId::ROOT, vp);
    let mut replica =
        StoreWireReplica::new(dyn_backend, nve_tables::tables::core_realization_order());

    let f1 = app.render(FrameId(1)).to_vec();
    replica.submit_nve2_frame(&f1).unwrap();
    assert_eq!(replica.backend().layout_engine().name(), "taffy");
    assert_eq!(read_root_layout_engine_prop(replica.store()), "taffy");
    assert!(
        replica
            .backend()
            .render_ascii_tree(replica.store())
            .contains("Active: Taffy (Rust)")
    );

    let press_dyn = |replica: &mut StoreWireReplica<HeadlessBackend<DynLayoutEngine>>,
                     app: &mut StudioWorkspaceModule,
                     tid: &str,
                     fid: u64|
     -> Vec<u8> {
        let nid =
            find_in_store(replica.store(), tid).unwrap_or_else(|| panic!("missing testID='{tid}'"));
        let rect = replica.backend().layout_of(nid).unwrap();
        let mut ev = EventBatchBuf::new();
        ev.push_press(
            SurfaceId::ROOT,
            nid,
            TimestampUs(fid * 16_000),
            rect.x + rect.width * 0.5,
            rect.y + rect.height * 0.5,
        );
        assert!(app.dispatch_events(ev.as_validated()));
        let frame = app.render(FrameId(fid)).to_vec();
        replica.submit_nve2_frame(&frame).unwrap();
        frame
    };

    press_dyn(&mut replica, &mut app, "btn-new-task", 2);
    press_dyn(&mut replica, &mut app, "accent-violet", 3);
    assert!(
        replica
            .backend()
            .render_ascii_tree(replica.store())
            .contains("AET-105")
    );

    press_dyn(&mut replica, &mut app, "layout-yoga", 4);
    assert_eq!(read_root_layout_engine_prop(replica.store()), "yoga");
    assert!(
        replica
            .backend()
            .render_ascii_tree(replica.store())
            .contains("Active: Yoga (C++20)")
    );
    assert_eq!(
        app.state_snapshot().current.studio.layout_engine,
        LayoutEngineMode::Yoga
    );

    let emitted = app.drain_emitted_events();
    assert_eq!(emitted.len(), 1);
    let ev0 = emitted.get(0).expect("emitted event");
    assert!(ev0.is_custom());
    assert_eq!(ev0.custom_name(), "layout_engine_changed");
    assert_eq!(ev0.custom_data(), b"yoga");

    let taffy_rects_before: Vec<(NodeId, LayoutRect)> = {
        let mut pairs: Vec<(NodeId, LayoutRect)> = live_node_ids(replica.store())
            .into_iter()
            .filter_map(|id| replica.backend().layout_of(id).map(|r| (id, r)))
            .collect();
        pairs.sort_by_key(|(id, _)| *id);
        pairs
    };

    let next_engine: DynLayoutEngine = if YogaLayoutEngine::is_available() {
        Box::new(YogaLayoutEngine::new().expect("real RN Yoga available"))
    } else {
        assert!(YogaLayoutEngine::new().is_err());
        Box::new(HostMirrorLayoutEngine::new())
    };
    let expected_swapped_name = next_engine.name();
    replica = replica.map_backend(|b| b.swap_layout_engine(next_engine));
    let full_changes = {
        let mut c = ChangeSet::default();
        c.created = live_node_ids(replica.store());
        c
    };
    replica.commit_changes(&full_changes).unwrap();
    assert_eq!(
        replica.backend().layout_engine().name(),
        expected_swapped_name
    );

    for (id, taffy_rect) in &taffy_rects_before {
        let swapped_rect = replica
            .backend()
            .layout_of(*id)
            .unwrap_or_else(|| panic!("missing swapped rect for node #{id}"));
        assert!(
            (taffy_rect.x - swapped_rect.x).abs() <= 2.0
                && (taffy_rect.y - swapped_rect.y).abs() <= 2.0
                && (taffy_rect.width - swapped_rect.width).abs() <= 2.0
                && (taffy_rect.height - swapped_rect.height).abs() <= 2.0,
            "node #{id} diverged across Taffy <-> {expected_swapped_name}: taffy={taffy_rect:?}, swapped={swapped_rect:?}"
        );
    }

    assert!(
        replica
            .backend()
            .render_ascii_tree(replica.store())
            .contains("AET-105")
    );
    press_dyn(&mut replica, &mut app, "layout-taffy", 5);
    assert_eq!(read_root_layout_engine_prop(replica.store()), "taffy");
    replica = replica.map_backend(|b| {
        b.swap_layout_engine(Box::new(TaffyStoreLayout::new()) as DynLayoutEngine)
    });
    let full_changes2 = {
        let mut c = ChangeSet::default();
        c.created = live_node_ids(replica.store());
        c
    };
    replica.commit_changes(&full_changes2).unwrap();
    assert_eq!(replica.backend().layout_engine().name(), "taffy");
    assert!(
        replica
            .backend()
            .render_ascii_tree(replica.store())
            .contains("AET-105")
    );

    press_dyn(&mut replica, &mut app, "tt-undo", 6);
    assert_eq!(read_root_layout_engine_prop(replica.store()), "yoga");
    assert!(
        replica
            .backend()
            .render_ascii_tree(replica.store())
            .contains("AET-105")
    );
}
