use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use nve_abi::{ColorRgba, EventMask, FrameId, NativeEventKind, SurfaceId};
use nve_backend_skia::gpu::{
    GpuBackendKind, GpuFramePresenter, GpuSurfaceConfig, GpuSwapchain,
};
use nve_backend_skia::raster::PixelBuffer;
use nve_components::tables::{
    register_core_tables, FlexDirection, Interaction, Layout, TextContent, Typography, Visual,
};
use nve_engine::loader::LoaderError;
use nve_engine::{Frame0Meta, Frame0Miss, HotPatchMsg, Viewport};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId as StoreNodeId, NodeStore, Rgba};
use tungstenite::Message;

use super::session::prewarm_frame0_into_replica;
use super::{
    write_pixel_buffer_ppm, DesktopApp, DesktopHostError, DesktopSession, HotAppSlot,
    HotReloadClient,
};

struct MockCounterApp {
    generation: u64,
    count: i32,
    label: String,
    button_color: ColorRgba,
    reloaded_paths: Vec<PathBuf>,
    store: NodeStore,
    encoder: WireEncoder,
    changes: ChangeSet,
    initialized: bool,
    frame_buf: Vec<u8>,
}

impl MockCounterApp {
    fn new() -> Self {
        register_core_tables();
        Self {
            generation: 1,
            count: 0,
            label: "Counter".to_string(),
            button_color: ColorRgba::rgb(37, 99, 235),
            reloaded_paths: Vec::new(),
            store: NodeStore::new(),
            encoder: WireEncoder::default(),
            changes: ChangeSet::default(),
            initialized: false,
            frame_buf: Vec::new(),
        }
    }
}

impl DesktopApp for MockCounterApp {
    fn generation(&self) -> u64 {
        self.generation
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        let mut tx = self.store.tx();
        if !self.initialized {
            let _ = tx.create_at(StoreNodeId::ROOT);
            let _ = tx.create_at(StoreNodeId(2));
            let _ = tx.create_at(StoreNodeId(3));
            let _ = tx.create_at(StoreNodeId(4));

            tx.set(StoreNodeId(2), Layout::WIDTH, Length::percent(100.0));
            tx.set(StoreNodeId(2), Layout::HEIGHT, Length::percent(100.0));
            let p = Length::points(20.0);
            tx.set(StoreNodeId(2), Layout::PADDING_TOP, p);
            tx.set(StoreNodeId(2), Layout::PADDING_RIGHT, p);
            tx.set(StoreNodeId(2), Layout::PADDING_BOTTOM, p);
            tx.set(StoreNodeId(2), Layout::PADDING_LEFT, p);
            tx.set(StoreNodeId(2), Layout::ROW_GAP, Length::points(12.0));
            tx.set(StoreNodeId(2), Layout::COLUMN_GAP, Length::points(12.0));
            tx.set(StoreNodeId(2), Layout::FLEX_DIRECTION, FlexDirection::Column);
            tx.set(StoreNodeId(2), Visual::BACKGROUND, Rgba::rgb(15, 23, 42));

            tx.set(StoreNodeId(3), Typography::COLOR, Rgba::WHITE);
            tx.set(StoreNodeId(3), Typography::FONT_SIZE, 20.0);

            tx.set(StoreNodeId(4), Layout::WIDTH, Length::points(160.0));
            tx.set(StoreNodeId(4), Layout::HEIGHT, Length::points(48.0));
            tx.set(StoreNodeId(4), Visual::BORDER_RADIUS, 10.0);
            tx.set(StoreNodeId(4), Interaction::EVENT_MASK, EventMask::PRESS as u32);

            tx.set_children(StoreNodeId(2), &[StoreNodeId(3), StoreNodeId(4)]);
            tx.set_children(StoreNodeId::ROOT, &[StoreNodeId(2)]);
            self.initialized = true;
        }
        tx.set_str(
            StoreNodeId(3),
            TextContent::TEXT,
            &format!("{}: {}", self.label, self.count),
        );
        tx.set(
            StoreNodeId(4),
            Visual::BACKGROUND,
            Rgba::new(
                self.button_color.r,
                self.button_color.g,
                self.button_color.b,
                self.button_color.a,
            ),
        );
        tx.commit_into(&mut self.changes);
        self.frame_buf.clear();
        self.encoder
            .encode(&self.store, &self.changes, &mut self.frame_buf);
        &self.frame_buf
    }

    fn dispatch_events(&mut self, events: nve_abi::NveEventBatchRef<'_>) -> bool {
        let mut handled = false;
        for ev in events.iter() {
            if ev.node_id() == 4 && ev.kind() == Some(NativeEventKind::Press) {
                self.count += 1;
                self.button_color = ColorRgba::rgb(16, 185, 129);
                handled = true;
            }
        }
        handled
    }

    fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool {
        match HotPatchMsg::from_bytes(patch_nvp1) {
            Ok(HotPatchMsg::TemplatePatch {
                text_updates,
                attr_updates,
                ..
            }) => {
                for (_, text) in text_updates {
                    self.label = text;
                }
                for (k, v) in attr_updates {
                    if k == "button_color" && v == "orange" {
                        self.button_color = ColorRgba::rgb(249, 115, 22);
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn reload(
        &mut self,
        new_dylib_path: &Path,
        _surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError> {
        self.reloaded_paths.push(new_dylib_path.to_path_buf());
        self.generation += 1;
        self.button_color = ColorRgba::rgb(168, 85, 247);
        self.store = NodeStore::new();
        self.encoder.reset();
        self.initialized = false;
        Ok(self.generation)
    }
}

#[test]
fn hit_test_click_dispatches_press_and_rerenders_pixels() {
    let viewport = Viewport {
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
    };
    let mut session = DesktopSession::new(
        MockCounterApp::new(),
        1,
        viewport,
        ColorRgba::BLACK,
        false,
    );
    session.render_and_rasterize().unwrap();

    let btn = session.layout_of(4).expect("button layout");
    let cx = btn.x + btn.width * 0.5;
    let cy = btn.y + btn.height * 0.5;

    assert_eq!(
        session.pixel_buffer().get_rgb(cx as u32, cy as u32),
        Some((37, 99, 235))
    );
    assert!(!session.handle_click_logical(280.0, 180.0).unwrap());
    assert_eq!(session.app().count, 0);

    assert!(session.handle_click_logical(cx, cy).unwrap());
    assert_eq!(session.app().count, 1);
    assert_eq!(
        session.pixel_buffer().get_rgb(cx as u32, cy as u32),
        Some((16, 185, 129))
    );
}

#[test]
fn handles_tier1_template_patch_tier2_dylib_reload_and_dev_status() {
    let viewport = Viewport {
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
    };
    let mut session = DesktopSession::new(
        MockCounterApp::new(),
        1,
        viewport,
        ColorRgba::BLACK,
        true,
    );
    session.render_and_rasterize().unwrap();

    let btn = session.layout_of(4).unwrap();
    let cx = btn.x + btn.width * 0.5;
    let cy = btn.y + btn.height * 0.5;
    assert!(session.handle_click_logical(cx, cy).unwrap());
    assert_eq!(session.app().count, 1);

    let updated = session
        .handle_hot_patch(HotPatchMsg::TemplatePatch {
            generation: 2,
            template_id: "counter".into(),
            target_test_id: None,
            node_path: vec![],
            attr_updates: vec![("button_color".into(), "orange".into())],
            text_updates: vec![(0, "Live".into())],
            text_replacements: vec![],
        })
        .unwrap();
    assert!(updated);
    assert_eq!(session.app().generation(), 1);
    assert_eq!(session.app().label, "Live");
    assert_eq!(
        session.pixel_buffer().get_rgb(cx as u32, cy as u32),
        Some((249, 115, 22))
    );

    let updated = session
        .handle_hot_patch(HotPatchMsg::DylibReload {
            generation: 2,
            dylib_path: "target/debug/libnve_example_kitchen_sink.dylib".into(),
            build_ms: 185,
        })
        .unwrap();
    assert!(updated);
    assert_eq!(session.app().generation(), 2);
    assert_eq!(session.app().count, 1);
    assert_eq!(session.app().reloaded_paths.len(), 1);
    assert_eq!(
        session.pixel_buffer().get_rgb(cx as u32, cy as u32),
        Some((168, 85, 247))
    );
    assert!(session
        .status_banner()
        .unwrap()
        .contains("Hot reload gen #2"));

    let updated = session
        .handle_hot_patch(HotPatchMsg::DevStatus {
            message: "Connected".into(),
            latency_ms: 12,
        })
        .unwrap();
    assert!(updated);
    assert_eq!(session.status_banner(), Some("Connected (12ms)"));
}

#[test]
fn websocket_hot_reload_client_receives_messages_and_invokes_wake() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral ws port");
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept client");
        let mut ws = tungstenite::accept(stream).expect("ws handshake");
        let m1 = HotPatchMsg::DevStatus {
            message: "Ready".into(),
            latency_ms: 4,
        };
        let m2 = HotPatchMsg::DylibReload {
            generation: 3,
            dylib_path: "target/debug/libdemo.dylib".into(),
            build_ms: 210,
        };
        ws.send(Message::Binary(m1.to_bytes().into())).unwrap();
        ws.send(Message::Binary(m2.to_bytes().into())).unwrap();
        let _ = ws.close(None);
    });

    let wake_count = Arc::new(AtomicUsize::new(0));
    let wake_counter = Arc::clone(&wake_count);
    let client = HotReloadClient::spawn(format!("ws://{addr}"), move || {
        wake_counter.fetch_add(1, Ordering::SeqCst);
    });

    let first = client
        .receiver()
        .recv_timeout(Duration::from_secs(3))
        .expect("first WS message");
    let second = client
        .receiver()
        .recv_timeout(Duration::from_secs(3))
        .expect("second WS message");

    assert_eq!(
        first,
        HotPatchMsg::DevStatus {
            message: "Ready".into(),
            latency_ms: 4,
        }
    );
    assert_eq!(
        second,
        HotPatchMsg::DylibReload {
            generation: 3,
            dylib_path: "target/debug/libdemo.dylib".into(),
            build_ms: 210,
        }
    );
    assert!(wake_count.load(Ordering::SeqCst) >= 2);

    drop(client);
    server.join().unwrap();
}

#[test]
fn resize_updates_hidpi_viewport_and_scales_pixel_buffer() {
    let viewport = Viewport {
        width: 200.0,
        height: 150.0,
        scale_factor: 1.0,
    };
    let mut session = DesktopSession::new(
        MockCounterApp::new(),
        1,
        viewport,
        ColorRgba::BLACK,
        false,
    );
    session.render_and_rasterize().unwrap();
    assert_eq!(session.pixel_buffer().width(), 200);
    assert_eq!(session.pixel_buffer().height(), 150);

    session.resize(600, 400, 2.0).unwrap();
    assert_eq!(session.viewport().width, 300.0);
    assert_eq!(session.viewport().height, 200.0);
    assert_eq!(session.viewport().scale_factor, 2.0);
    assert_eq!(session.pixel_buffer().width(), 600);
    assert_eq!(session.pixel_buffer().height(), 400);

    let btn = session.layout_of(4).unwrap();
    let px = ((btn.x + btn.width * 0.5) * 2.0) as u32;
    let py = ((btn.y + btn.height * 0.5) * 2.0) as u32;
    assert_eq!(session.pixel_buffer().get_rgb(px, py), Some((37, 99, 235)));

    let tmp_dir = std::env::temp_dir();
    let png_path = tmp_dir.join(format!("nve_skia_test_{}.png", std::process::id()));
    let ppm_path = tmp_dir.join(format!("nve_skia_test_{}.ppm", std::process::id()));
    session.write_screenshot(&png_path).unwrap();
    session.write_screenshot(&ppm_path).unwrap();
    let png_bytes = std::fs::read(&png_path).unwrap();
    let ppm_bytes = std::fs::read(&ppm_path).unwrap();
    let _ = std::fs::remove_file(&png_path);
    let _ = std::fs::remove_file(&ppm_path);
    assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(ppm_bytes.starts_with(b"P6\n"));
}

#[test]
fn desktop_session_presents_to_swapchain_and_records_frame_telemetry() {
    let viewport = Viewport {
        width: 240.0,
        height: 160.0,
        scale_factor: 1.0,
    };
    let mut session = DesktopSession::new(
        MockCounterApp::new(),
        1,
        viewport,
        ColorRgba::BLACK,
        true,
    );
    session.render_and_rasterize().unwrap();
    let init_tel = session
        .last_frame_telemetry()
        .expect("render_and_rasterize should record telemetry");
    assert_eq!(init_tel.backend, session.active_backend());
    assert_eq!(init_tel.frame_index, 1);

    let mut sw = GpuSwapchain::new_offscreen(GpuSurfaceConfig {
        width: 240,
        height: 160,
        scale_factor: 1.0,
        preferred_backend: GpuBackendKind::platform_preferred(),
        allow_cpu_fallback: true,
        ..Default::default()
    })
    .unwrap();
    let tel = session.present_to_swapchain(&mut sw).unwrap();
    assert_eq!(tel.backend, sw.kind());
    assert_eq!(tel.width, 240);
    assert_eq!(tel.height, 160);
    assert_eq!(tel.frame_index, 1);

    session.resize(480, 320, 2.0).unwrap();
    sw.resize(480, 320, 2.0).unwrap();
    let tel2 = session.present_to_swapchain(&mut sw).unwrap();
    assert_eq!(tel2.frame_index, 2);
    assert_eq!(tel2.swapchain_recreations, 1);

    let mut readback = PixelBuffer::new(1, 1);
    sw.copy_to_pixel_buffer(&mut readback).unwrap();
    let btn = session.layout_of(4).unwrap();
    let px = ((btn.x + btn.width * 0.5) * 2.0) as u32;
    let py = ((btn.y + btn.height * 0.5) * 2.0) as u32;
    assert_eq!(readback.get_rgb(px, py), Some((37, 99, 235)));
}

#[test]
fn frame0_nve2_save_cold_prewarm_and_meta_enforcement() {
    let viewport = Viewport {
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
    };
    let mut session = DesktopSession::new(
        MockCounterApp::new(),
        1,
        viewport,
        ColorRgba::BLACK,
        false,
    );
    session.render_and_rasterize().unwrap();

    let btn = session.layout_of(4).unwrap();
    let cx = btn.x + btn.width * 0.5;
    let cy = btn.y + btn.height * 0.5;
    assert!(session.handle_click_logical(cx, cy).unwrap());

    let frame0_path = std::env::temp_dir().join(format!(
        "nve_skia_shell_test_{}.nve2",
        std::process::id()
    ));
    let meta = Frame0Meta {
        table_schema_hash: nve_store::registered_tables_schema_hash(),
        app_build_id: 7,
        env_hash: 0,
    };
    let footer = session.save_frame0(&frame0_path, meta).expect("atomic save");
    assert_eq!(footer.app_build_id, 7);
    assert!(footer.node_count >= 4);

    let wrong_meta = Frame0Meta {
        app_build_id: 8,
        ..meta
    };
    let mismatch = prewarm_frame0_into_replica(&frame0_path, wrong_meta, 1, viewport, ColorRgba::BLACK);
    assert!(matches!(
        mismatch,
        Err(DesktopHostError::Frame0(Frame0Miss::BuildIdMismatch {
            expected: 8,
            actual: 7
        }))
    ));

    let (_replica, pixel_buffer, loaded_footer) =
        prewarm_frame0_into_replica(&frame0_path, meta, 1, viewport, ColorRgba::BLACK)
            .expect("prewarm frame0");
    assert_eq!(loaded_footer.node_count, footer.node_count);
    assert_eq!(
        pixel_buffer.get_rgb(cx as u32, cy as u32),
        Some((16, 185, 129))
    );

    // Booting the same frame0.nve2 at a wider viewport updates live layout immediately.
    let wide_vp = Viewport {
        width: 600.0,
        height: 400.0,
        scale_factor: 1.0,
    };
    let (wide_replica, wide_buf, _) =
        prewarm_frame0_into_replica(&frame0_path, meta, 1, wide_vp, ColorRgba::BLACK)
            .expect("prewarm wide viewport");
    assert_eq!(wide_buf.width(), 600);
    assert_eq!(
        wide_replica.backend().layout_tree().layout_of(2).unwrap().width,
        600.0
    );

    let ppm_path = std::env::temp_dir().join(format!(
        "nve_skia_shell_test_{}.ppm",
        std::process::id()
    ));
    session.write_screenshot_ppm(&ppm_path).expect("write ppm");
    let ppm = std::fs::read(&ppm_path).expect("read ppm");
    assert!(ppm.starts_with(b"P6\n300 200\n255\n"));
    assert_eq!(ppm.len(), b"P6\n300 200\n255\n".len() + 300 * 200 * 3);

    let _ = std::fs::remove_file(&frame0_path);
    let _ = std::fs::remove_file(&ppm_path);
}

#[test]
fn hot_app_slot_reports_loader_error_for_missing_dylib() {
    let err = HotAppSlot::load(Path::new("/nonexistent/path/libmissing_nve.dylib"), 1)
        .err()
        .expect("expected loader error for nonexistent dylib");
    assert!(matches!(err, DesktopHostError::Loader(LoaderError::Io(_))));
}

#[test]
#[ignore = "renders 2x HiDPI visual verification PPMs from compiled example cdylibs"]
fn render_visual_verification_ppms_at_2x_hidpi() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let dylib_ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };

    let targets = [
        (
            "libnve_studio_workspace",
            960.0f32,
            640.0f32,
            ColorRgba::rgb(11, 17, 32),
            "/tmp/nve_skia_cosmic_studio.ppm",
        ),
        (
            "libnve_example_kitchen_sink",
            640.0f32,
            420.0f32,
            ColorRgba::rgb(15, 23, 42),
            "/tmp/nve_skia_cosmic_counter.ppm",
        ),
    ];

    for (lib_stem, w, h, clear, out_path) in targets {
        let dylib = workspace_root.join(format!("target/debug/{lib_stem}.{dylib_ext}"));
        let app = HotAppSlot::load(&dylib, 1)
            .unwrap_or_else(|e| panic!("failed to load {dylib:?}: {e}"));
        let viewport = Viewport {
            width: w,
            height: h,
            scale_factor: 2.0,
        };
        let mut session = DesktopSession::new(app, 1, viewport, clear, false);
        session
            .render_and_rasterize()
            .expect("render and rasterize at 2x HiDPI");
        let buf = session.pixel_buffer();
        assert_eq!(buf.width(), (w * 2.0) as u32);
        assert_eq!(buf.height(), (h * 2.0) as u32);
        write_pixel_buffer_ppm(buf, Path::new(out_path)).expect("write ppm");
    }
}
