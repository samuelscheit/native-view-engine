//! Deep end-to-end unit tests for `PairClientMachine` and `PairedWebSession` exercising the
//! real `NVT1`/`NVD1` binary RFC 6455 WebSocket handshake, delta frame DOM mirror updates,
//! and upstream `InputEvents = 11` decoding by `nve_interop::TcpDeltaServer`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use nve_abi::events::{decode_keydown_bytes, KEYDOWN_WIRE_MAGIC};
use nve_abi::{decode_events, EventMask, EventPayload, NativeEvent, NativeEventKind, NodeId};
use nve_backend_web::WebBackend;
use nve_engine::Viewport;
use nve_interop::{
    decode_ws_frame_slice, encode_ws_frame, PairingConfig, RejectReason, TcpDeltaServer,
    TransportMsgKind, WsOpcode,
};

use super::*;

fn ws_handshake(stream: &mut TcpStream, host: &str, path: &str) {
    let req = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).expect("write ws upgrade");
    stream.flush().unwrap();

    let mut resp = Vec::new();
    let mut byte = [0u8; 1];
    while !resp.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).expect("read ws upgrade resp");
        resp.push(byte[0]);
    }
    let header = std::str::from_utf8(&resp).expect("utf8 http resp");
    assert!(
        header.starts_with("HTTP/1.1 101"),
        "expected 101 Switching Protocols, got: {header}"
    );
}

fn ws_send_binary(stream: &mut TcpStream, payload: &[u8]) {
    if payload.is_empty() {
        return;
    }
    let frame = encode_ws_frame(true, WsOpcode::Binary, payload, Some([0x12, 0x34, 0x56, 0x78]));
    stream.write_all(&frame).expect("write masked ws binary");
    stream.flush().unwrap();
}

fn ws_recv_binary(stream: &mut TcpStream, read_buf: &mut Vec<u8>) -> Vec<u8> {
    let mut tmp = [0u8; 4096];
    loop {
        if let Ok((frame, consumed)) = decode_ws_frame_slice(read_buf, 4 * 1024 * 1024) {
            read_buf.drain(..consumed);
            match frame.opcode {
                WsOpcode::Binary => return frame.payload,
                WsOpcode::Ping => {
                    let pong = encode_ws_frame(
                        true,
                        WsOpcode::Pong,
                        &frame.payload,
                        Some([0xAA, 0xBB, 0xCC, 0xDD]),
                    );
                    stream.write_all(&pong).unwrap();
                }
                _ => continue,
            }
        }
        let n = stream.read(&mut tmp).expect("read ws bytes");
        assert!(n > 0, "unexpected EOF on pairing WebSocket");
        read_buf.extend_from_slice(&tmp[..n]);
    }
}

fn find_first_pressable(backend: &WebBackend) -> NodeId {
    let mut ids: Vec<NodeId> = backend
        .iter_nodes()
        .filter_map(|(id, el)| ((el.event_mask & EventMask::PRESS) != 0).then_some(id))
        .collect();
    ids.sort_unstable();
    ids.into_iter().next().expect("at least one pressable node")
}

#[test]
fn web_pair_client_full_nvt1_ws_handshake_delta_dom_render_and_input_events_with_tcp_delta_server()
{
    let token = "web-pair-e2e-token";
    let mut server =
        TcpDeltaServer::bind("127.0.0.1:0", PairingConfig::new(token)).expect("bind TcpDeltaServer");
    let addr = server.local_addr();

    // Authoritative server-side CounterApp surface.
    let mut counter = nve_example_counter::create_surface(1);
    let (_, _, frame1) = counter.render_frame(1);
    server.send_frame(&frame1).expect("seed initial keyframe");

    // Browser-side PairedWebSession (no local app VDOM!).
    let viewport = Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 1.0,
    };
    let mut web_session = PairedWebSession::new(1, WebBackend::new(), viewport, token);

    let mut stream = TcpStream::connect(addr).expect("connect TCP");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    ws_handshake(&mut stream, &addr.to_string(), "/ws/nvd");

    // 1. ClientHello -> ServerHello (HelloAck) + Server Ping + Server initial Keyframe.
    let hello = web_session.build_hello();
    assert_eq!(&hello[0..2], b"NT");
    assert_eq!(hello[3], TransportMsgKind::Hello as u8);
    ws_send_binary(&mut stream, &hello);

    let mut rx_buf = Vec::new();
    while web_session.frames_received() == 0 {
        let msg = ws_recv_binary(&mut stream, &mut rx_buf);
        let reply = web_session
            .on_pair_binary_message(&msg)
            .expect("valid NVT1 server message");
        ws_send_binary(&mut stream, &reply);
        while let Some(extra) = web_session.take_outbound_message() {
            ws_send_binary(&mut stream, &extra);
        }
    }

    assert!(web_session.is_connected());
    assert_ne!(web_session.session_id(), 0);
    assert_eq!(web_session.frames_received(), 1);

    let html_f1 = web_session.backend().render_html();
    assert!(
        html_f1.contains("Count: 0"),
        "expected remote CounterApp Frame 1 DOM mirror to contain 'Count: 0', got: {html_f1}"
    );
    #[cfg(feature = "scene-view")]
    assert!(web_session.scene().view().node_count > 0);

    // Server should have received the initial Layout InputEvents automatically on handshake!
    let (_sid, layout_ring) = server
        .recv_events(Duration::from_secs(2))
        .expect("server received initial Layout InputEvents");
    let layout_evs = decode_events(&layout_ring).expect("valid Layout EventRing");
    assert_eq!(layout_evs.len(), 1);
    assert_eq!(layout_evs[0].kind, NativeEventKind::Layout);

    // 2. Simulate local DOM click in WebBackend -> encode into NVT1 InputEvents = 11 -> send to server.
    let btn_id = find_first_pressable(web_session.backend());
    web_session.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id: btn_id,
        timestamp_us: 12_345,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 24.0, y: 18.0 },
    });
    let click_nvt1 = web_session
        .drain_dom_events_as_nvt1()
        .expect("encode click NVT1")
        .expect("non-empty click frame");
    assert_eq!(click_nvt1[3], TransportMsgKind::InputEvents as u8);
    assert!(
        !click_nvt1.starts_with(b"{"),
        "must be zero-JSON binary NVT1"
    );
    ws_send_binary(&mut stream, &click_nvt1);

    // Server decodes the InputEvents = 11 backchannel, mutates CounterApp, and streams an NVD1 delta.
    let (peer_sid, click_ring) = server
        .recv_events(Duration::from_secs(2))
        .expect("server received click InputEvents");
    assert_eq!(peer_sid, u64::from(web_session.session_id()));
    let click_evs = decode_events(&click_ring).expect("valid click EventRing");
    assert!(counter.dispatch_events(&click_evs));
    let (_, _, frame2) = counter.render_frame(2);
    assert_eq!(server.send_frame(&frame2).expect("broadcast delta frame"), 1);

    // Client receives NVD1 delta -> WebBackend DOM mirror updates to "Count: 1"!
    while web_session.frames_received() < 2 {
        let msg = ws_recv_binary(&mut stream, &mut rx_buf);
        let reply = web_session
            .on_pair_binary_message(&msg)
            .expect("valid delta message");
        ws_send_binary(&mut stream, &reply);
        while let Some(extra) = web_session.take_outbound_message() {
            ws_send_binary(&mut stream, &extra);
        }
    }

    let html_f2 = web_session.backend().render_html();
    assert!(
        html_f2.contains("Count: 1"),
        "expected remote delta frame to update DOM mirror to 'Count: 1', got: {html_f2}"
    );

    // 3. Synthetic NK1 keydown -> NVT1 InputEvents = 11 -> decoded by TcpDeltaServer.
    let kd_nvt1 = web_session
        .encode_keydown("k", true, false)
        .expect("encode keydown")
        .expect("non-empty keydown packet");
    ws_send_binary(&mut stream, &kd_nvt1);
    let (_sid, kd_ring) = server
        .recv_events(Duration::from_secs(2))
        .expect("server received NK1 keydown");
    let kd_evs = decode_events(&kd_ring).expect("decode NK1 keydown EventRing");
    match &kd_evs[0].payload {
        EventPayload::Custom { name, data } => {
            assert_eq!(name, "keydown");
            assert!(data.starts_with(&KEYDOWN_WIRE_MAGIC));
            let decoded = decode_keydown_bytes(data).expect("valid NK1");
            assert!(decoded.meta && decoded.key == "k");
        }
        other => panic!("expected Custom keydown event, got {other:?}"),
    }

    // 4. Clean Bye disconnects cleanly.
    if let Some(bye) = web_session.build_bye() {
        ws_send_binary(&mut stream, &bye);
    }
    assert!(!web_session.is_connected());
    drop(stream);
    server.shutdown();
}

#[test]
fn pair_client_machine_rejects_bad_token_corrupt_envelopes_and_requests_keyframe_on_gap() {
    let mut client = PairClientMachine::new("secret-token");
    assert!(!client.is_connected());
    assert!(client.encode_input_events(&[NativeEvent {
        surface_id: 1,
        node_id: 1,
        timestamp_us: 0,
        kind: NativeEventKind::Focus,
        payload: EventPayload::Empty,
    }])
    .is_err());

    // Truncated / bad magic / bad version envelopes are rejected.
    assert!(client.on_binary_message(b"NT").is_err());
    assert!(client.on_binary_message(b"XX\x01\x02\x00\x00\x00\x00").is_err());
    assert!(client.on_binary_message(b"NT\x99\x02\x00\x00\x00\x00").is_err());

    // Server Reject message transitions state to Rejected.
    let _ = client.build_hello();
    let reject = encode_nvt1_envelope(
        TransportMsgKind::Reject,
        0,
        &[RejectReason::InvalidPairingToken as u8, 1, 0, 0],
    );
    let err = client.on_binary_message(&reject).unwrap_err();
    assert!(err.contains("InvalidPairingToken"));
    assert!(matches!(client.state(), PairClientState::Rejected(_)));

    // After valid handshake, receiving a delta referencing an unknown base triggers KeyframeRequest.
    let mut client = PairClientMachine::new("valid-token");
    let hello = client.build_hello();
    let client_nonce = u64::from_le_bytes(hello[16..24].try_into().unwrap());
    let server_nonce = 0x1122_3344_5566_7788u64;
    let sid = derive_session_id("valid-token", DEFAULT_SCHEMA_VERSION, client_nonce, server_nonce);
    let mut ack_payload = [0u8; 20];
    ack_payload[0..2].copy_from_slice(&DEFAULT_SCHEMA_VERSION.to_le_bytes());
    ack_payload[2..4].copy_from_slice(&64u16.to_le_bytes());
    ack_payload[8..16].copy_from_slice(&server_nonce.to_le_bytes());
    let ack = encode_nvt1_envelope(TransportMsgKind::HelloAck, sid, &ack_payload);
    let res = client.on_binary_message(&ack).expect("accept HelloAck");
    assert!(res.handshake_completed);
    assert!(client.is_connected());

    // Wrong session_id is rejected.
    let wrong_sid_ping = encode_nvt1_envelope(TransportMsgKind::Ping, sid ^ 1, &0u64.to_le_bytes());
    assert!(client.on_binary_message(&wrong_sid_ping).is_err());

    // Out-of-order future packet (seq = 5 while last_applied_seq = 0) emits ACK + NACK packets.
    let future_delta = nve_interop::encode_packet(
        nve_interop::PacketKind::XorRleDelta,
        0,
        5,
        4,
        0,
        &[0u8],
    );
    let future_env = encode_nvt1_envelope(TransportMsgKind::DeltaSingle, sid, &future_delta);
    let out_gap = client
        .on_binary_message(&future_env)
        .expect("out-of-order future packet buffered with ACK/NACK");
    assert!(
        out_gap.outbound.len() >= 2,
        "expected ACK + NACK responses for sequence gap"
    );
    assert!(out_gap
        .outbound
        .iter()
        .all(|p| p[3] == TransportMsgKind::DeltaSingle as u8));

    // Next-in-order delta (seq = 1) referencing a mismatched base_seq = 99 triggers KeyframeRequest!
    let bad_base_delta = nve_interop::encode_packet(
        nve_interop::PacketKind::XorRleDelta,
        0,
        1,
        99,
        0,
        &[0u8],
    );
    let delta_env = encode_nvt1_envelope(TransportMsgKind::DeltaSingle, sid, &bad_base_delta);
    let out = client
        .on_binary_message(&delta_env)
        .expect("graceful BaseSequenceMismatch -> KeyframeRequest");
    assert_eq!(out.outbound.len(), 1);
    assert_eq!(out.outbound[0][3], TransportMsgKind::KeyframeRequest as u8);
}
