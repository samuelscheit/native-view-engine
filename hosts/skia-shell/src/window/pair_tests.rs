//! End-to-end integration tests for live `NveWireFrame` v2 pairing in `nve-skia-shell`
//! (raw QUIC `nve-wire-2`, HTTP/3 WebTransport `h3`, TLS `wss://`, and loopback `ws://`)
//! plus `NVP1` binary hot-reload frames.

use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use nve_abi::{
    decode_events, ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets,
    EventMask, EventPayload, FlexDirection, FrameId, LayoutProps, NativeEventKind, StyleProps,
    NVE_KEY_MOD_META, ROOT_NODE_ID,
};
use nve_engine::{HotPatchMsg, Viewport};
use nve_wire::{NveWireKind, NveWireServer, NveWireServerConfig};
use tungstenite::Message;

use super::{
    render_paired_headless_screenshot, DesktopApp, DesktopHostError, DesktopPreviewOptions,
    DesktopSession, HotReloadClient, PairConnectConfig, PairTransport, PairedRemoteApp,
};

struct RemoteServerApp {
    clicks: u32,
    palette_open: bool,
}

impl RemoteServerApp {
    fn new() -> Self {
        Self {
            clicks: 0,
            palette_open: false,
        }
    }

    fn render(&self, frame_id: FrameId) -> Vec<u8> {
        let mut b = CommandBufferBuilder::new(1, frame_id);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Percent(100.0),
                height: Dimension::Percent(100.0),
                padding: EdgeInsets::all(20.0),
                gap: 12.0,
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: if self.palette_open {
                    ColorRgba::rgb(250, 204, 21)
                } else {
                    ColorRgba::rgb(15, 23, 42)
                },
                ..Default::default()
            },
        );
        b.set_event_mask(2, EventMask::KEY_DOWN | EventMask::CUSTOM);

        b.create_node(4, ComponentKind::Pressable, None);
        b.set_layout(
            4,
            &LayoutProps {
                width: Dimension::Points(160.0),
                height: Dimension::Points(56.0),
                ..Default::default()
            },
        );
        let btn_color = if self.clicks == 0 {
            ColorRgba::rgb(37, 99, 235)
        } else {
            ColorRgba::rgb(16, 185, 129)
        };
        b.set_style(
            4,
            &StyleProps {
                background: btn_color,
                border_radius: 8.0,
                ..Default::default()
            },
        );
        b.set_event_mask(4, EventMask::PRESS);

        b.set_children(2, &[4]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        b.into_bytes()
    }

    fn dispatch(&mut self, encoded_events: &[u8]) -> bool {
        let Ok(events) = decode_events(encoded_events) else {
            return false;
        };
        let mut changed = false;
        for ev in events {
            match (ev.kind, ev.payload) {
                (NativeEventKind::Press, _) if ev.node_id == 4 => {
                    self.clicks += 1;
                    changed = true;
                }
                (NativeEventKind::KeyDown, EventPayload::Key { key, modifiers, .. })
                    if (modifiers & NVE_KEY_MOD_META) != 0 && key == "k" =>
                {
                    self.palette_open = !self.palette_open;
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }
}

struct TestPairServer {
    quic_addr: SocketAddr,
    wss_addr: SocketAddr,
    cert_sha256: [u8; 32],
    token: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestPairServer {
    fn start(token: &str) -> Self {
        let server = NveWireServer::bind_sync(NveWireServerConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1:0".parse().unwrap(),
            token,
        ))
        .expect("bind NveWireServer");
        let quic_addr = server.quic_addr();
        let wss_addr = server.wss_addr();
        let cert_sha256 = server.cert_sha256();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            let mut app = RemoteServerApp::new();
            let mut frame_id = 1u64;
            let initial = app.render(frame_id);
            let _ = server.broadcast_command_delta(frame_id as u32, &initial);

            while !stop_flag.load(Ordering::Relaxed) {
                let mut mutated = false;
                while let Some(msg) = server.try_recv_inbound() {
                    if msg.frame.kind == NveWireKind::EventBatch {
                        mutated |= app.dispatch(&msg.frame.payload);
                    }
                }
                if mutated {
                    frame_id += 1;
                    let next = app.render(frame_id);
                    let _ = server.broadcast_command_delta(frame_id as u32, &next);
                } else {
                    thread::sleep(Duration::from_millis(2));
                }
            }
            server.shutdown();
        });

        Self {
            quic_addr,
            wss_addr,
            cert_sha256,
            token: token.to_string(),
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for TestPairServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn wait_until_pixel(
    session: &mut DesktopSession<PairedRemoteApp>,
    x: u32,
    y: u32,
    expected: (u8, u8, u8),
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if session.app().has_pending_remote_frame() {
            session.render_and_rasterize().expect("render remote frame");
        }
        if session.pixel_buffer().get_rgb(x, y) == Some(expected) {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "pixel ({x},{y}) never reached {expected:?}; last={:?}; transport={:?} connected={} frames={} seq={} metrics={:?}",
        session.pixel_buffer().get_rgb(x, y),
        session.app().transport(),
        session.app().is_connected(),
        session.app().frames_received(),
        session.app().last_seq(),
        session.app().metrics(),
    );
}

#[test]
fn hot_reload_client_decodes_nvp1_and_ignores_text_frames() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral ws port");
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept client");
        let mut ws = tungstenite::accept(stream).expect("ws handshake");
        let binary_msg = HotPatchMsg::TemplatePatch {
            generation: 7,
            template_id: "counter-header".into(),
            text_updates: vec![(0, "Live NVP1".into())],
            attr_updates: vec![],
            target_test_id: Some("counter-title".into()),
            node_path: vec![0, 1],
            text_replacements: vec![("Count: 0".into(), "Live NVP1".into())],
        };
        let status_msg = HotPatchMsg::DevStatus {
            message: "Synced".into(),
            latency_ms: 2,
        };
        ws.send(Message::Binary(binary_msg.to_bytes().into()))
            .unwrap();
        ws.send(Message::Text(
            r#"{"type":"dev_status","message":"x","latency_ms":0}"#.into(),
        ))
        .unwrap();
        ws.send(Message::Binary(b"NVP1\x03garbage".to_vec().into()))
            .unwrap();
        ws.send(Message::Binary(status_msg.to_bytes().into()))
            .unwrap();
        let _ = ws.close(None);
    });

    let wakes = Arc::new(AtomicUsize::new(0));
    let wakes_clone = Arc::clone(&wakes);
    let client = HotReloadClient::spawn(format!("ws://{addr}"), move || {
        wakes_clone.fetch_add(1, Ordering::SeqCst);
    });

    let first = client
        .receiver()
        .recv_timeout(Duration::from_secs(3))
        .expect("binary NVP1 message");
    let second = client
        .receiver()
        .recv_timeout(Duration::from_secs(3))
        .expect("second NVP1 message");

    assert_eq!(
        first,
        HotPatchMsg::TemplatePatch {
            generation: 7,
            template_id: "counter-header".into(),
            text_updates: vec![(0, "Live NVP1".into())],
            attr_updates: vec![],
            target_test_id: Some("counter-title".into()),
            node_path: vec![0, 1],
            text_replacements: vec![("Count: 0".into(), "Live NVP1".into())],
        }
    );
    assert_eq!(
        second,
        HotPatchMsg::DevStatus {
            message: "Synced".into(),
            latency_ms: 2,
        }
    );
    let wake_deadline = Instant::now() + Duration::from_secs(1);
    while wakes.load(Ordering::SeqCst) < 2 && Instant::now() < wake_deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(wakes.load(Ordering::SeqCst) >= 2);

    drop(client);
    server.join().unwrap();
}

#[test]
fn paired_remote_app_e2e_quic_webtransport_wss_and_ws_rasterizes_and_mutates_pixels() {
    for transport in [
        PairTransport::Quic,
        PairTransport::WebTransport,
        PairTransport::Wss,
        PairTransport::Ws,
    ] {
        let server = TestPairServer::start("skia-pair-e2e-token");
        let addr = match transport {
            PairTransport::Quic => format!("quic://{}", server.quic_addr),
            PairTransport::WebTransport => format!("https://{}/nve-wire", server.quic_addr),
            PairTransport::Wss => format!("wss://{}/nve-wire", server.wss_addr),
            PairTransport::Ws | PairTransport::Auto => format!("ws://{}/nve-wire", server.wss_addr),
        };
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_cb = Arc::clone(&wakes);

        let cfg = PairConnectConfig::new(addr, server.token.clone(), transport)
            .with_cert_sha256(server.cert_sha256);
        let mut app = PairedRemoteApp::connect(cfg)
            .unwrap_or_else(|e| panic!("connect {transport:?} failed: {e}"));
        app.install_wake_callback(Arc::new(move || {
            wakes_cb.fetch_add(1, Ordering::SeqCst);
        }));
        assert!(app.is_connected());
        assert_ne!(app.session_id(), 0);

        let viewport = Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        };
        let mut session = DesktopSession::new(app, 1, viewport, ColorRgba::rgb(0, 0, 0), false);

        session
            .app_mut()
            .wait_for_frame(Duration::from_secs(2))
            .expect("initial paired frame");
        session.render_and_rasterize().expect("rasterize Frame 1");

        assert_eq!(session.pixel_buffer().get_rgb(60, 48), Some((37, 99, 235)));
        assert_eq!(session.pixel_buffer().get_rgb(250, 150), Some((15, 23, 42)));

        assert!(session.handle_click_logical(60.0, 48.0).unwrap());
        wait_until_pixel(&mut session, 60, 48, (16, 185, 129), Duration::from_secs(2));

        assert!(session.handle_keydown("k", true, false, false, false).unwrap());
        wait_until_pixel(&mut session, 250, 150, (250, 204, 21), Duration::from_secs(2));

        assert!(session.handle_keydown("k", true, false, false, false).unwrap());
        wait_until_pixel(&mut session, 250, 150, (15, 23, 42), Duration::from_secs(2));

        assert!(wakes.load(Ordering::SeqCst) >= 2);
        assert!(session.app().remote_status_banner().unwrap().contains("LIVE"));
        session.app_mut().disconnect();
        assert!(!session.app().is_connected());
    }
}

#[test]
fn paired_headless_screenshot_writes_png_and_rejects_bad_token() {
    let server = TestPairServer::start("skia-screenshot-token");

    let bad = PairedRemoteApp::connect(PairConnectConfig::new(
        format!("ws://{}/nve-wire", server.wss_addr),
        "wrong-token",
        PairTransport::Ws,
    ));
    assert!(matches!(bad, Err(DesktopHostError::Pairing(_))));

    let out_path = std::env::temp_dir().join(format!(
        "nve_skia_paired_shot_{}.png",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_path);

    let opts = DesktopPreviewOptions {
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
        show_hud: false,
        ws_url: None,
        ..Default::default()
    };
    render_paired_headless_screenshot(
        PairConnectConfig::new(
            format!("wss://{}/nve-wire", server.wss_addr),
            server.token.clone(),
            PairTransport::Wss,
        )
        .with_cert_sha256(server.cert_sha256),
        opts,
        out_path.clone(),
    )
    .expect("paired headless PNG screenshot");

    let png = std::fs::read(&out_path).expect("read paired PNG");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(png.len() > 100);
    let _ = std::fs::remove_file(Path::new(&out_path));
}
