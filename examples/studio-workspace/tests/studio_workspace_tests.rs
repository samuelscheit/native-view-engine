mod common;

use std::path::PathBuf;

use common::{find_in_store, live_node_ids, register_studio_tables, write_ppm, MultiBackendHarness};
use nve_abi::{
    ColorRgba, EventBatchBuf, FrameId, NveGestureDisposition, NveMapEntry, NvePropsRef,
    NveSizeConstraints, NveValue, SurfaceId, TimestampUs,
};
use nve_backend_headless::HeadlessBackend;
use nve_components::tables::Visual;
use nve_engine::loader::HotAppInstance;
use nve_engine::{HotPatchMsg, NveApp, StoreBackend, StoreWireReplica, Viewport};
use nve_studio_workspace::{
    AccentTheme, ActiveView, ChannelKind, CornerRadiusMode, DensityMode, StudioWorkspaceModule,
    SyncTransportMode, WorkspaceState, MAX_HISTORY, WORKSPACE_STATE_BIN_MAGIC,
};

#[test]
fn single_command_buffer_drives_all_backends_views_and_interactions() {
    let mut h = MultiBackendHarness::new(960.0, 640.0);

    let tree = h.ascii();
    let html = h.html();
    for expected in [
        "AETHER",
        "Issue Board",
        "AET-101",
        "AET-102",
        "NVE STUDIO",
        "Snapshot 1 / 1",
    ] {
        assert!(tree.contains(expected), "headless missing '{expected}'");
        assert!(html.contains(expected), "web missing '{expected}'");
    }
    h.skia.rebuild_display_list(h.surface.store());
    assert!(!h.skia.display_list().ops.is_empty());

    h.press("btn-new-task");
    assert!(h.ascii().contains("AET-105"));
    assert!(h.html().contains("AET-105"));
    assert!(h.ascii().contains("All (5)"));
    assert!(h.ascii().contains("Active (4)"));

    h.press("task-status-2");
    assert!(h.ascii().contains("Status -> In Progress"));
    h.press("task-status-2");
    assert!(h.ascii().contains("Status -> Done"));
    assert!(h.ascii().contains("Done (2)"));

    h.press("filter-done");
    assert!(h.ascii().contains("AET-102"));
    assert!(h.ascii().contains("AET-103"));
    assert!(!h.ascii().contains("AET-101"));

    h.press("filter-active");
    assert!(h.ascii().contains("AET-101"));
    assert!(!h.ascii().contains("AET-103"));

    h.press("filter-all");
    assert!(h.ascii().contains("AET-101"));
    assert!(h.ascii().contains("AET-103"));

    h.press("nav-engineering");
    assert!(h.ascii().contains("# engineering"));
    assert!(h.ascii().contains("+1  4"));
    h.press("msg-react-1");
    assert!(h.ascii().contains("+1  5"));
    h.press("reply-ship");
    assert!(h.ascii().contains("Ship It - all checks green!"));
    h.press("reply-bench");
    assert!(h.ascii().contains("Run Benches - checking p99."));

    h.press("nav-design");
    assert!(h.ascii().contains("# design-sync"));
    h.press("msg-react-101");
    assert!(h.ascii().contains("+1  6"));
    h.press("reply-lgtm");
    assert!(h.ascii().contains("LGTM - clean single-ABI PR."));

    h.press("nav-metrics");
    assert!(h.ascii().contains("Live Metrics"));
    assert!(h.ascii().contains("Node Batch #04"));
    h.press("stress-add-8");
    assert!(h.ascii().contains("Node Batch #12"));
    h.press("stress-add-24");
    assert!(h.ascii().contains("Node Batch #36"));
    h.press("stress-reset");
    assert!(h.ascii().contains("Node Batch #04"));
    assert!(!h.ascii().contains("Node Batch #05"));
}

#[test]
fn studio_live_self_patching_xray_and_deterministic_time_travel() {
    let mut h = MultiBackendHarness::new(960.0, 640.0);

    h.press("accent-violet");
    assert!(h.ascii().contains("Accent -> Violet"));
    let ssr_violet = h.web.render_ssr(h.surface.store());
    assert!(
        ssr_violet.style_css.contains("#a78bfa"),
        "expected violet #a78bfa in SSR stylesheet, got:\n{}",
        ssr_violet.style_css
    );

    h.press("radius-pill");
    assert!(h.ascii().contains("Radius -> Pill (18)"));
    let ssr_pill = h.web.render_ssr(h.surface.store());
    assert!(
        ssr_pill.style_css.contains("border-radius:18px"),
        "expected border-radius:18px in SSR stylesheet, got:\n{}",
        ssr_pill.style_css
    );

    h.press("radius-sharp");
    assert!(
        h.ascii().contains("Radius -> Sharp (0)"),
        "expected 'Radius -> Sharp (0)' in tree:\n{}",
        h.ascii()
    );
    let ssr_sharp = h.web.render_ssr(h.surface.store());
    assert!(
        !ssr_sharp.style_css.contains("border-radius:18px"),
        "expected sharp radius to clear border-radius:18px from SSR stylesheet, got:\n{}",
        ssr_sharp.style_css
    );

    h.press("density-compact");
    assert!(h.ascii().contains("Density -> Compact"));

    h.press("toggle-xray");
    assert!(h.ascii().contains("X-Ray HUD: ON"));
    let has_xray_card_border = live_node_ids(h.surface.store()).into_iter().any(|id| {
        h.surface.store().try_row::<Visual>(id).is_some_and(|v| {
            v.border_width() >= 2.0
                && ColorRgba::from_u32(v.border_color().0) == ColorRgba::rgb(167, 139, 250)
        })
    });
    assert!(has_xray_card_border, "X-Ray violet card border expected");

    h.press("toggle-xray");
    assert!(h.ascii().contains("X-Ray HUD: OFF"));
    let still_has_xray = live_node_ids(h.surface.store()).into_iter().any(|id| {
        h.surface.store().try_row::<Visual>(id).is_some_and(|v| {
            v.border_width() >= 2.0
                && ColorRgba::from_u32(v.border_color().0) == ColorRgba::rgb(167, 139, 250)
        })
    });
    assert!(!still_has_xray, "X-Ray borders must cleanly reset when OFF");

    h.press("btn-new-task");
    assert!(h.ascii().contains("AET-105"));
    assert!(h.ascii().contains("Snapshot 8 / 8"));

    h.press("tt-undo");
    assert!(h.ascii().contains("Snapshot 7 / 8"));
    assert!(!h.ascii().contains("AET-105"));

    h.press("tt-first");
    assert!(h.ascii().contains("Snapshot 1 / 8"));
    assert!(h.ascii().contains("Workspace Boot"));
    let ssr_first = h.web.render_ssr(h.surface.store());
    assert!(
        ssr_first.style_css.contains("#38bdf8"),
        "expected cyan #38bdf8 in SSR stylesheet, got:\n{}",
        ssr_first.style_css
    );

    h.press("tt-redo");
    assert!(h.ascii().contains("Snapshot 2 / 8"));
    assert!(h.ascii().contains("Accent -> Violet"));

    h.press("tt-live");
    assert!(h.ascii().contains("Snapshot 8 / 8"));
    assert!(h.ascii().contains("AET-105"));

    h.press("tt-first");
    assert!(h.ascii().contains("Snapshot 1 / 8"));
    h.press("accent-green");
    assert!(h.ascii().contains("Snapshot 2 / 2"));
    assert!(h.ascii().contains("Accent -> Green"));

    assert!(h.find_by_test_id("studio-panel").is_some());
    h.press("btn-toggle-studio");
    assert!(h.find_by_test_id("studio-panel").is_none());
    assert!(h.ascii().contains("OFF"));

    h.press("btn-toggle-studio");
    assert!(h.find_by_test_id("studio-panel").is_some());
    assert!(h.ascii().contains("ON"));
}

#[test]
fn history_is_bounded_and_nve_app_snapshot_restore_roundtrips_cleanly() {
    let mut state = WorkspaceState::default();
    for i in 0..80 {
        if i % 2 == 0 {
            state.set_accent(AccentTheme::Amber);
        } else {
            state.set_accent(AccentTheme::Cyan);
        }
    }
    assert_eq!(state.history.len(), MAX_HISTORY);
    assert_eq!(state.cursor, MAX_HISTORY - 1);

    state.scrub_first();
    assert_eq!(state.cursor, 0);
    state.scrub_undo();
    assert_eq!(state.cursor, 0);

    state.scrub_live();
    assert_eq!(state.cursor, MAX_HISTORY - 1);
    state.scrub_redo();
    assert_eq!(state.cursor, MAX_HISTORY - 1);

    let mut app = StudioWorkspaceModule::create(SurfaceId::ROOT);
    let measured = app.measure(NveSizeConstraints {
        min_w: 320.0,
        max_w: 960.0,
        min_h: 240.0,
        max_h: 640.0,
    });
    assert_eq!((measured.width, measured.height), (960.0, 640.0));
    assert_eq!(app.hit_test(10.0, 10.0, 0), NveGestureDisposition::Claim);

    let direct_entries = [
        NveMapEntry::new("accent", NveValue::from_str("amber")),
        NveMapEntry::new("view", NveValue::from_str("metrics")),
        NveMapEntry::new("studioOpen", NveValue::bool(true)),
        NveMapEntry::new("syncTransport", NveValue::from_str("usb-mux")),
        NveMapEntry::new("syncPeers", NveValue::i64(3)),
        NveMapEntry::new("syncKbps", NveValue::f64(4.6)),
        NveMapEntry::new("syncCompression", NveValue::f64(7.2)),
        NveMapEntry::new("syncRttMs", NveValue::f64(0.8)),
    ];
    // SAFETY: `direct_entries` outlives `direct_props`.
    let direct_props = unsafe { NvePropsRef::from_slice(&direct_entries) };
    assert!(app.set_props(direct_props));
    let _ = app.render(FrameId(1));
    let patch = HotPatchMsg::TemplatePatch {
        generation: 1,
        template_id: "studio".into(),
        target_test_id: None,
        node_path: vec![],
        attr_updates: vec![("opacity".into(), "0.95".into())],
        text_updates: vec![],
        text_replacements: vec![],
    };
    assert!(app.apply_hot_patch(&patch));
    let _ = app.render(FrameId(2));

    let saved = app.snapshot_state().to_vec();
    assert!(!saved.is_empty());
    assert!(
        saved.starts_with(&WORKSPACE_STATE_BIN_MAGIC),
        "snapshot_state must emit binary NWS1 binary format"
    );

    let mut restored_app = StudioWorkspaceModule::create(SurfaceId::ROOT);
    restored_app.restore_state(b"{not-valid-binary");
    assert_eq!(
        restored_app.state_snapshot().current.studio.accent,
        AccentTheme::Cyan
    );

    restored_app.restore_state(&saved);
    let snap = restored_app.state_snapshot();
    assert_eq!(snap.current.studio.accent, AccentTheme::Amber);
    assert_eq!(snap.current.active_view, ActiveView::Metrics);
    assert!(snap.current.studio.panel_open);
    assert_eq!(snap.sync_telemetry.transport, SyncTransportMode::UsbMux);
    assert_eq!(snap.sync_telemetry.connected_peers, 3);
    assert_eq!(snap.sync_telemetry.kbps_tenths, 46);
    assert_eq!(snap.sync_telemetry.compression_tenths, 72);
    assert_eq!(snap.sync_telemetry.rtt_ms_tenths, 8);

    let green_entries = [NveMapEntry::new("accent", NveValue::from_str("green"))];
    // SAFETY: `green_entries` outlives the synchronous `set_props` call.
    assert!(restored_app.set_props(unsafe { NvePropsRef::from_slice(&green_entries) }));
    assert_eq!(
        restored_app.state_snapshot().current.studio.accent,
        AccentTheme::Green
    );

    register_studio_tables();
    let frame = restored_app.render(FrameId(3));
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 960.0,
            height: 640.0,
            scale_factor: 1.0,
        },
    );
    let mut host = StoreWireReplica::new(backend, nve_components::tables::core_realization_order());
    host.submit_nve2_frame(frame).unwrap();
    let tree = host.backend().render_ascii_tree(host.store());
    assert!(tree.contains("Live Metrics"));
    assert!(tree.contains("LIVE SYNC"));
    assert!(tree.contains("USB Mux"));
    assert!(tree.contains("3 peers"));
    assert!(tree.contains("4.6 KB/s | 7.2x | 0.8ms"));

    let mut s2 = WorkspaceState::default();
    s2.set_radius(CornerRadiusMode::Pill);
    s2.set_density(DensityMode::Compact);
    s2.toggle_xray();
    s2.send_quick_reply(ChannelKind::Engineering, "Checking roundtrip");
    let raw = s2.to_bytes();
    assert!(raw.starts_with(&WORKSPACE_STATE_BIN_MAGIC));
    let decoded = WorkspaceState::from_bytes(&raw).expect("roundtrip state");
    assert_eq!(decoded, s2);
}

#[test]
fn tier2_cdylib_hotswap_loads_via_nve_get_interface_and_preserves_full_state() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let dylib_path = workspace_root.join(format!(
        "target/debug/{}nve_studio_workspace.{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_EXTENSION
    ));
    if !dylib_path.exists() {
        let status = std::process::Command::new("cargo")
            .args(["build", "-p", "nve-studio-workspace"])
            .current_dir(workspace_root)
            .status()
            .expect("build nve-studio-workspace cdylib");
        assert!(status.success());
    }

    let vp = Viewport {
        width: 960.0,
        height: 640.0,
        scale_factor: 1.0,
    };
    register_studio_tables();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(SurfaceId::ROOT, vp);
    let mut host = StoreWireReplica::new(backend, nve_components::tables::core_realization_order());

    let mut inst = HotAppInstance::load(&dylib_path, SurfaceId::ROOT, 1, None).expect("load gen 1");
    let f1 = inst.render(FrameId(1));
    assert!(!f1.is_empty());
    host.submit_nve2_frame(f1).unwrap();
    assert!(host.backend().render_ascii_tree(host.store()).contains("AETHER"));
    assert!(host.backend().render_ascii_tree(host.store()).contains("AET-101"));

    let press_id = |host: &mut StoreWireReplica<HeadlessBackend>,
                    inst: &mut HotAppInstance,
                    tid: &str,
                    fid: u64| {
        let nid = find_in_store(host.store(), tid)
            .unwrap_or_else(|| panic!("missing testID='{tid}'"));
        let rect = host.backend().layout_of(nid).unwrap();
        let mut ev = EventBatchBuf::new();
        ev.push_press(
            SurfaceId::ROOT,
            nid,
            TimestampUs(fid * 16_000),
            rect.x + rect.width * 0.5,
            rect.y + rect.height * 0.5,
        );
        assert!(inst.dispatch_events(ev.as_validated()));
        let frame = inst.render(FrameId(fid));
        host.submit_nve2_frame(frame).unwrap();
    };

    press_id(&mut host, &mut inst, "btn-new-task", 2);
    press_id(&mut host, &mut inst, "accent-violet", 3);
    press_id(&mut host, &mut inst, "toggle-xray", 4);
    assert!(host.backend().render_ascii_tree(host.store()).contains("AET-105"));
    assert!(host.backend().render_ascii_tree(host.store()).contains("X-Ray HUD: ON"));
    assert!(host.backend().render_ascii_tree(host.store()).contains("Snapshot 4 / 4"));

    let outcome = inst.reload(&dylib_path).expect("reload gen 2");
    assert_eq!(outcome.generation, 2);
    assert_eq!(inst.generation(), 2);

    let mut backend2 = HeadlessBackend::new();
    backend2.set_viewport(SurfaceId::ROOT, vp);
    let mut host2 =
        StoreWireReplica::new(backend2, nve_components::tables::core_realization_order());
    let f5 = inst.render(FrameId(5));
    host2.submit_nve2_frame(f5).unwrap();
    let tree2 = host2.backend().render_ascii_tree(host2.store());
    assert!(tree2.contains("AET-105"), "lost new task after reload:\n{tree2}");
    assert!(
        tree2.contains("X-Ray HUD: ON"),
        "lost X-Ray toggle after reload:\n{tree2}"
    );
    assert!(
        tree2.contains("Snapshot 4 / 4"),
        "lost time-travel history after reload:\n{tree2}"
    );
}

#[test]
fn layout_fits_cleanly_at_960x640_and_640x420_and_rasterizes_screenshots() {
    let dump_dir = std::env::var("NVE_STUDIO_DUMP_DIR").ok().map(PathBuf::from);
    if let Some(dir) = &dump_dir {
        std::fs::create_dir_all(dir).expect("create dump dir");
    }

    let mut h960 = MultiBackendHarness::new(960.0, 640.0);
    h960.assert_no_horizontal_overflow("960x640-board");
    let buf_board = h960.rasterize();
    assert!(
        buf_board.pixels().iter().any(|&p| p != 0x000b1120),
        "board screenshot should render non-empty UI pixels"
    );
    if let Some(dir) = &dump_dir {
        write_ppm(&dir.join("01_board_960x640.ppm"), &buf_board);
    }

    h960.press("nav-engineering");
    h960.assert_no_horizontal_overflow("960x640-engineering");
    if let Some(dir) = &dump_dir {
        write_ppm(&dir.join("02_engineering_960x640.ppm"), &h960.rasterize());
    }

    h960.press("nav-metrics");
    h960.press("stress-add-8");
    h960.assert_no_horizontal_overflow("960x640-metrics");
    if let Some(dir) = &dump_dir {
        write_ppm(&dir.join("03_metrics_960x640.ppm"), &h960.rasterize());
    }

    h960.press("nav-board");
    h960.press("accent-violet");
    h960.press("radius-pill");
    h960.press("toggle-xray");
    h960.assert_no_horizontal_overflow("960x640-xray-violet");
    if let Some(dir) = &dump_dir {
        write_ppm(&dir.join("04_xray_violet_960x640.ppm"), &h960.rasterize());
    }

    let mut h640 = MultiBackendHarness::new(640.0, 420.0);
    h640.assert_no_horizontal_overflow("640x420-board");
    if let Some(dir) = &dump_dir {
        write_ppm(&dir.join("05_board_640x420.ppm"), &h640.rasterize());
    }
    h640.press("nav-engineering");
    h640.assert_no_horizontal_overflow("640x420-engineering");
    if let Some(dir) = &dump_dir {
        write_ppm(&dir.join("06_engineering_640x420.ppm"), &h640.rasterize());
    }
    h640.press("nav-metrics");
    h640.assert_no_horizontal_overflow("640x420-metrics");
    if let Some(dir) = &dump_dir {
        write_ppm(&dir.join("07_metrics_640x420.ppm"), &h640.rasterize());
    }
}
