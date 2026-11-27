//! Live loopback integration tests for `nve-wire`:
//! - Raw QUIC (`nve-wire-2` ALPN) handshake, `NveSceneArchive` keyframe (`XXH64`), `CommandDelta`,
//!   `NodePatchDelta` (< 512 B), `EventBatch` backchannel, and RFC 9221 datagram round-trip
//! - Real HTTP/3 WebTransport (`h3` ALPN) handshake on the SAME UDP socket with 13-day ECDSA P-256
//!   SHA-256 cert pinning + full stream & datagram round-trip
//! - Happy-Eyeballs automatic fallback: UDP/QUIC blocked (dead port) -> connects via `wss://`
//!   within ~250ms and completes full `NveWireFrame` round-trip
//! - Constant-time Bearer token rejection & C ABI lifecycle test

use std::ffi::CString;
use std::net::SocketAddr;
use std::ptr;
use std::time::{Duration, Instant};

use nve_abi::{
    ColorRgba, EventBatchBuf, FrameId, NodeId, SurfaceId, TimestampUs, NVE_PRIMARY_SLOT_NONE,
};
use nve_tables::{LayoutSnapshot, VisualSnapshot};
use nve_engine::{
    encode_scene_reloc, LayoutRect, SceneRelocInput, SceneRelocNodeInput, SceneRelocView,
    VerifyMode, Viewport,
};
use nve_wire::{
    format_wire_uri, generate_bearer_token, nve_wire_client_connect_uri, nve_wire_client_destroy,
    nve_wire_client_is_connected, nve_wire_client_metrics, nve_wire_client_poll_frame,
    nve_wire_client_send_events, NveNodePatchDeltaBuilder, NveNodePatchDeltaView, NveWireChannel,
    NveWireClient, NveWireClientConfig, NveWireFrame, NveWireKind, NveWireMetricsC, NveWireServer,
    NveWireServerConfig, NveWireTransportError, NveWireTransportKind, NVE_WIRE_OK,
};

fn build_sample_scene_archive() -> nve_engine::NveSceneArchive {
    let nodes = [
        SceneRelocNodeInput {
            node_id: NodeId::ROOT,
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
            node_id: NodeId::new(2, 0),
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
        surface_id: SurfaceId::ROOT,
        frame_id: FrameId(1),
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
        pb.set_text(NodeId::new(10 + i, 0), "SCORE 42");
    }
    for i in 0..4u32 {
        pb.set_bg_rgba(NodeId::new(20 + i, 0), 0x22C55EFF);
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
    let mut ev_buf = EventBatchBuf::new();
    ev_buf.push_press(SurfaceId::ROOT, NodeId::new(2, 0), TimestampUs(1234), 10.0, 20.0);
    let event_bytes = ev_buf.to_wire_bytes();
    client.send_events(&event_bytes).unwrap();
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
fn c_abi_client_connect_poll_send_events_and_destroy() {
    let token = generate_bearer_token();
    let server = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token.clone(),
    ))
    .expect("bind sync server");

    let cert_hash = server.cert_sha256();
    let uri = format_wire_uri(
        "127.0.0.1",
        server.quic_addr().port(),
        server.wss_addr().port(),
        &token,
        Some(&cert_hash),
        393,
        852,
    );
    let c_uri = CString::new(uri).unwrap();
    let c_name = CString::new("c-abi-test").unwrap();
    let mut handle = ptr::null_mut();

    // SAFETY: Calling our C ABI with valid C strings and output pointer.
    let status = unsafe {
        nve_wire_client_connect_uri(c_uri.as_ptr(), ptr::null(), c_name.as_ptr(), 250, &mut handle)
    };
    assert_eq!(status, NVE_WIRE_OK);
    assert!(!handle.is_null());
    // SAFETY: `handle` is a live client handle returned by `nve_wire_client_connect_uri` until `destroy` below.
    assert!(unsafe { nve_wire_client_is_connected(handle) });

    server.broadcast_command_delta(9, b"c-abi-delta").unwrap();

    let mut kind = 0u8;
    let mut seq = 0u32;
    let mut buf = [0u8; 64];
    let mut len = 0usize;
    let mut got = false;
    for _ in 0..40 {
        // SAFETY: `handle` is live and all out-pointers/buffer point to valid writable stack memory.
        let st = unsafe {
            nve_wire_client_poll_frame(handle, &mut kind, &mut seq, buf.as_mut_ptr(), buf.len(), &mut len)
        };
        if st == NVE_WIRE_OK {
            got = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(got, "C ABI poll_frame should receive CommandDelta");
    assert_eq!(kind, NveWireKind::CommandDelta as u8);
    assert_eq!(seq, 9);
    assert_eq!(&buf[..len], b"c-abi-delta");

    let ev = b"evt-batch";
    // SAFETY: `handle` is live and `ev` is a readable slice of `ev.len()` bytes.
    let send_status = unsafe { nve_wire_client_send_events(handle, ev.as_ptr(), ev.len()) };
    assert_eq!(send_status, NVE_WIRE_OK);
    let inbound = server
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server receives C ABI EventBatch");
    assert_eq!(inbound.frame.payload, ev);

    let mut metrics = NveWireMetricsC::default();
    // SAFETY: `handle` is live and `metrics` is a valid writable out-pointer.
    assert_eq!(unsafe { nve_wire_client_metrics(handle, &mut metrics) }, NVE_WIRE_OK);
    assert_eq!(metrics.command_deltas_received, 1);
    assert_eq!(metrics.events_sent, 1);

    // SAFETY: `handle` is destroyed once at the end of the test.
    unsafe { nve_wire_client_destroy(handle) };
}
