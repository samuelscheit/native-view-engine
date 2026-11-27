mod common;

use std::sync::Arc;
use std::time::Instant;

use common::{
    MultiBackendHarness, data_attr_in_store, find_in_store, live_node_ids, register_studio_tables,
};
use nve_abi::{
    EventPayload, NativeEvent, NativeEventKind, NodeId, NveDirectEvent, NveDirectEventRef,
    NveEventBatchRef,
};
use nve_backend_headless::{HeadlessBackend, TaffyStoreLayout};
use nve_engine::reloc::{
    MmapReloc, RelocBundleView, VerifyMode, encode_reloc_bundle, write_reloc_atomic,
};
use nve_engine::{
    DynLayoutEngine, LayoutRect, NveApp, StoreBackend, StoreLayoutEngine, StoreWireReplica,
    TextMeasurer, Viewport,
};
use nve_layout_yoga::YogaLayoutEngine;
use nve_store::{ChangeSet, NodeId as StoreNodeId, NodeStore};
use nve_studio_workspace::{
    AccentTheme, ActiveView, CornerRadiusMode, DensityMode, LayoutEngineMode, StudioWorkspaceModule,
};

#[test]
fn mmap_snapshot_instant_boot_frame0_and_studio_state_restore_sub_millisecond() {
    let snap_dir =
        std::env::temp_dir().join(format!("nve-studio-snap-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&snap_dir);
    std::fs::create_dir_all(&snap_dir).expect("create temp snapshot dir");
    let snap_path = snap_dir.join("studio_workspace.nvesnap");

    let vp = Viewport {
        width: 960.0,
        height: 640.0,
        scale_factor: 1.0,
    };

    register_studio_tables();
    let mut app = common::headless_module(1);
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(1, vp);
    let mut host = StoreWireReplica::new(backend, nve_components::tables::core_realization_order());
    let f1 = app.render(1).to_vec();
    host.submit_nve2_frame(&f1)
        .expect("submit initial studio frame");

    let press_test_id = |host: &mut StoreWireReplica<HeadlessBackend>,
                         app: &mut StudioWorkspaceModule,
                         tid: &str,
                         fid: u64| {
        let nid =
            find_in_store(host.store(), tid).unwrap_or_else(|| panic!("missing testID='{tid}'"));
        let rect = host.backend().layout_of(nid).expect("node layout");
        let ev = [NveDirectEvent::press(
            1,
            nid,
            fid * 16_000,
            rect.x + rect.width * 0.5,
            rect.y + rect.height * 0.5,
        )];
        // SAFETY: `ev` outlives the synchronous `dispatch_events` call.
        let batch = unsafe { NveEventBatchRef::from_slice(&ev) };
        assert!(app.dispatch_events(batch), "press '{tid}' not handled");
        let frame = app.render(fid).to_vec();
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

    let mut freeze_app = common::headless_module(1);
    freeze_app.restore_state(&saved_state_bytes);
    let frame0_bytes = freeze_app.render(1).to_vec();
    assert!(frame0_bytes.len() > 24);

    let bundle = encode_reloc_bundle(
        1,
        0,
        &frame0_bytes,
        &saved_state_bytes,
        b"viewport=960x640@1x;app=studio-workspace",
    )
    .expect("encode studio NVERELOC bundle");
    write_reloc_atomic(&snap_path, &bundle).expect("write studio NVERELOC atomically");

    let mut cold_backend = HeadlessBackend::new();
    cold_backend.set_viewport(1, vp);
    let mut cold_host = StoreWireReplica::new(
        cold_backend,
        nve_components::tables::core_realization_order(),
    );

    let t0 = Instant::now();
    let mapped =
        MmapReloc::open_with_schema(&snap_path, 1, VerifyMode::Full).expect("cold mmap open");
    let snap = RelocBundleView::from_view(mapped.view()).expect("parse NVERELOC bundle");
    let t_mmap = t0.elapsed();

    let base = mapped.as_bytes().as_ptr() as usize;
    let end = base + mapped.as_bytes().len();
    let f0_ptr = snap.frame0().as_ptr() as usize;
    let st_ptr = snap.state().as_ptr() as usize;
    assert!(f0_ptr >= base && f0_ptr + snap.frame0().len() <= end);
    assert!(st_ptr >= base && st_ptr + snap.state().len() <= end);

    cold_host
        .submit_nve2_frame(snap.frame0())
        .expect("zero-copy Frame-0 submit");
    let t_frame0 = t0.elapsed();

    let frame0_tree = cold_host.backend().render_ascii_tree(cold_host.store());
    assert!(frame0_tree.contains("# engineering"));
    assert!(frame0_tree.contains("Ship It - all checks green!"));
    assert!(frame0_tree.contains("X-Ray HUD: ON"));
    assert!(frame0_tree.contains("Snapshot 7 / 7"));

    let mut cold_app = common::headless_module(1);
    let t_state_start = Instant::now();
    cold_app.restore_state(snap.state());
    let t_state = t_state_start.elapsed();
    assert_eq!(cold_app.state_snapshot(), expected_state);

    eprintln!(
        "[studio-workspace reloc] size={} B | cold mmap open={} us | mmap+Frame0 submit={} us | state restore={} us",
        mapped.as_bytes().len(),
        t_mmap.as_micros(),
        t_frame0.as_micros(),
        t_state.as_micros(),
    );

    if let Ok(export_path) = std::env::var("NVE_TEST_EXPORT_RUST_SNAPSHOT")
        && !export_path.is_empty()
    {
        std::fs::copy(&snap_path, &export_path).expect("export rust NVERELOC for C++ test");
    }
    if let Ok(cpp_in) = std::env::var("NVE_TEST_CPP_SNAPSHOT_IN")
        && !cpp_in.is_empty()
    {
        let cpp_mapped = MmapReloc::open_with_schema(&cpp_in, 1, VerifyMode::Full)
            .expect("open C++-written NVERELOC");
        let cpp_snap = RelocBundleView::from_view(cpp_mapped.view()).expect("parse C++ bundle");
        assert_eq!(cpp_snap.meta(), b"cpp-roundtrip");
        let mut verify_backend = HeadlessBackend::new();
        verify_backend.set_viewport(1, vp);
        let mut verify_host = StoreWireReplica::new(
            verify_backend,
            nve_components::tables::core_realization_order(),
        );
        verify_host
            .submit_nve2_frame(cpp_snap.frame0())
            .expect("submit C++-written Frame-0 into Rust HeadlessBackend");
        let verify_tree = verify_host.backend().render_ascii_tree(verify_host.store());
        assert!(verify_tree.contains("# engineering"));
        assert!(verify_tree.contains("X-Ray HUD: ON"));
        let mut verify_app = common::headless_module(1);
        verify_app.restore_state(cpp_snap.state());
        assert_eq!(verify_app.state_snapshot(), expected_state);
    }

    let max_mmap_us: u128 = if cfg!(target_os = "windows") {
        25_000
    } else {
        5_000
    };
    assert!(
        t_mmap.as_micros() < max_mmap_us,
        "cold mmap open unexpectedly slow: {t_mmap:?} (limit {max_mmap_us} us)"
    );
    assert!(
        t_frame0.as_millis() < 50,
        "cold mmap + Frame-0 restore unexpectedly slow: {t_frame0:?}"
    );

    drop(mapped);
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
        let handled = h.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: nid,
            timestamp_us: h.next_frame * 16_000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: cx, y: cy },
        }]);
        assert!(handled);
        let t_disp = t0.elapsed().as_micros();
        let fid = h.next_frame;
        h.next_frame += 1;
        let rec_us = h.surface.reconcile(fid);
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

    fn compute_layout(&mut self, store: &NodeStore, root: StoreNodeId, viewport: Viewport) {
        self.inner.compute_layout(store, root, viewport);
    }

    fn layout_of(&self, id: StoreNodeId) -> Option<LayoutRect> {
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

    let mut app = common::headless_module(1);
    let mut dyn_backend: HeadlessBackend<DynLayoutEngine> =
        HeadlessBackend::with_layout(Box::new(TaffyStoreLayout::new()));
    dyn_backend.set_viewport(1, vp);
    let mut replica = StoreWireReplica::new(
        dyn_backend,
        nve_components::tables::core_realization_order(),
    );

    let f1 = app.render(1).to_vec();
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
        let ev = [NveDirectEvent::press(
            1,
            nid,
            fid * 16_000,
            rect.x + rect.width * 0.5,
            rect.y + rect.height * 0.5,
        )];
        // SAFETY: `ev` outlives the synchronous `dispatch_events` call.
        assert!(app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&ev) }));
        let frame = app.render(fid).to_vec();
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
    // SAFETY: `emitted` stays borrowed from `app` for the duration of `ev0`.
    let ev0 = unsafe { NveDirectEventRef::from_raw(&emitted[0]) };
    assert_eq!(ev0.kind(), Some(NativeEventKind::Custom));
    assert_eq!(ev0.name(), "layout_engine_changed");
    assert_eq!(ev0.value().as_str(), Some("yoga"));

    let taffy_rects_before: Vec<(NodeId, LayoutRect)> = {
        let mut pairs: Vec<(NodeId, LayoutRect)> = live_node_ids(replica.store())
            .into_iter()
            .filter_map(|id| replica.backend().layout_of(id.raw()).map(|r| (id.raw(), r)))
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
