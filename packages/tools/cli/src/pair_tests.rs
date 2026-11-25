use nve_backend_headless::HeadlessBackend;
use nve_engine::{RenderBackend, Viewport};
use nve_wire::{
    format_cert_hash_hex, parse_wire_uri, NveWireClient, NveWireClientConfig, NveWireTransportKind,
};

use super::*;
use crate::doctor::InspectDemo;

#[test]
fn pair_token_generation_and_uri_formatting() {
    let (t1, t2) = (generate_pairing_token(), generate_pairing_token());
    assert!(t1.starts_with("nve_") && t1 != t2);
    assert_eq!(
        resolve_or_generate_token(Some("  custom-tok  ".into())),
        "custom-tok"
    );
    assert!(resolve_or_generate_token(Some("   ".into())).starts_with("nve_"));
    assert!(resolve_or_generate_token(None).starts_with("nve_"));

    let fake_hash = [0xABu8; 32];
    let fake_hex = format_cert_hash_hex(&fake_hash);
    let uri = format_pairing_uri("0.0.0.0", 8091, 8092, "tok123", Some(&fake_hash), 393.0, 852.0);
    assert_eq!(
        uri,
        format!("nve://pair?host=127.0.0.1&quic=8091&wss=8092&token=tok123&w=393&h=852&cert_hash={fake_hex}")
    );
    assert_eq!(
        format_browser_pair_query("0.0.0.0", 8092, "tok123", Some(&fake_hex)),
        format!("?pair=ws://127.0.0.1:8092&token=tok123&cert_hash={fake_hex}")
    );
    assert_eq!(format_bind_addr("127.0.0.1", 0), "127.0.0.1:0");
    assert_eq!(format_bind_addr("::1", 0), "[::1]:0");

    let quic_addr: SocketAddr = "127.0.0.1:41001".parse().unwrap();
    let wss_addr: SocketAddr = "127.0.0.1:41002".parse().unwrap();
    let ready = format_pair_ready_line("0.0.0.0", quic_addr, wss_addr, "tok123", &fake_hex);
    assert_eq!(
        ready,
        format!("NVE_PAIR_READY ws=ws://127.0.0.1:41002 token=tok123 udp=127.0.0.1:41001 tcp=127.0.0.1:41002 cert_hash={fake_hex}")
    );
    let banner = format_pair_banner(
        Path::new("target/debug/libnve_example_kitchen_sink.dylib"),
        "0.0.0.0",
        quic_addr,
        wss_addr,
        "tok123",
        &fake_hash,
        &fake_hex,
        393.0,
        852.0,
    );
    assert!(banner.contains("nve://pair?host=127.0.0.1&quic=41001&wss=41002&token=tok123"));
    assert!(banner.contains(&fake_hex));
}

#[test]
fn pair_session_dispatches_remote_events_over_quic_webtransport_and_wss() {
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
                session.cert_sha256(),
                session.token().to_owned(),
            ))
            .unwrap();
        session.run_until(120, &AtomicBool::new(false), Some(&ctrl_rx));
    });

    let (quic_addr, wss_addr, cert_hash, token) = ready_rx.recv().unwrap();
    let cfg = NveWireClientConfig::new(token.clone(), "cli-test").with_cert_sha256(cert_hash);

    let quic_uri = parse_wire_uri(&format!("quic://{quic_addr}")).unwrap();
    let wt_uri = parse_wire_uri(&format!("https://{quic_addr}/nve-wire")).unwrap();
    let wss_uri = parse_wire_uri(&format!("wss://{wss_addr}/nve-wire")).unwrap();

    let quic = NveWireClient::connect_uri_sync(&quic_uri, &cfg).expect("QUIC connect");
    let wt = NveWireClient::connect_uri_sync(&wt_uri, &cfg).expect("WebTransport connect");
    let wss = NveWireClient::connect_uri_sync(&wss_uri, &cfg).expect("WSS connect");
    assert_eq!(quic.transport_kind(), NveWireTransportKind::Quic);
    assert_eq!(wt.transport_kind(), NveWireTransportKind::WebTransport);
    assert_eq!(wss.transport_kind(), NveWireTransportKind::Wss);

    let timeout = Duration::from_secs(2);
    let f0 = quic.recv_frame_timeout(timeout).unwrap().payload;
    assert_eq!(f0, wt.recv_frame_timeout(timeout).unwrap().payload);
    assert_eq!(f0, wss.recv_frame_timeout(timeout).unwrap().payload);

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

    // 1. QUIC tap -> Counter: 1 across QUIC, WebTransport, and WSS.
    quic.send_events(&make_tap(1000)).unwrap();
    let f1 = quic.recv_frame_timeout(timeout).unwrap().payload;
    assert_eq!(f1, wt.recv_frame_timeout(timeout).unwrap().payload);
    assert_eq!(f1, wss.recv_frame_timeout(timeout).unwrap().payload);
    headless.submit_frame(&f1).unwrap();
    assert!(headless.render_ascii_tree().contains("Counter: 1"));

    // 2. WebTransport tap -> Counter: 2 across all 3 transports.
    wt.send_events(&make_tap(2000)).unwrap();
    let f2 = wt.recv_frame_timeout(timeout).unwrap().payload;
    assert_eq!(f2, quic.recv_frame_timeout(timeout).unwrap().payload);
    assert_eq!(f2, wss.recv_frame_timeout(timeout).unwrap().payload);
    headless.submit_frame(&f2).unwrap();
    assert!(headless.render_ascii_tree().contains("Counter: 2"));

    // 3. WSS tap -> Counter: 3 across all 3 transports.
    wss.send_events(&make_tap(3000)).unwrap();
    let f3 = wss.recv_frame_timeout(timeout).unwrap().payload;
    assert_eq!(f3, quic.recv_frame_timeout(timeout).unwrap().payload);
    assert_eq!(f3, wt.recv_frame_timeout(timeout).unwrap().payload);
    headless.submit_frame(&f3).unwrap();
    assert!(headless.render_ascii_tree().contains("Counter: 3"));

    ctrl_tx.send(PairControlMsg::Shutdown).unwrap();
    quic.close();
    wt.close();
    wss.close();
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
    }

    struct TelemetryMockApp {
        updates: Vec<RecordedTelemetry>,
    }

    impl PairedApp for TelemetryMockApp {
        fn render(&mut self, frame_id: u64) -> Cow<'_, [u8]> {
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
                    _ => {}
                }
            }
            let rec = RecordedTelemetry { transport, peers };
            if self.updates.last() == Some(&rec) {
                return false;
            }
            self.updates.push(rec);
            true
        }
    }

    let app = TelemetryMockApp {
        updates: Vec::new(),
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

    assert_eq!(session.app.updates.first().unwrap().transport, "offline");
    assert_eq!(session.app.updates.first().unwrap().peers, 0);

    let cfg = NveWireClientConfig::new("telemetry-test-token", "tel-quic")
        .with_cert_sha256(session.cert_sha256());
    let uri = parse_wire_uri(&format!("quic://{}", session.udp_addr())).unwrap();
    let client = NveWireClient::connect_uri_sync(&uri, &cfg).expect("connect QUIC peer");

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        session.poll_once(None);
        if session
            .app
            .updates
            .iter()
            .any(|u| u.transport == "lan-quic" && u.peers == 1)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(session
        .app
        .updates
        .iter()
        .any(|u| u.transport == "lan-quic" && u.peers == 1));
    client.close();
}
