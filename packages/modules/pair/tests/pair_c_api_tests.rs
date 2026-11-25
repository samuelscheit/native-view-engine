use std::ptr;

use nve_abi::{
    CommandBufferBuilder, ComponentKind, EventPayload, NativeEventKind, decode_events,
};
use nve_interop::{
    RejectReason, SyncDeltaSession, TRANSPORT_HEADER_SIZE, TRANSPORT_MAGIC,
    TRANSPORT_PROTOCOL_VERSION, TransportMsgKind, derive_session_id, derive_token_proof,
};
use nve_pair::{
    NVE_PAIR_ERR_AUTH_FAILED, NVE_PAIR_ERR_BASE_SEQ_MISMATCH, NVE_PAIR_ERR_BUFFER_TOO_SMALL,
    NVE_PAIR_ERR_CHECKSUM, NVE_PAIR_ERR_EMPTY_QUEUE, NVE_PAIR_ERR_INVALID_MAGIC,
    NVE_PAIR_ERR_INVALID_URI, NVE_PAIR_ERR_NOT_PAIRED, NVE_PAIR_ERR_NULL_ARG,
    NVE_PAIR_ERR_SCHEMA_MISMATCH, NVE_PAIR_ERR_SESSION_MISMATCH, NVE_PAIR_ERR_TRUNCATED,
    NVE_PAIR_OK, NvePairClientConfig, NvePairIngestOutcome, NvePairInputEvent, NvePairMetrics,
    NvePairParsedUri, PairClientState, PairTransportKind, encode_envelope,
    nve_pair_client_begin_handshake, nve_pair_client_create, nve_pair_client_create_from_uri,
    nve_pair_client_current_frame, nve_pair_client_destroy, nve_pair_client_encode_input_event,
    nve_pair_client_ingest_packet, nve_pair_client_metrics, nve_pair_client_pop_outbound_packet,
    nve_pair_parse_uri, parse_pair_uri,
};

fn build_nve_frame(frame_id: u64, label: &str) -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, frame_id);
    b.create_node(2, ComponentKind::View, None);
    b.create_node(3, ComponentKind::Text, None);
    b.set_children(1, &[2]);
    b.set_children(2, &[3]);
    b.set_text(3, label);
    b.commit(true);
    b.finish().to_vec()
}

fn build_hello_ack_envelope(
    token: &str,
    schema_version: u16,
    client_nonce: u64,
    server_nonce: u64,
    server_max_window: u16,
    resync_required: bool,
) -> (u32, Vec<u8>) {
    let sid = derive_session_id(token, schema_version, client_nonce, server_nonce);
    let mut payload = [0u8; 20];
    payload[0..2].copy_from_slice(&schema_version.to_le_bytes());
    payload[2..4].copy_from_slice(&server_max_window.to_le_bytes());
    payload[8..16].copy_from_slice(&server_nonce.to_le_bytes());
    payload[16] = u8::from(resync_required);
    (sid, encode_envelope(TransportMsgKind::HelloAck, sid, &payload))
}

#[test]
fn parse_pair_uri_handles_all_schemes_and_rejects_bad_inputs() {
    let u1 = parse_pair_uri(
        "nve://pair?host=192.168.1.42&ws=7777&token=secret-1&schema=3&udp=7778&w=390&h=844",
        None,
    )
    .expect("nve://pair URI");
    assert_eq!(u1.transport, PairTransportKind::Ws);
    assert_eq!(u1.host, "192.168.1.42");
    assert_eq!(u1.port, 7777);
    assert_eq!(u1.path, "/ws/nvd");
    assert_eq!(u1.token, "secret-1");
    assert_eq!(u1.schema_version, 3);
    assert_eq!(u1.udp_port, Some(7778));
    assert_eq!(u1.width, Some(390.0));
    assert_eq!(u1.height, Some(844.0));
    assert_eq!(u1.endpoint_uri(), "ws://192.168.1.42:7777/ws/nvd");

    let u2 = parse_pair_uri("wss://studio.nve.dev/live?token=tok%202", None).expect("wss URI");
    assert_eq!(u2.transport, PairTransportKind::Wss);
    assert_eq!(u2.host, "studio.nve.dev");
    assert_eq!(u2.port, 443);
    assert_eq!(u2.path, "/live");
    assert_eq!(u2.token, "tok 2");

    let u3 = parse_pair_uri("tcp://[::1]:9000", Some("fallback-tok")).expect("tcp ipv6");
    assert_eq!(u3.transport, PairTransportKind::Tcp);
    assert_eq!(u3.host, "::1");
    assert_eq!(u3.port, 9000);
    assert_eq!(u3.token, "fallback-tok");

    assert!(parse_pair_uri("", Some("t")).is_err());
    assert!(parse_pair_uri("nve://other?ws=80&token=t", None).is_err());
    assert!(parse_pair_uri("ws://localhost:9000", None).is_err());
    assert!(parse_pair_uri("nve://pair?ws=0&token=t", None).is_err());
    assert!(parse_pair_uri("nve://pair?ws=8080&token=t&schema=0", None).is_err());

    // C-ABI URI parser roundtrip
    let raw = b"nve://pair?ws=ws://127.0.0.1:4567/custom&token=c-tok&udp=4568&schema=2";
    let mut parsed = NvePairParsedUri::default();
    let rc = unsafe {
        nve_pair_parse_uri(raw.as_ptr(), raw.len(), ptr::null(), 0, &mut parsed)
    };
    assert_eq!(rc, NVE_PAIR_OK);
    assert_eq!(parsed.transport, PairTransportKind::Ws as u8);
    assert_eq!(parsed.port, 4567);
    assert_eq!(parsed.has_udp_port, 1);
    assert_eq!(parsed.udp_port, 4568);
    assert_eq!(parsed.schema_version, 2);
    assert_eq!(&parsed.host_buf[..parsed.host_len], b"127.0.0.1");
    assert_eq!(&parsed.path_buf[..parsed.path_len], b"/custom");
    assert_eq!(&parsed.token_buf[..parsed.token_len], b"c-tok");

    let bad = b"nve://wrong";
    let rc_bad = unsafe {
        nve_pair_parse_uri(bad.as_ptr(), bad.len(), ptr::null(), 0, &mut parsed)
    };
    assert_eq!(rc_bad, NVE_PAIR_ERR_INVALID_URI);
}

#[test]
fn c_abi_full_handshake_delta_sync_out_of_order_and_input_events() {
    let token = "nve-shared-pair-token";
    let cfg = NvePairClientConfig {
        token_ptr: token.as_ptr(),
        token_len: token.len(),
        schema_version: 1,
        max_window: 64,
        max_frame_bytes: 0,
    };
    let client = unsafe { nve_pair_client_create(&cfg) };
    assert!(!client.is_null());

    // Before pairing, current_frame is empty and encode_input_event fails with NOT_PAIRED.
    let empty_slice = unsafe { nve_pair_client_current_frame(client) };
    assert!(empty_slice.ptr.is_null() && empty_slice.len == 0);

    let press_ev = NvePairInputEvent {
        surface_id: 1,
        node_id: 42,
        timestamp_us: 99_001,
        kind: 1, // Press
        reserved: [0; 7],
        x: 12.5,
        y: 34.0,
        width: 0.0,
        height: 0.0,
        text_ptr: ptr::null(),
        text_len: 0,
        data_ptr: ptr::null(),
        data_len: 0,
    };
    let mut scratch = [0u8; 256];
    let mut written = 0usize;
    let rc_unpaired = unsafe {
        nve_pair_client_encode_input_event(
            client,
            &press_ev,
            scratch.as_mut_ptr(),
            scratch.len(),
            &mut written,
        )
    };
    assert_eq!(rc_unpaired, NVE_PAIR_ERR_NOT_PAIRED);

    // 1. Begin handshake -> verify Hello NVT1 envelope & token proof.
    let client_nonce = 0x1122_3344_5566_7788u64;
    let mut hello_buf = [0u8; 64];
    let mut hello_len = 0usize;
    let rc_hello = unsafe {
        nve_pair_client_begin_handshake(
            client,
            client_nonce,
            hello_buf.as_mut_ptr(),
            hello_buf.len(),
            &mut hello_len,
        )
    };
    assert_eq!(rc_hello, NVE_PAIR_OK);
    assert_eq!(hello_len, TRANSPORT_HEADER_SIZE + 32);
    assert_eq!(&hello_buf[0..2], &TRANSPORT_MAGIC);
    assert_eq!(hello_buf[2], TRANSPORT_PROTOCOL_VERSION);
    assert_eq!(hello_buf[3], TransportMsgKind::Hello as u8);
    let expected_proof = derive_token_proof(token, 1);
    assert_eq!(&hello_buf[TRANSPORT_HEADER_SIZE + 16..hello_len], &expected_proof);

    // 2. Server replies with HelloAck -> client transitions to Paired.
    let (sid, hello_ack) = build_hello_ack_envelope(token, 1, client_nonce, 0xCAFE_BABE_0000_0001, 64, true);
    let mut outcome = NvePairIngestOutcome::default();
    let rc_ack = unsafe {
        nve_pair_client_ingest_packet(client, hello_ack.as_ptr(), hello_ack.len(), &mut outcome)
    };
    assert_eq!(rc_ack, NVE_PAIR_OK);
    assert_eq!(outcome.state, PairClientState::Paired as u32);

    // 3. Server sends Keyframe (seq 1), then seq 3 BEFORE seq 2 (out-of-order + NACK + reorder drain!).
    let mut server_delta = SyncDeltaSession::new(64);
    let f1 = build_nve_frame(1, "Frame One");
    let f2 = build_nve_frame(2, "Frame Two");
    let f3 = build_nve_frame(3, "Frame Three");

    let pkt1 = server_delta.encode_frame(&f1, 10);
    let pkt2 = server_delta.encode_frame(&f2, 20);
    let pkt3 = server_delta.encode_frame(&f3, 30);

    let env1 = encode_envelope(TransportMsgKind::DeltaSingle, sid, &pkt1);
    assert_eq!(
        unsafe { nve_pair_client_ingest_packet(client, env1.as_ptr(), env1.len(), &mut outcome) },
        NVE_PAIR_OK
    );
    assert_eq!(outcome.applied_frame_count, 1);
    assert_eq!(outcome.latest_seq, 1);
    assert_eq!(outcome.outbound_packet_count, 1); // ACK

    let cur1 = unsafe { nve_pair_client_current_frame(client) };
    let cur1_bytes = unsafe { std::slice::from_raw_parts(cur1.ptr, cur1.len) };
    assert_eq!(cur1_bytes, f1.as_slice());

    // Drain ACK packet (first query with NULL buffer to verify sizing contract).
    let mut out_len = 0usize;
    assert_eq!(
        unsafe { nve_pair_client_pop_outbound_packet(client, ptr::null_mut(), 0, &mut out_len) },
        NVE_PAIR_ERR_BUFFER_TOO_SMALL
    );
    assert!(out_len > TRANSPORT_HEADER_SIZE);
    let mut out_buf = vec![0u8; out_len];
    assert_eq!(
        unsafe {
            nve_pair_client_pop_outbound_packet(client, out_buf.as_mut_ptr(), out_buf.len(), &mut out_len)
        },
        NVE_PAIR_OK
    );
    assert_eq!(
        unsafe {
            nve_pair_client_pop_outbound_packet(client, out_buf.as_mut_ptr(), out_buf.len(), &mut out_len)
        },
        NVE_PAIR_ERR_EMPTY_QUEUE
    );

    // Feed seq 3 out-of-order -> buffered, emits ACK + NACK(2).
    let env3 = encode_envelope(TransportMsgKind::DeltaSingle, sid, &pkt3);
    assert_eq!(
        unsafe { nve_pair_client_ingest_packet(client, env3.as_ptr(), env3.len(), &mut outcome) },
        NVE_PAIR_OK
    );
    assert_eq!(outcome.applied_frame_count, 0);
    assert_eq!(outcome.latest_seq, 1);
    assert_eq!(outcome.outbound_packet_count, 2); // ACK + NACK

    // Now feed seq 2 -> drains both seq 2 AND buffered seq 3 in strict order!
    let env2 = encode_envelope(TransportMsgKind::DeltaSingle, sid, &pkt2);
    assert_eq!(
        unsafe { nve_pair_client_ingest_packet(client, env2.as_ptr(), env2.len(), &mut outcome) },
        NVE_PAIR_OK
    );
    assert_eq!(outcome.applied_frame_count, 2);
    assert_eq!(outcome.latest_seq, 3);

    let cur3 = unsafe { nve_pair_client_current_frame(client) };
    let cur3_bytes = unsafe { std::slice::from_raw_parts(cur3.ptr, cur3.len) };
    assert_eq!(cur3_bytes, f3.as_slice());

    // 4. Encode InputEvents = 11 (Press + Custom) and decode back with nve_abi::decode_events.
    let rc_ev = unsafe {
        nve_pair_client_encode_input_event(
            client,
            &press_ev,
            scratch.as_mut_ptr(),
            scratch.len(),
            &mut written,
        )
    };
    assert_eq!(rc_ev, NVE_PAIR_OK);
    assert_eq!(&scratch[0..2], &TRANSPORT_MAGIC);
    assert_eq!(scratch[2], TRANSPORT_PROTOCOL_VERSION);
    assert_eq!(scratch[3], TransportMsgKind::InputEvents as u8);
    assert_eq!(u32::from_le_bytes(scratch[4..8].try_into().unwrap()), sid);

    let decoded = decode_events(&scratch[TRANSPORT_HEADER_SIZE..written]).expect("valid EventRing");
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].surface_id, 1);
    assert_eq!(decoded[0].node_id, 42);
    assert_eq!(decoded[0].timestamp_us, 99_001);
    assert_eq!(decoded[0].kind, NativeEventKind::Press);
    assert_eq!(decoded[0].payload, EventPayload::Point { x: 12.5, y: 34.0 });

    // 5. Ping -> Pong auto-response + Pong timestamp tracking.
    let ping_ts = 777_888u64;
    let ping = encode_envelope(TransportMsgKind::Ping, sid, &ping_ts.to_le_bytes());
    assert_eq!(
        unsafe { nve_pair_client_ingest_packet(client, ping.as_ptr(), ping.len(), &mut outcome) },
        NVE_PAIR_OK
    );
    let pong = encode_envelope(TransportMsgKind::Pong, sid, &ping_ts.to_le_bytes());
    assert_eq!(
        unsafe { nve_pair_client_ingest_packet(client, pong.as_ptr(), pong.len(), &mut outcome) },
        NVE_PAIR_OK
    );

    // 6. Verify NvePairMetrics snapshot.
    let mut metrics = NvePairMetrics::default();
    assert_eq!(unsafe { nve_pair_client_metrics(client, &mut metrics) }, NVE_PAIR_OK);
    assert_eq!(metrics.session_id, sid);
    assert_eq!(metrics.last_applied_seq, 3);
    assert_eq!(metrics.state, PairClientState::Paired as u32);
    assert_eq!(metrics.reorder_buffer_len, 0);
    assert_eq!(metrics.frames_received, 3);
    assert_eq!(metrics.keyframes_received, 1);
    assert_eq!(metrics.deltas_received, 2);
    assert_eq!(metrics.events_sent, 1);
    assert!(metrics.acks_sent >= 3);
    assert_eq!(metrics.nacks_sent, 1);
    assert_eq!(metrics.last_pong_timestamp_us, ping_ts);
    assert_eq!(metrics.malformed_packets_dropped, 0);

    unsafe { nve_pair_client_destroy(client) };
    // Destroying NULL is a safe no-op.
    unsafe { nve_pair_client_destroy(ptr::null_mut()) };
}

#[test]
fn c_abi_rejects_corrupt_packets_wrong_session_and_auth_failures() {
    let uri = b"nve://pair?host=127.0.0.1&ws=7777&token=strict-token&schema=2";
    let client = unsafe {
        nve_pair_client_create_from_uri(uri.as_ptr(), uri.len(), ptr::null(), 0)
    };
    assert!(!client.is_null());

    // Null argument checks
    assert_eq!(
        unsafe { nve_pair_client_ingest_packet(ptr::null_mut(), ptr::null(), 0, ptr::null_mut()) },
        NVE_PAIR_ERR_NULL_ARG
    );
    assert_eq!(
        unsafe { nve_pair_client_metrics(client, ptr::null_mut()) },
        NVE_PAIR_ERR_NULL_ARG
    );

    // Corrupt magic & truncated envelopes
    let short = [b'N', b'T', 1];
    assert_eq!(
        unsafe { nve_pair_client_ingest_packet(client, short.as_ptr(), short.len(), ptr::null_mut()) },
        NVE_PAIR_ERR_TRUNCATED
    );
    let bad_magic = [b'X', b'X', 1, 1, 0, 0, 0, 0];
    assert_eq!(
        unsafe {
            nve_pair_client_ingest_packet(client, bad_magic.as_ptr(), bad_magic.len(), ptr::null_mut())
        },
        NVE_PAIR_ERR_INVALID_MAGIC
    );

    // Server Reject(InvalidPairingToken) -> AUTH_FAILED
    let mut hello = [0u8; 64];
    let mut hlen = 0usize;
    assert_eq!(
        unsafe {
            nve_pair_client_begin_handshake(client, 42, hello.as_mut_ptr(), hello.len(), &mut hlen)
        },
        NVE_PAIR_OK
    );
    let rej_auth = encode_envelope(
        TransportMsgKind::Reject,
        0,
        &[RejectReason::InvalidPairingToken as u8, 2, 0, 0],
    );
    assert_eq!(
        unsafe {
            nve_pair_client_ingest_packet(client, rej_auth.as_ptr(), rej_auth.len(), ptr::null_mut())
        },
        NVE_PAIR_ERR_AUTH_FAILED
    );

    // HelloAck with wrong schema -> SCHEMA_MISMATCH
    assert_eq!(
        unsafe {
            nve_pair_client_begin_handshake(client, 43, hello.as_mut_ptr(), hello.len(), &mut hlen)
        },
        NVE_PAIR_OK
    );
    let (_, bad_schema_ack) = build_hello_ack_envelope("strict-token", 99, 43, 100, 64, true);
    assert_eq!(
        unsafe {
            nve_pair_client_ingest_packet(
                client,
                bad_schema_ack.as_ptr(),
                bad_schema_ack.len(),
                ptr::null_mut(),
            )
        },
        NVE_PAIR_ERR_SCHEMA_MISMATCH
    );

    // Valid handshake then wrong session_id + corrupted NVD1 checksum + base_seq mismatch
    assert_eq!(
        unsafe {
            nve_pair_client_begin_handshake(client, 44, hello.as_mut_ptr(), hello.len(), &mut hlen)
        },
        NVE_PAIR_OK
    );
    let (sid, ok_ack) = build_hello_ack_envelope("strict-token", 2, 44, 200, 64, true);
    assert_eq!(
        unsafe { nve_pair_client_ingest_packet(client, ok_ack.as_ptr(), ok_ack.len(), ptr::null_mut()) },
        NVE_PAIR_OK
    );

    let wrong_sid_ping = encode_envelope(TransportMsgKind::Ping, sid ^ 0xFFFF, &0u64.to_le_bytes());
    assert_eq!(
        unsafe {
            nve_pair_client_ingest_packet(
                client,
                wrong_sid_ping.as_ptr(),
                wrong_sid_ping.len(),
                ptr::null_mut(),
            )
        },
        NVE_PAIR_ERR_SESSION_MISMATCH
    );

    // Corrupted checksum on NVD1 packet
    let mut srv = SyncDeltaSession::new(64);
    let f1 = build_nve_frame(1, "Alpha");
    let f2 = build_nve_frame(2, "Beta");
    let mut pkt1 = srv.encode_frame(&f1, 1);
    let pkt2 = srv.encode_frame(&f2, 2);
    *pkt1.last_mut().unwrap() ^= 0xFF;
    let bad_ck = encode_envelope(TransportMsgKind::DeltaSingle, sid, &pkt1);
    assert_eq!(
        unsafe { nve_pair_client_ingest_packet(client, bad_ck.as_ptr(), bad_ck.len(), ptr::null_mut()) },
        NVE_PAIR_ERR_CHECKSUM
    );

    // Delta with base_seq != last_applied_seq (seq 2 with base_seq=1 when last_applied_seq=0,
    // craft a seq=1 delta referencing base_seq=99 to hit BaseSequenceMismatch directly).
    let bad_base = nve_interop::encode_packet(
        nve_interop::PacketKind::XorRleDelta,
        0,
        1,
        99,
        0,
        &pkt2[32..],
    );
    let bad_base_env = encode_envelope(TransportMsgKind::DeltaSingle, sid, &bad_base);
    assert_eq!(
        unsafe {
            nve_pair_client_ingest_packet(
                client,
                bad_base_env.as_ptr(),
                bad_base_env.len(),
                ptr::null_mut(),
            )
        },
        NVE_PAIR_ERR_BASE_SEQ_MISMATCH
    );

    let mut metrics = NvePairMetrics::default();
    assert_eq!(unsafe { nve_pair_client_metrics(client, &mut metrics) }, NVE_PAIR_OK);
    assert!(metrics.malformed_packets_dropped >= 4);
    assert_eq!(metrics.keyframe_resyncs, 1);

    unsafe { nve_pair_client_destroy(client) };
}
