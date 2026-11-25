//! Shared zero-JSON `NVT1`/`NVD1` pairing state machine and `#[repr(C)]` C ABI.
//!
//! Wraps `nve-interop`'s `SyncDeltaSession`, `PairingConfig`, token proof/session derivation,
//! and `InputEvents = 11` framing so every native host (Apple, Windows, Android, Qt, GTK,
//! SwiftUI, Flutter) links one battle-tested pairing implementation.

pub mod c_api;
mod uri;

use std::collections::VecDeque;

use nve_abi::{EventPayload, NativeEvent, NativeEventKind, decode_events, encode_events};
use nve_interop::{
    DEFAULT_SCHEMA_VERSION, DELTA_HEADER_SIZE, MAX_INPUT_EVENT_BYTES, PairingConfig, RejectReason,
    SYNC_DELTA_MAGIC, SyncDeltaError, SyncDeltaSession, TRANSPORT_HEADER_SIZE, TRANSPORT_MAGIC,
    TRANSPORT_PROTOCOL_VERSION, TransportMsgKind, derive_session_id, derive_token_proof,
};

pub use c_api::{
    NVE_PAIR_ERR_AUTH_FAILED, NVE_PAIR_ERR_BASE_SEQ_MISMATCH, NVE_PAIR_ERR_BUFFER_TOO_SMALL,
    NVE_PAIR_ERR_CHECKSUM, NVE_PAIR_ERR_EMPTY_QUEUE, NVE_PAIR_ERR_FRAME_TOO_LARGE,
    NVE_PAIR_ERR_HANDSHAKE_REJECTED, NVE_PAIR_ERR_INTERNAL_PANIC, NVE_PAIR_ERR_INVALID_KIND,
    NVE_PAIR_ERR_INVALID_MAGIC, NVE_PAIR_ERR_INVALID_URI, NVE_PAIR_ERR_INVALID_UTF8,
    NVE_PAIR_ERR_MALFORMED, NVE_PAIR_ERR_NOT_PAIRED, NVE_PAIR_ERR_NULL_ARG,
    NVE_PAIR_ERR_SCHEMA_MISMATCH, NVE_PAIR_ERR_SESSION_MISMATCH, NVE_PAIR_ERR_TRUNCATED,
    NVE_PAIR_ERR_UNSUPPORTED_VERSION, NVE_PAIR_OK, NvePairByteSlice, NvePairClient,
    NvePairClientConfig, NvePairIngestOutcome, NvePairInputEvent, NvePairMetrics,
    NvePairParsedUri, nve_pair_client_begin_handshake, nve_pair_client_create,
    nve_pair_client_create_from_uri, nve_pair_client_current_frame, nve_pair_client_destroy,
    nve_pair_client_encode_input_event, nve_pair_client_ingest_packet, nve_pair_client_metrics,
    nve_pair_client_pop_outbound_packet, nve_pair_parse_uri,
};
pub use uri::{PairTransportKind, PairUriError, ParsedPairUri, parse_pair_uri};

/// Pairing client lifecycle state matching `NvePairState` in `nve_pair_abi.h`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairClientState {
    Disconnected = 0,
    Connecting = 1,
    Paired = 2,
    Rejected = 3,
    Error = 4,
}

/// Summary of ingesting one wire packet into [`PairClientEngine`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestSummary {
    pub applied_frame_count: u32,
    pub latest_seq: u32,
    pub outbound_packet_count: u32,
    pub state: PairClientState,
}

/// Sans-I/O `NVT1` + `NVD1` pairing client state machine backed by `nve_interop::SyncDeltaSession`.
#[derive(Debug)]
pub struct PairClientEngine {
    config: PairingConfig,
    delta: SyncDeltaSession,
    state: PairClientState,
    session_id: u32,
    pending_client_nonce: Option<u64>,
    outbound_queue: VecDeque<Vec<u8>>,
    metrics: NvePairMetrics,
}

impl PairClientEngine {
    pub fn new(config: PairingConfig) -> Self {
        let delta = SyncDeltaSession::new(config.max_window);
        Self {
            config,
            delta,
            state: PairClientState::Disconnected,
            session_id: 0,
            pending_client_nonce: None,
            outbound_queue: VecDeque::new(),
            metrics: NvePairMetrics::default(),
        }
    }

    pub fn from_uri(raw: &str, fallback_token: Option<&str>) -> Result<Self, PairUriError> {
        let parsed = parse_pair_uri(raw, fallback_token)?;
        Ok(Self::new(parsed.to_pairing_config()))
    }

    pub fn config(&self) -> &PairingConfig {
        &self.config
    }

    pub fn state(&self) -> PairClientState {
        self.state
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    pub fn current_frame(&self) -> &[u8] {
        self.delta.last_applied_frame()
    }

    pub fn metrics(&self) -> NvePairMetrics {
        let mut m = self.metrics;
        m.session_id = self.session_id;
        m.last_applied_seq = self.delta.last_applied_seq();
        m.state = self.state as u32;
        m.reorder_buffer_len = self.delta.reorder_buffer_len() as u32;
        m
    }

    pub fn pop_outbound_packet(&mut self) -> Option<Vec<u8>> {
        self.outbound_queue.pop_front()
    }

    pub fn peek_outbound_packet(&self) -> Option<&[u8]> {
        self.outbound_queue.front().map(Vec::as_slice)
    }

    pub fn outbound_len(&self) -> usize {
        self.outbound_queue.len()
    }

    /// Builds an `NVT1` `Hello` envelope for the configured pairing token and schema version.
    pub fn begin_handshake(&mut self, client_nonce: u64) -> Vec<u8> {
        self.pending_client_nonce = Some(client_nonce);
        self.state = PairClientState::Connecting;
        let proof = derive_token_proof(&self.config.pairing_token, self.config.schema_version);
        let mut hello = [0u8; 32];
        hello[0..2].copy_from_slice(&self.config.schema_version.to_le_bytes());
        hello[2..4].copy_from_slice(&(self.config.max_window.clamp(4, 1024) as u16).to_le_bytes());
        hello[4..8].copy_from_slice(&self.delta.last_applied_seq().to_le_bytes());
        hello[8..16].copy_from_slice(&client_nonce.to_le_bytes());
        hello[16..32].copy_from_slice(&proof);
        let env = encode_envelope(TransportMsgKind::Hello, 0, &hello);
        self.metrics.wire_bytes_sent += env.len() as u64;
        env
    }

    /// Ingests either a full `NVT1` envelope (`b"NT"`) or a standalone `NVD1` packet.
    pub fn ingest_packet(&mut self, packet: &[u8]) -> Result<IngestSummary, i32> {
        if packet.len() > self.config.max_frame_bytes.saturating_add(DELTA_HEADER_SIZE + TRANSPORT_HEADER_SIZE) {
            self.metrics.malformed_packets_dropped += 1;
            return Err(NVE_PAIR_ERR_FRAME_TOO_LARGE);
        }
        self.metrics.wire_bytes_received += packet.len() as u64;

        if packet.len() >= 4 && packet[0..4] == SYNC_DELTA_MAGIC {
            return self.ingest_delta_payload(packet, false);
        }

        if packet.len() < TRANSPORT_HEADER_SIZE {
            self.metrics.malformed_packets_dropped += 1;
            return Err(NVE_PAIR_ERR_TRUNCATED);
        }
        if packet[0..2] != TRANSPORT_MAGIC {
            self.metrics.malformed_packets_dropped += 1;
            return Err(NVE_PAIR_ERR_INVALID_MAGIC);
        }
        if packet[2] != TRANSPORT_PROTOCOL_VERSION {
            self.metrics.malformed_packets_dropped += 1;
            return Err(NVE_PAIR_ERR_UNSUPPORTED_VERSION);
        }
        let Ok(kind) = TransportMsgKind::from_u8(packet[3]) else {
            self.metrics.malformed_packets_dropped += 1;
            return Err(NVE_PAIR_ERR_INVALID_KIND);
        };
        let session_id = u32::from_le_bytes(packet[4..8].try_into().unwrap());
        let payload = &packet[TRANSPORT_HEADER_SIZE..];

        match kind {
            TransportMsgKind::HelloAck => self.handle_hello_ack(session_id, payload),
            TransportMsgKind::Reject => self.handle_reject(payload),
            TransportMsgKind::DeltaSingle => {
                self.require_paired_session(session_id)?;
                self.ingest_delta_payload(payload, true)
            }
            TransportMsgKind::Ping => {
                self.require_paired_session(session_id)?;
                if payload.len() != 8 {
                    self.metrics.malformed_packets_dropped += 1;
                    return Err(NVE_PAIR_ERR_MALFORMED);
                }
                let pong = encode_envelope(TransportMsgKind::Pong, self.session_id, payload);
                self.push_outbound(pong);
                Ok(self.summary(0, 1))
            }
            TransportMsgKind::Pong => {
                self.require_paired_session(session_id)?;
                if payload.len() != 8 {
                    self.metrics.malformed_packets_dropped += 1;
                    return Err(NVE_PAIR_ERR_MALFORMED);
                }
                self.metrics.last_pong_timestamp_us =
                    u64::from_le_bytes(payload.try_into().unwrap());
                Ok(self.summary(0, 0))
            }
            TransportMsgKind::Bye => {
                self.require_paired_session(session_id)?;
                self.state = PairClientState::Disconnected;
                self.session_id = 0;
                self.pending_client_nonce = None;
                Ok(self.summary(0, 0))
            }
            _ => {
                self.metrics.malformed_packets_dropped += 1;
                Err(NVE_PAIR_ERR_INVALID_KIND)
            }
        }
    }

    /// Encodes a single `NativeEvent` as an `NVT1` `InputEvents = 11` envelope (`EventRing`).
    pub fn encode_input_event(&mut self, event: &NativeEvent) -> Result<Vec<u8>, i32> {
        self.encode_input_events(std::slice::from_ref(event))
    }

    /// Encodes a batch of `NativeEvent`s as an `NVT1` `InputEvents = 11` envelope (`EventRing`).
    pub fn encode_input_events(&mut self, events: &[NativeEvent]) -> Result<Vec<u8>, i32> {
        if self.state != PairClientState::Paired || self.session_id == 0 {
            return Err(NVE_PAIR_ERR_NOT_PAIRED);
        }
        if events.is_empty() {
            return Err(NVE_PAIR_ERR_MALFORMED);
        }
        let ring = encode_events(events);
        if ring.len() > MAX_INPUT_EVENT_BYTES {
            return Err(NVE_PAIR_ERR_FRAME_TOO_LARGE);
        }
        if decode_events(&ring).is_err() {
            return Err(NVE_PAIR_ERR_MALFORMED);
        }
        let env = encode_envelope(TransportMsgKind::InputEvents, self.session_id, &ring);
        self.metrics.events_sent += events.len() as u64;
        self.metrics.wire_bytes_sent += env.len() as u64;
        Ok(env)
    }

    /// Encodes an `NVT1` `KeyframeRequest` envelope for the active paired session.
    pub fn encode_keyframe_request(&mut self, reason: u8) -> Result<Vec<u8>, i32> {
        if self.state != PairClientState::Paired || self.session_id == 0 {
            return Err(NVE_PAIR_ERR_NOT_PAIRED);
        }
        let mut payload = [0u8; 8];
        payload[0..4].copy_from_slice(&self.delta.last_applied_seq().to_le_bytes());
        payload[4] = reason;
        let env = encode_envelope(TransportMsgKind::KeyframeRequest, self.session_id, &payload);
        self.metrics.keyframe_resyncs += 1;
        self.metrics.wire_bytes_sent += env.len() as u64;
        Ok(env)
    }

    /// Encodes an `NVT1` `Bye` envelope for the active paired session.
    pub fn encode_bye(&mut self) -> Result<Vec<u8>, i32> {
        if self.state != PairClientState::Paired || self.session_id == 0 {
            return Err(NVE_PAIR_ERR_NOT_PAIRED);
        }
        let env = encode_envelope(TransportMsgKind::Bye, self.session_id, &[]);
        self.metrics.wire_bytes_sent += env.len() as u64;
        Ok(env)
    }

    fn require_paired_session(&mut self, incoming_sid: u32) -> Result<(), i32> {
        if self.state != PairClientState::Paired || self.session_id == 0 {
            self.metrics.malformed_packets_dropped += 1;
            return Err(NVE_PAIR_ERR_NOT_PAIRED);
        }
        if incoming_sid != self.session_id {
            self.metrics.malformed_packets_dropped += 1;
            return Err(NVE_PAIR_ERR_SESSION_MISMATCH);
        }
        Ok(())
    }

    fn handle_hello_ack(&mut self, session_id: u32, payload: &[u8]) -> Result<IngestSummary, i32> {
        let Some(client_nonce) = self.pending_client_nonce else {
            self.metrics.malformed_packets_dropped += 1;
            return Err(NVE_PAIR_ERR_MALFORMED);
        };
        if payload.len() != 20 {
            self.metrics.malformed_packets_dropped += 1;
            return Err(NVE_PAIR_ERR_MALFORMED);
        }
        let schema_version = u16::from_le_bytes(payload[0..2].try_into().unwrap());
        let server_max_window = u16::from_le_bytes(payload[2..4].try_into().unwrap());
        let server_nonce = u64::from_le_bytes(payload[8..16].try_into().unwrap());
        let resync_required = payload[16] != 0;

        if schema_version != self.config.schema_version {
            self.state = PairClientState::Error;
            return Err(NVE_PAIR_ERR_SCHEMA_MISMATCH);
        }
        let expected_sid = derive_session_id(
            &self.config.pairing_token,
            self.config.schema_version,
            client_nonce,
            server_nonce,
        );
        if session_id != expected_sid {
            self.state = PairClientState::Error;
            return Err(NVE_PAIR_ERR_SESSION_MISMATCH);
        }

        self.pending_client_nonce = None;
        self.session_id = session_id;
        let window = usize::from(server_max_window)
            .min(self.config.max_window)
            .clamp(4, 1024);
        if resync_required || self.delta.max_window() != window {
            self.delta = SyncDeltaSession::new(window);
        }
        self.state = PairClientState::Paired;
        Ok(self.summary(0, 0))
    }

    fn handle_reject(&mut self, payload: &[u8]) -> Result<IngestSummary, i32> {
        if payload.len() < 3 {
            self.metrics.malformed_packets_dropped += 1;
            return Err(NVE_PAIR_ERR_MALFORMED);
        }
        self.pending_client_nonce = None;
        self.state = PairClientState::Rejected;
        match RejectReason::from_u8(payload[0]) {
            Some(RejectReason::InvalidPairingToken) => Err(NVE_PAIR_ERR_AUTH_FAILED),
            Some(RejectReason::SchemaVersionMismatch) => Err(NVE_PAIR_ERR_SCHEMA_MISMATCH),
            Some(_) => Err(NVE_PAIR_ERR_HANDSHAKE_REJECTED),
            None => Err(NVE_PAIR_ERR_MALFORMED),
        }
    }

    fn ingest_delta_payload(
        &mut self,
        nvd_bytes: &[u8],
        wrap_envelope: bool,
    ) -> Result<IngestSummary, i32> {
        let packet_kind_byte = nvd_bytes.get(5).copied().unwrap_or(0);
        match self.delta.receive_packet(nvd_bytes) {
            Ok(outcome) => {
                let applied_count = outcome.applied_frames.len() as u32;
                for (_, frame) in &outcome.applied_frames {
                    self.metrics.frames_received += 1;
                    self.metrics.raw_bytes_received += frame.len() as u64;
                    if packet_kind_byte == 1 {
                        self.metrics.keyframes_received += 1;
                    } else {
                        self.metrics.deltas_received += 1;
                    }
                }
                let mut pushed = 0u32;
                if let Some(ack) = outcome.ack_packet {
                    self.metrics.acks_sent += 1;
                    let wire = if wrap_envelope {
                        encode_envelope(TransportMsgKind::DeltaSingle, self.session_id, &ack)
                    } else {
                        ack
                    };
                    self.push_outbound(wire);
                    pushed += 1;
                }
                for nack in outcome.nack_packets {
                    self.metrics.nacks_sent += 1;
                    let wire = if wrap_envelope {
                        encode_envelope(TransportMsgKind::DeltaSingle, self.session_id, &nack)
                    } else {
                        nack
                    };
                    self.push_outbound(wire);
                    pushed += 1;
                }
                Ok(self.summary(applied_count, pushed))
            }
            Err(SyncDeltaError::BaseSequenceMismatch { .. }) => {
                self.metrics.malformed_packets_dropped += 1;
                if wrap_envelope
                    && self.state == PairClientState::Paired
                    && self.session_id != 0
                    && let Ok(req) = self.encode_keyframe_request(3)
                {
                    self.outbound_queue.push_back(req);
                }
                Err(NVE_PAIR_ERR_BASE_SEQ_MISMATCH)
            }
            Err(err) => {
                self.metrics.malformed_packets_dropped += 1;
                Err(map_delta_error(err))
            }
        }
    }

    fn push_outbound(&mut self, bytes: Vec<u8>) {
        self.metrics.wire_bytes_sent += bytes.len() as u64;
        self.outbound_queue.push_back(bytes);
    }

    fn summary(&self, applied_frame_count: u32, outbound_packet_count: u32) -> IngestSummary {
        IngestSummary {
            applied_frame_count,
            latest_seq: self.delta.last_applied_seq(),
            outbound_packet_count,
            state: self.state,
        }
    }
}

/// Encodes an `NVT1` binary envelope (`b"NT"`, version 1, `kind`, little-endian `session_id`, `payload`).
pub fn encode_envelope(kind: TransportMsgKind, session_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(TRANSPORT_HEADER_SIZE + payload.len());
    out.extend_from_slice(&TRANSPORT_MAGIC);
    out.push(TRANSPORT_PROTOCOL_VERSION);
    out.push(kind as u8);
    out.extend_from_slice(&session_id.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

pub(crate) fn map_delta_error(err: SyncDeltaError) -> i32 {
    match err {
        SyncDeltaError::Truncated => NVE_PAIR_ERR_TRUNCATED,
        SyncDeltaError::InvalidMagic(_) => NVE_PAIR_ERR_INVALID_MAGIC,
        SyncDeltaError::UnsupportedVersion(_) => NVE_PAIR_ERR_UNSUPPORTED_VERSION,
        SyncDeltaError::InvalidPacketKind(_) => NVE_PAIR_ERR_INVALID_KIND,
        SyncDeltaError::ChecksumMismatch { .. } => NVE_PAIR_ERR_CHECKSUM,
        SyncDeltaError::LengthMismatch { .. } => NVE_PAIR_ERR_MALFORMED,
        SyncDeltaError::FrameTooLarge(_) => NVE_PAIR_ERR_FRAME_TOO_LARGE,
        SyncDeltaError::BaseSequenceMismatch { .. } => NVE_PAIR_ERR_BASE_SEQ_MISMATCH,
        SyncDeltaError::CorruptPayload(_) => NVE_PAIR_ERR_MALFORMED,
    }
}

pub(crate) fn build_native_event_from_c(ev: &NvePairInputEvent) -> Result<NativeEvent, i32> {
    let kind = match ev.kind {
        1 => NativeEventKind::Press,
        2 => NativeEventKind::LongPress,
        3 => NativeEventKind::Scroll,
        4 => NativeEventKind::TextChange,
        5 => NativeEventKind::Focus,
        6 => NativeEventKind::Blur,
        7 => NativeEventKind::Layout,
        8 => NativeEventKind::Custom,
        _ => return Err(NVE_PAIR_ERR_INVALID_KIND),
    };
    let payload = match kind {
        NativeEventKind::Press | NativeEventKind::LongPress => EventPayload::Point {
            x: ev.x,
            y: ev.y,
        },
        NativeEventKind::Scroll => EventPayload::Scroll {
            x: ev.x,
            y: ev.y,
            content_width: ev.width,
            content_height: ev.height,
        },
        NativeEventKind::TextChange => {
            let text = read_c_utf8(ev.text_ptr, ev.text_len)?;
            EventPayload::Text(text.to_owned())
        }
        NativeEventKind::Focus | NativeEventKind::Blur => EventPayload::Empty,
        NativeEventKind::Layout => EventPayload::Layout {
            x: ev.x,
            y: ev.y,
            width: ev.width,
            height: ev.height,
        },
        NativeEventKind::Custom => {
            let name = read_c_utf8(ev.text_ptr, ev.text_len)?;
            let data = read_c_bytes(ev.data_ptr, ev.data_len)?.to_vec();
            EventPayload::Custom {
                name: name.to_owned(),
                data,
            }
        }
    };
    Ok(NativeEvent {
        surface_id: ev.surface_id,
        node_id: ev.node_id,
        timestamp_us: ev.timestamp_us,
        kind,
        payload,
    })
}

pub(crate) fn read_c_bytes<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], i32> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err(NVE_PAIR_ERR_NULL_ARG);
    }
    // SAFETY: Caller guarantees `ptr` is valid for `len` bytes for the duration of the FFI call.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

pub(crate) fn read_c_utf8<'a>(ptr: *const u8, len: usize) -> Result<&'a str, i32> {
    let bytes = read_c_bytes(ptr, len)?;
    std::str::from_utf8(bytes).map_err(|_| NVE_PAIR_ERR_INVALID_UTF8)
}

pub(crate) fn default_schema_or(v: u16) -> u16 {
    if v == 0 { DEFAULT_SCHEMA_VERSION } else { v }
}
