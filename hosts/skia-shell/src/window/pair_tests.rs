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
    decode_events, ColorRgba, EventMask, EventPayload, FrameId, NativeEventKind, NVE_KEY_MOD_META,
};
use nve_components::tables::{register_core_tables, FlexDirection, Interaction, Layout, Visual};
use nve_engine::{HotPatchMsg, Viewport};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId as StoreNodeId, NodeStore, Rgba};
use nve_wire::{NveWireKind, NveWireServer, NveWireServerConfig, NveWireTransportKind};
use tungstenite::Message;

use super::{
    render_paired_headless_screenshot, DesktopApp, DesktopHostError, DesktopPreviewOptions,
    DesktopSession, HotReloadClient, PairConnectConfig, PairedRemoteApp,
};

struct RemoteServerApp {
    clicks: u32,
    palette_open: bool,
}

impl RemoteServerApp {
    fn new() -> Self {
        register_core_tables();
        Self {
            clicks: 0,
            palette_open: false,
        }
    }

    fn render(&self, _frame_id: FrameId) -> Vec<u8> {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        {
            let mut tx = store.tx();
            let _ = tx.create_at(StoreNodeId::ROOT);
            let _ = tx.create_at(StoreNodeId(2));
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
            tx.set(
                StoreNodeId(2),
                Visual::BACKGROUND,
                if self.palette_open {
                    Rgba::rgb(250, 204, 21)
                } else {
                    Rgba::rgb(15, 23, 42)
                },
            );
            tx.set(
                StoreNodeId(2),
                Interaction::EVENT_MASK,
                (EventMask::KEY_DOWN | EventMask::CUSTOM) as u32,
            );

            tx.set(StoreNodeId(4), Layout::WIDTH, Length::points(160.0));
            tx.set(StoreNodeId(4), Layout::HEIGHT, Length::points(56.0));
            tx.set(
                StoreNodeId(4),
                Visual::BACKGROUND,
                if self.clicks == 0 {
                    Rgba::rgb(37, 99, 235)
                } else {
                    Rgba::rgb(16, 185, 129)
                },
            );
            tx.set(StoreNodeId(4), Visual::BORDER_RADIUS, 8.0);
            tx.set(StoreNodeId(4), Interaction::EVENT_MASK, EventMask::PRESS as u32);
            tx.set_children(StoreNodeId(2), &[StoreNodeId(4)]);
            tx.set_children(StoreNodeId::ROOT, &[StoreNodeId(2)]);
            tx.commit_into(&mut changes);
        }
        let mut out = Vec::new();
        WireEncoder::default().encode(&store, &changes, &mut out);
        out
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
            let _ = server.broadcast_scene_keyframe(frame_id as u32, &initial);

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
        NveWireTransportKind::Quic,
        NveWireTransportKind::WebTransport,
        NveWireTransportKind::Wss,
        NveWireTransportKind::Ws,
    ] {
        let server = TestPairServer::start("skia-pair-e2e-token");
        let addr = match transport {
            NveWireTransportKind::Quic => format!("quic://{}", server.quic_addr),
            NveWireTransportKind::WebTransport => format!("https://{}/nve-wire", server.quic_addr),
            NveWireTransportKind::Wss => format!("wss://{}/nve-wire", server.wss_addr),
            NveWireTransportKind::Ws | NveWireTransportKind::Auto => format!("ws://{}/nve-wire", server.wss_addr),
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
        NveWireTransportKind::Ws,
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
            NveWireTransportKind::Wss,
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

#[test]
fn paired_remote_app_preserves_multi_delta_batch_order_and_resets_on_high_frame_keyframe() {
    register_core_tables();
    let token = "skia-ordered-batch-token";
    let server = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .expect("bind server");

    let cfg = PairConnectConfig::new(
        format!("ws://{}/nve-wire", server.wss_addr()),
        token,
        NveWireTransportKind::Ws,
    );
    let app = PairedRemoteApp::connect(cfg).expect("connect paired app");
    let viewport = Viewport {
        width: 240.0,
        height: 160.0,
        scale_factor: 1.0,
    };
    let mut session = DesktopSession::new(app, 1, viewport, ColorRgba::rgb(0, 0, 0), false);

    // Build Frame 1 (DeclareTables + CreateNodes + blue fill) and true incremental Frame 2
    // (Patch ONLY -> green fill, NO DeclareTables/CreateNodes).
    let mut store = NodeStore::new();
    let mut enc = WireEncoder::default();
    let mut f1_bytes = Vec::new();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        let _ = tx.create_at(StoreNodeId::ROOT);
        let _ = tx.create_at(StoreNodeId(2));
        tx.set(StoreNodeId(2), Layout::WIDTH, Length::percent(100.0));
        tx.set(StoreNodeId(2), Layout::HEIGHT, Length::percent(100.0));
        tx.set(StoreNodeId(2), Visual::BACKGROUND, Rgba::rgb(37, 99, 235));
        tx.set_children(StoreNodeId::ROOT, &[StoreNodeId(2)]);
        tx.commit_into(&mut changes);
    }
    enc.encode(&store, &changes, &mut f1_bytes);

    let mut f2_bytes = Vec::new();
    {
        let mut tx = store.tx();
        tx.set(StoreNodeId(2), Visual::BACKGROUND, Rgba::rgb(16, 185, 129));
        tx.commit_into(&mut changes);
    }
    enc.encode(&store, &changes, &mut f2_bytes);

    // Broadcast BOTH Frame 1 and Frame 2 back-to-back BEFORE a single render_and_rasterize()!
    assert_eq!(server.broadcast_command_delta(1, &f1_bytes).unwrap(), 1);
    assert_eq!(server.broadcast_command_delta(2, &f2_bytes).unwrap(), 1);
    let deadline = Instant::now() + Duration::from_secs(2);
    while session.app().frames_received() < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(session.app().frames_received() >= 2);
    assert!(session.app().has_pending_remote_frame());

    // A single render_and_rasterize() must drain Frame 1 AND Frame 2 in exact order -> green!
    session.render_and_rasterize().expect("apply ordered batch");
    assert_eq!(
        session.pixel_buffer().get_rgb(50, 50),
        Some((16, 185, 129)),
        "Frame 1 must not be overwritten by Frame 2 before applying"
    );

    // Now send a SceneKeyframe whose NVE2 header has frame = 5 (> 1) with a fresh tree (node 9,
    // yellow fill) and verify StoreWireReplica resets cleanly (node 2 gone, yellow rendered).
    let mut store2 = NodeStore::new();
    let mut changes2 = ChangeSet::default();
    {
        let mut tx = store2.tx();
        let _ = tx.create_at(StoreNodeId::ROOT);
        let _ = tx.create_at(StoreNodeId(9));
        tx.set(StoreNodeId(9), Layout::WIDTH, Length::percent(100.0));
        tx.set(StoreNodeId(9), Layout::HEIGHT, Length::percent(100.0));
        tx.set(StoreNodeId(9), Visual::BACKGROUND, Rgba::rgb(250, 204, 21));
        tx.set_children(StoreNodeId::ROOT, &[StoreNodeId(9)]);
        tx.commit_into(&mut changes2);
    }
    let mut kf_bytes = Vec::new();
    WireEncoder::default().encode_full(&store2, &mut kf_bytes);
    // Patch NVE2 frame_lo (offset 8..12) from 1 to 5 to verify SceneKeyframe resets even when frame > 1.
    kf_bytes[8..12].copy_from_slice(&5u32.to_le_bytes());

    assert_eq!(server.broadcast_scene_keyframe(5, &kf_bytes).unwrap(), 1);
    let kf_deadline = Instant::now() + Duration::from_secs(2);
    while session.app().frames_received() < 3 && Instant::now() < kf_deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(session.app().has_pending_remote_frame());
    session.render_and_rasterize().expect("apply frame-5 keyframe");
    assert_eq!(session.pixel_buffer().get_rgb(50, 50), Some((250, 204, 21)));
    assert!(
        !session.store().is_live(StoreNodeId(2)),
        "SceneKeyframe with frame > 1 must reset prior replica nodes"
    );
    assert!(session.store().is_live(StoreNodeId(9)));

    session.app_mut().disconnect();
    server.shutdown();
}

#[test]
fn paired_remote_app_recovers_from_corrupt_frame_and_shows_auth_rejected_banner() {
    register_core_tables();
    let token = "skia-corrupt-recover-token";
    let server = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .expect("bind server");

    let cfg = PairConnectConfig::new(
        format!("ws://{}/nve-wire", server.wss_addr()),
        token,
        NveWireTransportKind::Ws,
    );
    let app = PairedRemoteApp::connect(cfg).expect("connect paired app");
    let viewport = Viewport {
        width: 240.0,
        height: 160.0,
        scale_factor: 1.0,
    };
    let mut session = DesktopSession::new(app, 1, viewport, ColorRgba::rgb(0, 0, 0), false);

    // Build valid Frame 1 (blue) and corrupt Frame 2 (garbage payload).
    let mut store = NodeStore::new();
    let mut enc = WireEncoder::default();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        let _ = tx.create_at(StoreNodeId::ROOT);
        let _ = tx.create_at(StoreNodeId(2));
        tx.set(StoreNodeId(2), Layout::WIDTH, Length::percent(100.0));
        tx.set(StoreNodeId(2), Layout::HEIGHT, Length::percent(100.0));
        tx.set(StoreNodeId(2), Visual::BACKGROUND, Rgba::rgb(37, 99, 235));
        tx.set_children(StoreNodeId::ROOT, &[StoreNodeId(2)]);
        tx.commit_into(&mut changes);
    }
    let mut f1 = Vec::new();
    enc.encode(&store, &changes, &mut f1);

    assert_eq!(server.broadcast_scene_keyframe(1, &f1).unwrap(), 1);
    assert_eq!(server.broadcast_command_delta(2, b"CORRUPT_NVE2_FRAME").unwrap(), 1);
    let wait_rx = |sess: &DesktopSession<PairedRemoteApp>, target: u64| {
        let deadline = Instant::now() + Duration::from_secs(2);
        while sess.app().frames_received() < target && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            sess.app().frames_received() >= target,
            "expected >= {target} frames, got {}",
            sess.app().frames_received()
        );
    };
    wait_rx(&session, 2);

    // 1. render_and_rasterize MUST NOT abort on Frame 2 error, MUST still rasterize valid Frame 1
    //    (blue), and MUST trigger ONE KeyframeRequest!
    session
        .render_and_rasterize()
        .expect("corrupt remote frame must recover rather than return Err");
    assert_eq!(
        session.pixel_buffer().get_rgb(50, 50),
        Some((37, 99, 235)),
        "earlier valid Frame 1 must still be rasterized when Frame 2 fails"
    );
    let req = server
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server receives KeyframeRequest after corrupt Frame 2");
    assert_eq!(req.frame.kind, NveWireKind::KeyframeRequest);

    // 2. While waiting for a valid keyframe: a corrupt SceneKeyframe MUST NOT clear waiting state
    //    or spam another KeyframeRequest!
    assert!(server
        .send_scene_keyframe_to_peer(req.session_id, 3, b"BROKEN_KF")
        .unwrap());
    wait_rx(&session, 3);
    session
        .render_and_rasterize()
        .expect("corrupt keyframe must not abort");
    assert!(
        server
            .recv_inbound_timeout(Duration::from_millis(150))
            .is_none(),
        "must not spam duplicate KeyframeRequests in the same resync epoch"
    );

    // 3. Send valid recovery SceneKeyframe (yellow) + immediate trailing CommandDelta (green) in
    //    ONE burst -> single render_and_rasterize() must clear waiting_for_keyframe on the valid
    //    keyframe AND apply the trailing delta in order -> green!
    let mut store_rec = NodeStore::new();
    let mut enc_rec = WireEncoder::default();
    let mut ch_rec = ChangeSet::default();
    {
        let mut tx = store_rec.tx();
        let _ = tx.create_at(StoreNodeId::ROOT);
        let _ = tx.create_at(StoreNodeId(5));
        tx.set(StoreNodeId(5), Layout::WIDTH, Length::percent(100.0));
        tx.set(StoreNodeId(5), Layout::HEIGHT, Length::percent(100.0));
        tx.set(StoreNodeId(5), Visual::BACKGROUND, Rgba::rgb(250, 204, 21));
        tx.set_children(StoreNodeId::ROOT, &[StoreNodeId(5)]);
        tx.commit_into(&mut ch_rec);
    }
    let mut kf_yellow = Vec::new();
    enc_rec.encode(&store_rec, &ch_rec, &mut kf_yellow);
    {
        let mut tx = store_rec.tx();
        tx.set(StoreNodeId(5), Visual::BACKGROUND, Rgba::rgb(16, 185, 129));
        tx.commit_into(&mut ch_rec);
    }
    let mut delta_green = Vec::new();
    enc_rec.encode(&store_rec, &ch_rec, &mut delta_green);

    assert!(server
        .send_scene_keyframe_to_peer(req.session_id, 4, &kf_yellow)
        .unwrap());
    assert_eq!(server.broadcast_command_delta(5, &delta_green).unwrap(), 1);
    wait_rx(&session, 5);
    session
        .render_and_rasterize()
        .expect("valid keyframe + trailing delta must apply cleanly");
    assert_eq!(session.pixel_buffer().get_rgb(50, 50), Some((16, 185, 129)));

    // 3b. Mid-frame corrupt delta (valid NVE2 header + CreateNodes(99) + corrupt op 0xFE) must
    //     recover cleanly, request a keyframe, and reset cleanly on recovery (no leaked node 99)!
    let mut mid_corrupt = Vec::new();
    for _ in 0..6 {
        mid_corrupt.extend_from_slice(&0u32.to_le_bytes());
    }
    mid_corrupt.extend_from_slice(&(3u32 | (1u32 << 16)).to_le_bytes()); // CreateNodes count=1
    mid_corrupt.extend_from_slice(&1u32.to_le_bytes());
    mid_corrupt.extend_from_slice(&99u32.to_le_bytes());
    mid_corrupt.extend_from_slice(&(0xFEu32 | (1u32 << 16)).to_le_bytes()); // unknown op 0xFE
    mid_corrupt.extend_from_slice(&1u32.to_le_bytes());
    mid_corrupt.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    let total_words = (mid_corrupt.len() / 4) as u32;
    mid_corrupt[0..4].copy_from_slice(&0x3245_564Eu32.to_le_bytes());
    mid_corrupt[4..8].copy_from_slice(&1u32.to_le_bytes());
    mid_corrupt[8..12].copy_from_slice(&6u32.to_le_bytes());
    mid_corrupt[12..16].copy_from_slice(&0u32.to_le_bytes());
    mid_corrupt[16..20].copy_from_slice(&2u32.to_le_bytes());
    mid_corrupt[20..24].copy_from_slice(&total_words.to_le_bytes());

    assert_eq!(server.broadcast_command_delta(6, &mid_corrupt).unwrap(), 1);
    wait_rx(&session, 6);
    session
        .render_and_rasterize()
        .expect("mid-frame corrupt delta must recover, not return Err");
    let req2 = server
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server receives KeyframeRequest after mid-frame corrupt delta");
    assert_eq!(req2.frame.kind, NveWireKind::KeyframeRequest);

    assert!(server
        .send_scene_keyframe_to_peer(req2.session_id, 7, &kf_yellow)
        .unwrap());
    wait_rx(&session, 7);
    session
        .render_and_rasterize()
        .expect("recovery keyframe after mid-frame corrupt delta");
    assert_eq!(session.pixel_buffer().get_rgb(50, 50), Some((250, 204, 21)));
    assert!(
        !session.store().is_live(StoreNodeId(99)),
        "mid-frame corrupt node 99 must not survive recovery keyframe"
    );

    // 4. Terminal auth rejection surfaces `[AUTH REJECTED]` in remote_status_banner!
    let bad_cfg = PairConnectConfig::new(
        format!("ws://{}/nve-wire", server.wss_addr()),
        "wrong-token-for-banner",
        NveWireTransportKind::Ws,
    );
    let mut bad_app = PairedRemoteApp::spawn_for_test(bad_cfg).expect("spawn bad-token app");
    let auth_deadline = Instant::now() + Duration::from_secs(3);
    while bad_app.auth_rejections() == 0 && Instant::now() < auth_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(bad_app.auth_rejections(), 1);
    let banner = bad_app
        .remote_status_banner()
        .expect("paired app status banner");
    assert!(
        banner.contains("AUTH REJECTED"),
        "expected AUTH REJECTED in banner, got `{banner}`"
    );

    bad_app.disconnect();
    session.app_mut().disconnect();
    server.shutdown();
}
