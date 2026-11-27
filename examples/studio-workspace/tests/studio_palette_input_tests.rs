mod common;

use std::path::PathBuf;

use common::{find_in_store, register_studio_tables, MultiBackendHarness};
use nve_abi::{
    pack_key_modifiers, FrameId, NveDirectEvent, NveEventBatchRef, NveMapEntry, NvePropsRef,
    NveSizeConstraints, NveValue, SurfaceId, ROOT_NODE_ID,
};
use nve_backend_headless::HeadlessBackend;
use nve_engine::{NveApp, StoreBackend, StoreWireReplica, Viewport};
use nve_interop::PluginRequirement;
use nve_studio_workspace::{
    fuzzy_score, AccentTheme, ActiveView, LayoutEngineMode, StudioWorkspaceModule,
    MOBILE_BREAKPOINT_PT,
};
use nve_wire::{NveWireFrame, NveWireFrameRef, NveWireKind, DEFAULT_MAX_WIRE_FRAME_BYTES};

#[test]
fn responsive_mobile_portrait_layout_at_393pt_collapses_drawers_without_crushing_center() {
    assert_eq!(MOBILE_BREAKPOINT_PT, 620.0);

    let mut h = MultiBackendHarness::new(393.0, 852.0);
    h.assert_no_horizontal_overflow("393x852-board");

    assert!(h.find_by_test_id("mobile-top-bar").is_some());
    assert!(h.find_by_test_id("desktop-sidebar").is_none());
    assert!(
        h.find_by_test_id("studio-panel").is_none(),
        "Studio drawer must start collapsed on < 620pt portrait screens"
    );

    let root_id = h.find_by_test_id("workspace-root").unwrap();
    assert_eq!(
        h.data_attr(root_id, "viewport-mode").as_deref(),
        Some("mobile-portrait")
    );

    let center_id = h.find_by_test_id("center-workspace").unwrap();
    let center_rect = h.headless.layout_of(center_id).unwrap();
    assert!(
        center_rect.width >= 390.0 && center_rect.width <= 393.5,
        "expected full-width center workspace at 393pt portrait, got {center_rect:?}"
    );
    assert!(
        center_rect.height >= 790.0,
        "expected tall center workspace below 42pt top bar, got {center_rect:?}"
    );

    h.press("mobile-toggle-nav");
    assert!(h.find_by_test_id("mobile-nav-drawer").is_some());
    h.assert_no_horizontal_overflow("393x852-nav-open");

    h.press("nav-stream");
    assert!(
        h.find_by_test_id("mobile-nav-drawer").is_none(),
        "mobile nav drawer should auto-close after selecting a tab"
    );
    assert!(h.ascii().contains("100k Stream"));
    h.assert_no_horizontal_overflow("393x852-stream");

    h.press("stream-jump-50k");
    assert!(h.ascii().contains("STR-50001"));
    h.assert_no_horizontal_overflow("393x852-stream-50k");

    h.press("btn-toggle-studio");
    let studio_id = h
        .find_by_test_id("studio-panel")
        .expect("mobile studio drawer should open");
    let studio_rect = h.headless.layout_of(studio_id).unwrap();
    assert!(
        studio_rect.width >= 390.0 && studio_rect.width <= 393.5,
        "expected full-width mobile studio drawer, got {studio_rect:?}"
    );
    h.assert_no_horizontal_overflow("393x852-studio-open");

    let center_after_studio = h
        .headless
        .layout_of(h.find_by_test_id("center-workspace").unwrap())
        .unwrap();
    assert!(center_after_studio.width >= 390.0);
    assert!(center_after_studio.height > 100.0);

    h.press("accent-amber");
    h.press("layout-yoga");
    assert!(h.ascii().contains("Active: Yoga (C++20)"));
    h.press("btn-toggle-studio");
    assert!(h.find_by_test_id("studio-panel").is_none());

    let mut app = StudioWorkspaceModule::create(SurfaceId::ROOT);
    let _ = app.render(FrameId(1));
    assert!(!app.state_snapshot().is_mobile_portrait());

    let resize_mobile = [NveDirectEvent::layout(
        SurfaceId::ROOT,
        ROOT_NODE_ID,
        1000,
        0.0,
        0.0,
        393.0,
        852.0,
    )];
    // SAFETY: `resize_mobile` outlives the synchronous `dispatch_events` call.
    assert!(app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&resize_mobile) }));
    assert!(app.state_snapshot().is_mobile_portrait());
    register_studio_tables();
    let mobile_frame = app.render(FrameId(2)).to_vec();
    let mut host = HeadlessBackend::new();
    host.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 393.0,
            height: 852.0,
            scale_factor: 3.0,
        },
    );
    let mut rep = StoreWireReplica::new(host, nve_components::tables::core_realization_order());
    rep.submit_nve2_frame(&mobile_frame).unwrap();
    assert!(rep.backend().render_ascii_tree(rep.store()).contains("Studio OFF"));

    let measured_mobile = app.measure(NveSizeConstraints {
        min_w: 0.0,
        max_w: 393.0,
        min_h: 0.0,
        max_h: 852.0,
    });
    assert_eq!(measured_mobile.width, 393.0);
}

#[test]
fn live_pairing_binary_delta_and_dynamic_plugin_host_with_studio_cdylib() {
    let h = MultiBackendHarness::new(960.0, 640.0);

    let mut snap_app = StudioWorkspaceModule::create(SurfaceId::ROOT);
    let f1 = snap_app.render(FrameId(1)).to_vec();
    let p1 = NveWireFrame::new(NveWireKind::CommandDelta, 1, f1.clone())
        .encode()
        .unwrap();
    let r1 = NveWireFrameRef::decode(&p1, DEFAULT_MAX_WIRE_FRAME_BYTES).unwrap();
    assert_eq!(r1.payload, f1.as_slice());

    let nid = h.find_by_test_id("task-status-2").unwrap();
    let rect = h.headless.layout_of(nid).unwrap();
    let ev = [NveDirectEvent::press(
        SurfaceId::ROOT,
        nid,
        16_000,
        rect.x + rect.width * 0.5,
        rect.y + rect.height * 0.5,
    )];
    // SAFETY: `ev` outlives the synchronous `dispatch_events` call.
    assert!(snap_app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&ev) }));
    let f2_incremental = snap_app.render(FrameId(2)).to_vec();

    let p2 = NveWireFrame::new(NveWireKind::CommandDelta, 2, f2_incremental.clone())
        .encode()
        .unwrap();
    let r2 = NveWireFrameRef::decode(&p2, DEFAULT_MAX_WIRE_FRAME_BYTES).unwrap();
    assert_eq!(r2.header.sequence, 2);
    assert_eq!(r2.payload, f2_incremental.as_slice());

    // SAFETY: `ev` outlives the synchronous `dispatch_events` call.
    assert!(snap_app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&ev) }));
    let f3_incremental = snap_app.render(FrameId(3)).to_vec();
    let p3 = NveWireFrame::new(NveWireKind::CommandDelta, 3, f3_incremental.clone())
        .encode()
        .unwrap();
    let r3 = NveWireFrameRef::decode(&p3, DEFAULT_MAX_WIRE_FRAME_BYTES).unwrap();
    assert_eq!(r3.payload, f3_incremental.as_slice());

    let delta_bytes = p3.len();
    let kbps_60 = (delta_bytes as f64 * 60.0) / 1024.0;
    eprintln!(
        "[studio live pairing] frame2_inc={}B -> wire={}B | frame3_inc={}B -> wire={}B ({:.2} KB/s @60fps)",
        f2_incremental.len(),
        p2.len(),
        f3_incremental.len(),
        delta_bytes,
        kbps_60
    );
    assert!(
        delta_bytes < 256,
        "expected direct zero-reverse-parse Studio CommandDelta wire frame < 256B, got {delta_bytes}B (raw {}B)",
        f3_incremental.len()
    );
    assert!(
        kbps_60 < 15.0,
        "expected direct CommandDelta < 15 KB/s @ 60 edits/sec (zero reverse-parsing), got {kbps_60:.2} KB/s"
    );

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

    let mut runtime = nve_interop::InteropRuntime::new();
    let pid = runtime
        .load_plugin_dynamic("aether-studio", &dylib_path, PluginRequirement::EMBEDDED_COMPONENT)
        .expect("load studio cdylib as plugin");
    let slot = runtime.plugins.get_mut(pid).unwrap();
    let plugin_sid = slot.surface_id();
    assert!(!plugin_sid.is_null());
    assert!(slot.capabilities().surface);
    assert!(slot.capabilities().layout);
    assert!(slot.capabilities().component);
    assert!(slot.capabilities().dev);

    let plugin_entries = [
        NveMapEntry::new("accent", NveValue::from_str("green")),
        NveMapEntry::new("layoutEngine", NveValue::from_str("yoga")),
        NveMapEntry::new("view", NveValue::from_str("stream")),
    ];
    assert!(slot.set_props(&plugin_entries).unwrap());
    let pf1 = slot.render(FrameId(1)).unwrap().to_vec();
    assert!(pf1.len() > 24);

    let gen2 = runtime
        .plugins
        .hot_swap_dynamic(pid, &dylib_path, PluginRequirement::EMBEDDED_COMPONENT)
        .expect("hot-swap studio plugin");
    assert_eq!(gen2, 2);
    let slot2 = runtime.plugins.get_mut(pid).unwrap();
    assert_eq!(slot2.surface_id(), plugin_sid);
    let pf2 = slot2.render(FrameId(2)).unwrap().to_vec();

    register_studio_tables();
    let mut verify_host = HeadlessBackend::new();
    verify_host.set_viewport(
        plugin_sid,
        Viewport {
            width: 960.0,
            height: 640.0,
            scale_factor: 1.0,
        },
    );
    let mut verify_rep =
        StoreWireReplica::new(verify_host, nve_components::tables::core_realization_order());
    verify_rep.submit_nve2_frame(&pf2).unwrap();
    let tree = verify_rep.backend().render_ascii_tree(verify_rep.store());
    assert!(tree.contains("100k Stream"));
    assert!(tree.contains("Active: Yoga (C++20)"));
}

#[test]
fn cmd_k_command_palette_fuzzy_filtering_keyboard_navigation_and_execution() {
    assert!(fuzzy_score("isld", "View: Islands").is_some());
    assert!(fuzzy_score("yga", "Layout Engine: Yoga").is_some());
    assert!(fuzzy_score("vlt", "Accent: Violet").is_some());
    assert!(fuzzy_score("zzzz", "View: Board").is_none());
    assert!(
        fuzzy_score("islands", "View: Islands").unwrap()
            > fuzzy_score("isld", "View: Islands").unwrap()
    );

    let vp = Viewport {
        width: 960.0,
        height: 640.0,
        scale_factor: 1.0,
    };
    register_studio_tables();
    let mut app = StudioWorkspaceModule::create(SurfaceId::ROOT);
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(SurfaceId::ROOT, vp);
    let mut host = StoreWireReplica::new(backend, nve_components::tables::core_realization_order());
    let f1 = app.render(FrameId(1)).to_vec();
    host.submit_nve2_frame(&f1).unwrap();
    assert!(!app.state_snapshot().palette.open);

    let send_keydown = |app: &mut StudioWorkspaceModule,
                        host: &mut StoreWireReplica<HeadlessBackend>,
                        key: &str,
                        meta: bool,
                        ctrl: bool,
                        fid: u64| {
        let ev = [NveDirectEvent::key_down(
            SurfaceId::ROOT,
            ROOT_NODE_ID,
            fid * 16_000,
            key,
            "",
            pack_key_modifiers(meta, ctrl, false, false, false),
            0,
            false,
        )];
        // SAFETY: `ev` outlives the synchronous `dispatch_events` call.
        let batch = unsafe { NveEventBatchRef::from_slice(&ev) };
        assert!(app.dispatch_events(batch), "keydown {key:?} not handled");
        let frame = app.render(FrameId(fid)).to_vec();
        host.submit_nve2_frame(&frame).unwrap();
    };

    let type_in = |app: &mut StudioWorkspaceModule,
                   host: &mut StoreWireReplica<HeadlessBackend>,
                   tid: &str,
                   text: &str,
                   fid: u64| {
        let nid = find_in_store(host.store(), tid)
            .unwrap_or_else(|| panic!("missing testID='{tid}'"));
        let ev = [NveDirectEvent::text_change(SurfaceId::ROOT, nid, fid * 16_000, text)];
        // SAFETY: `ev` outlives the synchronous `dispatch_events` call.
        let batch = unsafe { NveEventBatchRef::from_slice(&ev) };
        assert!(app.dispatch_events(batch), "text change on '{tid}' not handled");
        let frame = app.render(FrameId(fid)).to_vec();
        host.submit_nve2_frame(&frame).unwrap();
    };

    send_keydown(&mut app, &mut host, "k", true, false, 2);
    assert!(app.state_snapshot().palette.open);
    assert!(host.backend().render_ascii_tree(host.store()).contains("CMD+K PALETTE"));

    send_keydown(&mut app, &mut host, "Escape", false, false, 3);
    assert!(!app.state_snapshot().palette.open);

    send_keydown(&mut app, &mut host, "k", false, true, 4);
    assert!(app.state_snapshot().palette.open);
    type_in(&mut app, &mut host, "palette-input", "Accent:", 5);
    assert_eq!(app.state_snapshot().palette.selected, 0);

    send_keydown(&mut app, &mut host, "ArrowDown", false, false, 6);
    assert_eq!(app.state_snapshot().palette.selected, 1);

    send_keydown(&mut app, &mut host, "ArrowUp", false, false, 7);
    assert_eq!(app.state_snapshot().palette.selected, 0);
    send_keydown(&mut app, &mut host, "ArrowDown", false, false, 8);
    assert_eq!(app.state_snapshot().palette.selected, 1);

    send_keydown(&mut app, &mut host, "Enter", false, false, 9);
    assert!(!app.state_snapshot().palette.open);
    assert_eq!(
        app.state_snapshot().current.studio.accent,
        AccentTheme::Violet
    );

    send_keydown(&mut app, &mut host, "k", true, false, 10);
    type_in(&mut app, &mut host, "palette-input", "yga", 11);
    send_keydown(&mut app, &mut host, "Enter", false, false, 12);
    assert_eq!(
        app.state_snapshot().current.studio.layout_engine,
        LayoutEngineMode::Yoga
    );

    send_keydown(&mut app, &mut host, "k", true, false, 13);
    type_in(&mut app, &mut host, "palette-input", "isld", 14);
    send_keydown(&mut app, &mut host, "Enter", false, false, 15);
    assert_eq!(app.state_snapshot().current.active_view, ActiveView::Islands);

    let mut h = MultiBackendHarness::new(960.0, 640.0);
    h.press("accent-amber");
    h.press("btn-command-palette");
    assert!(h.find_by_test_id("command-palette").is_some());
    h.assert_no_horizontal_overflow("960x640-palette-open");
    h.type_text("palette-input", "Workspace Boot");
    h.press("palette-item-0");
    assert!(h.find_by_test_id("command-palette").is_none());
    assert!(h.ascii().contains("Snapshot 1 / 2"));
}

#[test]
fn text_input_chat_composer_board_search_create_time_travel_and_snapshot_roundtrip() {
    let mut h = MultiBackendHarness::new(960.0, 640.0);

    h.type_text("board-search-input", "AppKit");
    assert!(h.ascii().contains("AET-102"));
    assert!(!h.ascii().contains("AET-101"));
    h.press("board-search-clear");
    assert!(h.ascii().contains("AET-101"));

    h.type_text("board-new-task-input", "Ship NVE_DRAW1 binary vector IR");
    h.press("board-new-task-submit");
    assert!(h.ascii().contains("AET-105"));
    assert!(h.ascii().contains("Ship NVE_DRAW1 binary vector IR"));

    h.press("tt-undo");
    assert!(!h.ascii().contains("Ship NVE_DRAW1 binary vector IR"));
    h.press("tt-redo");
    assert!(h.ascii().contains("Ship NVE_DRAW1 binary vector IR"));

    h.press("nav-engineering");
    h.type_text("chat-composer-input", "Binary NVE_DRAW1 sparkline is live!");
    h.press("chat-composer-send");
    assert!(h.ascii().contains("Binary NVE_DRAW1 sparkline is live!"));

    h.press("tt-undo");
    assert!(!h.ascii().contains("Binary NVE_DRAW1 sparkline is live!"));
    h.press("tt-redo");
    assert!(h.ascii().contains("Binary NVE_DRAW1 sparkline is live!"));

    h.press("nav-design");
    h.type_text("chat-composer-input", "PNG avatars + Cmd+K contrast look crisp.");
    h.press("chat-composer-send");
    assert!(h.ascii().contains("PNG avatars + Cmd+K contrast look crisp."));

    register_studio_tables();
    let mut app = StudioWorkspaceModule::create(SurfaceId::ROOT);
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
    let f1 = app.render(FrameId(1)).to_vec();
    host.submit_nve2_frame(&f1).unwrap();

    let new_task_nid = find_in_store(host.store(), "board-new-task-input").unwrap();
    let ev_type = [NveDirectEvent::text_change(
        SurfaceId::ROOT,
        new_task_nid,
        16_000,
        "Persist across hot-swap",
    )];
    // SAFETY: `ev_type` outlives the synchronous `dispatch_events` call.
    assert!(app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&ev_type) }));
    let f2 = app.render(FrameId(2)).to_vec();
    host.submit_nve2_frame(&f2).unwrap();

    let ev_submit = [NveDirectEvent::submit(SurfaceId::ROOT, new_task_nid, 32_000, "")];
    // SAFETY: `ev_submit` outlives the synchronous `dispatch_events` call.
    assert!(app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&ev_submit) }));
    let _ = app.render(FrameId(3));

    let eng_entries = [NveMapEntry::new("view", NveValue::from_str("engineering"))];
    // SAFETY: `eng_entries` outlives the synchronous `set_props` call.
    assert!(app.set_props(unsafe { NvePropsRef::from_slice(&eng_entries) }));
    let f4 = app.render(FrameId(4)).to_vec();
    host.submit_nve2_frame(&f4).unwrap();
    let chat_nid = find_in_store(host.store(), "chat-composer-input").unwrap();
    let ev_chat = [NveDirectEvent::text_change(
        SurfaceId::ROOT,
        chat_nid,
        48_000,
        "Live typed message survives reload",
    )];
    // SAFETY: `ev_chat` outlives the synchronous `dispatch_events` call.
    assert!(app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&ev_chat) }));
    let _ = app.render(FrameId(5));
    let ev_chat_submit = [NveDirectEvent::submit(SurfaceId::ROOT, chat_nid, 64_000, "")];
    // SAFETY: `ev_chat_submit` outlives the synchronous `dispatch_events` call.
    assert!(app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&ev_chat_submit) }));
    let _ = app.render(FrameId(6));

    let saved = app.snapshot_state().to_vec();
    let mut restored = StudioWorkspaceModule::create(SurfaceId::ROOT);
    restored.restore_state(&saved);
    let snap = restored.state_snapshot();
    assert!(snap
        .current
        .tasks
        .iter()
        .any(|t| t.title == "Persist across hot-swap"));
    assert!(snap
        .current
        .eng_messages
        .iter()
        .any(|m| m.body == "Live typed message survives reload"));
}

#[test]
fn islands_multi_host_sandbox_orchestrator_bus_and_gesture_arbitration() {
    let mut h = MultiBackendHarness::new(960.0, 640.0);

    h.press("nav-islands");
    let tree = h.ascii();
    for expected in [
        "Multi-Host Islands",
        "React Native",
        "RNLiveMapView",
        "Flutter",
        "RevenueSparkChart",
        "SwiftUI",
        "ApplePayCheckoutCard",
        "Compose",
        "MediaPlayerSheet",
    ] {
        assert!(tree.contains(expected), "Islands view missing '{expected}'");
    }
    h.assert_no_horizontal_overflow("960x640-islands");

    assert!(h.ascii().contains("LazyOnVisible | Mounted"));
    h.press("island-toggle-rn");
    assert!(h.ascii().contains("Island rn -> Resumed"));
    assert!(h.ascii().contains("LazyOnVisible | Resumed"));
    h.press("island-toggle-rn");
    assert!(h.ascii().contains("Island rn -> Paused"));
    assert!(h.ascii().contains("LazyOnVisible | Paused"));

    h.press("island-bus-theme");
    assert!(h.ascii().contains("Bus: 1 tx (4 deliv)"));
    assert!(h.ascii().contains("Bus rx: 1"));

    h.press("island-bus-sync");
    assert!(h.ascii().contains("Bus: 2 tx (8 deliv)"));
    assert!(h.ascii().contains("Bus rx: 2"));

    h.press("island-gesture-claim");
    assert!(h.ascii().contains("ScrollAxisClaimed"));
    assert!(h.ascii().contains("Gesture -> Claim"));

    h.press("island-gesture-passthrough");
    assert!(h.ascii().contains("OrthogonalAxisPassThrough"));
    assert!(h.ascii().contains("Gesture -> PassThrough"));

    h.press("island-gesture-defer");
    assert!(h.ascii().contains("ScrollBoundaryDeferredToParent"));
    assert!(h.ascii().contains("cancelled: 1"));
    assert!(h.ascii().contains("Gesture -> DeferToParent"));

    h.press("tt-undo");
    assert!(h.ascii().contains("OrthogonalAxisPassThrough"));

    let mut h640 = MultiBackendHarness::new(640.0, 420.0);
    h640.press("nav-islands");
    h640.assert_no_horizontal_overflow("640x420-islands");

    let mut h393 = MultiBackendHarness::new(393.0, 852.0);
    h393.press("mobile-toggle-nav");
    h393.press("nav-islands");
    h393.assert_no_horizontal_overflow("393x852-islands");
}
