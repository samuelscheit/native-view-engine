//! Binary wire transport protocol v1 (`TRANSPORT_MAGIC = b"NT"`, `TRANSPORT_PROTOCOL_VERSION = 1`)
//! constants, errors, pairing config, envelope codecs, handshake helpers, and bounded inbound
//! `EventRing` queue.

use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use nve_abi::decode_events;

use crate::error::InteropError;
use crate::sync_delta::{
    fnv1a_64_pair, SyncDeltaError, DEFAULT_SYNC_WINDOW, DELTA_HEADER_SIZE, MAX_DECODED_FRAME_BYTES,
};

/// 2-byte transport envelope magic (`b"NT"`).
pub const TRANSPORT_MAGIC: [u8; 2] = *b"NT";

/// Current binary transport envelope protocol version (`1`, paired with 2-byte magic `b"NT"`).
pub const TRANSPORT_PROTOCOL_VERSION: u8 = 1;

/// Fixed transport envelope header size (`magic[2] + version[1] + kind[1] + session_id[4]`).
pub const TRANSPORT_HEADER_SIZE: usize = 8;

/// Maximum UDP datagram size on the wire (MTU-safe for QUIC, IPv6 min MTU 1280, Wi-Fi/LAN).
pub const MAX_UDP_DATAGRAM_BYTES: usize = 1200;

/// Fragment subheader size (`frag_msg_id: u32 + frag_idx: u16 + frag_count: u16 + total_len: u32`).
pub const FRAG_SUBHEADER_SIZE: usize = 12;

/// Maximum unfragmented `NVD1` packet bytes that fit in a single `DeltaSingle` UDP datagram.
pub const MAX_UDP_SINGLE_PAYLOAD: usize = MAX_UDP_DATAGRAM_BYTES - TRANSPORT_HEADER_SIZE;

/// Maximum payload bytes carried per `DeltaFrag` UDP fragment datagram.
pub const MAX_UDP_FRAG_CHUNK: usize =
    MAX_UDP_DATAGRAM_BYTES - TRANSPORT_HEADER_SIZE - FRAG_SUBHEADER_SIZE;

/// Default application schema version negotiated during pairing handshake.
pub const DEFAULT_SCHEMA_VERSION: u16 = 1;

/// Default maximum accepted wire frame size (4 MiB).
pub const DEFAULT_MAX_WIRE_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Absolute upper bound on wire frame size (matches `MAX_DECODED_FRAME_BYTES + DELTA_HEADER_SIZE`).
pub const MAX_ALLOWED_WIRE_FRAME_BYTES: usize = MAX_DECODED_FRAME_BYTES + DELTA_HEADER_SIZE;

/// Strict upper bound on a single inbound/outbound `InputEvents` `EventRing` payload (64 KiB).
pub const MAX_INPUT_EVENT_BYTES: usize = 64 * 1024;

/// Maximum queued inbound `InputEvents` batches buffered per server to prevent unbounded memory growth.
const MAX_QUEUED_EVENT_BATCHES: usize = 1024;

/// Minimum encoded byte size of a single `NativeEvent` (`surface_id[4] + node_id[4] + ts[8] + kind[1] + tag[1]`).
const MIN_ENCODED_EVENT_BYTES: usize = 18;

/// Global monotonic nonce generator for handshakes and WebSocket mask keys.
static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_nonce(seed_hint: u64) -> u64 {
    let seq = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut x = seq
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(seed_hint ^ 0xA076_1D64_78BD_642F);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

// =============================================================================
// Error Types
// =============================================================================

/// Rejection reason returned in a `Reject` handshake response.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    InvalidPairingToken = 1,
    SchemaVersionMismatch = 2,
    UnsupportedProtocolVersion = 3,
    ServerBusy = 4,
}

impl RejectReason {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::InvalidPairingToken),
            2 => Some(Self::SchemaVersionMismatch),
            3 => Some(Self::UnsupportedProtocolVersion),
            4 => Some(Self::ServerBusy),
            _ => None,
        }
    }
}

/// Typed error returned by live delta transports, framing codecs, and pairing handshakes.
#[derive(Debug)]
pub enum TransportError {
    Io(io::Error),
    Delta(SyncDeltaError),
    TruncatedFrame,
    InvalidMagic([u8; 2]),
    UnsupportedProtocolVersion(u8),
    InvalidMessageKind(u8),
    MalformedFrame(&'static str),
    FrameTooLarge { declared: usize, max: usize },
    AuthenticationFailed,
    SchemaVersionMismatch { expected: u16, actual: u16 },
    SessionIdMismatch { expected: u32, actual: u32 },
    HandshakeTimeout,
    HandshakeRejected(RejectReason),
    Disconnected,
    WebSocketProtocol(&'static str),
}

/// Alias for [`TransportError`] for ergonomic naming alongside `SyncDeltaError`.
pub type SyncTransportError = TransportError;

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "transport I/O error: {err}"),
            Self::Delta(err) => write!(f, "delta codec error: {err}"),
            Self::TruncatedFrame => f.write_str("truncated transport frame"),
            Self::InvalidMagic(m) => write!(f, "invalid transport magic: {m:?}"),
            Self::UnsupportedProtocolVersion(v) => {
                write!(f, "unsupported transport protocol version: {v}")
            }
            Self::InvalidMessageKind(k) => write!(f, "invalid transport message kind: {k}"),
            Self::MalformedFrame(msg) => write!(f, "malformed transport frame: {msg}"),
            Self::FrameTooLarge { declared, max } => {
                write!(f, "declared frame size {declared} exceeds max limit {max}")
            }
            Self::AuthenticationFailed => f.write_str("pairing token authentication failed"),
            Self::SchemaVersionMismatch { expected, actual } => write!(
                f,
                "schema version mismatch: expected {expected}, got {actual}"
            ),
            Self::SessionIdMismatch { expected, actual } => write!(
                f,
                "negotiated session id mismatch: expected {expected:#x}, got {actual:#x}"
            ),
            Self::HandshakeTimeout => f.write_str("pairing handshake timed out"),
            Self::HandshakeRejected(reason) => write!(f, "pairing handshake rejected: {reason:?}"),
            Self::Disconnected => f.write_str("transport peer disconnected"),
            Self::WebSocketProtocol(msg) => write!(f, "WebSocket protocol error: {msg}"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<io::Error> for TransportError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<SyncDeltaError> for TransportError {
    fn from(err: SyncDeltaError) -> Self {
        Self::Delta(err)
    }
}

impl From<TransportError> for InteropError {
    fn from(err: TransportError) -> Self {
        Self::SyncTransport(err.to_string())
    }
}

// =============================================================================
// Pairing Configuration, Token Proof & Session ID Negotiation
// =============================================================================

/// Configuration for pairing handshake, window bounds, retransmit timing, and telemetry.
#[derive(Debug, Clone)]
pub struct PairingConfig {
    pub pairing_token: String,
    pub schema_version: u16,
    pub max_window: usize,
    pub max_frame_bytes: usize,
    pub rto_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub handshake_timeout_ms: u64,
    pub metrics_window_ms: u64,
}

impl PairingConfig {
    pub fn new(pairing_token: impl Into<String>) -> Self {
        Self {
            pairing_token: pairing_token.into(),
            schema_version: DEFAULT_SCHEMA_VERSION,
            max_window: DEFAULT_SYNC_WINDOW,
            max_frame_bytes: DEFAULT_MAX_WIRE_FRAME_BYTES,
            rto_ms: 55,
            heartbeat_interval_ms: 250,
            handshake_timeout_ms: 2_000,
            metrics_window_ms: 1_000,
        }
    }

    pub fn with_schema_version(mut self, schema_version: u16) -> Self {
        self.schema_version = schema_version;
        self
    }

    pub fn with_max_window(mut self, max_window: usize) -> Self {
        self.max_window = max_window.clamp(4, 1024);
        self
    }

    pub fn with_max_frame_bytes(mut self, max_frame_bytes: usize) -> Self {
        self.max_frame_bytes = max_frame_bytes.clamp(64, MAX_ALLOWED_WIRE_FRAME_BYTES);
        self
    }

    pub fn with_rto_ms(mut self, rto_ms: u64) -> Self {
        self.rto_ms = rto_ms.max(5);
        self
    }

    pub fn with_heartbeat_interval_ms(mut self, heartbeat_interval_ms: u64) -> Self {
        self.heartbeat_interval_ms = heartbeat_interval_ms.max(10);
        self
    }

    pub fn with_handshake_timeout_ms(mut self, handshake_timeout_ms: u64) -> Self {
        self.handshake_timeout_ms = handshake_timeout_ms.max(50);
        self
    }

    pub fn with_metrics_window_ms(mut self, metrics_window_ms: u64) -> Self {
        self.metrics_window_ms = metrics_window_ms.max(50);
        self
    }

    #[inline]
    pub(crate) fn clamped_max_frame_bytes(&self) -> usize {
        self.max_frame_bytes.clamp(64, MAX_ALLOWED_WIRE_FRAME_BYTES)
    }

    #[inline]
    pub(crate) fn clamped_max_event_bytes(&self) -> usize {
        self.clamped_max_frame_bytes().min(MAX_INPUT_EVENT_BYTES)
    }

    #[inline]
    pub(crate) fn clamped_window(&self) -> usize {
        self.max_window.clamp(4, 1024)
    }
}

/// Computes a deterministic 128-bit domain-separated proof for `(pairing_token, schema_version)`.
pub fn derive_token_proof(pairing_token: &str, schema_version: u16) -> [u8; 16] {
    let ver_bytes = schema_version.to_le_bytes();
    let mut prefix_a = Vec::with_capacity(16);
    prefix_a.extend_from_slice(b"NVE-PAIR-A-v1:");
    prefix_a.extend_from_slice(&ver_bytes);
    let h1 = fnv1a_64_pair(&prefix_a, pairing_token.as_bytes());

    let mut suffix_b = Vec::with_capacity(16);
    suffix_b.extend_from_slice(b":NVE-PAIR-B-v1");
    suffix_b.extend_from_slice(&ver_bytes);
    let h2 = fnv1a_64_pair(pairing_token.as_bytes(), &suffix_b);

    let mut proof = [0u8; 16];
    proof[0..8].copy_from_slice(&h1.to_le_bytes());
    proof[8..16].copy_from_slice(&h2.to_le_bytes());
    proof
}

/// Deterministically derives a non-zero 32-bit `session_id` agreed upon by both client and server.
pub fn derive_session_id(
    pairing_token: &str,
    schema_version: u16,
    client_nonce: u64,
    server_nonce: u64,
) -> u32 {
    let mut material = [0u8; 18];
    material[0..2].copy_from_slice(&schema_version.to_le_bytes());
    material[2..10].copy_from_slice(&client_nonce.to_le_bytes());
    material[10..18].copy_from_slice(&server_nonce.to_le_bytes());
    let h = fnv1a_64_pair(pairing_token.as_bytes(), &material);
    let folded = ((h ^ (h >> 32)) as u32) ^ 0x5A3C_96F1;
    if folded == 0 {
        1
    } else {
        folded
    }
}

#[inline]
pub(crate) fn ct_eq_16(a: &[u8; 16], b: &[u8; 16]) -> bool {
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

// =============================================================================
// Transport Message Codec (`b"NT"` + version 1 Zero-Copy Envelope)
// =============================================================================

/// Transport envelope message kind.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMsgKind {
    Hello = 1,
    HelloAck = 2,
    Reject = 3,
    DeltaSingle = 4,
    DeltaFrag = 5,
    FragNack = 6,
    Ping = 7,
    Pong = 8,
    KeyframeRequest = 9,
    Bye = 10,
    /// Raw binary encoded `nve_abi::EventRing` batch sent from paired client to host.
    InputEvents = 11,
}

impl TransportMsgKind {
    pub const fn from_u8(v: u8) -> Result<Self, TransportError> {
        match v {
            1 => Ok(Self::Hello),
            2 => Ok(Self::HelloAck),
            3 => Ok(Self::Reject),
            4 => Ok(Self::DeltaSingle),
            5 => Ok(Self::DeltaFrag),
            6 => Ok(Self::FragNack),
            7 => Ok(Self::Ping),
            8 => Ok(Self::Pong),
            9 => Ok(Self::KeyframeRequest),
            10 => Ok(Self::Bye),
            11 => Ok(Self::InputEvents),
            other => Err(TransportError::InvalidMessageKind(other)),
        }
    }
}

/// Zero-copy parsed view of an incoming binary transport message (`b"NT"` header + version 1).
#[derive(Debug, Clone, Copy)]
pub(crate) struct TransportEnvelopeRef<'a> {
    pub kind: TransportMsgKind,
    pub session_id: u32,
    pub payload: &'a [u8],
}

impl<'a> TransportEnvelopeRef<'a> {
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self, TransportError> {
        if bytes.len() < TRANSPORT_HEADER_SIZE {
            return Err(TransportError::TruncatedFrame);
        }
        let magic = [bytes[0], bytes[1]];
        if magic != TRANSPORT_MAGIC {
            return Err(TransportError::InvalidMagic(magic));
        }
        let version = bytes[2];
        if version != TRANSPORT_PROTOCOL_VERSION {
            return Err(TransportError::UnsupportedProtocolVersion(version));
        }
        let kind = TransportMsgKind::from_u8(bytes[3])?;
        let session_id = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        Ok(Self {
            kind,
            session_id,
            payload: &bytes[TRANSPORT_HEADER_SIZE..],
        })
    }
}

pub(crate) fn encode_envelope(kind: TransportMsgKind, session_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(TRANSPORT_HEADER_SIZE + payload.len());
    out.extend_from_slice(&TRANSPORT_MAGIC);
    out.push(TRANSPORT_PROTOCOL_VERSION);
    out.push(kind as u8);
    out.extend_from_slice(&session_id.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HelloPayload {
    pub schema_version: u16,
    pub max_window: u16,
    pub client_last_seq: u32,
    pub client_nonce: u64,
    pub token_proof: [u8; 16],
}

impl HelloPayload {
    const SIZE: usize = 32;

    pub(crate) fn encode(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..2].copy_from_slice(&self.schema_version.to_le_bytes());
        buf[2..4].copy_from_slice(&self.max_window.to_le_bytes());
        buf[4..8].copy_from_slice(&self.client_last_seq.to_le_bytes());
        buf[8..16].copy_from_slice(&self.client_nonce.to_le_bytes());
        buf[16..32].copy_from_slice(&self.token_proof);
        buf
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, TransportError> {
        if bytes.len() != Self::SIZE {
            return Err(TransportError::MalformedFrame("invalid Hello length"));
        }
        let mut token_proof = [0u8; 16];
        token_proof.copy_from_slice(&bytes[16..32]);
        Ok(Self {
            schema_version: u16::from_le_bytes(bytes[0..2].try_into().unwrap()),
            max_window: u16::from_le_bytes(bytes[2..4].try_into().unwrap()),
            client_last_seq: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            client_nonce: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            token_proof,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HelloAckPayload {
    pub schema_version: u16,
    pub server_max_window: u16,
    pub server_seq: u32,
    pub server_nonce: u64,
    pub resync_required: bool,
}

impl HelloAckPayload {
    const SIZE: usize = 20;

    pub(crate) fn encode(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..2].copy_from_slice(&self.schema_version.to_le_bytes());
        buf[2..4].copy_from_slice(&self.server_max_window.to_le_bytes());
        buf[4..8].copy_from_slice(&self.server_seq.to_le_bytes());
        buf[8..16].copy_from_slice(&self.server_nonce.to_le_bytes());
        buf[16] = u8::from(self.resync_required);
        buf
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, TransportError> {
        if bytes.len() != Self::SIZE {
            return Err(TransportError::MalformedFrame("invalid HelloAck length"));
        }
        Ok(Self {
            schema_version: u16::from_le_bytes(bytes[0..2].try_into().unwrap()),
            server_max_window: u16::from_le_bytes(bytes[2..4].try_into().unwrap()),
            server_seq: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            server_nonce: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            resync_required: bytes[16] != 0,
        })
    }
}

pub(crate) fn build_hello_envelope(
    config: &PairingConfig,
    client_last_seq: u32,
    client_nonce: u64,
) -> Vec<u8> {
    let hello = HelloPayload {
        schema_version: config.schema_version,
        max_window: config.clamped_window() as u16,
        client_last_seq,
        client_nonce,
        token_proof: derive_token_proof(&config.pairing_token, config.schema_version),
    };
    encode_envelope(TransportMsgKind::Hello, 0, &hello.encode())
}

pub(crate) fn verify_hello_ack(
    env: TransportEnvelopeRef<'_>,
    config: &PairingConfig,
    client_nonce: u64,
) -> Result<(u32, usize), TransportError> {
    let ack = HelloAckPayload::decode(env.payload)?;
    if ack.schema_version != config.schema_version {
        return Err(TransportError::SchemaVersionMismatch {
            expected: config.schema_version,
            actual: ack.schema_version,
        });
    }
    let expected_sid = derive_session_id(
        &config.pairing_token,
        config.schema_version,
        client_nonce,
        ack.server_nonce,
    );
    if env.session_id != expected_sid {
        return Err(TransportError::SessionIdMismatch {
            expected: expected_sid,
            actual: env.session_id,
        });
    }
    Ok((env.session_id, usize::from(ack.server_max_window).max(4)))
}

pub(crate) fn encode_reject(reason: RejectReason, expected_schema: u16) -> Vec<u8> {
    let mut payload = [0u8; 4];
    payload[0] = reason as u8;
    payload[1..3].copy_from_slice(&expected_schema.to_le_bytes());
    encode_envelope(TransportMsgKind::Reject, 0, &payload)
}

pub(crate) fn decode_reject(payload: &[u8]) -> Result<(RejectReason, u16), TransportError> {
    if payload.len() < 3 {
        return Err(TransportError::MalformedFrame("truncated Reject payload"));
    }
    let reason = RejectReason::from_u8(payload[0])
        .ok_or(TransportError::MalformedFrame("unknown Reject reason"))?;
    let expected_schema = u16::from_le_bytes(payload[1..3].try_into().unwrap());
    Ok((reason, expected_schema))
}

pub(crate) fn reject_to_error(
    reason: RejectReason,
    expected_schema: u16,
    client_schema: u16,
) -> TransportError {
    match reason {
        RejectReason::InvalidPairingToken => TransportError::AuthenticationFailed,
        RejectReason::SchemaVersionMismatch => TransportError::SchemaVersionMismatch {
            expected: expected_schema,
            actual: client_schema,
        },
        other => TransportError::HandshakeRejected(other),
    }
}

pub(crate) fn encode_ping_or_pong(
    kind: TransportMsgKind,
    session_id: u32,
    timestamp_us: u64,
) -> Vec<u8> {
    encode_envelope(kind, session_id, &timestamp_us.to_le_bytes())
}

pub(crate) fn decode_timestamp_us(payload: &[u8]) -> Result<u64, TransportError> {
    if payload.len() != 8 {
        return Err(TransportError::MalformedFrame("invalid Ping/Pong payload"));
    }
    Ok(u64::from_le_bytes(payload.try_into().unwrap()))
}

pub(crate) fn encode_keyframe_request(
    session_id: u32,
    client_last_seq: u32,
    reason: u8,
) -> Vec<u8> {
    let mut payload = [0u8; 8];
    payload[0..4].copy_from_slice(&client_last_seq.to_le_bytes());
    payload[4] = reason;
    encode_envelope(TransportMsgKind::KeyframeRequest, session_id, &payload)
}

pub(crate) fn decode_keyframe_request(payload: &[u8]) -> Result<(u32, u8), TransportError> {
    if payload.len() != 8 {
        return Err(TransportError::MalformedFrame(
            "invalid KeyframeRequest payload",
        ));
    }
    let seq = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    Ok((seq, payload[4]))
}

pub(crate) fn encode_frag_nack(session_id: u32, frag_msg_id: u32, missing: &[u16]) -> Vec<u8> {
    let capped = missing.len().min(64);
    let mut payload = Vec::with_capacity(6 + capped * 2);
    payload.extend_from_slice(&frag_msg_id.to_le_bytes());
    payload.extend_from_slice(&(capped as u16).to_le_bytes());
    for &idx in &missing[..capped] {
        payload.extend_from_slice(&idx.to_le_bytes());
    }
    encode_envelope(TransportMsgKind::FragNack, session_id, &payload)
}

pub(crate) fn decode_frag_nack(payload: &[u8]) -> Result<(u32, Vec<u16>), TransportError> {
    if payload.len() < 6 {
        return Err(TransportError::MalformedFrame("truncated FragNack"));
    }
    let frag_msg_id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    let count = usize::from(u16::from_le_bytes(payload[4..6].try_into().unwrap()));
    if count == 0 || count > 64 || payload.len() != 6 + count * 2 {
        return Err(TransportError::MalformedFrame("invalid FragNack count"));
    }
    let mut indices = Vec::with_capacity(count);
    for i in 0..count {
        let off = 6 + i * 2;
        indices.push(u16::from_le_bytes(payload[off..off + 2].try_into().unwrap()));
    }
    Ok((frag_msg_id, indices))
}

/// Validates a raw binary `EventRing` payload before transmission or server ingestion.
/// Enforces OOM payload bounds and exact `nve_abi::decode_events` structural integrity.
pub(crate) fn validate_input_events_payload(
    payload: &[u8],
    max_bytes: usize,
) -> Result<(), TransportError> {
    if payload.len() > max_bytes {
        return Err(TransportError::FrameTooLarge {
            declared: payload.len(),
            max: max_bytes,
        });
    }
    if payload.len() < 4 {
        return Err(TransportError::MalformedFrame(
            "empty or truncated InputEvents payload",
        ));
    }
    let count = u32::from_le_bytes(payload[0..4].try_into().unwrap()) as usize;
    let min_needed = 4usize.saturating_add(count.saturating_mul(MIN_ENCODED_EVENT_BYTES));
    if payload.len() < min_needed {
        return Err(TransportError::MalformedFrame(
            "InputEvents count exceeds payload length",
        ));
    }
    decode_events(payload).map_err(|_| TransportError::MalformedFrame("corrupt EventRing bytes"))?;
    Ok(())
}

// =============================================================================
// Bounded Thread-Safe Inbound EventRing Queue (Shared by UDP & TCP Servers)
// =============================================================================

pub(crate) struct EventQueue {
    queue: Mutex<VecDeque<(u64, Vec<u8>)>>,
    cv: Condvar,
}

impl EventQueue {
    pub(crate) fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(64)),
            cv: Condvar::new(),
        }
    }

    pub(crate) fn push(&self, peer_session_id: u64, encoded_event_ring: Vec<u8>) {
        let mut q = self.queue.lock().unwrap();
        if q.len() >= MAX_QUEUED_EVENT_BATCHES {
            q.pop_front();
        }
        q.push_back((peer_session_id, encoded_event_ring));
        self.cv.notify_one();
    }

    pub(crate) fn try_recv(&self) -> Option<(u64, Vec<u8>)> {
        self.queue.lock().unwrap().pop_front()
    }

    pub(crate) fn recv_timeout(
        &self,
        timeout: Duration,
        shutdown: &AtomicBool,
    ) -> Option<(u64, Vec<u8>)> {
        let deadline = Instant::now() + timeout;
        let mut q = self.queue.lock().unwrap();
        loop {
            if let Some(item) = q.pop_front() {
                return Some(item);
            }
            if shutdown.load(Ordering::Relaxed) {
                return None;
            }
            let rem = deadline.saturating_duration_since(Instant::now());
            if rem.is_zero() {
                return None;
            }
            let (guard, _) = self.cv.wait_timeout(q, rem).unwrap();
            q = guard;
        }
    }

    pub(crate) fn notify_all(&self) {
        self.cv.notify_all();
    }
}
