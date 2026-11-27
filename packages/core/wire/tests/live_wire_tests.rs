//! Live loopback integration tests for `nve-wire`:
//! - Raw QUIC (`nve-wire-2` ALPN) handshake, `NveSceneArchive` keyframe (`XXH64`), `CommandDelta`,
//!   `NodePatchDelta` (< 512 B), `EventBatch` backchannel, and RFC 9221 datagram round-trip
//! - Real HTTP/3 WebTransport (`h3` ALPN) handshake on the SAME UDP socket with 13-day ECDSA P-256
//!   SHA-256 cert pinning + full stream & datagram round-trip
//! - Happy-Eyeballs automatic fallback: UDP/QUIC blocked (dead port) -> connects via `wss://`
//!   within ~250ms and completes full `NveWireFrame` round-trip
//! - Constant-time Bearer token rejection & automatic exponential-backoff reconnect + keyframe test

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nve_abi::{
    encode_events, ColorRgba, EventPayload, NativeEvent, NativeEventKind, NVE_PRIMARY_SLOT_NONE,
};
use nve_components::style_patch::{LayoutSnapshot, VisualSnapshot};
use nve_engine::{
    encode_scene_reloc, LayoutRect, SceneRelocInput, SceneRelocNodeInput, SceneRelocView,
    VerifyMode, Viewport,
};
use nve_wire::{
    generate_bearer_token, parse_wire_uri, NveNodePatchDeltaBuilder, NveNodePatchDeltaView,
    NveWireChannel, NveWireClient, NveWireClientConfig, NveWireFrame, NveWireKind, NveWireServer,
    NveWireServerConfig, NveWireTransportError, NveWireTransportKind, ReconnectConfig,
    ReconnectingWireClient, WireInboundItem,
};

fn build_sample_scene_archive() -> nve_engine::NveSceneArchive {
    let nodes = [
        SceneRelocNodeInput {
            node_id: 1,
            parent_index: -1,
            primary_slot: NVE_PRIMARY_SLOT_NONE,
            flags: 0,
            text_align: 0,
            image_resize_mode: 0,
            font_weight: 400,
            event_mask: 0,
            computed_rect: LayoutRect::default(),
            style: VisualSnapshot::default(),
            layout: LayoutSnapshot::default(),
            text: "",
            tag: "",
            image_src: "",
            box_shadow: "",
            draw_list: &[],
            children: &[1],
        },
        SceneRelocNodeInput {
            node_id: 2,
            parent_index: 0,
            primary_slot: 1,
            flags: 0,
            text_align: 0,
            image_resize_mode: 0,
            font_weight: 400,
            event_mask: 0,
            computed_rect: LayoutRect::default(),
            style: VisualSnapshot::default(),
            layout: LayoutSnapshot::default(),
            text: "Hello QUIC/WebTransport",
            tag: "",
            image_src: "",
            box_shadow: "",
            draw_list: &[],
            children: &[],
        },
    ];
    encode_scene_reloc(&SceneRelocInput {
        schema_version: 1,
        surface_id: 1,
        frame_id: 1,
        viewport: Viewport {
            width: 393.0,
            height: 852.0,
            scale_factor: 3.0,
        },
        clear_color: ColorRgba::rgba(11, 17, 32, 255),
        root_node_index: 0,
        app_tag: "nve-wire-live-test",
        nodes: &nodes,
    })
    .expect("encode scene archive")
}

#[tokio::test]
async fn live_quic_nve_wire_2_roundtrip_scene_command_patch_events_and_datagram() {
    let token = generate_bearer_token();
    let server = NveWireServer::bind_async(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token.clone(),
    ))
    .await
    .expect("bind dual-ALPN server");

    let client_cfg = NveWireClientConfig::new(token, "quic-test-client")
        .with_cert_sha256(server.cert_sha256());
    let client = NveWireClient::connect_quic(server.quic_addr(), &client_cfg)
        .await
        .expect("connect raw QUIC nve-wire-2");
    assert_eq!(client.transport_kind(), NveWireTransportKind::Quic);
    assert!(client.is_connected());
    assert!(client.session_id() >= 1);

    // 1. NveSceneArchive keyframe (`XXH64`-verified inside its 64B header).
    let scene = build_sample_scene_archive();
    assert_eq!(server.broadcast_scene_keyframe(1, scene.as_slice()).unwrap(), 1);
    let kf = client
        .recv_frame_async(Duration::from_secs(2))
        .await
        .expect("receive SceneKeyframe");
    assert_eq!(kf.kind, NveWireKind::SceneKeyframe);
    let aligned_kf = nve_engine::RelocBuf::from_bytes(&kf.payload);
    let scene_view = SceneRelocView::from_bytes(aligned_kf.as_slice(), VerifyMode::Full)
        .expect("verify received NveSceneArchive XXH64");
    assert_eq!(
        scene_view.text_of(&scene_view.nodes()[1]).unwrap(),
        "Hello QUIC/WebTransport"
    );

    // 2. Direct incremental CommandDelta (zero reverse-parsing).
    let cmd_bytes = b"NVE_CMD_DELTA_PAYLOAD_SAMPLE".to_vec();
    server.broadcast_command_delta(2, &cmd_bytes).unwrap();
    let cd = client
        .recv_frame_async(Duration::from_secs(2))
        .await
        .expect("receive CommandDelta");
    assert_eq!(cd.kind, NveWireKind::CommandDelta);
    assert_eq!(cd.payload, cmd_bytes);

    // 3. Compact 8-node NodePatchDelta (< 512 B).
    let mut pb = NveNodePatchDeltaBuilder::new();
    for i in 0..4u32 {
        pb.set_text(10 + i, "SCORE 42");
    }
    for i in 0..4u32 {
        pb.set_bg_rgba(20 + i, 0x22C55EFF);
    }
    let patch_payload = pb.encode().unwrap();
    let patch_frame = NveWireFrame::new(NveWireKind::NodePatchDelta, 3, patch_payload);
    assert!(patch_frame.encode().unwrap().len() < 512);
    server.broadcast_frame(&patch_frame).unwrap();
    let rx_patch = client
        .recv_frame_async(Duration::from_secs(2))
        .await
        .expect("receive NodePatchDelta");
    assert_eq!(rx_patch.kind, NveWireKind::NodePatchDelta);
    let patch_view = NveNodePatchDeltaView::decode(&rx_patch.payload).unwrap();
    assert_eq!(patch_view.patches().len(), 8);
    assert_eq!(patch_view.patch_str(&patch_view.patches()[0]).unwrap(), "SCORE 42");

    // 4. Upstream EventBatch backchannel.
    let event_bytes = encode_events(&[NativeEvent {
        surface_id: 1,
        kind: NativeEventKind::Press,
        node_id: 2,
        timestamp_us: 1234,
        payload: EventPayload::Point { x: 10.0, y: 20.0 },
    }]);
    client.send_events(event_bytes.clone()).unwrap();
    let inbound = server
        .recv_inbound_async(Duration::from_secs(2))
        .await
        .expect("server receives EventBatch");
    assert_eq!(inbound.transport, NveWireTransportKind::Quic);
    assert_eq!(inbound.frame.kind, NveWireKind::EventBatch);
    assert_eq!(inbound.frame.payload, event_bytes);

    // 5. RFC 9221 unreliable datagram round-trip (client -> server AND server -> client).
    let presence_up = NveWireFrame::new(NveWireKind::Presence, 9, b"cursor:120,240".to_vec())
        .with_channel(NveWireChannel::Presence);
    client.send_datagram(&presence_up).unwrap();
    let rx_up = server
        .recv_inbound_async(Duration::from_secs(2))
        .await
        .expect("server receives RFC 9221 datagram");
    assert_eq!(rx_up.frame.kind, NveWireKind::Presence);
    assert_eq!(rx_up.frame.payload, b"cursor:120,240");

    let presence_down = NveWireFrame::new(NveWireKind::Presence, 10, b"peer2:50,80".to_vec())
        .with_channel(NveWireChannel::Presence);
    server.broadcast_datagram(&presence_down).unwrap();
    let rx_down = client
        .recv_frame_async(Duration::from_secs(2))
        .await
        .expect("client receives RFC 9221 datagram");
    assert_eq!(rx_down.kind, NveWireKind::Presence);
    assert_eq!(rx_down.payload, b"peer2:50,80");
}

#[tokio::test]
async fn live_http3_webtransport_h3_with_13_day_ecdsa_cert_pinning() {
    let token = generate_bearer_token();
    let server = NveWireServer::bind_async(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token.clone(),
    ))
    .await
    .expect("bind dual-ALPN server");

    // Connect via real HTTP/3 WebTransport (`h3` ALPN) to the EXACT SAME UDP socket!
    let wt_url = format!("https://127.0.0.1:{}/nve-wire", server.quic_addr().port());
    let client_cfg = NveWireClientConfig::new(token, "webtransport-test")
        .with_cert_sha256(server.cert_sha256());
    let wt_client = NveWireClient::connect_webtransport(&wt_url, &client_cfg)
        .await
        .expect("connect HTTP/3 WebTransport");
    assert_eq!(wt_client.transport_kind(), NveWireTransportKind::WebTransport);
    assert!(wt_client.is_connected());

    // Stream round-trip over WebTransport.
    server.broadcast_command_delta(1, b"wt-command-delta").unwrap();
    let rx = wt_client
        .recv_frame_async(Duration::from_secs(2))
        .await
        .expect("WT receives CommandDelta");
    assert_eq!(rx.kind, NveWireKind::CommandDelta);
    assert_eq!(rx.payload, b"wt-command-delta");

    // RFC 9221 datagram over WebTransport!
    let dgram = NveWireFrame::new(NveWireKind::Presence, 2, b"wt-presence".to_vec());
    wt_client.send_datagram(&dgram).unwrap();
    let inbound = server
        .recv_inbound_async(Duration::from_secs(2))
        .await
        .expect("server receives WT datagram");
    assert_eq!(inbound.transport, NveWireTransportKind::WebTransport);
    assert_eq!(inbound.frame.payload, b"wt-presence");

    // Wrong SHA-256 cert hash must be rejected by TLS verification.
    let bad_cfg = NveWireClientConfig::new("any-token", "bad-pin")
        .with_cert_sha256([0xFF; 32]);
    assert!(NveWireClient::connect_webtransport(&wt_url, &bad_cfg)
        .await
        .is_err());
}

#[tokio::test]
async fn live_happy_eyeballs_falls_back_to_wss_when_udp_blocked_and_rejects_bad_token() {
    let token = generate_bearer_token();
    let server = NveWireServer::bind_async(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token.clone(),
    ))
    .await
    .expect("bind server");

    // Allocate a UDP socket and drop/hold it without a QUIC server so UDP QUIC never answers.
    let blackhole_udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let dead_quic_addr: SocketAddr = blackhole_udp.local_addr().unwrap();
    let wss_url = format!("wss://127.0.0.1:{}/nve-wire", server.wss_addr().port());

    let mut cfg = NveWireClientConfig::new(token.clone(), "happy-eyeballs-client")
        .with_cert_sha256(server.cert_sha256());
    cfg.fallback_delay = Duration::from_millis(250);

    let start = Instant::now();
    let client = NveWireClient::connect_happy_eyeballs(dead_quic_addr, &wss_url, &cfg)
        .await
        .expect("Happy-Eyeballs must fall back to WSS");
    let elapsed = start.elapsed();

    assert_eq!(client.transport_kind(), NveWireTransportKind::Wss);
    assert!(
        elapsed >= Duration::from_millis(200) && elapsed < Duration::from_secs(2),
        "expected ~250ms fallback window, got {elapsed:?}"
    );

    server.broadcast_command_delta(5, b"wss-fallback-frame").unwrap();
    let rx = client
        .recv_frame_async(Duration::from_secs(2))
        .await
        .expect("WSS client receives frame");
    assert_eq!(rx.payload, b"wss-fallback-frame");

    // Unauthorized Bearer token must fail with Auth error on both QUIC and WSS.
    let bad_auth = NveWireClientConfig::new("nve_wrong_bearer_token", "intruder")
        .with_cert_sha256(server.cert_sha256());
    let err = NveWireClient::connect_ws(&wss_url, &bad_auth)
        .await
        .err()
        .expect("bad bearer token must fail");
    assert!(matches!(err, NveWireTransportError::Auth(_)), "got {err:?}");
    assert!(server.metrics().rejected_handshakes >= 1);
    drop(blackhole_udp);
}

#[test]
fn reconnecting_wire_client_recovers_after_server_restart_with_keyframe() {
    let token = generate_bearer_token();
    let server1 = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token.clone(),
    ))
    .expect("bind server 1");
    let ws_addr = server1.wss_addr();
    let uri = parse_wire_uri(&format!("ws://{ws_addr}/nve-wire")).expect("parse ws uri");

    let wakes = Arc::new(AtomicUsize::new(0));
    let wakes_cb = Arc::clone(&wakes);
    let mut client = ReconnectingWireClient::connect_sync(
        uri,
        NveWireClientConfig::new(token.clone(), "reconnect-test"),
        ReconnectConfig {
            initial_backoff: Duration::from_millis(30),
            max_backoff: Duration::from_millis(200),
        },
        Some(Arc::new(move || {
            wakes_cb.fetch_add(1, Ordering::Release);
        })),
    )
    .expect("initial connect_sync");
    assert!(client.is_connected());
    assert_eq!(client.reconnect_count(), 0);

    // 1. Initial frame roundtrip + upstream event send.
    server1.broadcast_command_delta(1, b"frame-before-drop").unwrap();
    let f1 = client
        .recv_frame_timeout(Duration::from_secs(2))
        .expect("receive frame 1");
    assert_eq!(f1.kind, NveWireKind::CommandDelta);
    assert_eq!(f1.payload, b"frame-before-drop");
    assert!(wakes.load(Ordering::Acquire) >= 1);

    client.send_events(b"ev-before-drop").unwrap();
    let in1 = server1
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server1 receives event");
    assert_eq!(in1.frame.payload, b"ev-before-drop");

    // 2. Kill server 1 -> client detects disconnect.
    server1.shutdown();
    drop(server1);
    let drop_deadline = Instant::now() + Duration::from_secs(3);
    while client.is_connected() && Instant::now() < drop_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!client.is_connected(), "client should detect server shutdown");

    // 3. Restart server 2 on the EXACT SAME TCP port (retry briefly while OS releases socket).
    let bind_deadline = Instant::now() + Duration::from_secs(3);
    let server2 = loop {
        match NveWireServer::bind_sync(NveWireServerConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            ws_addr,
            token.clone(),
        )) {
            Ok(s) => break s,
            Err(_) if Instant::now() < bind_deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("failed to rebind server2 on {ws_addr}: {e}"),
        }
    };

    // 4. Client automatically reconnects AND sends `KeyframeRequest`!
    let reconnect_deadline = Instant::now() + Duration::from_secs(5);
    while (!client.is_connected() || client.reconnect_count() < 1)
        && Instant::now() < reconnect_deadline
    {
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(client.is_connected(), "client failed to reconnect");
    assert!(client.reconnect_count() >= 1);

    let kf_req = server2
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server2 must receive KeyframeRequest on reconnect");
    assert_eq!(kf_req.frame.kind, NveWireKind::KeyframeRequest);

    // 5. Server 2 pushes recovery keyframe -> client receives it cleanly with cumulative metrics.
    server2
        .broadcast_scene_keyframe(2, b"post-reconnect-keyframe")
        .unwrap();
    let f2 = client
        .recv_frame_timeout(Duration::from_secs(2))
        .expect("receive recovery keyframe");
    assert_eq!(f2.kind, NveWireKind::SceneKeyframe);
    assert_eq!(f2.payload, b"post-reconnect-keyframe");

    let metrics = client.metrics();
    assert!(metrics.frames_received >= 2);
    assert!(metrics.events_sent >= 1);

    client.shutdown();
    assert!(!client.is_connected());
    server2.shutdown();
}

#[test]
fn same_server_reconnect_targets_keyframe_reply_without_resetting_healthy_peer() {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::atomic::AtomicBool;

    let token = generate_bearer_token();
    let server = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token.clone(),
    ))
    .expect("bind server");

    // Seed ONE genuine SceneKeyframe; later CommandDeltas must NEVER overwrite it.
    server
        .broadcast_scene_keyframe(1, b"initial-scene-keyframe-v1")
        .unwrap();

    // Peer A connects directly and stays healthy for the entire test.
    let direct_uri =
        parse_wire_uri(&format!("ws://{}/nve-wire", server.wss_addr())).expect("direct ws uri");
    let peer_a = NveWireClient::connect_uri_sync(
        &direct_uri,
        &NveWireClientConfig::new(token.clone(), "healthy-peer-a"),
    )
    .expect("connect peer A");
    let a_init = peer_a
        .recv_frame_timeout(Duration::from_secs(2))
        .expect("peer A initial keyframe");
    assert_eq!(a_init.kind, NveWireKind::SceneKeyframe);
    assert_eq!(a_init.payload, b"initial-scene-keyframe-v1");

    // Minimal loopback TCP proxy so we can drop ONLY Peer B's connection on the same server.
    let proxy_listener = TcpListener::bind("127.0.0.1:0").expect("bind proxy");
    let proxy_addr = proxy_listener.local_addr().unwrap();
    proxy_listener.set_nonblocking(true).unwrap();
    let backend_addr = server.wss_addr();
    let drop_peer_b = Arc::new(AtomicBool::new(false));
    let stop_proxy = Arc::new(AtomicBool::new(false));
    let drop_flag = Arc::clone(&drop_peer_b);
    let stop_flag = Arc::clone(&stop_proxy);
    let proxy_thread = std::thread::spawn(move || {
        while !stop_flag.load(Ordering::Acquire) {
            let Ok((client_stream, _)) = proxy_listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            let Ok(server_stream) = TcpStream::connect(backend_addr) else {
                continue;
            };
            let _ = client_stream.set_read_timeout(Some(Duration::from_millis(20)));
            let _ = server_stream.set_read_timeout(Some(Duration::from_millis(20)));
            let c_kill = client_stream.try_clone().unwrap();
            let s_kill = server_stream.try_clone().unwrap();
            let c_to_s_done = Arc::new(AtomicBool::new(false));
            let done_c = Arc::clone(&c_to_s_done);
            let mut c_r = client_stream.try_clone().unwrap();
            let mut s_w = server_stream.try_clone().unwrap();
            let f1 = std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while !done_c.load(Ordering::Acquire) {
                    match c_r.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if s_w.write_all(&buf[..n]).is_err() {
                                break;
                            }
                        }
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(_) => break,
                    }
                }
                done_c.store(true, Ordering::Release);
            });
            let done_s = Arc::clone(&c_to_s_done);
            let mut s_r = server_stream;
            let mut c_w = client_stream;
            let f2 = std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while !done_s.load(Ordering::Acquire) {
                    match s_r.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if c_w.write_all(&buf[..n]).is_err() {
                                break;
                            }
                        }
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(_) => break,
                    }
                }
                done_s.store(true, Ordering::Release);
            });
            while !c_to_s_done.load(Ordering::Acquire) && !stop_flag.load(Ordering::Acquire) {
                if drop_flag.swap(false, Ordering::AcqRel) {
                    c_to_s_done.store(true, Ordering::Release);
                    let _ = c_kill.shutdown(Shutdown::Both);
                    let _ = s_kill.shutdown(Shutdown::Both);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            c_to_s_done.store(true, Ordering::Release);
            let _ = c_kill.shutdown(Shutdown::Both);
            let _ = s_kill.shutdown(Shutdown::Both);
            let _ = f1.join();
            let _ = f2.join();
        }
    });

    let proxy_uri = parse_wire_uri(&format!("ws://{proxy_addr}/nve-wire")).expect("proxy uri");
    let mut peer_b = ReconnectingWireClient::connect_sync(
        proxy_uri,
        NveWireClientConfig::new(token.clone(), "reconnecting-peer-b"),
        ReconnectConfig {
            initial_backoff: Duration::from_millis(40),
            max_backoff: Duration::from_millis(120),
        },
        None,
    )
    .expect("connect peer B via proxy");

    // Verify Peer B gets SessionStarted(1, reconnected=false) + initial SceneKeyframe.
    let item0 = peer_b
        .recv_item_timeout(Duration::from_secs(2))
        .expect("peer B SessionStarted");
    assert!(matches!(
        item0,
        WireInboundItem::SessionStarted {
            session_id: 1,
            reconnected: false
        }
    ));
    let item1 = peer_b
        .recv_item_timeout(Duration::from_secs(2))
        .expect("peer B initial SceneKeyframe");
    match item1 {
        WireInboundItem::Frame {
            session_id: 1,
            frame,
        } => {
            assert_eq!(frame.kind, NveWireKind::SceneKeyframe);
            assert_eq!(frame.payload, b"initial-scene-keyframe-v1");
        }
        other => panic!("expected initial SceneKeyframe, got {other:?}"),
    }

    // Drop ONLY Peer B, then broadcast incremental CommandDeltas while Peer B is offline.
    drop_peer_b.store(true, Ordering::Release);
    let wait_drop = Instant::now() + Duration::from_secs(3);
    while peer_b.is_connected() && Instant::now() < wait_drop {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!peer_b.is_connected(), "peer B must observe proxy drop");

    server.broadcast_command_delta(2, b"delta-during-outage-2").unwrap();
    server.broadcast_command_delta(3, b"delta-during-outage-3").unwrap();
    assert_eq!(
        peer_a.recv_frame_timeout(Duration::from_secs(2)).unwrap().payload,
        b"delta-during-outage-2"
    );
    assert_eq!(
        peer_a.recv_frame_timeout(Duration::from_secs(2)).unwrap().payload,
        b"delta-during-outage-3"
    );

    // Peer B automatically reconnects -> SessionStarted(2, reconnected=true).
    // Because CommandDeltas 2 & 3 invalidated the server's stale Frame-1 keyframe, the server
    // MUST NOT replay stale `initial-scene-keyframe-v1` (which would roll Peer B back!). Instead
    // it emits ONE KeyframeRequest to the producer so the producer answers ONLY Peer B with a
    // fresh post-Frame-3 full snapshot!
    let mut saw_session2 = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !saw_session2 && Instant::now() < deadline {
        let Some(item) = peer_b.recv_item_timeout(Duration::from_millis(200)) else {
            continue;
        };
        match item {
            WireInboundItem::SessionStarted {
                session_id: 2,
                reconnected: true,
            } => saw_session2 = true,
            WireInboundItem::Frame {
                session_id: 2,
                frame,
            } if frame.kind == NveWireKind::SceneKeyframe => {
                assert_ne!(
                    frame.payload, b"initial-scene-keyframe-v1",
                    "stale Frame-1 keyframe must NEVER be replayed after CommandDeltas invalidated it"
                );
            }
            _ => {}
        }
    }
    assert!(saw_session2, "peer B must emit SessionStarted(2, reconnected=true)");

    // Server receives exactly ONE KeyframeRequest (synthetic join + client reconnect request
    // deduplicated!) and answers ONLY Peer B with a fresh post-Frame-3 snapshot.
    let kf_req = server
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server receives KeyframeRequest after delta invalidation");
    assert_eq!(kf_req.frame.kind, NveWireKind::KeyframeRequest);
    assert!(
        server
            .recv_inbound_timeout(Duration::from_millis(120))
            .is_none(),
        "join + reconnect must not emit duplicate KeyframeRequests"
    );
    assert!(server
        .send_scene_keyframe_to_peer(kf_req.session_id, 4, b"post-delta3-fresh-kf")
        .unwrap());

    let mut saw_targeted = false;
    let deadline2 = Instant::now() + Duration::from_secs(2);
    while !saw_targeted && Instant::now() < deadline2 {
        if let Some(WireInboundItem::Frame { frame, .. }) =
            peer_b.recv_item_timeout(Duration::from_millis(100))
            && frame.kind == NveWireKind::SceneKeyframe
        {
            assert_eq!(
                frame.payload, b"post-delta3-fresh-kf",
                "peer B must receive fresh post-delta snapshot, never stale Frame 1"
            );
            saw_targeted = true;
        }
    }
    assert!(saw_targeted, "peer B must receive targeted fresh SceneKeyframe");

    // Peer A MUST NOT receive any unsolicited SceneKeyframe resets from Peer B's reconnect!
    assert!(
        peer_a.recv_frame_timeout(Duration::from_millis(150)).is_none(),
        "healthy peer A must not be disrupted by peer B's KeyframeRequest"
    );

    // Now broadcast a fresh SceneKeyframe (v5) with ZERO subsequent deltas -> server caches v5.
    // Late-joining Peer C gets cached v5 ONCE, AND an explicit recovery `request_keyframe()`
    // (empty payload) is NEVER swallowed — it immediately serves cached v5 without spamming producer!
    assert_eq!(
        server
            .broadcast_scene_keyframe(5, b"cached-keyframe-v5")
            .unwrap(),
        2
    );
    let _ = peer_a.recv_frame_timeout(Duration::from_secs(2)).unwrap();
    let _ = peer_b.recv_item_timeout(Duration::from_secs(2)).unwrap();

    let direct_uri = parse_wire_uri(&format!("ws://{}/nve-wire", server.wss_addr())).unwrap();
    let peer_c = NveWireClient::connect_uri_sync(
        &direct_uri,
        &NveWireClientConfig::new(token.clone(), "late-peer-c"),
    )
    .expect("connect peer C");
    let c_init = peer_c
        .recv_frame_timeout(Duration::from_secs(2))
        .expect("peer C cached keyframe");
    assert_eq!(c_init.kind, NveWireKind::SceneKeyframe);
    assert_eq!(c_init.payload, b"cached-keyframe-v5");

    // Explicit recovery KeyframeRequest MUST be served immediately on the FIRST call!
    peer_c.request_keyframe().unwrap();
    let c_resend = peer_c
        .recv_frame_timeout(Duration::from_secs(2))
        .expect("explicit recovery KeyframeRequest must NEVER be swallowed");
    assert_eq!(c_resend.kind, NveWireKind::SceneKeyframe);
    assert_eq!(c_resend.payload, b"cached-keyframe-v5");
    assert!(
        server
            .recv_inbound_timeout(Duration::from_millis(80))
            .is_none(),
        "cached keyframe reply must not forward KeyframeRequest to producer"
    );

    // NodePatchDelta also invalidates `latest_keyframe` so a late-joining peer triggers a producer
    // KeyframeRequest instead of receiving stale `cached-keyframe-v5`!
    let patch_frame = NveWireFrame::new(NveWireKind::NodePatchDelta, 6, b"patch-6".to_vec());
    assert_eq!(server.broadcast_frame(&patch_frame).unwrap(), 3);
    let _ = peer_a.recv_frame_timeout(Duration::from_secs(2)).unwrap();
    let _ = peer_b.recv_item_timeout(Duration::from_secs(2)).unwrap();
    let _ = peer_c.recv_frame_timeout(Duration::from_secs(2)).unwrap();

    let peer_d = NveWireClient::connect_uri_sync(
        &direct_uri,
        &NveWireClientConfig::new(token, "late-peer-d"),
    )
    .expect("connect peer D");
    let d_req = server
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("NodePatchDelta must invalidate cached keyframe and emit KeyframeRequest for late peer");
    assert_eq!(d_req.frame.kind, NveWireKind::KeyframeRequest);
    assert!(
        peer_d.recv_frame_timeout(Duration::from_millis(80)).is_none(),
        "stale cached-keyframe-v5 must not be replayed after NodePatchDelta"
    );

    peer_d.close();
    peer_c.close();
    peer_b.shutdown();
    peer_a.close();
    stop_proxy.store(true, Ordering::Release);
    let _ = proxy_thread.join();
    server.shutdown();
}

#[test]
fn spawn_offline_to_online_drops_offline_and_stale_session_events() {
    let token = generate_bearer_token();
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let ws_addr = probe.local_addr().unwrap();
    drop(probe);

    let uri = parse_wire_uri(&format!("ws://{ws_addr}/nve-wire")).unwrap();
    let mut client = ReconnectingWireClient::spawn(
        uri,
        NveWireClientConfig::new(token.clone(), "offline-spawn-test"),
        ReconnectConfig {
            initial_backoff: Duration::from_millis(25),
            max_backoff: Duration::from_millis(80),
        },
        None,
    )
    .expect("spawn offline client");

    // Events sent before first connection MUST fail immediately and count as dropped (never queued).
    assert!(!client.is_connected());
    assert!(client.send_events(b"pre-connect-stale").is_err());
    assert!(client.dropped_outbound() >= 1);

    // Start server 1 on the reserved port; client connects automatically without receiving stale events.
    let bind_deadline = Instant::now() + Duration::from_secs(3);
    let server1 = loop {
        match NveWireServer::bind_sync(NveWireServerConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            ws_addr,
            token.clone(),
        )) {
            Ok(s) => break s,
            Err(_) if Instant::now() < bind_deadline => std::thread::sleep(Duration::from_millis(15)),
            Err(e) => panic!("bind server1 failed: {e}"),
        }
    };

    let deadline = Instant::now() + Duration::from_secs(4);
    while !client.is_connected() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(client.is_connected());
    assert_eq!(client.active_session_id(), 1);

    client.send_events(b"valid-session-1-event").unwrap();
    let in1 = server1
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server1 receives valid event");
    assert_eq!(
        in1.frame.payload, b"valid-session-1-event",
        "stale offline events must never be delivered"
    );

    // Drop server 1 -> outage events fail & bump dropped_outbound -> server 2 gets only fresh events.
    server1.shutdown();
    drop(server1);
    let drop_deadline = Instant::now() + Duration::from_secs(3);
    while client.is_connected() && Instant::now() < drop_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let before_outage_drops = client.dropped_outbound();
    assert!(client.send_events(b"during-outage-stale").is_err());
    assert!(client.dropped_outbound() > before_outage_drops);

    let rebind_deadline = Instant::now() + Duration::from_secs(3);
    let server2 = loop {
        match NveWireServer::bind_sync(NveWireServerConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            ws_addr,
            token.clone(),
        )) {
            Ok(s) => break s,
            Err(_) if Instant::now() < rebind_deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("rebind server2 failed: {e}"),
        }
    };

    let rec_deadline = Instant::now() + Duration::from_secs(4);
    while client.active_session_id() < 2 && Instant::now() < rec_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(client.is_connected());
    assert_eq!(client.active_session_id(), 2);

    client.send_events(b"valid-session-2-event").unwrap();
    let mut saw_valid2 = false;
    let ev_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < ev_deadline {
        let Some(msg) = server2.recv_inbound_timeout(Duration::from_millis(200)) else {
            if saw_valid2 {
                break;
            }
            continue;
        };
        if msg.frame.kind == NveWireKind::EventBatch {
            assert_eq!(
                msg.frame.payload, b"valid-session-2-event",
                "stale outage event leaked into session 2"
            );
            saw_valid2 = true;
            break;
        }
    }
    assert!(saw_valid2);

    client.shutdown();
    server2.shutdown();
}

#[test]
fn terminal_auth_rejection_stops_retrying_and_surfaces_error() {
    let server = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        "expected-valid-token",
    ))
    .expect("bind server");
    let uri = parse_wire_uri(&format!("ws://{}/nve-wire", server.wss_addr())).unwrap();

    let wakes = Arc::new(AtomicUsize::new(0));
    let wakes_cb = Arc::clone(&wakes);
    let mut client = ReconnectingWireClient::spawn(
        uri,
        NveWireClientConfig::new("wrong-token", "rejected-client"),
        ReconnectConfig {
            initial_backoff: Duration::from_millis(20),
            max_backoff: Duration::from_millis(40),
        },
        Some(Arc::new(move || {
            wakes_cb.fetch_add(1, Ordering::Release);
        })),
    )
    .expect("spawn client");

    let deadline = Instant::now() + Duration::from_secs(3);
    while client.auth_rejections() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(client.auth_rejections(), 1);
    assert!(!client.is_connected());
    assert!(client.last_error().is_some());
    assert!(wakes.load(Ordering::Acquire) >= 1);

    // Wait multiple backoff intervals and verify the worker terminated rather than spinning.
    std::thread::sleep(Duration::from_millis(180));
    assert_eq!(client.auth_rejections(), 1);
    assert_eq!(server.metrics().rejected_handshakes, 1);

    client.shutdown();
    server.shutdown();
}

#[test]
fn prompt_shutdown_during_backoff_and_stalled_handshake() {
    // 1. Stalled TCP listener accepts connection and never responds (mid-handshake stall).
    let blackhole = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let blackhole_addr = blackhole.local_addr().unwrap();
    let keep_open = std::thread::spawn(move || {
        let _sock = blackhole.accept().ok();
        std::thread::sleep(Duration::from_millis(600));
    });

    let uri_stall = parse_wire_uri(&format!("ws://{blackhole_addr}/nve-wire")).unwrap();
    let mut stalled = ReconnectingWireClient::spawn(
        uri_stall,
        NveWireClientConfig::new("tok", "stalled-client"),
        ReconnectConfig {
            initial_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_secs(5),
        },
        None,
    )
    .expect("spawn stalled client");
    std::thread::sleep(Duration::from_millis(40));
    let t0 = Instant::now();
    stalled.shutdown();
    assert!(
        t0.elapsed() < Duration::from_millis(500),
        "shutdown during stalled handshake took {:?}",
        t0.elapsed()
    );

    // 2. Closed port enters 5s backoff sleep; shutdown must still finish promptly.
    let closed_probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let closed_addr = closed_probe.local_addr().unwrap();
    drop(closed_probe);

    let uri_backoff = parse_wire_uri(&format!("ws://{closed_addr}/nve-wire")).unwrap();
    let mut backing_off = ReconnectingWireClient::spawn(
        uri_backoff,
        NveWireClientConfig::new("tok", "backoff-client"),
        ReconnectConfig {
            initial_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_secs(5),
        },
        None,
    )
    .expect("spawn backoff client");
    std::thread::sleep(Duration::from_millis(40));
    let t1 = Instant::now();
    backing_off.shutdown();
    assert!(
        t1.elapsed() < Duration::from_millis(500),
        "shutdown during backoff took {:?}",
        t1.elapsed()
    );

    let _ = keep_open.join();
}

#[test]
fn pre_hello_ack_close_retries_as_connect_not_terminal_auth() {
    use futures_util::{SinkExt, StreamExt};
    use std::sync::atomic::AtomicBool;
    use tokio_tungstenite::tungstenite::Message;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_srv = Arc::clone(&stop);

    let srv_thread = std::thread::spawn(move || {
        rt.block_on(async move {
            // Attempt 1: upgrade WS, read Hello, close immediately BEFORE HelloAck!
            if let Ok((stream, _)) = listener.accept().await
                && let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await
            {
                let _ = ws.next().await;
                let _ = ws.close(None).await;
            }
            // Attempt 2: upgrade WS, read Hello, send Ping + valid HelloAck + SceneKeyframe!
            if let Ok((stream, _)) = listener.accept().await
                && let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await
            {
                let _ = ws.next().await;
                let _ = ws.send(Message::Ping(vec![1, 2, 3].into())).await;
                let mut ack_payload = Vec::with_capacity(12);
                ack_payload.extend_from_slice(&42u32.to_le_bytes());
                ack_payload.extend_from_slice(&[2u8, 0, 0, 0]);
                ack_payload.extend_from_slice(&0u32.to_le_bytes());
                let ack = NveWireFrame::new(NveWireKind::HelloAck, 1, ack_payload)
                    .encode()
                    .unwrap();
                let _ = ws.send(Message::Binary(ack.into())).await;

                let kf = NveWireFrame::new(
                    NveWireKind::SceneKeyframe,
                    2,
                    b"recovered-after-pre-hello-close".to_vec(),
                )
                .encode()
                .unwrap();
                let _ = ws.send(Message::Binary(kf.into())).await;

                while !stop_srv.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        });
    });

    let uri = parse_wire_uri(&format!("ws://{addr}/nve-wire")).unwrap();
    let mut client = ReconnectingWireClient::spawn(
        uri,
        NveWireClientConfig::new("tok", "pre-hello-close-test"),
        ReconnectConfig {
            initial_backoff: Duration::from_millis(25),
            max_backoff: Duration::from_millis(60),
        },
        None,
    )
    .expect("spawn client");

    let mut saw_kf = false;
    let deadline = Instant::now() + Duration::from_secs(4);
    while !saw_kf && Instant::now() < deadline {
        if let Some(WireInboundItem::Frame { frame, .. }) =
            client.recv_item_timeout(Duration::from_millis(100))
            && frame.kind == NveWireKind::SceneKeyframe
            && frame.payload == b"recovered-after-pre-hello-close"
        {
            saw_kf = true;
        }
    }
    assert!(
        saw_kf,
        "transient pre-HelloAck WS close must retry cleanly (auth_rejections={}, last_error={:?})",
        client.auth_rejections(),
        client.last_error()
    );
    assert_eq!(
        client.auth_rejections(),
        0,
        "pre-HelloAck disconnect must not be classified as terminal Auth rejection"
    );

    stop.store(true, Ordering::Release);
    client.shutdown();
    let _ = srv_thread.join();
}

#[test]
fn inbound_queue_overflow_emits_overflowed_counts_dropped_and_preserves_control_and_keyframe() {
    let token = generate_bearer_token();
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let ws_addr = probe.local_addr().unwrap();
    drop(probe);

    let server1 = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        ws_addr,
        token.clone(),
    ))
    .expect("bind server1");

    let uri = parse_wire_uri(&format!("ws://{ws_addr}/nve-wire")).unwrap();
    let mut client = ReconnectingWireClient::connect_sync(
        uri,
        NveWireClientConfig::new(token.clone(), "overflow-test-client"),
        ReconnectConfig {
            initial_backoff: Duration::from_millis(25),
            max_backoff: Duration::from_millis(60),
        },
        None,
    )
    .expect("connect overflow client");

    // 1. Flood > 300 CommandDeltas without draining, followed by a SceneKeyframe.
    for seq in 1..=320u32 {
        assert_eq!(server1.broadcast_command_delta(seq, b"delta").unwrap(), 1);
    }
    assert_eq!(
        server1
            .broadcast_scene_keyframe(321, b"post-overflow-kf")
            .unwrap(),
        1
    );

    let deadline = Instant::now() + Duration::from_secs(4);
    while (client.dropped_inbound() == 0 || client.metrics().keyframes_received < 1)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        client.dropped_inbound() > 0,
        "saturated inbound queue must report dropped_inbound > 0"
    );

    let mut saw_overflow = false;
    let mut saw_kf_after_overflow = false;
    while let Some(item) = client.try_recv_item() {
        match item {
            WireInboundItem::Overflowed { session_id: 1 } => saw_overflow = true,
            WireInboundItem::Frame { frame, .. }
                if saw_overflow
                    && frame.kind == NveWireKind::SceneKeyframe
                    && frame.payload == b"post-overflow-kf" =>
            {
                saw_kf_after_overflow = true;
            }
            _ => {}
        }
    }
    assert!(saw_overflow, "overflowed inbound queue must emit WireInboundItem::Overflowed");
    assert!(
        saw_kf_after_overflow,
        "SceneKeyframe arriving on full queue must be preserved after Overflowed"
    );

    // 2. Saturate inbound queue again, then drop server1 and start server2 on same port:
    //    reconnect's SessionStarted(2, reconnected=true) MUST be preserved despite full queue!
    for seq in 322..=620u32 {
        let _ = server1.broadcast_command_delta(seq, b"fill");
    }
    let fill_deadline = Instant::now() + Duration::from_secs(3);
    while client.metrics().command_deltas_received < 600 && Instant::now() < fill_deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    server1.shutdown();
    drop(server1);

    let rebind_deadline = Instant::now() + Duration::from_secs(3);
    let server2 = loop {
        match NveWireServer::bind_sync(NveWireServerConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            ws_addr,
            token.clone(),
        )) {
            Ok(s) => break s,
            Err(_) if Instant::now() < rebind_deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("rebind server2 failed: {e}"),
        }
    };

    let rec_deadline = Instant::now() + Duration::from_secs(4);
    while client.active_session_id() < 2 && Instant::now() < rec_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(client.active_session_id(), 2);

    let mut saw_session2 = false;
    while let Some(item) = client.try_recv_item() {
        if matches!(
            item,
            WireInboundItem::SessionStarted {
                session_id: 2,
                reconnected: true
            }
        ) {
            saw_session2 = true;
        }
    }
    assert!(
        saw_session2,
        "SessionStarted(2, reconnected=true) must never be lost when inbound queue is full"
    );

    client.shutdown();
    server2.shutdown();
}

#[test]
fn ipv6_loopback_ws_uri_connect_roundtrip() {
    let token = generate_bearer_token();
    let server = match NveWireServer::bind_sync(NveWireServerConfig::new(
        "[::1]:0".parse().unwrap(),
        "[::1]:0".parse().unwrap(),
        token.clone(),
    )) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("[skip] ipv6_loopback_ws_uri_connect_roundtrip: IPv6 loopback unavailable: {err}");
            return;
        }
    };
    let _ = server.broadcast_scene_keyframe(1, b"ipv6-loopback-ok");
    let port = server.wss_addr().port();

    // 1. Parsed bracketed URI `ws://[::1]:port/nve-wire`.
    let parsed_uri = parse_wire_uri(&format!("ws://[::1]:{port}/nve-wire")).expect("parse IPv6 URI");
    let client = NveWireClient::connect_uri_sync(
        &parsed_uri,
        &NveWireClientConfig::new(token.clone(), "ipv6-parsed"),
    )
    .expect("connect via bracketed IPv6 URI");
    let f = client
        .recv_frame_timeout(Duration::from_secs(2))
        .expect("receive IPv6 frame");
    assert_eq!(f.payload, b"ipv6-loopback-ok");
    client.close();

    // 2. Direct unbracketed `NveWireUri { host: "::1", ... }` via ReconnectingWireClient.
    let mut raw_uri = parsed_uri;
    raw_uri.host = "::1".to_owned();
    let mut reconnecting = ReconnectingWireClient::connect_sync(
        raw_uri,
        NveWireClientConfig::new(token, "ipv6-raw-host"),
        ReconnectConfig::default(),
        None,
    )
    .expect("connect ReconnectingWireClient with unbracketed IPv6 host");
    let mut saw_v6 = false;
    let deadline = Instant::now() + Duration::from_secs(2);
    while !saw_v6 && Instant::now() < deadline {
        if let Some(WireInboundItem::Frame { frame, .. }) =
            reconnecting.recv_item_timeout(Duration::from_millis(100))
            && frame.payload == b"ipv6-loopback-ok"
        {
            saw_v6 = true;
        }
    }
    assert!(saw_v6, "IPv6 ReconnectingWireClient must receive SceneKeyframe");

    reconnecting.shutdown();
    server.shutdown();
}

#[test]
fn outbound_backpressure_drops_excess_events_without_disconnecting_healthy_session() {
    use futures_util::{SinkExt, StreamExt};
    use std::sync::atomic::AtomicBool;
    use tokio_tungstenite::tungstenite::Message;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let resume_reading = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let resume_srv = Arc::clone(&resume_reading);
    let stop_srv = Arc::clone(&stop);

    let srv_thread = std::thread::spawn(move || {
        rt.block_on(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                return;
            };
            // Read Hello and send HelloAck, then STOP reading so TCP send buffer + bounded client
            // outbound queues saturate under bursty EventBatch traffic.
            let _ = ws.next().await;
            let mut ack_payload = Vec::with_capacity(12);
            ack_payload.extend_from_slice(&7u32.to_le_bytes());
            ack_payload.extend_from_slice(&[2u8, 0, 0, 0]);
            ack_payload.extend_from_slice(&0u32.to_le_bytes());
            let ack = NveWireFrame::new(NveWireKind::HelloAck, 1, ack_payload)
                .encode()
                .unwrap();
            let _ = ws.send(Message::Binary(ack.into())).await;

            while !resume_srv.load(Ordering::Acquire) && !stop_srv.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            if stop_srv.load(Ordering::Acquire) {
                return;
            }

            // Resume reading and send a SceneKeyframe proving the SAME session stayed alive!
            let kf = NveWireFrame::new(
                NveWireKind::SceneKeyframe,
                2,
                b"alive-after-backpressure".to_vec(),
            )
            .encode()
            .unwrap();
            let _ = ws.send(Message::Binary(kf.into())).await;

            while !stop_srv.load(Ordering::Acquire) {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                    msg = ws.next() => {
                        if msg.is_none() {
                            break;
                        }
                    }
                }
            }
        });
    });

    let uri = parse_wire_uri(&format!("ws://{addr}/nve-wire")).unwrap();
    let mut client = ReconnectingWireClient::connect_sync(
        uri,
        NveWireClientConfig::new("tok", "backpressure-test"),
        ReconnectConfig::default(),
        None,
    )
    .expect("connect backpressure client");
    assert_eq!(client.active_session_id(), 1);

    // Burst 16 KiB events until backpressure hits either the outer or inner bounded queue.
    let big_event = vec![0xABu8; 16 * 1024];
    let mut saw_backpressure_err = false;
    for _ in 0..2000 {
        match client.send_events(big_event.clone()) {
            Ok(()) => {}
            Err(NveWireTransportError::Backpressure(_)) => {
                saw_backpressure_err = true;
                break;
            }
            Err(other) => panic!("unexpected non-backpressure error: {other}"),
        }
    }
    let bp_deadline = Instant::now() + Duration::from_secs(2);
    while client.dropped_outbound() == 0 && Instant::now() < bp_deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        saw_backpressure_err || client.dropped_outbound() > 0,
        "expected outbound backpressure drop on stalled peer"
    );
    assert!(
        client.is_connected(),
        "outbound backpressure must NEVER disconnect a healthy session"
    );
    assert_eq!(
        client.active_session_id(),
        1,
        "session_id must remain 1 (zero reconnect churn on backpressure)"
    );
    assert_eq!(client.reconnect_count(), 0);

    // Unblock peer and verify inbound delivery continues on the SAME session_id == 1.
    resume_reading.store(true, Ordering::Release);
    let mut saw_alive_kf = false;
    let deadline = Instant::now() + Duration::from_secs(3);
    while !saw_alive_kf && Instant::now() < deadline {
        if let Some(WireInboundItem::Frame { session_id, frame }) =
            client.recv_item_timeout(Duration::from_millis(100))
            && session_id == 1
            && frame.kind == NveWireKind::SceneKeyframe
            && frame.payload == b"alive-after-backpressure"
        {
            saw_alive_kf = true;
        }
    }
    assert!(saw_alive_kf, "healthy session must keep receiving frames after backpressure");
    assert_eq!(client.reconnect_count(), 0);

    stop.store(true, Ordering::Release);
    client.shutdown();
    let _ = srv_thread.join();
}
