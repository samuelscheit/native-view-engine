//! Deep end-to-end integration tests for live `NVT1`/`NVD1` pairing in `nve-skia-shell`
//! (UDP, raw TCP, and RFC 6455 binary WebSocket) plus `NVP1` binary hot-reload frames.

use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use nve_abi::events::{decode_keydown_bytes, KEYDOWN_WIRE_MAGIC};
use nve_abi::{
    decode_events, ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets,
    EventMask, EventPayload, FlexDirection, FrameId, LayoutProps, NativeEventKind, StyleProps,
    ROOT_NODE_ID,
};
use nve_engine::{HotPatchMsg, Viewport};
use nve_interop::{PairingConfig, TcpDeltaServer, UdpDeltaServer};
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
        b.set_event_mask(2, EventMask::CUSTOM);

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
                (NativeEventKind::Custom, EventPayload::Custom { name, data })
                    if name == "keydown" =>
                {
                    assert!(
                        data.starts_with(&KEYDOWN_WIRE_MAGIC),
                        "remote keydown must use NK1 binary payload"
                    );
                    if let Some(dk) = decode_keydown_bytes(&data)
                        && dk.meta
                        && dk.key == "k"
                    {
                        self.palette_open = !self.palette_open;
                        changed = true;
                    }
                }
                _ => {}
            }
        }
        changed
    }
}

struct TestPairServer {
    udp_addr: SocketAddr,
    tcp_addr: SocketAddr,
    token: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestPairServer {
    fn start(token: &str) -> Self {
        let cfg = PairingConfig::new(token);
        let udp = UdpDeltaServer::bind("127.0.0.1:0", cfg.clone()).expect("bind UDP server");
        let tcp = TcpDeltaServer::bind("127.0.0.1:0", cfg).expect("bind TCP/WS server");
        let udp_addr = udp.local_addr();
        let tcp_addr = tcp.local_addr();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            let mut app = RemoteServerApp::new();
            let mut frame_id = 1u64;
            let initial = app.render(frame_id);
            let _ = udp.send_frame(&initial);
            let _ = tcp.send_frame(&initial);

            while !stop_flag.load(Ordering::Relaxed) {
                let mut mutated = false;
                while let Some((_sid, ev_bytes)) = udp.try_recv_events() {
                    mutated |= app.dispatch(&ev_bytes);
                }
                while let Some((_sid, ev_bytes)) = tcp.try_recv_events() {
                    mutated |= app.dispatch(&ev_bytes);
                }
                if mutated {
                    frame_id += 1;
                    let next = app.render(frame_id);
                    let _ = udp.send_frame(&next);
                    let _ = tcp.send_frame(&next);
                } else {
                    thread::sleep(Duration::from_millis(2));
                }
            }
            let mut udp = udp;
            let mut tcp = tcp;
            udp.shutdown();
            tcp.shutdown();
        });

        Self {
            udp_addr,
            tcp_addr,
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
        "pixel ({x},{y}) never reached {expected:?}; last={:?}",
        session.pixel_buffer().get_rgb(x, y)
    );
}

#[test]
fn hot_reload_client_decodes_nvp1_binary_and_json_frames() {
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
        let json_msg = HotPatchMsg::DevStatus {
            message: "Synced".into(),
            latency_ms: 2,
        };
        ws.send(Message::Binary(binary_msg.to_bytes().into()))
            .unwrap();
        ws.send(Message::Text(json_msg.to_json().into())).unwrap();
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
        .expect("json text message");

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
    assert!(wakes.load(Ordering::SeqCst) >= 2);

    drop(client);
    server.join().unwrap();
}

#[test]
fn paired_remote_app_e2e_udp_tcp_and_ws_rasterizes_and_mutates_pixels() {
    // Exercise ALL three transports independently with fresh server state so Frame-1 blue -> green,
    // NK1 Cmd+K palette toggle, EventLoop wake callback, and status banner are verified on each.
    for transport in [PairTransport::Udp, PairTransport::Tcp, PairTransport::Ws] {
        let server = TestPairServer::start("skia-pair-e2e-token");
        let addr = match transport {
            PairTransport::Udp => server.udp_addr.to_string(),
            PairTransport::Tcp => server.tcp_addr.to_string(),
            PairTransport::Ws => format!("ws://{}/ws/nvd", server.tcp_addr),
        };
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_cb = Arc::clone(&wakes);

        let mut app = PairedRemoteApp::connect(PairConnectConfig::new(
            addr,
            server.token.clone(),
            transport,
        ))
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
        let mut session =
            DesktopSession::new(app, 1, viewport, ColorRgba::rgb(0, 0, 0), false);

        // Wait for initial Frame 1 over NVD1 and rasterize pixels.
        session
            .app_mut()
            .wait_for_frame(Duration::from_secs(2))
            .expect("initial paired frame");
        session.render_and_rasterize().expect("rasterize Frame 1");

        // Button starts blue (37, 99, 235) at logical (60, 48) inside padding 20..180, 20..76.
        assert_eq!(
            session.pixel_buffer().get_rgb(60, 48),
            Some((37, 99, 235)),
            "{transport:?}: initial button pixel should be blue"
        );
        // Background starts dark slate (15, 23, 42) at (250, 150).
        assert_eq!(
            session.pixel_buffer().get_rgb(250, 150),
            Some((15, 23, 42)),
            "{transport:?}: initial background should be dark slate"
        );

        // 1. Remote click -> NVT1 InputEvents -> server mutates -> NVD1 delta -> client pixel green.
        assert!(session.handle_click_logical(60.0, 48.0).unwrap());
        wait_until_pixel(
            &mut session,
            60,
            48,
            (16, 185, 129),
            Duration::from_secs(2),
        );

        // 2. Remote NK1 Cmd+K keydown -> server toggles palette -> client background gold (250, 204, 21).
        assert!(session.handle_keydown("k", true, false, false, false).unwrap());
        wait_until_pixel(
            &mut session,
            250,
            150,
            (250, 204, 21),
            Duration::from_secs(2),
        );

        // Reset palette back.
        assert!(session.handle_keydown("k", true, false, false, false).unwrap());
        wait_until_pixel(
            &mut session,
            250,
            150,
            (15, 23, 42),
            Duration::from_secs(2),
        );

        assert!(
            wakes.load(Ordering::SeqCst) >= 2,
            "{transport:?}: expected EventLoop wake callback on incoming frames"
        );
        assert!(session.app().remote_status_banner().unwrap().contains("LIVE"));
        session.app_mut().disconnect();
        assert!(!session.app().is_connected());
    }
}

#[test]
fn paired_headless_screenshot_writes_png_and_rejects_bad_token() {
    let server = TestPairServer::start("skia-screenshot-token");

    // Wrong token fails fast with DesktopHostError::Pairing.
    let bad = PairedRemoteApp::connect(PairConnectConfig::new(
        server.tcp_addr.to_string(),
        "wrong-token",
        PairTransport::Ws,
    ));
    assert!(matches!(bad, Err(DesktopHostError::Pairing(_))));

    // Valid --screenshot flow writes a valid PNG file and disconnects cleanly.
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
            format!("ws://{}/ws/nvd", server.tcp_addr),
            server.token.clone(),
            PairTransport::Ws,
        ),
        opts,
        out_path.clone(),
    )
    .expect("paired headless PNG screenshot");

    let png = std::fs::read(&out_path).expect("read paired PNG");
    assert!(
        png.starts_with(b"\x89PNG\r\n\x1a\n"),
        "expected valid PNG signature"
    );
    assert!(png.len() > 100);
    let _ = std::fs::remove_file(Path::new(&out_path));
}
