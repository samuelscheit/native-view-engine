use std::path::{Path, PathBuf};
use std::time::Instant;

use nve_abi::{
    encode_events, ColorRgba, ComponentKind, EventPayload, NativeEvent, NativeEventKind, NodeId,
    NveGestureDisposition, NveSizeConstraints, RawPropValue,
};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::{PixelBuffer, SkiaBackend};
use nve_backend_web::WebBackend;
use nve_dioxus::DioxusSurface;
use nve_engine::loader::HotAppInstance;
use nve_engine::snapshot::{write_snapshot_atomic, MmapSnapshot};
use nve_engine::{HotPatchMsg, NveApp, RenderBackend, Viewport};
use nve_studio_workspace::{
    create_surface, AccentTheme, ActiveView, ChannelKind, CornerRadiusMode, DensityMode,
    StudioWorkspaceModule, WorkspaceState, MAX_HISTORY,
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
            surface: create_surface(1),
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

    assert!(app.set_props(br#"{"accent":"amber","view":"metrics","studioOpen":true}"#));
    let _ = app.render(1);
    let patch_json = HotPatchMsg::TemplatePatch {
        generation: 1,
        template_id: "studio".into(),
        attr_updates: vec![("opacity".into(), "0.95".into())],
        text_updates: vec![],
    }
    .to_json();
    assert!(app.apply_hot_patch(&patch_json));
    let _ = app.render(2);

    let saved = app.snapshot_state().to_vec();
    assert!(!saved.is_empty());

    let mut restored_app = StudioWorkspaceModule::create(1);
    // Corrupt JSON must be safely ignored.
    restored_app.restore_state(b"{not-valid-json");
    assert_eq!(
        restored_app.state_snapshot().current.studio.accent,
        AccentTheme::Cyan
    );

    restored_app.restore_state(&saved);
    let snap = restored_app.state_snapshot();
    assert_eq!(snap.current.studio.accent, AccentTheme::Amber);
    assert_eq!(snap.current.active_view, ActiveView::Metrics);
    assert!(snap.current.studio.panel_open);

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
    assert!(host.render_ascii_tree().contains("Live Metrics"));

    // Verify additional state transitions serialize cleanly.
    let mut s2 = WorkspaceState::default();
    s2.set_radius(CornerRadiusMode::Pill);
    s2.set_density(DensityMode::Compact);
    s2.toggle_xray();
    s2.send_quick_reply(ChannelKind::Engineering, "Checking roundtrip");
    let raw = s2.to_json_bytes();
    let decoded = WorkspaceState::from_json_bytes(&raw).expect("roundtrip state");
    assert_eq!(decoded, s2);
}

#[test]
fn tier2_cdylib_hotswap_loads_via_nve_get_interface_and_preserves_full_state() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let dylib_ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };
    let dylib_path =
        workspace_root.join(format!("target/debug/libnve_studio_workspace.{dylib_ext}"));
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
    let mut inst2 = inst.reload(&dylib_path, 1).expect("reload gen 2");
    assert_eq!(inst2.generation(), 2);

    let mut host2 = HeadlessBackend::new();
    host2.set_viewport(1, vp);
    let f5 = inst2.render(5);
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
    if let Ok(export_path) = std::env::var("NVE_TEST_EXPORT_RUST_SNAPSHOT") {
        if !export_path.is_empty() {
            std::fs::copy(&snap_path, &export_path).expect("export rust snapshot for C++ test");
        }
    }
    if let Ok(cpp_in) = std::env::var("NVE_TEST_CPP_SNAPSHOT_IN") {
        if !cpp_in.is_empty() {
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
    }

    // Sub-millisecond cold mmap open; generous ceiling for Frame-0 layout in unoptimized debug builds.
    assert!(
        t_mmap.as_micros() < 5_000,
        "cold mmap open unexpectedly slow: {t_mmap:?}"
    );
    assert!(
        t_frame0.as_millis() < 50,
        "cold mmap + Frame-0 restore unexpectedly slow: {t_frame0:?}"
    );

    drop(snap);
    let _ = std::fs::remove_dir_all(&snap_dir);
}
