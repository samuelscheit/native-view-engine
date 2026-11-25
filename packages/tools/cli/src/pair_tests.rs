use nve_backend_headless::HeadlessBackend;
use nve_engine::{RenderBackend, Viewport};
use nve_interop::{PairingConfig, TcpDeltaClient, UdpDeltaClient};

use super::*;
use crate::doctor::InspectDemo;

#[test]
fn pair_token_generation_and_uri_formatting() {
    let (t1, t2) = (generate_pairing_token(), generate_pairing_token());
    assert!(t1.starts_with("nve-") && t1 != t2);
    assert_eq!(
        resolve_or_generate_token(Some("  custom-tok  ".into())),
        "custom-tok"
    );
    assert!(resolve_or_generate_token(Some("   ".into())).starts_with("nve-"));
    assert!(resolve_or_generate_token(None).starts_with("nve-"));
    assert_eq!(
        format_pairing_uri("0.0.0.0", 8091, 8092, "tok123", 393.0, 852.0),
        "nve://pair?host=127.0.0.1&udp=8091&ws=8092&token=tok123&w=393&h=852"
    );
    assert_eq!(
        format_browser_pair_query("0.0.0.0", 8092, "tok123"),
        "?pair=ws://127.0.0.1:8092&token=tok123"
    );
    assert!(format_pairing_uri("192.168.1.42", 9001, 9002, "lan", 430.0, 932.0)
        .contains("host=192.168.1.42"));
    assert_eq!(format_bind_addr("127.0.0.1", 0), "127.0.0.1:0");
    assert_eq!(format_bind_addr("::1", 0), "[::1]:0");

    let udp_addr: SocketAddr = "127.0.0.1:41001".parse().unwrap();
    let ws_addr: SocketAddr = "127.0.0.1:41002".parse().unwrap();
    let ready = format_pair_ready_line("0.0.0.0", udp_addr, ws_addr, "tok123");
    assert_eq!(
        ready,
        "NVE_PAIR_READY ws=ws://127.0.0.1:41002 token=tok123 udp=127.0.0.1:41001 tcp=127.0.0.1:41002"
    );
    let banner = format_pair_banner(
        Path::new("target/debug/libnve_example_counter.dylib"),
        "0.0.0.0",
        udp_addr,
        ws_addr,
        "tok123",
        393.0,
        852.0,
    );
    assert!(banner.contains("nve://pair?host=127.0.0.1&udp=41001&ws=41002&token=tok123"));
    assert!(banner.contains("?pair=ws://127.0.0.1:41002&token=tok123"));
}

#[test]
fn pair_session_dispatches_remote_input_events_over_udp_tcp_and_ws_and_streams_deltas() {
    let (ctrl_tx, ctrl_rx) = unbounded();
    let (ready_tx, ready_rx) = bounded(1);
    let server_thread = std::thread::spawn(move || {
        let surface = DioxusSurface::new(1, InspectDemo);
        let mut session = PairSession::start(
            surface,
            "127.0.0.1",
            0,
            0,
            "cli-pair-e2e-token".into(),
            393.0,
            852.0,
        )
        .expect("start PairSession");
        ready_tx
            .send((
                session.udp_addr(),
                session.ws_addr(),
                session.token().to_owned(),
            ))
            .unwrap();
        session.run_until(120, &AtomicBool::new(false), Some(&ctrl_rx));
    });

    let (udp_addr, ws_addr, token) = ready_rx.recv().unwrap();
    let cfg = PairingConfig::new(token);
    let mut udp = UdpDeltaClient::connect(udp_addr, cfg.clone()).unwrap();
    let mut tcp = TcpDeltaClient::connect(ws_addr, cfg.clone()).unwrap();
    let mut ws = TcpDeltaClient::connect_websocket(ws_addr, "/ws/nvd", cfg).unwrap();

    let timeout = Duration::from_secs(2);
    let f0 = udp.recv_frame(timeout).unwrap().unwrap().1;
    assert_eq!(f0, tcp.recv_frame(timeout).unwrap().unwrap().1);
    assert_eq!(f0, ws.recv_frame(timeout).unwrap().unwrap().1);

    let mut headless = HeadlessBackend::new();
    headless.set_viewport(
        1,
        Viewport {
            width: 393.0,
            height: 852.0,
            scale_factor: 1.0,
        },
    );
    headless.submit_frame(&f0).unwrap();
    assert!(headless.render_ascii_tree().contains("Counter: 0"));
    let btn_node = headless
        .hit_test(60.0, 75.0, true)
        .expect("pressable button");

    let make_tap = |ts: u64| {
        encode_events(&[NativeEvent {
            surface_id: 1,
            node_id: btn_node,
            timestamp_us: ts,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 60.0, y: 75.0 },
        }])
    };

    // 1. UDP tap -> Counter: 1 across UDP, TCP, and WS.
    udp.send_events(&make_tap(1000)).unwrap();
    let f1 = udp.recv_frame(timeout).unwrap().unwrap().1;
    assert_eq!(f1, tcp.recv_frame(timeout).unwrap().unwrap().1);
    assert_eq!(f1, ws.recv_frame(timeout).unwrap().unwrap().1);
    headless.submit_frame(&f1).unwrap();
    assert!(headless.render_ascii_tree().contains("Counter: 1"));

    // 2. RFC-6455 WebSocket tap -> Counter: 2 across UDP, TCP, and WS.
    ws.send_events(&make_tap(2000)).unwrap();
    let f2 = ws.recv_frame(timeout).unwrap().unwrap().1;
    assert_eq!(f2, udp.recv_frame(timeout).unwrap().unwrap().1);
    assert_eq!(f2, tcp.recv_frame(timeout).unwrap().unwrap().1);
    headless.submit_frame(&f2).unwrap();
    assert!(headless.render_ascii_tree().contains("Counter: 2"));

    // 3. Raw TCP tap -> Counter: 3 across UDP, TCP, and WS.
    tcp.send_events(&make_tap(3000)).unwrap();
    let f3 = tcp.recv_frame(timeout).unwrap().unwrap().1;
    assert_eq!(f3, udp.recv_frame(timeout).unwrap().unwrap().1);
    assert_eq!(f3, ws.recv_frame(timeout).unwrap().unwrap().1);
    headless.submit_frame(&f3).unwrap();
    assert!(headless.render_ascii_tree().contains("Counter: 3"));

    ctrl_tx.send(PairControlMsg::Shutdown).unwrap();
    udp.shutdown();
    tcp.shutdown();
    ws.shutdown();
    server_thread
        .join()
        .expect("pair server thread exited cleanly");
}

#[test]
fn pair_session_pushes_live_sync_telemetry_props_on_peer_connect_and_traffic() {
    #[derive(Debug, Clone, PartialEq)]
    struct RecordedTelemetry {
        transport: String,
        peers: i64,
        kbps: f64,
        compression: f64,
        rtt_ms: f64,
    }

    struct TelemetryMockApp {
        updates: Vec<RecordedTelemetry>,
        renders: u64,
    }

    impl PairedApp for TelemetryMockApp {
        fn render(&mut self, frame_id: u64) -> Cow<'_, [u8]> {
            self.renders += 1;
            let mut b = nve_abi::CommandBufferBuilder::new(1, frame_id);
            b.create_node(2, nve_abi::ComponentKind::Text, None);
            b.set_text(
                2,
                &format!(
                    "peers={}",
                    self.updates.last().map(|u| u.peers).unwrap_or(-1)
                ),
            );
            b.set_children(ROOT_NODE_ID, &[2]);
            b.commit(true);
            Cow::Owned(b.into_bytes())
        }

        fn dispatch_events(&mut self, _encoded_events: &[u8]) -> bool {
            false
        }

        fn set_props_direct(&mut self, entries: &[NveMapEntry]) -> bool {
            let mut transport = String::new();
            let mut peers = -1i64;
            let mut kbps = -1.0f64;
            let mut compression = -1.0f64;
            let mut rtt_ms = -1.0f64;
            for entry in entries {
                let key = unsafe { entry.key.as_str() }.expect("valid utf8 prop key");
                let val = unsafe { entry.value.as_ref() };
                match key {
                    "syncTransport" => {
                        if let Some(s) = val.as_str() {
                            transport = s.to_owned();
                        }
                    }
                    "syncPeers" => {
                        if let Some(n) = val.as_i64() {
                            peers = n;
                        }
                    }
                    "syncKbps" => {
                        if let Some(v) = val.as_number() {
                            kbps = v;
                        }
                    }
                    "syncCompression" => {
                        if let Some(v) = val.as_number() {
                            compression = v;
                        }
                    }
                    "syncRttMs" => {
                        if let Some(v) = val.as_number() {
                            rtt_ms = v;
                        }
                    }
                    _ => {}
                }
            }
            let rec = RecordedTelemetry {
                transport,
                peers,
                kbps,
                compression,
                rtt_ms,
            };
            if self.updates.last() == Some(&rec) {
                return false;
            }
            self.updates.push(rec);
            true
        }
    }

    let app = TelemetryMockApp {
        updates: Vec::new(),
        renders: 0,
    };
    let mut session = PairSession::start(
        app,
        "127.0.0.1",
        0,
        0,
        "telemetry-test-token".into(),
        640.0,
        480.0,
    )
    .expect("start telemetry PairSession");

    // Initial startup push reports offline with 0 peers.
    assert_eq!(session.app.updates.len(), 1);
    assert_eq!(session.app.updates[0].transport, "offline");
    assert_eq!(session.app.updates[0].peers, 0);
    assert!(session.app.updates[0].compression >= 1.0);

    let cfg = PairingConfig::new("telemetry-test-token");
    let mut ws = TcpDeltaClient::connect_websocket(session.ws_addr(), "/ws/nvd", cfg.clone())
        .expect("connect WS client");
    assert!(session.tcp.wait_for_peers(1, Duration::from_secs(2)));

    let step_ws = session.poll_once(None);
    assert!(
        step_ws.frames_broadcast >= 1,
        "telemetry update on peer connect should re-render and broadcast"
    );
    let after_ws = session.app.updates.last().unwrap().clone();
    assert_eq!(after_ws.transport, "relay-wss");
    assert_eq!(after_ws.peers, 1);

    let mut udp = UdpDeltaClient::connect(session.udp_addr(), cfg).expect("connect UDP client");
    assert!(session.udp.wait_for_peers(1, Duration::from_secs(2)));

    let step_udp = session.poll_once(None);
    assert!(step_udp.frames_broadcast >= 1);
    let after_udp = session.app.updates.last().unwrap().clone();
    assert_eq!(after_udp.transport, "lan-quic");
    assert_eq!(after_udp.peers, 2);
    assert!(after_udp.kbps >= 0.0);
    assert!(after_udp.rtt_ms >= 0.0);

    udp.shutdown();
    ws.shutdown();
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let _ = session.poll_once(None);
        if session.app.updates.last().is_some_and(|u| u.peers == 0) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let after_drop = session.app.updates.last().unwrap();
    assert_eq!(after_drop.peers, 0);
    assert_eq!(after_drop.transport, "offline");

    session.udp.shutdown();
    session.tcp.shutdown();
}
