use nve_abi::{encode_events, EventPayload, NativeEvent, NativeEventKind};
use nve_backend_headless::HeadlessBackend;
use nve_components::tables::{core_realization_order, register_core_tables, TextContent};
use nve_engine::{StoreBackend, StoreWireReplica, Viewport};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, NodeId as StoreNodeId, NodeStore};
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

    register_core_tables();
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(
        1,
        Viewport {
            width: 393.0,
            height: 852.0,
            scale_factor: 1.0,
        },
    );
    let mut replica = StoreWireReplica::new(headless, core_realization_order());
    replica.submit_nve2_frame(&f0).unwrap();
    assert!(replica
        .backend()
        .render_ascii_tree(replica.store())
        .contains("Counter: 0"));
    let btn_node = replica
        .backend()
        .hit_test(replica.store(), 60.0, 75.0)
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
    quic.send_events(make_tap(1000)).unwrap();
    let f1 = quic.recv_frame_timeout(timeout).unwrap().payload;
    assert_eq!(f1, wt.recv_frame_timeout(timeout).unwrap().payload);
    assert_eq!(f1, wss.recv_frame_timeout(timeout).unwrap().payload);
    replica.submit_nve2_frame(&f1).unwrap();
    assert!(replica
        .backend()
        .render_ascii_tree(replica.store())
        .contains("Counter: 1"));

    // 2. WebTransport tap -> Counter: 2 across all 3 transports.
    wt.send_events(make_tap(2000)).unwrap();
    let f2 = wt.recv_frame_timeout(timeout).unwrap().payload;
    assert_eq!(f2, quic.recv_frame_timeout(timeout).unwrap().payload);
    assert_eq!(f2, wss.recv_frame_timeout(timeout).unwrap().payload);
    replica.submit_nve2_frame(&f2).unwrap();
    assert!(replica
        .backend()
        .render_ascii_tree(replica.store())
        .contains("Counter: 2"));

    // 3. WSS tap -> Counter: 3 across all 3 transports.
    wss.send_events(make_tap(3000)).unwrap();
    let f3 = wss.recv_frame_timeout(timeout).unwrap().payload;
    assert_eq!(f3, quic.recv_frame_timeout(timeout).unwrap().payload);
    assert_eq!(f3, wt.recv_frame_timeout(timeout).unwrap().payload);
    replica.submit_nve2_frame(&f3).unwrap();
    assert!(replica
        .backend()
        .render_ascii_tree(replica.store())
        .contains("Counter: 3"));

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
        fn render(&mut self, _frame_id: u64) -> Cow<'_, [u8]> {
            register_core_tables();
            let mut store = NodeStore::new();
            let mut changes = ChangeSet::default();
            {
                let mut tx = store.tx();
                let _ = tx.create_at(StoreNodeId::ROOT);
                let _ = tx.create_at(StoreNodeId(2));
                tx.set_str(
                    StoreNodeId(2),
                    TextContent::TEXT,
                    &format!(
                        "peers={}",
                        self.updates.last().map(|u| u.peers).unwrap_or(-1)
                    ),
                );
                tx.set_children(StoreNodeId::ROOT, &[StoreNodeId(2)]);
                tx.commit_into(&mut changes);
            }
            let mut out = Vec::new();
            WireEncoder::default().encode(&store, &changes, &mut out);
            Cow::Owned(out)
        }

        fn dispatch_events(&mut self, _events: &[NveDirectEvent]) -> bool {
            false
        }

        fn set_props(&mut self, entries: &[NveMapEntry]) -> bool {
            let mut transport = String::new();
            let mut peers = -1i64;
            for entry in entries {
                // SAFETY: `entries` and their string/value buffers stay live for this synchronous callback.
                let (key, val) = unsafe { (entry.key.as_str().expect("valid utf8 prop key"), entry.value.as_ref()) };
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

#[test]
fn pair_session_answers_keyframe_request_after_deltas_with_fresh_full_snapshot_for_requesting_peer_only() {
    let (ctrl_tx, ctrl_rx) = unbounded();
    let (ready_tx, ready_rx) = bounded(1);
    let server_thread = std::thread::spawn(move || {
        let surface = DioxusSurface::new(1, InspectDemo);
        let mut session = PairSession::start(
            surface,
            "127.0.0.1",
            0,
            0,
            "cli-late-join-kf-token".into(),
            393.0,
            852.0,
        )
        .expect("start PairSession");
        ready_tx
            .send((session.ws_addr(), session.token().to_owned()))
            .unwrap();
        session.run_until(120, &AtomicBool::new(false), Some(&ctrl_rx));
    });

    let (ws_addr, token) = ready_rx.recv().unwrap();
    let uri = parse_wire_uri(&format!("ws://{ws_addr}/nve-wire")).unwrap();
    let cfg_a = NveWireClientConfig::new(token.clone(), "peer-a");
    let peer_a = NveWireClient::connect_uri_sync(&uri, &cfg_a).expect("connect peer A");

    let timeout = Duration::from_secs(2);
    let f0 = peer_a.recv_frame_timeout(timeout).expect("peer A initial frame");
    assert_eq!(f0.kind, nve_wire::NveWireKind::SceneKeyframe);

    register_core_tables();
    let make_replica = || {
        let mut headless = HeadlessBackend::new();
        headless.set_viewport(
            1,
            Viewport {
                width: 393.0,
                height: 852.0,
                scale_factor: 1.0,
            },
        );
        StoreWireReplica::new(headless, core_realization_order())
    };

    let mut rep_a = make_replica();
    rep_a.submit_nve2_keyframe(&f0.payload).unwrap();
    assert!(rep_a
        .backend()
        .render_ascii_tree(rep_a.store())
        .contains("Counter: 0"));
    let btn_node = rep_a
        .backend()
        .hit_test(rep_a.store(), 60.0, 75.0)
        .expect("pressable button");

    let tap = |ts: u64| {
        encode_events(&[NativeEvent {
            surface_id: 1,
            node_id: btn_node,
            timestamp_us: ts,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 60.0, y: 75.0 },
        }])
    };

    // Advance scene via incremental CommandDelta -> invalidates server's stale Frame-1 keyframe!
    peer_a.send_events(tap(1000)).unwrap();
    let f1 = peer_a.recv_frame_timeout(timeout).expect("peer A delta 1");
    assert_eq!(f1.kind, nve_wire::NveWireKind::CommandDelta);
    rep_a.submit_nve2_frame(&f1.payload).unwrap();
    assert!(rep_a
        .backend()
        .render_ascii_tree(rep_a.store())
        .contains("Counter: 1"));

    // Late-joining Peer B connects AFTER the delta: server emits synthetic KeyframeRequest and
    // PairSession answers ONLY Peer B with a fresh full snapshot containing Counter: 1!
    let cfg_b = NveWireClientConfig::new(token, "peer-b");
    let peer_b = NveWireClient::connect_uri_sync(&uri, &cfg_b).expect("connect peer B");
    let kf_b = peer_b
        .recv_frame_timeout(timeout)
        .expect("peer B receives fresh full SceneKeyframe");
    assert_eq!(kf_b.kind, nve_wire::NveWireKind::SceneKeyframe);

    let mut rep_b = make_replica();
    rep_b.submit_nve2_keyframe(&kf_b.payload).unwrap();
    let tree_b = rep_b.backend().render_ascii_tree(rep_b.store());
    assert!(
        tree_b.contains("Counter: 1") && !tree_b.contains("Counter: 0"),
        "late-joining peer B must receive fresh Counter: 1 snapshot, not stale Frame-1 rollback: {tree_b}"
    );

    // Healthy Peer A MUST NOT receive any unsolicited SceneKeyframe reset!
    assert!(
        peer_a.recv_frame_timeout(Duration::from_millis(150)).is_none(),
        "peer B's targeted keyframe must not disrupt healthy peer A"
    );

    // Next tap advances BOTH Peer A and Peer B cleanly to Counter: 2.
    peer_b.send_events(tap(2000)).unwrap();
    let d_a = peer_a.recv_frame_timeout(timeout).expect("peer A delta 2");
    let d_b = peer_b.recv_frame_timeout(timeout).expect("peer B delta 2");
    assert_eq!(d_a.kind, nve_wire::NveWireKind::CommandDelta);
    assert_eq!(d_b.kind, nve_wire::NveWireKind::CommandDelta);
    rep_a.submit_nve2_frame(&d_a.payload).unwrap();
    rep_b.submit_nve2_frame(&d_b.payload).unwrap();
    assert!(rep_a
        .backend()
        .render_ascii_tree(rep_a.store())
        .contains("Counter: 2"));
    assert!(rep_b
        .backend()
        .render_ascii_tree(rep_b.store())
        .contains("Counter: 2"));

    ctrl_tx.send(PairControlMsg::Shutdown).unwrap();
    peer_a.close();
    peer_b.close();
    server_thread.join().expect("pair server thread exited cleanly");
}
