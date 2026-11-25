//! Production live network transport for NVE's zero-copy `NVD1` binary delta protocol.
//!
//! Wires [`SyncDeltaSession`](crate::sync_delta::SyncDeltaSession) directly onto two real socket
//! transports with zero additional external runtime dependencies:
//!
//! 1. **Low-latency UDP datagram transport (`UdpDeltaServer` / `UdpDeltaClient`)**:
//!    - Strict MTU-safe packetization (`<= 1200` bytes per datagram) for QUIC/LAN/Wi-Fi paths.
//!    - Zero-copy fast path (`DeltaSingle`) for incremental `NVD1` deltas (`<= 1192` bytes) and
//!      bounded fragmentation/reassembly (`DeltaFrag` + selective `FragNack`) for large keyframes.
//!    - Automatic background SACK/NACK/RTO retransmit pump, heartbeat Ping/Pong RTT µs estimator,
//!      graceful peer reconnect, automatic keyframe resync, and bidirectional `InputEvents` backchannel.
//!
//! 2. **Framed binary stream transport (`TcpDeltaServer` / `TcpDeltaClient`) + RFC 6455 WebSocket**:
//!    - Length-prefixed binary TCP stream (`StreamFraming::RawLengthPrefixed`) with pre-allocation
//!      OOM-bomb length checks.
//!    - Zero-dependency RFC 6455 binary WebSocket handshake (SHA-1 + Base64) and masked/unmasked
//!      frame codec (`StreamFraming::WebSocketBinary`), auto-detected by `TcpDeltaServer` on accept.
//!    - Seamless mid-stream reconnect with automatic server keyframe resync and bidirectional
//!      `InputEvents` backchannel.
//!
//! 3. **Deterministic Pairing & Live Telemetry (`PairingConfig`, `PairingMetrics`)**:
//!    - Domain-separated 128-bit pairing token verification + schema version check + deterministic
//!      mutual `session_id` negotiation.
//!    - Real-time 1-second sliding-window wire bytes/sec & packets/sec meter, EWMA RTT µs, and
//!      remote input event counters.

mod frag;
mod metrics;
mod protocol;
mod tcp;
mod udp;
mod ws_codec;

pub use frag::packetize_udp_datagrams;
pub use metrics::PairingMetrics;
pub use protocol::{
    derive_session_id, derive_token_proof, PairingConfig, RejectReason, SyncTransportError,
    TransportError, TransportMsgKind, DEFAULT_MAX_WIRE_FRAME_BYTES, DEFAULT_SCHEMA_VERSION,
    FRAG_SUBHEADER_SIZE, MAX_ALLOWED_WIRE_FRAME_BYTES, MAX_INPUT_EVENT_BYTES,
    MAX_UDP_DATAGRAM_BYTES, MAX_UDP_FRAG_CHUNK, MAX_UDP_SINGLE_PAYLOAD, TRANSPORT_HEADER_SIZE,
    TRANSPORT_MAGIC, TRANSPORT_PROTOCOL_VERSION,
};
pub use tcp::{TcpDeltaClient, TcpDeltaServer};
pub use udp::{UdpDeltaClient, UdpDeltaServer};
pub use ws_codec::{
    compute_ws_accept_key, decode_ws_frame_slice, encode_ws_frame, StreamFraming, WsFrame, WsOpcode,
};

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::sync_delta::SyncDeltaSession;

    use super::frag::FragReassembler;
    use super::protocol::{encode_envelope, validate_input_events_payload, TransportEnvelopeRef};
    use super::*;

    #[test]
    fn rfc6455_sec_websocket_accept_vector_matches_spec() {
        // Exact test vector from RFC 6455 Section 1.3.
        assert_eq!(
            compute_ws_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn ws_frame_codec_roundtrips_small_medium_large_masked_and_unmasked() {
        let sizes = [0usize, 1, 125, 126, 512, 65_535, 70_000];
        for (idx, &size) in sizes.iter().enumerate() {
            let payload: Vec<u8> = (0..size).map(|i| ((i * 31 + idx) & 0xFF) as u8).collect();
            let mask = (idx % 2 == 0).then_some([0x37, 0xFA, 0x21, 0x3D]);
            let encoded = encode_ws_frame(true, WsOpcode::Binary, &payload, mask);
            let (decoded, consumed) = decode_ws_frame_slice(&encoded, 128 * 1024).unwrap();
            assert_eq!(consumed, encoded.len());
            assert!(decoded.fin);
            assert_eq!(decoded.opcode, WsOpcode::Binary);
            assert_eq!(decoded.masked, mask.is_some());
            assert_eq!(decoded.payload, payload);
        }

        // OOM-bomb 64-bit WebSocket length must be rejected before allocation.
        let mut bomb = vec![0x82, 0x7F]; // FIN + Binary, 64-bit length
        bomb.extend_from_slice(&(1024u64 * 1024 * 1024).to_be_bytes());
        assert!(matches!(
            decode_ws_frame_slice(&bomb, 64 * 1024),
            Err(TransportError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn udp_packetizer_enforces_1200_byte_mtu_and_reassembles_out_of_order() {
        let mut session = SyncDeltaSession::default();
        let big_frame = vec![0x5Au8; 6_500];
        let nvd_pkt = session.encode_frame(&big_frame, 0);
        let dgrams = packetize_udp_datagrams(0x1234_5678, &nvd_pkt);
        assert!(dgrams.len() >= 6);
        for d in &dgrams {
            assert!(
                d.len() <= MAX_UDP_DATAGRAM_BYTES,
                "datagram {} exceeded MTU {}",
                d.len(),
                MAX_UDP_DATAGRAM_BYTES
            );
        }

        // Feed fragments in reverse order to FragReassembler -> exact original NVD1 packet.
        let mut reasm = FragReassembler::new(DEFAULT_MAX_WIRE_FRAME_BYTES);
        let now = Instant::now();
        let mut completed = None;
        for d in dgrams.iter().rev() {
            let env = TransportEnvelopeRef::parse(d).unwrap();
            assert_eq!(env.kind, TransportMsgKind::DeltaFrag);
            let (done, _) = reasm.push(env.payload, now).unwrap();
            if done.is_some() {
                completed = done;
            }
        }
        assert_eq!(completed.as_deref(), Some(nvd_pkt.as_slice()));
    }

    #[test]
    fn deterministic_pairing_token_proof_and_session_id() {
        let p1 = derive_token_proof("studio-secret-token", 1);
        let p2 = derive_token_proof("studio-secret-token", 1);
        let p_other_token = derive_token_proof("wrong-token", 1);
        let p_other_schema = derive_token_proof("studio-secret-token", 2);
        assert_eq!(p1, p2);
        assert_ne!(p1, p_other_token);
        assert_ne!(p1, p_other_schema);

        let sid1 = derive_session_id("studio-secret-token", 1, 111, 222);
        let sid2 = derive_session_id("studio-secret-token", 1, 111, 222);
        let sid3 = derive_session_id("studio-secret-token", 1, 111, 223);
        assert_eq!(sid1, sid2);
        assert_ne!(sid1, 0);
        assert_ne!(sid1, sid3);
    }

    #[test]
    fn input_events_wire_kind_and_payload_validation() {
        assert_eq!(TransportMsgKind::InputEvents as u8, 11);
        assert_eq!(
            TransportMsgKind::from_u8(11).unwrap(),
            TransportMsgKind::InputEvents
        );

        let ev = nve_abi::encode_events(&[nve_abi::NativeEvent {
            surface_id: 1,
            node_id: 7,
            timestamp_us: 42,
            kind: nve_abi::NativeEventKind::Press,
            payload: nve_abi::EventPayload::Point { x: 10.0, y: 20.0 },
        }]);
        assert!(validate_input_events_payload(&ev, MAX_INPUT_EVENT_BYTES).is_ok());

        let env_bytes = encode_envelope(TransportMsgKind::InputEvents, 0xABCD, &ev);
        let parsed = TransportEnvelopeRef::parse(&env_bytes).unwrap();
        assert_eq!(parsed.kind, TransportMsgKind::InputEvents);
        assert_eq!(parsed.session_id, 0xABCD);
        assert_eq!(parsed.payload, ev.as_slice());

        // Reject empty, truncated, corrupt count bomb, and oversized payloads.
        assert!(matches!(
            validate_input_events_payload(&[], MAX_INPUT_EVENT_BYTES),
            Err(TransportError::MalformedFrame(_))
        ));
        assert!(matches!(
            validate_input_events_payload(&[1, 0], MAX_INPUT_EVENT_BYTES),
            Err(TransportError::MalformedFrame(_))
        ));
        assert!(matches!(
            validate_input_events_payload(&u32::MAX.to_le_bytes(), MAX_INPUT_EVENT_BYTES),
            Err(TransportError::MalformedFrame(_))
        ));
        assert!(matches!(
            validate_input_events_payload(&ev, 8),
            Err(TransportError::FrameTooLarge { .. })
        ));
    }
}
