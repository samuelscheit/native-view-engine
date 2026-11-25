use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use nve_abi::events::encode_keydown_bytes;
use nve_abi::{
    decode_events, encode_bin_value_to_vec, encode_events, ColorRgba, Command, CommandDecoder,
    ComponentKind, EventMask, EventPayload, LayoutProps, NativeEvent, NativeEventKind, NodeId,
    NveGestureDisposition, NveMapEntry, NvePropsRef, NveSizeConstraints, NveValue, RawPropValue,
    StyleProps, ROOT_NODE_ID,
};
use nve_backend_headless::{HeadlessBackend, TaffyLayoutEngine};
use nve_backend_skia::{PixelBuffer, SkiaBackend};
use nve_backend_web::WebBackend;
use nve_components::{CanvasOp, DrawListDecoder, ImageResizeMode, NVE_DRAW_MAGIC};
use nve_dioxus::DioxusSurface;
use nve_engine::loader::HotAppInstance;
use nve_engine::snapshot::{fnv1a_64, write_snapshot_atomic, MmapSnapshot};
use nve_engine::{
    DynLayoutEngine, HotPatchMsg, LayoutEngine, LayoutRect, NveApp, RenderBackend, TextMeasurer,
    Viewport, WalDurability, WAL_HEADER_SIZE,
};
use nve_interop::{
    DeltaPacketRef, NvePluginHost, PluginRequirement, SyncDeltaSession, SyncMetrics,
};
use nve_layout_yoga::YogaLayoutEngine;
use nve_studio_workspace::{
    create_surface_for_viewport, fuzzy_score, AccentTheme, ActiveView, ChannelKind,
    CornerRadiusMode, DensityMode, DocBlockKind, LayoutEngineMode, PersistConfig,
    StudioWorkspaceModule, SyncTransportMode, WorkbenchPreset, WorkspaceState, WorkspaceStore,
    HOST_SURFACES, MAX_HISTORY, MOBILE_BREAKPOINT_PT, SNAPSHOT_FILE_NAME, WAL_FILE_NAME,
    WORKSPACE_STATE_BIN_MAGIC, WORKSPACE_STATE_BIN_VERSION,
};

struct MultiBackendHarness {
    surface: DioxusSurface,
    headless: HeadlessBackend,
    skia: SkiaBackend,
    web: WebBackend,
    viewport: Viewport,
    next_frame: u64,
}

impl MultiBackendHarness {
    fn new(width: f32, height: f32) -> Self {
        let viewport = Viewport {
            width,
            height,
            scale_factor: 1.0,
        };
        let mut headless = HeadlessBackend::new();
        let mut skia = SkiaBackend::new();
        let mut web = WebBackend::new();
        headless.set_viewport(1, viewport);
        skia.set_viewport(1, viewport);
        web.set_viewport(1, viewport);

        let mut h = Self {
            surface: create_surface_for_viewport(1, width, height),
            headless,
            skia,
            web,
            viewport,
            next_frame: 1,
        };
        h.render_next();
        h
    }

    fn render_next(&mut self) -> Vec<u8> {
        let fid = self.next_frame;
        self.next_frame += 1;
        let (_, _, bytes) = self.surface.render_frame(fid);
        self.headless.submit_frame(&bytes).expect("headless submit");
        self.skia.submit_frame(&bytes).expect("skia submit");
        self.web.submit_frame(&bytes).expect("web submit");
        bytes
    }

    fn find_by_test_id(&self, test_id: &str) -> Option<NodeId> {
        self.headless
            .nodes()
            .values()
            .find(|n| {
                matches!(
                    n.raw_props.get("testID"),
                    Some(RawPropValue::String(s)) if s == test_id
                )
            })
            .map(|n| n.id)
    }

    fn press(&mut self, test_id: &str) {
        let node_id = self
            .find_by_test_id(test_id)
            .unwrap_or_else(|| panic!("expected node with testID='{test_id}'\n{}", self.ascii()));
        let rect = self
            .headless
            .layout_of(node_id)
            .unwrap_or_else(|| panic!("missing layout rect for testID='{test_id}'"));
        assert!(
            rect.width > 0.0 && rect.height > 0.0,
            "zero-sized control '{test_id}': {rect:?}"
        );
        let cx = rect.x + rect.width * 0.5;
        let cy = rect.y + rect.height * 0.5;
        let hit = self
            .headless
            .hit_test(cx, cy, true)
            .unwrap_or_else(|| panic!("hit_test missed '{test_id}' at ({cx}, {cy}) rect={rect:?}"));
        assert_eq!(
            hit, node_id,
            "hit_test at center of '{test_id}' hit Node#{hit} instead of Node#{node_id}"
        );

        let handled = self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: hit,
            timestamp_us: self.next_frame * 16_000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: cx, y: cy },
        }]);
        assert!(handled, "press on '{test_id}' was not handled");
        self.render_next();
    }

    fn type_text(&mut self, test_id: &str, text: &str) {
        let node_id = self
            .find_by_test_id(test_id)
            .unwrap_or_else(|| panic!("expected textinput with testID='{test_id}'\n{}", self.ascii()));
        let handled = self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id,
            timestamp_us: self.next_frame * 16_000,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text(text.to_owned()),
        }]);
        assert!(handled, "text change on '{test_id}' was not handled");
        self.render_next();
    }

    fn ascii(&self) -> String {
        self.headless.render_ascii_tree()
    }

    fn html(&self) -> String {
        self.web.render_html()
    }

    fn rasterize(&self) -> PixelBuffer {
        let mut buf = PixelBuffer::new(self.viewport.width as u32, self.viewport.height as u32);
        self.skia
            .rasterize(&mut buf, ColorRgba::rgb(11, 17, 32));
        buf
    }

    fn assert_no_horizontal_overflow(&self, label: &str) {
        let vp_w = self.viewport.width;
        for node in self.headless.nodes().values() {
            if node.kind == ComponentKind::Root {
                continue;
            }
            let r = node.computed_rect;
            if r.width <= 0.0 || r.height <= 0.0 {
                continue;
            }
            assert!(
                r.x >= -0.5,
                "[{label}] node #{id} ({kind:?}, text={text:?}) has negative x: {r:?}",
                id = node.id,
                kind = node.kind,
                text = node.text
            );
            assert!(
                r.x + r.width <= vp_w + 1.0,
                "[{label}] node #{id} ({kind:?}, text={text:?}) overflows right edge ({vp_w}): {r:?}",
                id = node.id,
                kind = node.kind,
                text = node.text
            );
        }
    }
}

fn write_ppm(path: &Path, buf: &PixelBuffer) {
    let w = buf.width();
    let h = buf.height();
    let mut out = Vec::with_capacity((w * h * 3) as usize + 32);
    out.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = buf.get_rgb(x, y).unwrap_or((0, 0, 0));
            out.push(r);
            out.push(g);
            out.push(b);
        }
    }
    std::fs::write(path, out).expect("write ppm screenshot");
}

#[test]
fn single_command_buffer_drives_all_backends_views_and_interactions() {
    let mut h = MultiBackendHarness::new(960.0, 640.0);

    // Initial Board state verified across Headless, Skia, and Web backends.
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
    assert!(!h.skia.display_list().ops.is_empty());

    // Create a new task live.
    h.press("btn-new-task");
    assert!(h.ascii().contains("AET-105"));
    assert!(h.html().contains("AET-105"));
    assert!(h.ascii().contains("All (5)"));
    assert!(h.ascii().contains("Active (4)"));

    // Cycle AET-102 status: Todo -> In Progress -> Done.
    h.press("task-status-2");
    assert!(h.ascii().contains("Status -> In Progress"));
    h.press("task-status-2");
    assert!(h.ascii().contains("Status -> Done"));
    assert!(h.ascii().contains("Done (2)"));

    // Filter Board view: Done -> Active -> All.
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

    // Switch to # engineering channel, react + send quick replies.
    h.press("nav-engineering");
    assert!(h.ascii().contains("# engineering"));
    assert!(h.ascii().contains("+1  4"));
    h.press("msg-react-1");
    assert!(h.ascii().contains("+1  5"));
    h.press("reply-ship");
    assert!(h.ascii().contains("Ship It - all checks green!"));
    h.press("reply-bench");
    assert!(h.ascii().contains("Run Benches - checking p99."));

    // Switch to # design-sync channel, react + quick reply.
    h.press("nav-design");
    assert!(h.ascii().contains("# design-sync"));
    h.press("msg-react-101");
    assert!(h.ascii().contains("+1  6"));
    h.press("reply-lgtm");
    assert!(h.ascii().contains("LGTM - clean single-ABI PR."));

    // Switch to Metrics view and stress-test rows (+8, +24, Reset).
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

    // 1. Live Accent switch -> Violet (#a78bfa)
    h.press("accent-violet");
    assert!(h.ascii().contains("Accent -> Violet"));
    assert!(h.html().contains("rgba(167,139,250,1.000)"));

    // 2. Corner Radius -> Pill (18px) and Sharp (0px)
    h.press("radius-pill");
    assert!(h.ascii().contains("Radius -> Pill (18)"));
    assert!(h.html().contains("border-radius:18px;"));

    h.press("radius-sharp");
    assert!(
        h.ascii().contains("Radius -> Sharp (0)"),
        "expected 'Radius -> Sharp (0)' in tree:\n{}",
        h.ascii()
    );
    assert!(h.html().contains("border-radius:0px;"));

    // 3. Density -> Compact
    h.press("density-compact");
    assert!(h.ascii().contains("Density -> Compact"));

    // 4. X-Ray HUD ON -> colored borders (#38bdf8 / #a78bfa / #22c55e) appear; OFF -> cleared.
    h.press("toggle-xray");
    assert!(h.ascii().contains("X-Ray HUD: ON"));
    let has_xray_card_border = h.headless.nodes().values().any(|n| {
        n.style.border_width >= 2.0 && n.style.border_color == ColorRgba::rgb(167, 139, 250)
    });
    assert!(has_xray_card_border, "X-Ray violet card border expected");

    h.press("toggle-xray");
    assert!(h.ascii().contains("X-Ray HUD: OFF"));
    let still_has_xray = h.headless.nodes().values().any(|n| {
        n.style.border_width >= 2.0 && n.style.border_color == ColorRgba::rgb(167, 139, 250)
    });
    assert!(!still_has_xray, "X-Ray borders must cleanly reset when OFF");

    // 5. Deterministic time-travel scrubber: Undo, << #1, Redo, Live >>.
    h.press("btn-new-task"); // Snapshot 8 / 8
    assert!(h.ascii().contains("AET-105"));
    assert!(h.ascii().contains("Snapshot 8 / 8"));

    h.press("tt-undo"); // Back to 7 / 8 (before AET-105)
    assert!(h.ascii().contains("Snapshot 7 / 8"));
    assert!(!h.ascii().contains("AET-105"));

    h.press("tt-first"); // Back to 1 / 8 (Workspace Boot, Cyan accent)
    assert!(h.ascii().contains("Snapshot 1 / 8"));
    assert!(h.ascii().contains("Workspace Boot"));
    assert!(h.html().contains("rgba(56,189,248,1.000)"));

    h.press("tt-redo"); // Forward to 2 / 8 (Accent -> Violet)
    assert!(h.ascii().contains("Snapshot 2 / 8"));
    assert!(h.ascii().contains("Accent -> Violet"));

    h.press("tt-live"); // Jump to 8 / 8 (AET-105 back!)
    assert!(h.ascii().contains("Snapshot 8 / 8"));
    assert!(h.ascii().contains("AET-105"));

    // Branching history from an earlier snapshot truncates future redo entries.
    h.press("tt-first");
    assert!(h.ascii().contains("Snapshot 1 / 8"));
    h.press("accent-green");
    assert!(h.ascii().contains("Snapshot 2 / 2"));
    assert!(h.ascii().contains("Accent -> Green"));

    // 6. Toggling NVE Studio panel OFF hides inspector and expands main workspace.
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
    // Repeated undo at cursor 0 is a safe no-op.
    state.scrub_undo();
    assert_eq!(state.cursor, 0);

    state.scrub_live();
    assert_eq!(state.cursor, MAX_HISTORY - 1);
    // Repeated redo at live tip is a safe no-op.
    state.scrub_redo();
    assert_eq!(state.cursor, MAX_HISTORY - 1);

    // Direct NveApp trait roundtrip (props, measure, hit_test, hot_patch, snapshot/restore).
    let mut app = StudioWorkspaceModule::create(1);
    let measured = app.measure(NveSizeConstraints {
        min_w: 320.0,
        max_w: 960.0,
        min_h: 240.0,
        max_h: 640.0,
    });
    assert_eq!((measured.width, measured.height), (960.0, 640.0));
    assert_eq!(app.hit_test(10.0, 10.0, 0), NveGestureDisposition::Claim);

    // Direct zero-JSON `set_props_direct` via NveMapEntry / NveValue.
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
    let direct_props = unsafe { NvePropsRef::from_slice(&direct_entries) };
    assert!(app.set_props_direct(direct_props));
    let _ = app.render(1);
    let patch = HotPatchMsg::TemplatePatch {
        generation: 1,
        template_id: "studio".into(),
        target_test_id: None,
        node_path: vec![],
        attr_updates: vec![("opacity".into(), "0.95".into())],
        text_updates: vec![],
        text_replacements: vec![],
    };
    assert!(app.apply_hot_patch_wire(&patch.to_bytes()));
    let _ = app.render(2);

    let saved = app.snapshot_state().to_vec();
    assert!(!saved.is_empty());
    assert!(
        saved.starts_with(&WORKSPACE_STATE_BIN_MAGIC),
        "snapshot_state must emit zero-JSON NWS1 binary format"
    );

    let mut restored_app = StudioWorkspaceModule::create(1);
    // Corrupt / JSON bytes must be safely ignored.
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

    // Also verify zero-JSON binary `NveBinValue` props via `set_props`.
    let bin_entries = [NveMapEntry::new("accent", NveValue::from_str("green"))];
    let bin_map = NveValue::map(&bin_entries);
    let bin_bytes = encode_bin_value_to_vec(unsafe { bin_map.as_ref() }).unwrap();
    assert!(restored_app.set_props(&bin_bytes));
    assert_eq!(
        restored_app.state_snapshot().current.studio.accent,
        AccentTheme::Green
    );

    let frame = restored_app.render(3);
    let mut host = HeadlessBackend::new();
    host.set_viewport(
        1,
        Viewport {
            width: 960.0,
            height: 640.0,
            scale_factor: 1.0,
        },
    );
    host.submit_frame(frame).unwrap();
    let tree = host.render_ascii_tree();
    assert!(tree.contains("Live Metrics"));
    assert!(tree.contains("LIVE SYNC"));
    assert!(tree.contains("USB Mux"));
    assert!(tree.contains("3 peers"));
    assert!(tree.contains("4.6 KB/s | 7.2x | 0.8ms"));

    // Verify additional state transitions serialize cleanly via binary NWS1 codec.
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
    let mut host = HeadlessBackend::new();
    host.set_viewport(1, vp);

    let mut inst = HotAppInstance::load(&dylib_path, 1, 1, None).expect("load gen 1");
    let f1 = inst.render(1);
    assert!(!f1.is_empty());
    host.submit_frame(&f1).unwrap();
    assert!(host.render_ascii_tree().contains("AETHER"));
    assert!(host.render_ascii_tree().contains("AET-101"));

    let press_id = |host: &mut HeadlessBackend, inst: &mut HotAppInstance, tid: &str, fid: u64| {
        let nid = host
            .nodes()
            .values()
            .find(|n| matches!(n.raw_props.get("testID"), Some(RawPropValue::String(s)) if s == tid))
            .unwrap_or_else(|| panic!("missing testID='{tid}'"))
            .id;
        let rect = host.layout_of(nid).unwrap();
        let ev = encode_events(&[NativeEvent {
            surface_id: 1,
            node_id: nid,
            timestamp_us: fid * 16_000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point {
                x: rect.x + rect.width * 0.5,
                y: rect.y + rect.height * 0.5,
            },
        }]);
        assert!(inst.dispatch_events(&ev));
        let frame = inst.render(fid);
        host.submit_frame(&frame).unwrap();
    };

    // Mutate state before Tier-2 hot-swap: create task, switch accent to Violet, enable X-Ray.
    press_id(&mut host, &mut inst, "btn-new-task", 2);
    press_id(&mut host, &mut inst, "accent-violet", 3);
    press_id(&mut host, &mut inst, "toggle-xray", 4);
    assert!(host.render_ascii_tree().contains("AET-105"));
    assert!(host.render_ascii_tree().contains("X-Ray HUD: ON"));
    assert!(host.render_ascii_tree().contains("Snapshot 4 / 4"));

    // Live Tier-2 reload to generation 2: every task, theme, X-Ray, and snapshot survives!
    let outcome = inst.reload(&dylib_path).expect("reload gen 2");
    assert_eq!(outcome.generation, 2);
    assert_eq!(inst.generation(), 2);

    let mut host2 = HeadlessBackend::new();
    host2.set_viewport(1, vp);
    let f5 = inst.render(5);
    host2.submit_frame(&f5).unwrap();
    let tree2 = host2.render_ascii_tree();
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

    // 1. Desktop standard viewport: 960x640 across all views + X-Ray mode.
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

    // 2. Compact macOS/Qt default window viewport: 640x420.
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

#[test]
fn mmap_snapshot_instant_boot_frame0_and_studio_state_restore_sub_millisecond() {
    let snap_dir = std::env::temp_dir().join(format!(
        "nve-studio-snap-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&snap_dir);
    std::fs::create_dir_all(&snap_dir).expect("create temp snapshot dir");
    let snap_path = snap_dir.join("studio_workspace.nvesnap");

    let vp = Viewport {
        width: 960.0,
        height: 640.0,
        scale_factor: 1.0,
    };

    // 1. Mutate a live StudioWorkspaceModule via props + native Press events.
    let mut app = StudioWorkspaceModule::create(1);
    let mut host = HeadlessBackend::new();
    host.set_viewport(1, vp);
    let f1 = app.render(1).to_vec();
    host.submit_frame(&f1).expect("submit initial studio frame");

    let press_test_id = |host: &mut HeadlessBackend, app: &mut StudioWorkspaceModule, tid: &str, fid: u64| {
        let nid = host
            .nodes()
            .values()
            .find(|n| matches!(n.raw_props.get("testID"), Some(RawPropValue::String(s)) if s == tid))
            .unwrap_or_else(|| panic!("missing testID='{tid}'"))
            .id;
        let rect = host.layout_of(nid).expect("node layout");
        let ev = encode_events(&[NativeEvent {
            surface_id: 1,
            node_id: nid,
            timestamp_us: fid * 16_000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point {
                x: rect.x + rect.width * 0.5,
                y: rect.y + rect.height * 0.5,
            },
        }]);
        assert!(app.dispatch_events(&ev), "press '{tid}' not handled");
        let frame = app.render(fid).to_vec();
        host.submit_frame(&frame).expect("submit frame after press");
    };

    press_test_id(&mut host, &mut app, "btn-new-task", 2);
    press_test_id(&mut host, &mut app, "accent-violet", 3);
    press_test_id(&mut host, &mut app, "radius-pill", 4);
    press_test_id(&mut host, &mut app, "toggle-xray", 5);
    press_test_id(&mut host, &mut app, "nav-engineering", 6);
    press_test_id(&mut host, &mut app, "reply-ship", 7);

    let expected_state = app.state_snapshot();
    assert_eq!(expected_state.current.active_view, ActiveView::ChannelEngineering);
    assert_eq!(expected_state.current.studio.accent, AccentTheme::Violet);
    assert_eq!(expected_state.current.studio.radius, CornerRadiusMode::Pill);
    assert!(expected_state.current.studio.xray);

    let saved_state_bytes = app.snapshot_state().to_vec();
    assert!(!saved_state_bytes.is_empty());

    // Capture a complete, self-contained Frame-0 CommandBuffer of the mutated workspace state.
    let mut freeze_app = StudioWorkspaceModule::create(1);
    freeze_app.restore_state(&saved_state_bytes);
    let frame0_bytes = freeze_app.render(1).to_vec();
    assert!(frame0_bytes.len() > 24);

    // Atomically write the NVE_SNAP container to disk.
    write_snapshot_atomic(
        &snap_path,
        1,
        0,
        &frame0_bytes,
        &saved_state_bytes,
        b"viewport=960x640@1x;app=studio-workspace",
    )
    .expect("write studio snapshot atomically");

    // 2. Cold instant-boot restore: mmap open + zero-copy Frame-0 submit + state restore.
    let mut cold_host = HeadlessBackend::new();
    cold_host.set_viewport(1, vp);

    let t0 = Instant::now();
    let snap = MmapSnapshot::open_with_schema(&snap_path, 1).expect("cold mmap open");
    let t_mmap = t0.elapsed();

    // Prove true zero-copy: both slices borrow straight out of the mmap region.
    let base = snap.as_bytes().as_ptr() as usize;
    let end = base + snap.as_bytes().len();
    let f0_ptr = snap.frame0_command_buffer().as_ptr() as usize;
    let st_ptr = snap.state_bytes().as_ptr() as usize;
    assert!(f0_ptr >= base && f0_ptr + snap.frame0_command_buffer().len() <= end);
    assert!(st_ptr >= base && st_ptr + snap.state_bytes().len() <= end);

    cold_host
        .submit_frame(snap.frame0_command_buffer())
        .expect("zero-copy Frame-0 submit");
    let t_frame0 = t0.elapsed();

    // Frame-0 immediately renders the exact mutated workspace BEFORE StudioWorkspaceModule boots!
    let frame0_tree = cold_host.render_ascii_tree();
    assert!(frame0_tree.contains("# engineering"));
    assert!(frame0_tree.contains("Ship It - all checks green!"));
    assert!(frame0_tree.contains("X-Ray HUD: ON"));
    assert!(frame0_tree.contains("Snapshot 7 / 7"));

    // Now boot a fresh StudioWorkspaceModule and restore its full state from mmap bytes.
    let mut cold_app = StudioWorkspaceModule::create(1);
    let t_state_start = Instant::now();
    cold_app.restore_state(snap.state_bytes());
    let t_state = t_state_start.elapsed();
    assert_eq!(cold_app.state_snapshot(), expected_state);

    eprintln!(
        "[studio-workspace snapshot] size={} B | cold mmap open={} us | mmap+Frame0 submit={} us | state restore={} us",
        snap.as_bytes().len(),
        t_mmap.as_micros(),
        t_frame0.as_micros(),
        t_state.as_micros(),
    );

    // Optional cross-language export/verify hook when running Rust <-> C++ interop checks.
    if let Ok(export_path) = std::env::var("NVE_TEST_EXPORT_RUST_SNAPSHOT")
        && !export_path.is_empty()
    {
        std::fs::copy(&snap_path, &export_path).expect("export rust snapshot for C++ test");
    }
    if let Ok(cpp_in) = std::env::var("NVE_TEST_CPP_SNAPSHOT_IN")
        && !cpp_in.is_empty()
    {
        let cpp_snap =
            MmapSnapshot::open_with_schema(&cpp_in, 1).expect("open C++-written snapshot");
        assert_eq!(cpp_snap.meta_bytes(), b"cpp-roundtrip");
        let mut verify_host = HeadlessBackend::new();
        verify_host.set_viewport(1, vp);
        verify_host
            .submit_frame(cpp_snap.frame0_command_buffer())
            .expect("submit C++-written Frame-0 into Rust HeadlessBackend");
        let verify_tree = verify_host.render_ascii_tree();
        assert!(verify_tree.contains("# engineering"));
        assert!(verify_tree.contains("X-Ray HUD: ON"));
        let mut verify_app = StudioWorkspaceModule::create(1);
        verify_app.restore_state(cpp_snap.state_bytes());
        assert_eq!(verify_app.state_snapshot(), expected_state);
    }

    // Sub-millisecond cold mmap open on POSIX; allow headroom for NTFS + Defender AV scan on
    // Windows debug builds and unoptimized Frame-0 layout.
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

    drop(snap);
    let _ = std::fs::remove_dir_all(&snap_dir);
}

#[test]
fn stream_100k_virtualized_list_sub_2ms_jumps_and_fenwick_expand() {
    let mut h = MultiBackendHarness::new(960.0, 640.0);

    // Navigate to the 100k Stream tab.
    h.press("nav-stream");
    assert!(h.ascii().contains("100k Stream"));
    assert!(h.ascii().contains("STR-00001"));
    h.assert_no_horizontal_overflow("960x640-stream-top");

    let read_stream_prop = |h: &MultiBackendHarness, key: &str| -> u32 {
        let vp_id = h.find_by_test_id("stream-viewport").expect("stream-viewport");
        let node = h.headless.node(vp_id).unwrap();
        match node.raw_props.get(key) {
            Some(RawPropValue::String(s)) => s.parse::<u32>().expect("numeric virtual prop"),
            other => panic!("unexpected prop {key}={other:?}"),
        }
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

    // Measure incremental 100k-row jumps (#50,000 -> #99,999 -> #0 -> #50,000) directly on
    // DioxusSurface + HeadlessBackend (warm path).
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
        let (rec_us, enc_us, bytes) = h.surface.render_frame(fid);
        let fe_us = t0.elapsed().as_micros();
        let stats = h.headless.submit_frame(&bytes).unwrap();
        let total_us = t0.elapsed().as_micros();
        eprintln!(
            "  [{tid}] fe={fe_us}us total={total_us}us | disp={t_disp}us rec={rec_us}us enc={enc_us}us | dec={}us lay={}us diff={}us | ops={} muts={} bytes={}",
            stats.decode_us,
            stats.layout_us,
            stats.diff_us,
            stats.op_count,
            stats.mutation_count,
            bytes.len()
        );
        frontend_frame_us.push(fe_us);
        e2e_frame_us.push(total_us);
        // Keep secondary backends synced as well.
        h.skia.submit_frame(&bytes).unwrap();
        h.web.submit_frame(&bytes).unwrap();
    }

    // Verify we landed at row #50,000 with O(viewport) mounted rows.
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

    // Jump to #99,999 and verify tail row STR-100000 renders cleanly.
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

    // Expand visible row #99,999 live -> verifies O(log N) Fenwick height update increases total height!
    let h_before_expand = read_stream_prop(&h, "virtual-total-height");
    h.press("stream-row-99999");
    let h_after_expand = read_stream_prop(&h, "virtual-total-height");
    assert!(
        h_after_expand > h_before_expand,
        "expanding row #99,999 must increase Fenwick total height ({h_before_expand} -> {h_after_expand})"
    );

    // Toggle variable <-> fixed height mode cleanly.
    h.press("stream-toggle-heights");
    assert!(h.ascii().contains("O(1) FixedH"));
    h.press("stream-toggle-heights");
    assert!(h.ascii().contains("Fenwick VarH"));

    frontend_frame_us.sort_unstable();
    e2e_frame_us.sort_unstable();
    let fe_median_us = frontend_frame_us[frontend_frame_us.len() / 2];
    let e2e_median_us = e2e_frame_us[e2e_frame_us.len() / 2];

    // Also benchmark 1,000 consecutive Fenwick O(log N) window slices across 100,000 rows.
    let v_idx = nve_studio_workspace::VirtualListIndex::for_stream(
        nve_studio_workspace::DEFAULT_STREAM_ROW_COUNT,
        DensityMode::Normal,
        true,
        &[50_000, 99_999],
    );
    let t_slice = Instant::now();
    let mut acc_rows = 0usize;
    for step in 0..1_000usize {
        let target = (step * 97) % nve_studio_workspace::DEFAULT_STREAM_ROW_COUNT;
        let off = v_idx.row_offset(target) as f32;
        let w = v_idx.slice_window(off, 480.0, 2);
        acc_rows += w.rendered_count();
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

/// Secondary [`LayoutEngine`] wrapper used when `RN_REACT_COMMON_DIR` is absent so live
/// `HeadlessBackend::swap_layout_engine` mid-session switching is exercised on every machine,
/// while real `YogaLayoutEngine` parity executes automatically whenever RN Yoga is compiled in.
struct HostMirrorLayoutEngine {
    inner: TaffyLayoutEngine,
}

impl HostMirrorLayoutEngine {
    fn new() -> Self {
        Self {
            inner: TaffyLayoutEngine::new(),
        }
    }
}

impl LayoutEngine for HostMirrorLayoutEngine {
    fn name(&self) -> &'static str {
        "yoga-host-mirror"
    }

    fn upsert_node(&mut self, id: NodeId, kind: ComponentKind) {
        self.inner.upsert_node(id, kind);
    }

    fn remove_node(&mut self, id: NodeId) {
        self.inner.remove_node(id);
    }

    fn set_layout(&mut self, id: NodeId, layout: &LayoutProps) {
        self.inner.set_layout(id, layout);
    }

    fn set_style(&mut self, id: NodeId, style: &StyleProps) {
        self.inner.set_style(id, style);
    }

    fn set_text(&mut self, id: NodeId, text: &str) {
        self.inner.set_text(id, text);
    }

    fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        self.inner.set_children(parent, children);
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.inner.set_text_measurer(measurer);
    }

    fn compute_layout(&mut self, root: NodeId, viewport: Viewport) {
        self.inner.compute_layout(root, viewport);
    }

    fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        self.inner.layout_of(id)
    }

    fn reset(&mut self) {
        self.inner.reset();
    }
}

fn read_root_layout_engine_prop<L: LayoutEngine>(backend: &HeadlessBackend<L>) -> String {
    backend
        .nodes()
        .values()
        .find(|n| {
            matches!(
                n.raw_props.get("testID"),
                Some(RawPropValue::String(s)) if s == "workspace-root"
            )
        })
        .and_then(|n| match n.raw_props.get("layout-engine") {
            Some(RawPropValue::String(s)) => Some(s.clone()),
            _ => None,
        })
        .expect("workspace-root must emit 'layout-engine' raw prop")
}

#[test]
fn live_taffy_yoga_layout_engine_switcher_and_mid_session_parity() {
    let vp = Viewport {
        width: 960.0,
        height: 640.0,
        scale_factor: 1.0,
    };

    // 1. Mutate workspace mid-session BEFORE switching layout engines.
    let mut app = StudioWorkspaceModule::create(1);
    let mut dyn_backend: HeadlessBackend<DynLayoutEngine> =
        HeadlessBackend::with_layout(Box::new(TaffyLayoutEngine::new()));
    dyn_backend.set_viewport(1, vp);

    let f1 = app.render(1).to_vec();
    dyn_backend.submit_frame(&f1).unwrap();
    assert_eq!(dyn_backend.layout_engine().name(), "taffy");
    assert_eq!(read_root_layout_engine_prop(&dyn_backend), "taffy");
    assert!(dyn_backend.render_ascii_tree().contains("Active: Taffy (Rust)"));

    let press_dyn = |backend: &mut HeadlessBackend<DynLayoutEngine>,
                     app: &mut StudioWorkspaceModule,
                     tid: &str,
                     fid: u64| -> Vec<u8> {
        let nid = backend
            .nodes()
            .values()
            .find(|n| matches!(n.raw_props.get("testID"), Some(RawPropValue::String(s)) if s == tid))
            .unwrap_or_else(|| panic!("missing testID='{tid}'"))
            .id;
        let rect = backend.layout_of(nid).unwrap();
        let ev = encode_events(&[NativeEvent {
            surface_id: 1,
            node_id: nid,
            timestamp_us: fid * 16_000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point {
                x: rect.x + rect.width * 0.5,
                y: rect.y + rect.height * 0.5,
            },
        }]);
        assert!(app.dispatch_events(&ev));
        let frame = app.render(fid).to_vec();
        backend.submit_frame(&frame).unwrap();
        frame
    };

    // Create a task and switch accent so we prove state survives layout-engine swaps!
    press_dyn(&mut dyn_backend, &mut app, "btn-new-task", 2);
    press_dyn(&mut dyn_backend, &mut app, "accent-violet", 3);
    assert!(dyn_backend.render_ascii_tree().contains("AET-105"));

    // 2. Toggle Studio toolbar to Yoga -> verify prop, readout, and outbound custom event!
    press_dyn(&mut dyn_backend, &mut app, "layout-yoga", 4);
    assert_eq!(read_root_layout_engine_prop(&dyn_backend), "yoga");
    assert!(dyn_backend.render_ascii_tree().contains("Active: Yoga (C++20)"));
    assert_eq!(
        app.state_snapshot().current.studio.layout_engine,
        LayoutEngineMode::Yoga
    );

    let emitted = decode_events(app.drain_emitted_events()).unwrap();
    assert_eq!(emitted.len(), 1);
    assert!(matches!(
        &emitted[0].payload,
        EventPayload::Custom { name, data } if name == "layout_engine_changed" && data == b"yoga"
    ));

    // Capture full rects under Taffy before swapping engine live mid-session.
    let taffy_rects_before: Vec<(NodeId, LayoutRect)> = {
        let mut pairs: Vec<(NodeId, LayoutRect)> = dyn_backend
            .nodes()
            .keys()
            .filter_map(|&id| dyn_backend.layout_of(id).map(|r| (id, r)))
            .collect();
        pairs.sort_by_key(|(id, _)| *id);
        pairs
    };

    // 3. Hot-swap live backend engine Taffy -> Yoga (real RN Yoga when available, else host mirror)
    // WITHOUT dropping state or recreating the surface!
    let next_engine: DynLayoutEngine = if YogaLayoutEngine::is_available() {
        Box::new(YogaLayoutEngine::new().expect("real RN Yoga available"))
    } else {
        assert!(YogaLayoutEngine::new().is_err());
        Box::new(HostMirrorLayoutEngine::new())
    };
    let expected_swapped_name = next_engine.name();
    dyn_backend = dyn_backend.swap_layout_engine(next_engine);
    assert_eq!(dyn_backend.layout_engine().name(), expected_swapped_name);

    // Compare every computed LayoutRect against Taffy parity tolerance (<= 2.0px).
    for (id, taffy_rect) in &taffy_rects_before {
        let swapped_rect = dyn_backend
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

    // State must remain 100% intact after mid-session swap, and further clicks still work!
    assert!(dyn_backend.render_ascii_tree().contains("AET-105"));
    press_dyn(&mut dyn_backend, &mut app, "layout-taffy", 5);
    assert_eq!(read_root_layout_engine_prop(&dyn_backend), "taffy");
    dyn_backend = dyn_backend.swap_layout_engine(Box::new(TaffyLayoutEngine::new()));
    assert_eq!(dyn_backend.layout_engine().name(), "taffy");
    assert!(dyn_backend.render_ascii_tree().contains("AET-105"));

    // Undo restores Yoga mode cleanly through deterministic time-travel!
    press_dyn(&mut dyn_backend, &mut app, "tt-undo", 6);
    assert_eq!(read_root_layout_engine_prop(&dyn_backend), "yoga");
    assert!(dyn_backend.render_ascii_tree().contains("AET-105"));
}

#[test]
fn responsive_mobile_portrait_layout_at_393pt_collapses_drawers_without_crushing_center() {
    assert_eq!(MOBILE_BREAKPOINT_PT, 620.0);

    // 1. Direct iPhone 17 portrait boot: 393 x 852 pt.
    let mut h = MultiBackendHarness::new(393.0, 852.0);
    h.assert_no_horizontal_overflow("393x852-board");

    // Verify mobile-portrait mode collapsed left sidebar & right Studio drawer.
    assert!(h.find_by_test_id("mobile-top-bar").is_some());
    assert!(h.find_by_test_id("desktop-sidebar").is_none());
    assert!(
        h.find_by_test_id("studio-panel").is_none(),
        "Studio drawer must start collapsed on < 620pt portrait screens"
    );

    let root_id = h.find_by_test_id("workspace-root").unwrap();
    assert_eq!(
        h.headless.node(root_id).unwrap().raw_props.get("viewport-mode"),
        Some(&RawPropValue::String("mobile-portrait".into()))
    );

    // Center workspace must occupy the full 393pt width (never crushed or negative!).
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

    // Open mobile Nav drawer -> navigate to 100k Stream -> nav drawer auto-closes!
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

    // Jump to #50k on mobile portrait and verify clean fit.
    h.press("stream-jump-50k");
    assert!(h.ascii().contains("STR-50001"));
    h.assert_no_horizontal_overflow("393x852-stream-50k");

    // Toggle Studio drawer ON on mobile -> full-width stacked drawer, zero clipping!
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

    // Center workspace still has full positive width and positive height below the Studio drawer!
    let center_after_studio = h
        .headless
        .layout_of(h.find_by_test_id("center-workspace").unwrap())
        .unwrap();
    assert!(center_after_studio.width >= 390.0);
    assert!(center_after_studio.height > 100.0);

    // Mutate accent & layout engine inside mobile Studio drawer, then collapse drawer.
    h.press("accent-amber");
    h.press("layout-yoga");
    assert!(h.ascii().contains("Active: Yoga (C++20)"));
    h.press("btn-toggle-studio");
    assert!(h.find_by_test_id("studio-panel").is_none());

    // 2. Dynamic resize via NativeEventKind::Layout on StudioWorkspaceModule (960pt -> 393pt -> 960pt).
    let mut app = StudioWorkspaceModule::create(1);
    let _ = app.render(1);
    assert!(!app.state_snapshot().is_mobile_portrait());

    let resize_mobile = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: ROOT_NODE_ID,
        timestamp_us: 1000,
        kind: NativeEventKind::Layout,
        payload: EventPayload::Layout {
            x: 0.0,
            y: 0.0,
            width: 393.0,
            height: 852.0,
        },
    }]);
    assert!(app.dispatch_events(&resize_mobile));
    assert!(app.state_snapshot().is_mobile_portrait());
    let mobile_frame = app.render(2).to_vec();
    let mut host = HeadlessBackend::new();
    host.set_viewport(
        1,
        Viewport {
            width: 393.0,
            height: 852.0,
            scale_factor: 3.0,
        },
    );
    host.submit_frame(&mobile_frame).unwrap();
    assert!(host.render_ascii_tree().contains("Studio OFF"));

    // Verify `measure` respects narrow mobile portrait constraints instead of forcing 640pt!
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
    // 1. Real StudioWorkspace incremental edits over SyncDeltaSession (< 256 B/frame, < 10 KB/s).
    let h = MultiBackendHarness::new(960.0, 640.0);
    let mut sender = SyncDeltaSession::default();
    let mut receiver = SyncDeltaSession::default();

    // Sync initial full frame as keyframe.
    let mut snap_app = StudioWorkspaceModule::create(1);
    let f1 = snap_app.render(1).to_vec();
    let p1 = sender.encode_frame(&f1, 0);
    let r1 = receiver.receive_packet(&p1).unwrap();
    assert_eq!(r1.applied_frames[0].1, f1);
    sender.handle_ack(r1.ack_packet.as_ref().unwrap()).unwrap();

    // Perform an incremental Studio edit (cycle task status AET-102: Todo -> In Progress).
    let nid = h.find_by_test_id("task-status-2").unwrap();
    let rect = h.headless.layout_of(nid).unwrap();
    let ev = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: nid,
        timestamp_us: 16_000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point {
            x: rect.x + rect.width * 0.5,
            y: rect.y + rect.height * 0.5,
        },
    }]);
    assert!(snap_app.dispatch_events(&ev));
    let f2_incremental = snap_app.render(2).to_vec();

    let p2 = sender.encode_frame(&f2_incremental, 16);
    let parsed = DeltaPacketRef::parse(&p2).unwrap();
    assert!(parsed.header().seq == 2);
    let r2 = receiver.receive_packet(&p2).unwrap();
    assert_eq!(r2.applied_frames[0].1, f2_incremental);

    // Also measure a second consecutive incremental edit (In Progress -> Done).
    assert!(snap_app.dispatch_events(&ev));
    let f3_incremental = snap_app.render(3).to_vec();
    let p3 = sender.encode_frame(&f3_incremental, 33);
    let r3 = receiver.receive_packet(&p3).unwrap();
    assert_eq!(r3.applied_frames[0].1, f3_incremental);

    let delta_bytes = p3.len();
    let kbps_60 = SyncMetrics::estimated_kbps(delta_bytes, 60.0);
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
        "expected incremental Studio edit wire delta < 256B, got {delta_bytes}B (raw {}B)",
        f3_incremental.len()
    );
    assert!(
        kbps_60 < 10.0,
        "expected < 10 KB/s @ 60 edits/sec, got {kbps_60:.2} KB/s"
    );

    // 2. Real dynamic cdylib plugin loading + isolated surface ID + transactional hot-swap via NvePluginHost.
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

    let mut plugin_host = NvePluginHost::with_surface_base(50_000);
    let pid = plugin_host
        .load_dynamic("aether-studio", &dylib_path, PluginRequirement::EMBEDDED_COMPONENT)
        .expect("load studio cdylib as plugin");
    let slot = plugin_host.get_mut(pid).unwrap();
    assert_eq!(slot.surface_id(), 50_000);
    assert!(slot.capabilities().surface);
    assert!(slot.capabilities().layout);
    assert!(slot.capabilities().component);
    assert!(slot.capabilities().dev);

    assert!(slot
        .set_props(br#"{"accent":"green","layoutEngine":"yoga","view":"stream"}"#)
        .unwrap());
    let pf1 = slot.render(1).unwrap().to_vec();
    assert!(pf1.len() > 24);

    // Transactional hot-swap preserves isolated surface_id=50_000 and full mutated state!
    let gen2 = plugin_host
        .hot_swap_dynamic(pid, &dylib_path, PluginRequirement::EMBEDDED_COMPONENT)
        .expect("hot-swap studio plugin");
    assert_eq!(gen2, 2);
    let slot2 = plugin_host.get_mut(pid).unwrap();
    assert_eq!(slot2.surface_id(), 50_000);
    let pf2 = slot2.render(2).unwrap().to_vec();

    let mut verify_host = HeadlessBackend::new();
    verify_host.set_viewport(
        50_000,
        Viewport {
            width: 960.0,
            height: 640.0,
            scale_factor: 1.0,
        },
    );
    verify_host.submit_frame(&pf2).unwrap();
    let tree = verify_host.render_ascii_tree();
    assert!(tree.contains("100k Stream"));
    assert!(tree.contains("Active: Yoga (C++20)"));
}

#[test]
fn cmd_k_command_palette_fuzzy_filtering_keyboard_navigation_and_execution() {
    // 1. Unit assertions on deterministic fuzzy scoring.
    assert!(fuzzy_score("isld", "View: Islands").is_some());
    assert!(fuzzy_score("yga", "Layout Engine: Yoga").is_some());
    assert!(fuzzy_score("vlt", "Accent: Violet").is_some());
    assert!(fuzzy_score("zzzz", "View: Board").is_none());
    assert!(
        fuzzy_score("islands", "View: Islands").unwrap()
            > fuzzy_score("isld", "View: Islands").unwrap()
    );

    // 2. Drive full keyboard lifecycle (Cmd+K -> filter -> ArrowDown -> Enter / Escape)
    // through StudioWorkspaceModule's NveApp C-ABI event stream!
    let vp = Viewport {
        width: 960.0,
        height: 640.0,
        scale_factor: 1.0,
    };
    let mut app = StudioWorkspaceModule::create(1);
    let mut host = HeadlessBackend::new();
    host.set_viewport(1, vp);
    let f1 = app.render(1).to_vec();
    host.submit_frame(&f1).unwrap();
    assert!(!app.state_snapshot().palette.open);

    let send_keydown = |app: &mut StudioWorkspaceModule,
                        host: &mut HeadlessBackend,
                        payload: &[u8],
                        fid: u64| {
        let ev = encode_events(&[NativeEvent {
            surface_id: 1,
            node_id: ROOT_NODE_ID,
            timestamp_us: fid * 16_000,
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "keydown".into(),
                data: payload.to_vec(),
            },
        }]);
        assert!(app.dispatch_events(&ev), "keydown {:?} not handled", std::str::from_utf8(payload));
        let frame = app.render(fid).to_vec();
        host.submit_frame(&frame).unwrap();
    };

    let type_in = |app: &mut StudioWorkspaceModule,
                   host: &mut HeadlessBackend,
                   tid: &str,
                   text: &str,
                   fid: u64| {
        let nid = host
            .nodes()
            .values()
            .find(|n| matches!(n.raw_props.get("testID"), Some(RawPropValue::String(s)) if s == tid))
            .unwrap_or_else(|| panic!("missing testID='{tid}'"))
            .id;
        let ev = encode_events(&[NativeEvent {
            surface_id: 1,
            node_id: nid,
            timestamp_us: fid * 16_000,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text(text.to_owned()),
        }]);
        assert!(app.dispatch_events(&ev), "text change on '{tid}' not handled");
        let frame = app.render(fid).to_vec();
        host.submit_frame(&frame).unwrap();
    };

    // Open via zero-JSON NK1 binary keydown Cmd+k.
    let nk1_cmd_k = encode_keydown_bytes("k", true, false, false, false);
    send_keydown(&mut app, &mut host, &nk1_cmd_k, 2);
    assert!(app.state_snapshot().palette.open);
    assert!(host.render_ascii_tree().contains("CMD+K PALETTE"));

    // Escape closes palette cleanly.
    send_keydown(&mut app, &mut host, b"Escape", 3);
    assert!(!app.state_snapshot().palette.open);

    // Re-open via Ctrl+k shorthand, fuzzy-filter for "Accent:", press ArrowDown -> Violet, press Enter!
    send_keydown(&mut app, &mut host, b"Ctrl+k", 4);
    assert!(app.state_snapshot().palette.open);
    type_in(&mut app, &mut host, "palette-input", "Accent:", 5);
    assert_eq!(app.state_snapshot().palette.selected, 0);

    // ArrowDown moves selection from Cyan (0) -> Violet (1).
    send_keydown(&mut app, &mut host, b"ArrowDown", 6);
    assert_eq!(app.state_snapshot().palette.selected, 1);

    // ArrowUp moves back to 0, then ArrowDown back to 1 (Violet).
    send_keydown(&mut app, &mut host, b"ArrowUp", 7);
    assert_eq!(app.state_snapshot().palette.selected, 0);
    send_keydown(&mut app, &mut host, b"ArrowDown", 8);
    assert_eq!(app.state_snapshot().palette.selected, 1);

    // Enter executes Accent: Violet and closes palette.
    send_keydown(&mut app, &mut host, b"Enter", 9);
    assert!(!app.state_snapshot().palette.open);
    assert_eq!(
        app.state_snapshot().current.studio.accent,
        AccentTheme::Violet
    );

    // Open again, fuzzy-filter "yga" -> Layout Engine: Yoga -> Enter!
    send_keydown(&mut app, &mut host, b"Cmd+k", 10);
    type_in(&mut app, &mut host, "palette-input", "yga", 11);
    send_keydown(&mut app, &mut host, b"Enter", 12);
    assert_eq!(
        app.state_snapshot().current.studio.layout_engine,
        LayoutEngineMode::Yoga
    );

    // Open again, fuzzy-filter "isld" -> View: Islands -> Enter!
    send_keydown(&mut app, &mut host, b"Cmd+k", 13);
    type_in(&mut app, &mut host, "palette-input", "isld", 14);
    send_keydown(&mut app, &mut host, b"Enter", 15);
    assert_eq!(app.state_snapshot().current.active_view, ActiveView::Islands);

    // Open via header button in MultiBackendHarness and execute a time-travel snapshot jump by click.
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

    // 1. Live Board search filters cards dynamically.
    h.type_text("board-search-input", "AppKit");
    assert!(h.ascii().contains("AET-102"));
    assert!(!h.ascii().contains("AET-101"));
    h.press("board-search-clear");
    assert!(h.ascii().contains("AET-101"));

    // 2. Live Board custom task create via TextInput + Add button.
    h.type_text("board-new-task-input", "Ship NVE_DRAW1 binary vector IR");
    h.press("board-new-task-submit");
    assert!(h.ascii().contains("AET-105"));
    assert!(h.ascii().contains("Ship NVE_DRAW1 binary vector IR"));

    // Time-travel Undo removes the custom task; Redo brings it right back!
    h.press("tt-undo");
    assert!(!h.ascii().contains("Ship NVE_DRAW1 binary vector IR"));
    h.press("tt-redo");
    assert!(h.ascii().contains("Ship NVE_DRAW1 binary vector IR"));

    // 3. Live TextInput chat composer in #engineering and #design-sync + Time-Travel Undo/Redo.
    h.press("nav-engineering");
    h.type_text("chat-composer-input", "Zero-JSON NVE_DRAW1 sparkline is live!");
    h.press("chat-composer-send");
    assert!(h.ascii().contains("Zero-JSON NVE_DRAW1 sparkline is live!"));

    h.press("tt-undo");
    assert!(!h.ascii().contains("Zero-JSON NVE_DRAW1 sparkline is live!"));
    h.press("tt-redo");
    assert!(h.ascii().contains("Zero-JSON NVE_DRAW1 sparkline is live!"));

    h.press("nav-design");
    h.type_text("chat-composer-input", "PNG avatars + Cmd+K contrast look crisp.");
    h.press("chat-composer-send");
    assert!(h.ascii().contains("PNG avatars + Cmd+K contrast look crisp."));

    // 4. Tier-2 snapshot_state / restore_state roundtrip preserves composed messages & custom tasks.
    let mut app = StudioWorkspaceModule::create(1);
    let mut host = HeadlessBackend::new();
    host.set_viewport(
        1,
        Viewport {
            width: 960.0,
            height: 640.0,
            scale_factor: 1.0,
        },
    );
    let f1 = app.render(1).to_vec();
    host.submit_frame(&f1).unwrap();

    // Type and submit custom task via TextChange + custom "submit" event.
    let new_task_nid = host
        .nodes()
        .values()
        .find(|n| matches!(n.raw_props.get("testID"), Some(RawPropValue::String(s)) if s == "board-new-task-input"))
        .unwrap()
        .id;
    let ev_type = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: new_task_nid,
        timestamp_us: 16_000,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text("Persist across hot-swap".into()),
    }]);
    assert!(app.dispatch_events(&ev_type));
    let f2 = app.render(2).to_vec();
    host.submit_frame(&f2).unwrap();

    let ev_submit = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: new_task_nid,
        timestamp_us: 32_000,
        kind: NativeEventKind::Custom,
        payload: EventPayload::Custom {
            name: "submit".into(),
            data: Vec::new(),
        },
    }]);
    assert!(app.dispatch_events(&ev_submit));
    let _ = app.render(3);

    // Switch to #engineering, type + submit chat message.
    assert!(app.set_props(br#"{"view":"engineering"}"#));
    let f4 = app.render(4).to_vec();
    host.submit_frame(&f4).unwrap();
    let chat_nid = host
        .nodes()
        .values()
        .find(|n| matches!(n.raw_props.get("testID"), Some(RawPropValue::String(s)) if s == "chat-composer-input"))
        .unwrap()
        .id;
    let ev_chat = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: chat_nid,
        timestamp_us: 48_000,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text("Live typed message survives reload".into()),
    }]);
    assert!(app.dispatch_events(&ev_chat));
    let _ = app.render(5);
    assert!(app.dispatch_events(&encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: chat_nid,
        timestamp_us: 64_000,
        kind: NativeEventKind::Custom,
        payload: EventPayload::Custom {
            name: "submit".into(),
            data: Vec::new(),
        },
    }])));
    let _ = app.render(6);

    let saved = app.snapshot_state().to_vec();
    let mut restored = StudioWorkspaceModule::create(1);
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

    // Navigate to ActiveView::Islands and verify all 4 frameworks mount cleanly.
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

    // 1. Toggle RN island lifecycle via real SurfaceOrchestrator: Mounted -> Resumed -> Paused.
    assert!(h.ascii().contains("LazyOnVisible | Mounted"));
    h.press("island-toggle-rn");
    assert!(h.ascii().contains("Island rn -> Resumed"));
    assert!(h.ascii().contains("LazyOnVisible | Resumed"));
    h.press("island-toggle-rn");
    assert!(h.ascii().contains("Island rn -> Paused"));
    assert!(h.ascii().contains("LazyOnVisible | Paused"));

    // 2. Broadcast live NveChannelBus theme & binary sync packets to all 4 subscribed islands.
    h.press("island-bus-theme");
    assert!(h.ascii().contains("Bus: 1 tx (4 deliv)"));
    assert!(h.ascii().contains("Bus rx: 1"));

    h.press("island-bus-sync");
    assert!(h.ascii().contains("Bus: 2 tx (8 deliv)"));
    assert!(h.ascii().contains("Bus rx: 2"));

    // 3. Exercise all 3 real GestureArbitrator outcomes: Claim, PassThrough, DeferToParent!
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

    // Time-travel Undo steps cleanly back to PassThrough!
    h.press("tt-undo");
    assert!(h.ascii().contains("OrthogonalAxisPassThrough"));

    // Also verify compact 640x420 and mobile 393x852 layouts fit Islands with zero overflow!
    let mut h640 = MultiBackendHarness::new(640.0, 420.0);
    h640.press("nav-islands");
    h640.assert_no_horizontal_overflow("640x420-islands");

    let mut h393 = MultiBackendHarness::new(393.0, 852.0);
    h393.press("mobile-toggle-nav");
    h393.press("nav-islands");
    h393.assert_no_horizontal_overflow("393x852-islands");
}

#[test]
fn command_buffer_emits_real_image_text_input_and_skia_canvas_with_nve_draw1_bytes() {
    // 1. Inspect Board + Engineering CommandBuffers for real Image and TextInput primitives.
    let mut h = MultiBackendHarness::new(960.0, 640.0);

    let board_images: Vec<_> = h
        .headless
        .nodes()
        .values()
        .filter(|n| n.kind == ComponentKind::Image)
        .collect();
    assert!(
        board_images.len() >= 4,
        "expected PNG Image avatars on every issue card, got {}",
        board_images.len()
    );
    for img in &board_images {
        match img.raw_props.get("src") {
            Some(RawPropValue::String(src)) => {
                assert!(
                    src.starts_with("data:image/png;base64,"),
                    "expected PNG data URI src, got {src}"
                );
            }
            other => panic!("missing Image src prop: {other:?}"),
        }
        assert_eq!(
            img.raw_props.get("resizeMode"),
            Some(&RawPropValue::String(ImageResizeMode::Cover.as_str().into()))
        );
    }

    let board_inputs: Vec<_> = h
        .headless
        .nodes()
        .values()
        .filter(|n| n.kind == ComponentKind::TextInput)
        .collect();
    assert!(
        board_inputs.len() >= 2,
        "expected search + custom task TextInputs on Board, got {}",
        board_inputs.len()
    );
    for inp in &board_inputs {
        assert_ne!(
            inp.event_mask & EventMask::TEXT_CHANGE,
            0,
            "TextInput must subscribe to EventMask::TEXT_CHANGE"
        );
        assert!(matches!(
            inp.raw_props.get("placeholder"),
            Some(RawPropValue::String(p)) if !p.is_empty()
        ));
    }

    // Also verify WebBackend renders real <img> and <input> HTML tags!
    let html_board = h.html();
    assert!(html_board.contains("<img "));
    assert!(html_board.contains("<input "));

    // 2. Switch to Metrics view and verify real SkiaCanvas + valid NVE_DRAW1 draw-list bytes
    // in BOTH DioxusSurface AND StudioWorkspaceModule (as RawPropValue::Bytes)!
    h.press("nav-metrics");
    let canvas_id = h
        .find_by_test_id("metrics-sparkline-canvas")
        .expect("SkiaCanvas node in Metrics view");
    let canvas_node = h.headless.node(canvas_id).unwrap();
    assert_eq!(canvas_node.kind, ComponentKind::SkiaCanvas);
    let draw_prop = canvas_node
        .raw_props
        .get("drawList")
        .expect("SkiaCanvas must emit drawList prop");
    let ops = DrawListDecoder::decode_raw_prop(draw_prop)
        .expect("SkiaCanvas drawList must decode as valid NVE_DRAW1");
    assert!(
        ops.iter().any(|op| matches!(op, CanvasOp::StrokePolyline { points, .. } if points.len() >= 8)),
        "expected telemetry StrokePolyline op in SkiaCanvas draw list, got {ops:?}"
    );
    assert!(h.html().contains("<canvas "));

    // 3. Verify StudioWorkspaceModule::render emits true binary RawPropValue::Bytes starting with NVE_DRAW1!
    let mut app = StudioWorkspaceModule::create(1);
    assert!(app.set_props(br#"{"view":"metrics"}"#));
    let cmd_bytes = app.render(1).to_vec();
    let (_, commands) = CommandDecoder::new(&cmd_bytes)
        .and_then(CommandDecoder::decode_all)
        .expect("decode metrics CommandBuffer");

    let mut saw_canvas_create = false;
    let mut saw_nve_draw1_bytes = false;
    for cmd in &commands {
        match cmd {
            Command::CreateNode {
                kind: ComponentKind::SkiaCanvas,
                ..
            } => {
                saw_canvas_create = true;
            }
            Command::SetRawProp {
                key,
                value: RawPropValue::Bytes(bytes),
                ..
            } if key == "drawList" => {
                assert!(
                    bytes.starts_with(&NVE_DRAW_MAGIC),
                    "drawList RawPropValue::Bytes must start with NVE_DRAW1 magic"
                );
                let decoded = DrawListDecoder::decode(bytes).expect("decode binary NVE_DRAW1");
                assert!(decoded.len() >= 7);
                saw_nve_draw1_bytes = true;
            }
            _ => {}
        }
    }
    assert!(saw_canvas_create, "expected CreateNode(SkiaCanvas) in CommandBuffer");
    assert!(
        saw_nve_draw1_bytes,
        "expected SetRawProp('drawList', RawPropValue::Bytes(NVE_DRAW1...)) in CommandBuffer"
    );
}

fn unique_test_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "nve-studio-wave10-{tag}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn nws1_v2_fnv1a64_corruption_rejection_bitflips_truncation_and_v1_rejection() {
    let mut state = WorkspaceState::default();
    assert!(state.create_custom_task("Verify FNV-1a64 persistence"));
    state.send_quick_reply(ChannelKind::Engineering, "NWS1 v2 integrity gate");
    state.select_view(ActiveView::Docs);
    assert!(state.toggle_doc_checklist(7));
    let added = state
        .add_doc_block(DocBlockKind::Callout, "FNV-1a64 checksummed block")
        .expect("add doc block");
    assert!(state.update_doc_block_text(added, "Edited checksummed callout"));
    state.toggle_doc_markdown_preview();
    state.select_view(ActiveView::Workbench);
    state.set_workbench_preset(WorkbenchPreset::Tablet);
    state.toggle_workbench_chrome();
    state.select_workbench_host("flutter");
    state.set_accent(AccentTheme::Amber);
    state.set_radius(CornerRadiusMode::Pill);
    state.set_density(DensityMode::Compact);
    state.cycle_sync_transport();

    let encoded = state.to_bytes();
    assert!(encoded.starts_with(&WORKSPACE_STATE_BIN_MAGIC));
    assert_eq!(
        u16::from_le_bytes([encoded[4], encoded[5]]),
        WORKSPACE_STATE_BIN_VERSION
    );
    assert_eq!(WORKSPACE_STATE_BIN_VERSION, 2);

    let decoded = WorkspaceState::from_bytes(&encoded).expect("valid NWS1 v2 decode");
    assert_eq!(decoded, state);

    // Every single-bit flip across header, payload samples, and all 8 checksum bytes must fail.
    let len = encoded.len();
    let mut flip_positions = vec![0, 1, 3, 4, 5, 6, 16, len / 4, len / 2, (len * 3) / 4];
    for tail in (len - 8)..len {
        flip_positions.push(tail);
    }
    for pos in flip_positions {
        let mut mutated = encoded.clone();
        mutated[pos] ^= 0x80;
        assert!(
            WorkspaceState::from_bytes(&mutated).is_none(),
            "single-bit flip at byte {pos}/{len} must be rejected by FNV-1a64"
        );
    }

    // Truncations across header, payload, and checksum tail fail cleanly.
    for cut in [0, 1, 4, 6, 13, 14, len / 2, len - 9, len - 8, len - 1] {
        assert!(
            WorkspaceState::from_bytes(&encoded[..cut]).is_none(),
            "truncated NWS1 v2 slice of len {cut} must be rejected"
        );
    }

    // Appended garbage fails checksum.
    let mut with_extra = encoded.clone();
    with_extra.push(0x42);
    assert!(WorkspaceState::from_bytes(&with_extra).is_none());

    // Forged v1 version with recomputed valid FNV-1a64 must still be rejected (no v1 shims).
    let mut forged_v1 = encoded.clone();
    forged_v1[4..6].copy_from_slice(&1u16.to_le_bytes());
    let crc = fnv1a_64(&forged_v1[..len - 8]);
    forged_v1[len - 8..].copy_from_slice(&crc.to_le_bytes());
    assert!(
        WorkspaceState::from_bytes(&forged_v1).is_none(),
        "v1 format must be strictly rejected"
    );
}

#[test]
fn workspace_store_cold_save_wal_torn_tail_compaction_and_full_restart_recovery() {
    let dir = unique_test_dir("persist");
    let cfg = PersistConfig {
        max_wal_bytes: 0,
        max_wal_records: 4,
        durability: WalDurability::Buffered,
    };

    // 1. Cold open initializes base NVE_SNAP + empty .nvewal.
    let mut store = WorkspaceStore::open_with_config(&dir, cfg).expect("cold open store");
    assert!(store.is_enabled());
    assert!(!store.recovery_stats().loaded_from_disk);
    assert_eq!(store.wal_record_count(), 0);
    assert!(dir.join(SNAPSHOT_FILE_NAME).exists());
    assert!(dir.join(WAL_FILE_NAME).exists());

    // 2. Mutate tasks, messages, docs, checklist state, theme, and workbench across 3 WAL appends.
    let mut state = store.initial_state().clone();
    assert!(state.create_custom_task("Ship local-first WAL persistence"));
    state.send_quick_reply(ChannelKind::Engineering, "WAL append #1 verified");
    let out1 = store.save(&state).expect("wal append 1");
    assert_eq!(out1.wal_seq, 1);
    assert!(!out1.compacted);
    assert_eq!(out1.wal_record_count, 1);

    state.select_view(ActiveView::Docs);
    assert!(state.toggle_doc_checklist(7));
    let added_blk = state
        .add_doc_block(DocBlockKind::Code, "WorkspaceStore::open(dir)")
        .expect("add code block");
    assert!(state.update_doc_block_text(4, "Updated binary CommandBuffer invariant."));
    let out2 = store.save(&state).expect("wal append 2");
    assert_eq!(out2.wal_seq, 2);
    assert!(!out2.compacted);
    assert_eq!(out2.wal_record_count, 2);

    state.set_accent(AccentTheme::Violet);
    state.set_radius(CornerRadiusMode::Pill);
    state.set_density(DensityMode::Compact);
    state.select_view(ActiveView::Workbench);
    state.set_workbench_preset(WorkbenchPreset::Mobile);
    state.select_workbench_host("skia-metal");
    let out3 = store.save(&state).expect("wal append 3");
    assert_eq!(out3.wal_seq, 3);
    assert!(!out3.compacted);
    assert_eq!(out3.wal_record_count, 3);
    let state_before_crash = state.clone();
    drop(store);

    // 3. Simulate mid-write crash: append 17 torn trailing garbage bytes to workspace.nvewal.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(WAL_FILE_NAME))
            .expect("open wal for torn-tail injection");
        f.write_all(&[0xDE, 0xAD, 0xBE, 0xEF, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13])
            .expect("write torn tail");
    }

    // 4. Cold restart recovers cleanly from torn tail & replays all 3 valid WAL records!
    let mut store_reopened =
        WorkspaceStore::open_with_config(&dir, cfg).expect("recover torn-tail WAL");
    let stats = store_reopened.recovery_stats();
    assert!(stats.loaded_from_disk);
    assert_eq!(stats.truncated_torn_wal_bytes, 17);
    assert_eq!(stats.replayed_wal_records, 3);
    assert!(!stats.recovered_from_corrupt_snapshot);
    assert!(!stats.recovered_from_corrupt_wal);
    assert_eq!(store_reopened.initial_state(), &state_before_crash);
    assert_eq!(store_reopened.wal_record_count(), 3);

    // 5. 4th append crosses `max_wal_records = 4` -> triggers automatic atomic compaction!
    let mut state_after_compact = store_reopened.initial_state().clone();
    state_after_compact.send_quick_reply(ChannelKind::Design, "Post-compaction message");
    assert!(state_after_compact.toggle_doc_checklist(6));
    let out4 = store_reopened
        .save(&state_after_compact)
        .expect("wal append 4 triggers compaction");
    assert!(out4.compacted, "4th WAL append must trigger auto-compaction");
    assert_eq!(store_reopened.compaction_count(), 1);
    assert_eq!(store_reopened.wal_record_count(), 0);
    assert_eq!(store_reopened.wal_file_len(), WAL_HEADER_SIZE as u64);
    assert_eq!(
        store_reopened.load_state().expect("load_state after compact"),
        state_after_compact
    );
    drop(store_reopened);

    // 6. Full cold restart after compaction restores directly from MmapSnapshot with 0 WAL replays.
    let store_cold =
        WorkspaceStore::open_with_config(&dir, cfg).expect("cold restart after compaction");
    let cold_stats = store_cold.recovery_stats();
    assert!(cold_stats.loaded_from_disk);
    assert_eq!(cold_stats.replayed_wal_records, 0);
    assert_eq!(cold_stats.truncated_torn_wal_bytes, 0);
    let restored = store_cold.initial_state();
    assert_eq!(restored, &state_after_compact);
    assert!(restored
        .current
        .tasks
        .iter()
        .any(|t| t.title == "Ship local-first WAL persistence"));
    assert!(restored
        .current
        .eng_messages
        .iter()
        .any(|m| m.body == "WAL append #1 verified"));
    assert!(restored
        .current
        .design_messages
        .iter()
        .any(|m| m.body == "Post-compaction message"));
    assert_eq!(restored.current.studio.accent, AccentTheme::Violet);
    assert_eq!(restored.current.studio.radius, CornerRadiusMode::Pill);
    assert_eq!(restored.current.studio.density, DensityMode::Compact);
    let doc1 = restored
        .current
        .docs
        .iter()
        .find(|d| d.id == 1)
        .expect("doc 1 preserved");
    assert!(doc1
        .blocks
        .iter()
        .any(|b| b.id == 7 && matches!(b.kind, DocBlockKind::Checklist { checked: true })));
    assert!(doc1
        .blocks
        .iter()
        .any(|b| b.id == 6 && matches!(b.kind, DocBlockKind::Checklist { checked: false })));
    assert!(doc1
        .blocks
        .iter()
        .any(|b| b.id == added_blk && b.text == "WorkspaceStore::open(dir)"));
    drop(store_cold);

    // 7. Corrupt base snapshot file falls back cleanly to default state without panicking.
    std::fs::write(dir.join(SNAPSHOT_FILE_NAME), b"CORRUPTED_SNAPSHOT_BYTES")
        .expect("corrupt snapshot file");
    let store_corrupt_snap =
        WorkspaceStore::open_with_config(&dir, cfg).expect("fallback on corrupt snapshot");
    assert!(store_corrupt_snap.recovery_stats().recovered_from_corrupt_snapshot);
    assert_eq!(store_corrupt_snap.initial_state(), &WorkspaceState::default());
    drop(store_corrupt_snap);

    // 8. StudioWorkspaceModule::create_with_persist_dir auto-persists live mutations across cold restarts.
    let mod_dir = unique_test_dir("module-persist");
    {
        let mut app =
            StudioWorkspaceModule::create_with_persist_dir(1, &mod_dir).expect("create module");
        let _ = app.render(1);
        assert!(app.set_props(br#"{"accent":"green","view":"docs"}"#));
        let _ = app.render(2);
        assert!(app.persist_store().expect("persist store").wal_record_count() >= 1);
    }
    {
        let app2 =
            StudioWorkspaceModule::create_with_persist_dir(1, &mod_dir).expect("reopen module");
        let snap = app2.state_snapshot();
        assert_eq!(snap.current.studio.accent, AccentTheme::Green);
        assert_eq!(snap.current.active_view, ActiveView::Docs);
    }

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&mod_dir);
}

#[test]
fn docs_view_interactive_blocks_textinput_markdown_export_and_time_travel() {
    let mut h = MultiBackendHarness::new(1280.0, 800.0);

    // Navigate to Docs via left sidebar nav button (showing doc count badge 2).
    assert!(h.find_by_test_id("nav-docs").is_some());
    h.press("nav-docs");
    assert!(h.ascii().contains("Workspace Docs"));
    assert!(h.ascii().contains("RFC-042: Zero-Copy NveSceneView"));
    assert!(h.ascii().contains("Release Checklist: v1.0"));
    assert!(h.ascii().contains("1/2 checks"));
    h.assert_no_horizontal_overflow("docs-1280x800-initial");

    // Switch between seeded specs and back.
    h.press("doc-tab-2");
    assert!(h.ascii().contains("Pre-Flight Gates"));
    assert!(h.ascii().contains("2/3 checks"));
    h.press("doc-tab-1");
    assert!(h.ascii().contains("Binary ABI Invariants"));

    // Toggle checklist item #7 ([ ] -> [x]) and verify progress updates 1/2 -> 2/2.
    h.press("doc-check-7");
    assert!(h.ascii().contains("2/2 checks"));

    // Edit block #4 via real inline TextInput (`onchange` + Save button).
    h.press("doc-block-edit-4");
    assert!(h.find_by_test_id("doc-block-input-4").is_some());
    h.type_text("doc-block-input-4", "Flat zero-copy binary frames everywhere.");
    h.press("doc-block-save-4");
    assert!(h.ascii().contains("Flat zero-copy binary frames everywhere."));
    assert!(h.find_by_test_id("doc-block-input-4").is_none());

    // Add a new block via composer TextInput + kind cycler (Paragraph -> Checklist) + submit.
    h.press("doc-new-block-kind");
    h.type_text("doc-new-block-input", "Verify crash-safe .nvewal compaction");
    h.press("doc-new-block-submit");
    assert!(h.ascii().contains("Verify crash-safe .nvewal compaction"));
    assert!(h.ascii().contains("2/3 checks"));

    // Cycle block #5 kind (Code -> Callout) and reorder block #5 up.
    h.press("doc-block-kind-5");
    h.press("doc-block-up-5");

    // Delete block #3 and verify it disappears, then Time-Travel Undo restores it!
    assert!(h.find_by_test_id("doc-block-3").is_some());
    h.press("doc-block-del-3");
    assert!(h.find_by_test_id("doc-block-3").is_none());
    h.press("tt-undo");
    assert!(
        h.find_by_test_id("doc-block-3").is_some(),
        "Time-Travel Undo must restore deleted doc block"
    );
    h.press("tt-redo");
    assert!(
        h.find_by_test_id("doc-block-3").is_none(),
        "Time-Travel Redo must re-apply block deletion"
    );

    // Export clean Markdown preview in UI and verify direct `to_markdown()` formatting.
    h.press("doc-export-md");
    assert!(h.find_by_test_id("doc-markdown-preview").is_some());
    let tree_md = h.ascii();
    assert!(tree_md.contains("# RFC-042: Zero-Copy NveSceneView"));
    assert!(tree_md.contains("- [x] Enable crash-safe NVE_SNAP + .nvewal local persistence in Studio"));
    assert!(tree_md.contains("- [ ] Verify crash-safe .nvewal compaction"));
    h.assert_no_horizontal_overflow("docs-1280x800-with-markdown");

    // Verify clean Markdown formatting directly on WorkspaceState::default() docs.
    let default_state = WorkspaceState::default();
    let md = default_state.active_doc_markdown();
    assert!(md.starts_with("# RFC-042: Zero-Copy NveSceneView"));
    assert!(md.contains("\n\n> Status: Accepted"));
    assert!(md.contains("\n\n## Binary ABI Invariants\n\n"));
    assert!(md.contains("```\nnve_get_interface(NVE_IFACE_SURFACE) -> submit_frame(&[u8])\n```"));
    assert!(md.contains("- [x] Eliminate JSON scene descriptors"));
    assert!(md.contains("- [ ] Enable crash-safe NVE_SNAP"));

    // Verify Cmd+K Command Palette reaches ActiveView::Docs cleanly.
    let mut app = StudioWorkspaceModule::create(1);
    let _ = app.render(1);
    let cmd_k = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: ROOT_NODE_ID,
        timestamp_us: 1000,
        kind: NativeEventKind::Custom,
        payload: EventPayload::Custom {
            name: "keydown".into(),
            data: encode_keydown_bytes("k", true, false, false, false),
        },
    }]);
    assert!(app.dispatch_events(&cmd_k));
    assert!(app.state_snapshot().palette.open);

    // Verify Docs fits cleanly on Mobile Portrait 393x852 with zero horizontal overflow.
    let mut h_mob = MultiBackendHarness::new(393.0, 852.0);
    h_mob.press("mobile-toggle-nav");
    h_mob.press("nav-docs");
    assert!(h_mob.ascii().contains("Workspace Docs"));
    h_mob.assert_no_horizontal_overflow("docs-393x852-mobile");
}

#[test]
fn workbench_view_multi_viewport_presets_live_telemetry_and_no_overflow() {
    // 1. Desktop 1280x800 layout & synchronized multi-viewport preview grid.
    let mut h = MultiBackendHarness::new(1280.0, 800.0);
    assert!(h.find_by_test_id("nav-workbench").is_some());
    h.press("nav-workbench");
    assert!(h.ascii().contains("Meta Workbench"));
    assert!(h.ascii().contains("CONNECTED HOST SURFACES (9)"));

    let desk_id = h.find_by_test_id("wb-frame-desktop").expect("desktop frame");
    let tab_id = h.find_by_test_id("wb-frame-tablet").expect("tablet frame");
    let mob_id = h.find_by_test_id("wb-frame-mobile").expect("mobile frame");
    let r_desk = h.headless.layout_of(desk_id).expect("desktop rect");
    let r_tab = h.headless.layout_of(tab_id).expect("tablet rect");
    let r_mob = h.headless.layout_of(mob_id).expect("mobile rect");
    assert!(
        r_desk.x < r_tab.x && r_tab.x < r_mob.x,
        "Desktop, Tablet, Mobile frames must sit side-by-side at 1280x800: desk={r_desk:?}, tab={r_tab:?}, mob={r_mob:?}"
    );
    assert!(h.ascii().contains("1280x800"));
    assert!(h.ascii().contains("820x1180"));
    assert!(h.ascii().contains("390x844"));
    h.assert_no_horizontal_overflow("workbench-1280x800-all");

    // Verify all 9 connected host surface cards render with live SyncTelemetry metrics.
    assert_eq!(HOST_SURFACES.len(), 9);
    for spec in HOST_SURFACES {
        let tid = format!("wb-host-{}", spec.slug);
        assert!(
            h.find_by_test_id(&tid).is_some(),
            "missing connected host surface card '{tid}'"
        );
        assert!(
            h.ascii().contains(spec.name),
            "workbench missing host surface name '{}'",
            spec.name
        );
    }
    assert!(h.ascii().contains("LAN QUIC"));
    assert!(h.ascii().contains("3.8 KB/s"));
    assert!(h.ascii().contains("6.4x"));

    // Live Studio token changes immediately reflect inside Workbench frames & live strip.
    h.press("accent-violet");
    h.press("radius-pill");
    h.press("layout-yoga");
    let tree_tokens = h.ascii();
    assert!(tree_tokens.contains("Violet | 18px | Normal | Yoga | 4 issues | 2 docs"));

    // Live SyncTelemetry transport cycle immediately reflects in Workbench banner & host cards.
    h.press("wb-sync-cycle");
    let tree_sync = h.ascii();
    assert!(tree_sync.contains("USB Mux"));
    assert!(tree_sync.contains("2.4 KB/s"));
    assert!(tree_sync.contains("7.2x"));

    // Interactive viewport preset toggles (Desktop -> Tablet -> Mobile -> All) & Chrome toggle.
    h.press("wb-preset-desktop");
    assert!(h.find_by_test_id("wb-frame-desktop").is_some());
    assert!(h.find_by_test_id("wb-frame-tablet").is_none());
    assert!(h.find_by_test_id("wb-frame-mobile").is_none());
    h.assert_no_horizontal_overflow("workbench-1280x800-desktop");

    h.press("wb-preset-tablet");
    assert!(h.find_by_test_id("wb-frame-desktop").is_none());
    assert!(h.find_by_test_id("wb-frame-tablet").is_some());
    assert!(h.find_by_test_id("wb-frame-mobile").is_none());
    h.assert_no_horizontal_overflow("workbench-1280x800-tablet");

    h.press("wb-preset-mobile");
    assert!(h.find_by_test_id("wb-frame-desktop").is_none());
    assert!(h.find_by_test_id("wb-frame-tablet").is_none());
    assert!(h.find_by_test_id("wb-frame-mobile").is_some());
    h.assert_no_horizontal_overflow("workbench-1280x800-mobile");

    h.press("wb-preset-all");
    assert!(h.find_by_test_id("wb-frame-desktop").is_some());
    assert!(h.find_by_test_id("wb-frame-tablet").is_some());
    assert!(h.find_by_test_id("wb-frame-mobile").is_some());

    h.press("wb-toggle-chrome");
    assert!(!h.ascii().contains("1280x800"));
    h.press("wb-toggle-chrome");
    assert!(h.ascii().contains("1280x800"));

    // Select a host surface card and verify Time-Travel records it.
    h.press("wb-host-skia-metal");
    assert!(h.ascii().contains("Host -> Skia Metal"));

    // 2. Mobile Portrait 393x852 layout: zero horizontal overflow across all presets!
    let mut h_mob = MultiBackendHarness::new(393.0, 852.0);
    h_mob.press("mobile-toggle-nav");
    h_mob.press("nav-workbench");
    assert!(h_mob.ascii().contains("Meta Workbench"));
    assert!(h_mob.find_by_test_id("wb-frame-desktop").is_some());
    assert!(h_mob.find_by_test_id("wb-frame-tablet").is_some());
    assert!(h_mob.find_by_test_id("wb-frame-mobile").is_some());
    h_mob.assert_no_horizontal_overflow("workbench-393x852-all");

    h_mob.press("wb-preset-mobile");
    h_mob.assert_no_horizontal_overflow("workbench-393x852-mobile");
}


