use crate::{
    constant_time_eq, format_cert_hash_hex, format_wire_uri, parse_cert_hash_hex, parse_wire_uri,
    NveNodePatchDeltaBuilder, NveNodePatchDeltaView, NveWireChannel, NveWireClientSession,
    NveWireClientState, NveWireError, NveWireFrame, NveWireFrameRef, NveWireHeader, NveWireHello,
    NveWireHelloAck, NveWireIngestOutcome, NveWireKind, NveWireRejectReason, NveWireSlotChunk,
    NveWireTransportKind, DEFAULT_MAX_WIRE_FRAME_BYTES, NVE_WIRE_FLAG_XXH64, NVE_WIRE_HEADER_SIZE,
};

#[test]
fn wire_frame_roundtrip_and_xxh64_corruption_rejection() {
    let payload = b"direct-command-buffer-delta-bytes".to_vec();
    let frame = NveWireFrame::new(NveWireKind::CommandDelta, 42, payload.clone());
    assert_eq!(frame.channel, NveWireChannel::UiScene);
    assert_ne!(frame.flags & NVE_WIRE_FLAG_XXH64, 0);

    let encoded = frame.encode().expect("encode");
    assert_eq!(encoded.len(), NVE_WIRE_HEADER_SIZE + payload.len() + 8);

    let decoded = NveWireFrameRef::decode(&encoded, DEFAULT_MAX_WIRE_FRAME_BYTES).expect("decode");
    assert_eq!(decoded.header.channel, NveWireChannel::UiScene);
    assert_eq!(decoded.header.kind, NveWireKind::CommandDelta);
    assert_eq!(decoded.header.sequence, 42);
    assert_eq!(decoded.payload, payload.as_slice());

    // Flip 1 bit in payload -> XXH64 checksum must fail.
    let mut corrupted = encoded.clone();
    corrupted[NVE_WIRE_HEADER_SIZE + 3] ^= 0x01;
    assert!(matches!(
        NveWireFrameRef::decode(&corrupted, DEFAULT_MAX_WIRE_FRAME_BYTES),
        Err(NveWireError::ChecksumMismatch { .. })
    ));

    // SceneKeyframe omits redundant outer XXH64 trailer by default.
    let kf = NveWireFrame::new(NveWireKind::SceneKeyframe, 1, vec![0xAA; 64]);
    let kf_bytes = kf.encode().unwrap();
    assert_eq!(kf_bytes.len(), NVE_WIRE_HEADER_SIZE + 64);
    assert!(!NveWireFrameRef::decode(&kf_bytes, DEFAULT_MAX_WIRE_FRAME_BYTES)
        .unwrap()
        .header
        .has_xxh64());
}

#[test]
fn wire_header_enforces_max_frame_bytes_before_allocation() {
    let mut fake_header = [0u8; NVE_WIRE_HEADER_SIZE];
    fake_header[0..2].copy_from_slice(b"NW");
    fake_header[2] = 2;
    fake_header[3] = NveWireChannel::UiScene as u8;
    fake_header[4] = NveWireKind::CommandDelta as u8;
    fake_header[12..16].copy_from_slice(&(100_000_000u32).to_le_bytes());

    assert!(matches!(
        NveWireHeader::decode(&fake_header, 1024 * 1024),
        Err(NveWireError::FrameTooLarge { len: 100_000_000, .. })
    ));
}

#[test]
fn realistic_eight_node_live_score_patch_encodes_under_512_bytes() {
    // Realistic 8-node Live Activity sports score & status update:
    // well under Apple's 4 KB APNs push budget (< 512 B total wire frame including header + XXH64).
    let mut builder = NveNodePatchDeltaBuilder::with_capacity(8, 64);
    builder
        .set_text(101, "LAL 108")
        .set_text(102, "BOS 104")
        .set_text(103, "Q4 01:42")
        .set_text(104, "3PT MADE")
        .set_bg_rgba(105, 0x16A34AFF)
        .set_text_rgba(106, 0xF8FAFCFF)
        .set_opacity(107, 0.95, 180)
        .set_rect_width(108, 284.5, 240);

    let patch_payload = builder.encode().expect("encode 8-node patch");
    let wire_frame = NveWireFrame::new(NveWireKind::NodePatchDelta, 7, patch_payload)
        .encode()
        .expect("encode wire frame");

    assert!(
        wire_frame.len() < 512,
        "8-node live score patch wire frame must be < 512 B (got {} B)",
        wire_frame.len()
    );

    let decoded_frame =
        NveWireFrameRef::decode(&wire_frame, DEFAULT_MAX_WIRE_FRAME_BYTES).expect("decode frame");
    assert_eq!(decoded_frame.header.kind, NveWireKind::NodePatchDelta);

    let view = NveNodePatchDeltaView::decode(decoded_frame.payload).expect("decode patch view");
    assert_eq!(view.patches().len(), 8);
    assert_eq!(view.patch_str(&view.patches()[0]).unwrap(), "LAL 108");
    assert_eq!(view.patch_str(&view.patches()[1]).unwrap(), "BOS 104");
    assert_eq!(view.patch_str(&view.patches()[2]).unwrap(), "Q4 01:42");
    assert_eq!(view.patch_str(&view.patches()[3]).unwrap(), "3PT MADE");
    assert_eq!(view.patches()[4].value_lo, 0x16A34AFF);
    assert_eq!(view.patches()[6].as_f32(), 0.95);
    assert_eq!(view.patches()[7].as_f32(), 284.5);
    assert_eq!(view.patches()[7].transition_ms, 240);
}

#[test]
fn handshake_slot_chunk_uri_and_session_roundtrip() {
    let hello = NveWireHello::new("nve_secret_bearer_token", "ios-test-client");
    let encoded_hello = hello.encode().unwrap();
    let decoded_hello = NveWireHello::decode(&encoded_hello).unwrap();
    assert_eq!(decoded_hello, hello);
    assert!(constant_time_eq(b"secret-123", b"secret-123"));
    assert!(!constant_time_eq(b"secret-123", b"secret-124"));
    assert!(!constant_time_eq(b"short", b"longer"));

    let chunk = NveWireSlotChunk {
        slot_id: 9,
        chunk_index: 2,
        total_chunks: 4,
        data: b"streaming-genui-fragment",
    };
    let chunk_bytes = chunk.encode();
    assert_eq!(NveWireSlotChunk::decode(&chunk_bytes).unwrap(), chunk);

    let hash = [0xABu8; 32];
    let hex = format_cert_hash_hex(&hash);
    assert_eq!(parse_cert_hash_hex(&hex).unwrap(), hash);

    let uri_str = format_wire_uri("127.0.0.1", 8091, 8092, "tok-xyz", Some(&hash), 393, 852);
    let parsed = parse_wire_uri(&uri_str).unwrap();
    assert_eq!(parsed.host, "127.0.0.1");
    assert_eq!(parsed.quic_port, Some(8091));
    assert_eq!(parsed.wss_port, Some(8092));
    assert_eq!(parsed.token, "tok-xyz");
    assert_eq!(parsed.cert_sha256, Some(hash));
    assert_eq!(parsed.transport, NveWireTransportKind::Auto);

    let mut session = NveWireClientSession::new("tok-xyz", "wasm");
    let _hello_frame = session.build_hello().unwrap();
    assert_eq!(*session.state(), NveWireClientState::Connecting);

    let ack_frame = NveWireFrame::new(
        NveWireKind::HelloAck,
        1,
        NveWireHelloAck {
            session_id: 77,
            schema_version: 2,
            server_capabilities: 0x0F,
        }
        .encode()
        .to_vec(),
    )
    .encode()
    .unwrap();
    assert!(matches!(
        session.ingest_frame_bytes(&ack_frame).unwrap(),
        NveWireIngestOutcome::Connected { session_id: 77, .. }
    ));
    assert!(session.is_connected());
    assert_eq!(session.session_id(), Some(77));

    let ping_frame = NveWireFrame::new(NveWireKind::Ping, 2, b"ping!".to_vec())
        .encode()
        .unwrap();
    assert_eq!(
        session.ingest_frame_bytes(&ping_frame).unwrap(),
        NveWireIngestOutcome::Ping
    );
    let pong = session.pop_outbound().expect("auto pong enqueued");
    assert_eq!(
        NveWireFrameRef::decode(&pong, DEFAULT_MAX_WIRE_FRAME_BYTES)
            .unwrap()
            .header
            .kind,
        NveWireKind::Pong
    );

    let reject_frame = NveWireFrame::new(
        NveWireKind::Reject,
        3,
        NveWireRejectReason::Unauthorized.encode("bad bearer token"),
    )
    .encode()
    .unwrap();
    assert!(matches!(
        session.ingest_frame_bytes(&reject_frame).unwrap(),
        NveWireIngestOutcome::Rejected {
            reason: NveWireRejectReason::Unauthorized,
            ..
        }
    ));
}
