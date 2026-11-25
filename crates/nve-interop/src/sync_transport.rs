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
//!      graceful peer reconnect, and automatic keyframe resync when packet loss exceeds the window.
//!
//! 2. **Framed binary stream transport (`TcpDeltaServer` / `TcpDeltaClient`) + RFC 6455 WebSocket**:
//!    - Length-prefixed binary TCP stream (`StreamFraming::RawLengthPrefixed`) with pre-allocation
//!      OOM-bomb length checks.
//!    - Zero-dependency RFC 6455 binary WebSocket handshake (SHA-1 + Base64) and masked/unmasked
//!      frame codec (`StreamFraming::WebSocketBinary`), auto-detected by `TcpDeltaServer` on accept.
//!    - Seamless mid-stream reconnect with automatic server keyframe resync.
//!
//! 3. **Deterministic Pairing & Live Telemetry (`PairingConfig`, `PairingMetrics`)**:
//!    - Domain-separated 128-bit pairing token verification + schema version check + deterministic
//!      mutual `session_id` negotiation.
//!    - Real-time 1-second sliding-window wire bytes/sec & packets/sec meter plus EWMA RTT µs.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::error::InteropError;
use crate::sync_delta::{
    fnv1a_64, fnv1a_64_pair, DeltaPacketRef, PacketKind, SyncDeltaError, SyncDeltaSession,
    DEFAULT_SYNC_WINDOW, DELTA_HEADER_SIZE, MAX_DECODED_FRAME_BYTES,
};

// =============================================================================
// Wire Constants & Protocol Version
// =============================================================================

/// 2-byte transport envelope magic (`"NT"`).
pub const TRANSPORT_MAGIC: [u8; 2] = *b"NT";

/// Current `NVT1` transport envelope version.
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

/// Maximum concurrent in-flight fragmented messages retained by a UDP receiver.
const MAX_INFLIGHT_FRAG_MESSAGES: usize = 16;

/// Maximum recently sent fragmented messages retained by a UDP sender for `FragNack` recovery.
const MAX_CACHED_FRAG_MESSAGES: usize = 16;

/// Maximum HTTP WebSocket upgrade header size (8 KiB) to prevent header OOM attacks.
const MAX_WS_UPGRADE_HEADER_BYTES: usize = 8 * 1024;

/// RFC 6455 magic GUID concatenated to `Sec-WebSocket-Key`.
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Global monotonic nonce generator for handshakes and WebSocket mask keys.
static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_nonce(seed_hint: u64) -> u64 {
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
// Pairing Configuration, Deterministic Token Proof & Session ID Negotiation
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
    fn clamped_max_frame_bytes(&self) -> usize {
        self.max_frame_bytes.clamp(64, MAX_ALLOWED_WIRE_FRAME_BYTES)
    }

    #[inline]
    fn clamped_window(&self) -> usize {
        self.max_window.clamp(4, 1024)
    }
}

/// Computes a deterministic 128-bit domain-separated proof for `(pairing_token, schema_version)`.
pub fn derive_token_proof(pairing_token: &str, schema_version: u16) -> [u8; 16] {
    let ver_bytes = schema_version.to_le_bytes();
    let mut prefix_a = Vec::with_capacity(14 + 2);
    prefix_a.extend_from_slice(b"NVE-PAIR-A-v1:");
    prefix_a.extend_from_slice(&ver_bytes);
    let h1 = fnv1a_64_pair(&prefix_a, pairing_token.as_bytes());

    let mut suffix_b = Vec::with_capacity(14 + 2);
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
    let mut material = [0u8; 2 + 8 + 8];
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
fn ct_eq_16(a: &[u8; 16], b: &[u8; 16]) -> bool {
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

// =============================================================================
// Live Bandwidth Telemetry & RTT Tracking (PairingMetrics)
// =============================================================================

/// Sliding-window byte and packet rate meter.
#[derive(Debug, Clone)]
struct SlidingWindowMeter {
    window: Duration,
    samples: VecDeque<(Instant, usize)>,
    total_bytes: usize,
}

impl SlidingWindowMeter {
    fn new(window: Duration) -> Self {
        Self {
            window,
            samples: VecDeque::with_capacity(128),
            total_bytes: 0,
        }
    }

    fn prune(&mut self, now: Instant) {
        while let Some(&(ts, bytes)) = self.samples.front() {
            if now.saturating_duration_since(ts) > self.window {
                self.samples.pop_front();
                self.total_bytes = self.total_bytes.saturating_sub(bytes);
            } else {
                break;
            }
        }
    }

    fn record(&mut self, now: Instant, bytes: usize) {
        self.prune(now);
        self.samples.push_back((now, bytes));
        self.total_bytes = self.total_bytes.saturating_add(bytes);
    }

    fn rates(&mut self, now: Instant) -> (f64, f64) {
        self.prune(now);
        let secs = self.window.as_secs_f64().max(0.001);
        (
            self.total_bytes as f64 / secs,
            self.samples.len() as f64 / secs,
        )
    }
}

/// Snapshot of live pairing handshake, reliability, bandwidth, and RTT telemetry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PairingMetrics {
    pub session_id: u32,
    pub schema_version: u16,
    pub connected_peers: usize,
    pub frames_sent: u64,
    pub frames_received: u64,
    pub keyframes_sent: u64,
    pub deltas_sent: u64,
    pub retransmits_sent: u64,
    pub acks_sent: u64,
    pub acks_received: u64,
    pub nacks_sent: u64,
    pub nacks_received: u64,
    pub keyframe_resyncs: u64,
    pub reconnects: u64,
    pub rejected_handshakes: u64,
    pub malformed_packets_dropped: u64,
    pub wire_bytes_sent: u64,
    pub wire_bytes_received: u64,
    pub raw_bytes_sent: u64,
    pub raw_bytes_received: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub wire_bytes_per_sec_tx: f64,
    pub wire_bytes_per_sec_rx: f64,
    pub packets_per_sec_tx: f64,
    pub packets_per_sec_rx: f64,
    pub last_rtt_us: Option<u64>,
    pub smoothed_rtt_us: Option<u64>,
    pub min_rtt_us: Option<u64>,
    pub rtt_samples: u64,
}

impl PairingMetrics {
    /// Current sliding-window outbound wire bandwidth in KiB/s.
    pub fn tx_kbps(&self) -> f64 {
        self.wire_bytes_per_sec_tx / 1024.0
    }

    /// Current sliding-window inbound wire bandwidth in KiB/s.
    pub fn rx_kbps(&self) -> f64 {
        self.wire_bytes_per_sec_rx / 1024.0
    }

    /// Cumulative raw-to-wire compression ratio (`raw_bytes_sent / wire_bytes_sent`).
    pub fn compression_ratio(&self) -> f64 {
        if self.wire_bytes_sent == 0 {
            1.0
        } else {
            self.raw_bytes_sent as f64 / self.wire_bytes_sent as f64
        }
    }
}

#[derive(Debug)]
struct MetricsTracker {
    snapshot: PairingMetrics,
    tx_meter: SlidingWindowMeter,
    rx_meter: SlidingWindowMeter,
}

impl MetricsTracker {
    fn new(schema_version: u16, window: Duration) -> Self {
        let mut snapshot = PairingMetrics::default();
        snapshot.schema_version = schema_version;
        Self {
            snapshot,
            tx_meter: SlidingWindowMeter::new(window),
            rx_meter: SlidingWindowMeter::new(window),
        }
    }

    fn record_tx(&mut self, now: Instant, wire_bytes: usize) {
        self.snapshot.wire_bytes_sent += wire_bytes as u64;
        self.snapshot.packets_sent += 1;
        self.tx_meter.record(now, wire_bytes);
    }

    fn record_rx(&mut self, now: Instant, wire_bytes: usize) {
        self.snapshot.wire_bytes_received += wire_bytes as u64;
        self.snapshot.packets_received += 1;
        self.rx_meter.record(now, wire_bytes);
    }

    fn record_rtt_us(&mut self, rtt_us: u64) {
        self.snapshot.last_rtt_us = Some(rtt_us);
        self.snapshot.rtt_samples += 1;
        self.snapshot.min_rtt_us = Some(match self.snapshot.min_rtt_us {
            Some(prev) => prev.min(rtt_us),
            None => rtt_us,
        });
        self.snapshot.smoothed_rtt_us = Some(match self.snapshot.smoothed_rtt_us {
            Some(srtt) => (srtt * 7 + rtt_us) / 8,
            None => rtt_us,
        });
    }

    fn snapshot(&mut self, now: Instant) -> PairingMetrics {
        let (tx_bps, tx_pps) = self.tx_meter.rates(now);
        let (rx_bps, rx_pps) = self.rx_meter.rates(now);
        self.snapshot.wire_bytes_per_sec_tx = tx_bps;
        self.snapshot.packets_per_sec_tx = tx_pps;
        self.snapshot.wire_bytes_per_sec_rx = rx_bps;
        self.snapshot.packets_per_sec_rx = rx_pps;
        self.snapshot.clone()
    }
}

// =============================================================================
// NVT1 Transport Message Codec (Zero-Copy Envelope)
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
            other => Err(TransportError::InvalidMessageKind(other)),
        }
    }
}

/// Zero-copy parsed view of an incoming `NVT1` transport message.
#[derive(Debug, Clone, Copy)]
struct TransportEnvelopeRef<'a> {
    kind: TransportMsgKind,
    session_id: u32,
    payload: &'a [u8],
}

impl<'a> TransportEnvelopeRef<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, TransportError> {
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

fn encode_envelope(kind: TransportMsgKind, session_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(TRANSPORT_HEADER_SIZE + payload.len());
    out.extend_from_slice(&TRANSPORT_MAGIC);
    out.push(TRANSPORT_PROTOCOL_VERSION);
    out.push(kind as u8);
    out.extend_from_slice(&session_id.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

#[derive(Debug, Clone, Copy)]
struct HelloPayload {
    schema_version: u16,
    max_window: u16,
    client_last_seq: u32,
    client_nonce: u64,
    token_proof: [u8; 16],
}

impl HelloPayload {
    const SIZE: usize = 32;

    fn encode(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..2].copy_from_slice(&self.schema_version.to_le_bytes());
        buf[2..4].copy_from_slice(&self.max_window.to_le_bytes());
        buf[4..8].copy_from_slice(&self.client_last_seq.to_le_bytes());
        buf[8..16].copy_from_slice(&self.client_nonce.to_le_bytes());
        buf[16..32].copy_from_slice(&self.token_proof);
        buf
    }

    fn decode(bytes: &[u8]) -> Result<Self, TransportError> {
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
struct HelloAckPayload {
    schema_version: u16,
    server_max_window: u16,
    server_seq: u32,
    server_nonce: u64,
    resync_required: bool,
}

impl HelloAckPayload {
    const SIZE: usize = 20;

    fn encode(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..2].copy_from_slice(&self.schema_version.to_le_bytes());
        buf[2..4].copy_from_slice(&self.server_max_window.to_le_bytes());
        buf[4..8].copy_from_slice(&self.server_seq.to_le_bytes());
        buf[8..16].copy_from_slice(&self.server_nonce.to_le_bytes());
        buf[16] = u8::from(self.resync_required);
        buf
    }

    fn decode(bytes: &[u8]) -> Result<Self, TransportError> {
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

fn encode_reject(reason: RejectReason, expected_schema: u16) -> Vec<u8> {
    let mut payload = [0u8; 4];
    payload[0] = reason as u8;
    payload[1..3].copy_from_slice(&expected_schema.to_le_bytes());
    encode_envelope(TransportMsgKind::Reject, 0, &payload)
}

fn decode_reject(payload: &[u8]) -> Result<(RejectReason, u16), TransportError> {
    if payload.len() < 3 {
        return Err(TransportError::MalformedFrame("truncated Reject payload"));
    }
    let reason = RejectReason::from_u8(payload[0])
        .ok_or(TransportError::MalformedFrame("unknown Reject reason"))?;
    let expected_schema = u16::from_le_bytes(payload[1..3].try_into().unwrap());
    Ok((reason, expected_schema))
}

fn reject_to_error(reason: RejectReason, expected_schema: u16, client_schema: u16) -> TransportError {
    match reason {
        RejectReason::InvalidPairingToken => TransportError::AuthenticationFailed,
        RejectReason::SchemaVersionMismatch => TransportError::SchemaVersionMismatch {
            expected: expected_schema,
            actual: client_schema,
        },
        other => TransportError::HandshakeRejected(other),
    }
}

fn encode_ping_or_pong(kind: TransportMsgKind, session_id: u32, timestamp_us: u64) -> Vec<u8> {
    encode_envelope(kind, session_id, &timestamp_us.to_le_bytes())
}

fn decode_timestamp_us(payload: &[u8]) -> Result<u64, TransportError> {
    if payload.len() != 8 {
        return Err(TransportError::MalformedFrame("invalid Ping/Pong payload"));
    }
    Ok(u64::from_le_bytes(payload.try_into().unwrap()))
}

fn encode_keyframe_request(session_id: u32, client_last_seq: u32, reason: u8) -> Vec<u8> {
    let mut payload = [0u8; 8];
    payload[0..4].copy_from_slice(&client_last_seq.to_le_bytes());
    payload[4] = reason;
    encode_envelope(TransportMsgKind::KeyframeRequest, session_id, &payload)
}

fn decode_keyframe_request(payload: &[u8]) -> Result<(u32, u8), TransportError> {
    if payload.len() != 8 {
        return Err(TransportError::MalformedFrame(
            "invalid KeyframeRequest payload",
        ));
    }
    let seq = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    Ok((seq, payload[4]))
}

fn encode_frag_nack(session_id: u32, frag_msg_id: u32, missing: &[u16]) -> Vec<u8> {
    let capped = missing.len().min(64);
    let mut payload = Vec::with_capacity(6 + capped * 2);
    payload.extend_from_slice(&frag_msg_id.to_le_bytes());
    payload.extend_from_slice(&(capped as u16).to_le_bytes());
    for &idx in &missing[..capped] {
        payload.extend_from_slice(&idx.to_le_bytes());
    }
    encode_envelope(TransportMsgKind::FragNack, session_id, &payload)
}

fn decode_frag_nack(payload: &[u8]) -> Result<(u32, Vec<u16>), TransportError> {
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

// =============================================================================
// UDP MTU-Safe Packetization & Bounded Fragment Reassembly (<= 1200 bytes)
// =============================================================================

/// Splits an `NVD1` packet into one or more UDP datagrams guaranteed `<= MAX_UDP_DATAGRAM_BYTES`
/// (1200 bytes). Small packets take the zero-overhead `DeltaSingle` path; large packets (e.g.
/// initial keyframes) use `DeltaFrag`.
pub fn packetize_udp_datagrams(session_id: u32, nvd_packet: &[u8]) -> Vec<Vec<u8>> {
    if nvd_packet.len() <= MAX_UDP_SINGLE_PAYLOAD {
        return vec![encode_envelope(
            TransportMsgKind::DeltaSingle,
            session_id,
            nvd_packet,
        )];
    }

    let frag_msg_id = if nvd_packet.len() >= DELTA_HEADER_SIZE {
        let seq = u32::from_le_bytes(nvd_packet[8..12].try_into().unwrap());
        if seq != 0 {
            seq
        } else {
            fnv1a_64(nvd_packet) as u32
        }
    } else {
        fnv1a_64(nvd_packet) as u32
    };

    let total_len = nvd_packet.len() as u32;
    let frag_count = nvd_packet.len().div_ceil(MAX_UDP_FRAG_CHUNK);
    let frag_count_u16 = frag_count as u16;
    let mut out = Vec::with_capacity(frag_count);

    for frag_idx in 0..frag_count {
        let start = frag_idx * MAX_UDP_FRAG_CHUNK;
        let end = (start + MAX_UDP_FRAG_CHUNK).min(nvd_packet.len());
        let chunk = &nvd_packet[start..end];
        let mut dgram =
            Vec::with_capacity(TRANSPORT_HEADER_SIZE + FRAG_SUBHEADER_SIZE + chunk.len());
        dgram.extend_from_slice(&TRANSPORT_MAGIC);
        dgram.push(TRANSPORT_PROTOCOL_VERSION);
        dgram.push(TransportMsgKind::DeltaFrag as u8);
        dgram.extend_from_slice(&session_id.to_le_bytes());
        dgram.extend_from_slice(&frag_msg_id.to_le_bytes());
        dgram.extend_from_slice(&(frag_idx as u16).to_le_bytes());
        dgram.extend_from_slice(&frag_count_u16.to_le_bytes());
        dgram.extend_from_slice(&total_len.to_le_bytes());
        dgram.extend_from_slice(chunk);
        debug_assert!(dgram.len() <= MAX_UDP_DATAGRAM_BYTES);
        out.push(dgram);
    }
    out
}

#[derive(Debug)]
struct InFlightFrag {
    frag_count: u16,
    total_len: usize,
    buf: Vec<u8>,
    received: Vec<bool>,
    received_count: usize,
    highest_idx_seen: u16,
    first_seen: Instant,
    last_nack_at: Option<Instant>,
}

#[derive(Debug)]
struct FragReassembler {
    max_frame_bytes: usize,
    inflight: HashMap<u32, InFlightFrag>,
    order: VecDeque<u32>,
}

impl FragReassembler {
    fn new(max_frame_bytes: usize) -> Self {
        Self {
            max_frame_bytes,
            inflight: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn clear(&mut self) {
        self.inflight.clear();
        self.order.clear();
    }

    /// Feeds one `DeltaFrag` payload. Returns `(completed_nvd_packet, optional_immediate_frag_nack)`.
    fn push(
        &mut self,
        payload: &[u8],
        now: Instant,
    ) -> Result<(Option<Vec<u8>>, Option<(u32, Vec<u16>)>), TransportError> {
        if payload.len() < FRAG_SUBHEADER_SIZE {
            return Err(TransportError::TruncatedFrame);
        }
        let frag_msg_id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
        let frag_idx = u16::from_le_bytes(payload[4..6].try_into().unwrap());
        let frag_count = u16::from_le_bytes(payload[6..8].try_into().unwrap());
        let total_len = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
        let chunk = &payload[FRAG_SUBHEADER_SIZE..];

        if frag_count == 0 || frag_idx >= frag_count {
            return Err(TransportError::MalformedFrame("invalid fragment index/count"));
        }
        if total_len == 0 || total_len > self.max_frame_bytes {
            return Err(TransportError::FrameTooLarge {
                declared: total_len,
                max: self.max_frame_bytes,
            });
        }
        let expected_count = total_len.div_ceil(MAX_UDP_FRAG_CHUNK);
        if usize::from(frag_count) != expected_count {
            return Err(TransportError::MalformedFrame(
                "fragment count mismatches total_len",
            ));
        }
        let start = usize::from(frag_idx) * MAX_UDP_FRAG_CHUNK;
        let expected_chunk_len = (total_len - start).min(MAX_UDP_FRAG_CHUNK);
        if chunk.len() != expected_chunk_len {
            return Err(TransportError::MalformedFrame("fragment chunk length mismatch"));
        }

        // Evict oldest entry if we reached the bounded inflight limit.
        if !self.inflight.contains_key(&frag_msg_id) {
            while self.inflight.len() >= MAX_INFLIGHT_FRAG_MESSAGES {
                if let Some(oldest) = self.order.pop_front() {
                    self.inflight.remove(&oldest);
                } else {
                    break;
                }
            }
            self.order.push_back(frag_msg_id);
            self.inflight.insert(
                frag_msg_id,
                InFlightFrag {
                    frag_count,
                    total_len,
                    buf: vec![0u8; total_len],
                    received: vec![false; usize::from(frag_count)],
                    received_count: 0,
                    highest_idx_seen: 0,
                    first_seen: now,
                    last_nack_at: None,
                },
            );
        }

        let entry = self.inflight.get_mut(&frag_msg_id).unwrap();
        if entry.frag_count != frag_count || entry.total_len != total_len {
            return Err(TransportError::MalformedFrame(
                "conflicting fragment metadata for msg_id",
            ));
        }

        // Always copy latest bytes (so a retransmitted fragment 0 updates flags/checksum cleanly).
        entry.buf[start..start + chunk.len()].copy_from_slice(chunk);
        let idx_u = usize::from(frag_idx);
        if !entry.received[idx_u] {
            entry.received[idx_u] = true;
            entry.received_count += 1;
        }
        if frag_idx > entry.highest_idx_seen {
            entry.highest_idx_seen = frag_idx;
        }

        if entry.received_count == usize::from(entry.frag_count) {
            let done = self.inflight.remove(&frag_msg_id).unwrap();
            self.order.retain(|&id| id != frag_msg_id);
            return Ok((Some(done.buf), None));
        }

        // Let brief in-flight packet reordering settle before emitting FragNack via poll_stale_nacks.
        Ok((None, None))
    }

    fn poll_stale_nacks(&mut self, now: Instant, retry_ms: u64) -> Vec<(u32, Vec<u16>)> {
        let mut out = Vec::new();
        let retry = Duration::from_millis(retry_ms);
        let max_age = Duration::from_secs(5);
        let mut expired = Vec::new();

        for (&msg_id, entry) in &mut self.inflight {
            if now.saturating_duration_since(entry.first_seen) > max_age {
                expired.push(msg_id);
                continue;
            }
            let due = match entry.last_nack_at {
                Some(t) => now.saturating_duration_since(t) >= retry,
                None => now.saturating_duration_since(entry.first_seen) >= retry,
            };
            if due {
                let missing: Vec<u16> = entry
                    .received
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &got)| (!got).then_some(i as u16))
                    .collect();
                if !missing.is_empty() {
                    entry.last_nack_at = Some(now);
                    out.push((msg_id, missing));
                }
            }
        }
        for id in expired {
            self.inflight.remove(&id);
            self.order.retain(|&x| x != id);
        }
        out
    }
}

// =============================================================================
// UDP Datagram Transport (UdpDeltaServer & UdpDeltaClient)
// =============================================================================

struct UdpPeerState {
    session_id: u32,
    session: SyncDeltaSession,
    frag_cache: VecDeque<(u32, Vec<Vec<u8>>)>,
    last_activity: Instant,
    last_ping_sent: Instant,
}

impl UdpPeerState {
    fn cache_fragments(&mut self, frag_msg_id: u32, dgrams: Vec<Vec<u8>>) {
        if dgrams.len() <= 1 {
            return;
        }
        if let Some(pos) = self.frag_cache.iter().position(|(id, _)| *id == frag_msg_id) {
            self.frag_cache.remove(pos);
        }
        while self.frag_cache.len() >= MAX_CACHED_FRAG_MESSAGES {
            self.frag_cache.pop_front();
        }
        self.frag_cache.push_back((frag_msg_id, dgrams));
    }

    fn lookup_frag(&self, frag_msg_id: u32, frag_idx: u16) -> Option<&[u8]> {
        self.frag_cache
            .iter()
            .find(|(id, _)| *id == frag_msg_id)
            .and_then(|(_, dgrams)| dgrams.get(usize::from(frag_idx)))
            .map(Vec::as_slice)
    }
}

struct UdpServerInner {
    config: PairingConfig,
    expected_proof: [u8; 16],
    started_at: Instant,
    peers: HashMap<SocketAddr, UdpPeerState>,
    seen_peers_total: u64,
    latest_frame: Option<Vec<u8>>,
    metrics: MetricsTracker,
}

impl UdpServerInner {
    fn now_ms(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_millis() as u64
    }

    fn now_us(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_micros() as u64
    }

    fn send_nvd_to_peer(
        socket: &UdpSocket,
        addr: SocketAddr,
        peer: &mut UdpPeerState,
        metrics: &mut MetricsTracker,
        nvd_packet: &[u8],
        now: Instant,
    ) -> Result<(), TransportError> {
        let dgrams = packetize_udp_datagrams(peer.session_id, nvd_packet);
        for dgram in &dgrams {
            socket.send_to(dgram, addr)?;
            metrics.record_tx(now, dgram.len());
        }
        if dgrams.len() > 1 && nvd_packet.len() >= DELTA_HEADER_SIZE {
            let seq = u32::from_le_bytes(nvd_packet[8..12].try_into().unwrap());
            peer.cache_fragments(seq, dgrams);
        }
        Ok(())
    }
}

/// Low-latency UDP delta server with MTU-safe packetization (`<= 1200` B), SACK/NACK/FragNack
/// retransmit pump, heartbeat RTT estimation, and multi-peer reconnect/resync support.
pub struct UdpDeltaServer {
    local_addr: SocketAddr,
    socket: UdpSocket,
    inner: Arc<(Mutex<UdpServerInner>, Condvar)>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl UdpDeltaServer {
    pub fn bind(addr: impl ToSocketAddrs, config: PairingConfig) -> Result<Self, TransportError> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_read_timeout(Some(Duration::from_millis(5)))?;
        let local_addr = socket.local_addr()?;
        let worker_sock = socket.try_clone()?;

        let expected_proof = derive_token_proof(&config.pairing_token, config.schema_version);
        let metrics = MetricsTracker::new(
            config.schema_version,
            Duration::from_millis(config.metrics_window_ms),
        );
        let inner = Arc::new((
            Mutex::new(UdpServerInner {
                config,
                expected_proof,
                started_at: Instant::now(),
                peers: HashMap::new(),
                seen_peers_total: 0,
                latest_frame: None,
                metrics,
            }),
            Condvar::new(),
        ));
        let shutdown = Arc::new(AtomicBool::new(false));

        let worker_inner = Arc::clone(&inner);
        let worker_shutdown = Arc::clone(&shutdown);
        let worker = thread::Builder::new()
            .name("nve-udp-delta-server".into())
            .spawn(move || {
                udp_server_loop(worker_sock, worker_inner, worker_shutdown);
            })?;

        Ok(Self {
            local_addr,
            socket,
            inner,
            shutdown,
            worker: Some(worker),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn peer_count(&self) -> usize {
        self.inner.0.lock().unwrap().peers.len()
    }

    /// Blocks until at least `min_peers` have completed pairing handshake or `timeout` elapses.
    pub fn wait_for_peers(&self, min_peers: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut guard = self.inner.0.lock().unwrap();
        while guard.peers.len() < min_peers {
            let rem = deadline.saturating_duration_since(Instant::now());
            if rem.is_zero() {
                return false;
            }
            let (g, _) = self.inner.1.wait_timeout(guard, rem).unwrap();
            guard = g;
        }
        true
    }

    /// Broadcasts `frame_bytes` to all paired UDP peers using per-peer `SyncDeltaSession` encoding.
    /// Returns the number of paired peers the frame was dispatched to.
    pub fn send_frame(&self, frame_bytes: &[u8]) -> Result<usize, TransportError> {
        let now = Instant::now();
        let mut guard = self.inner.0.lock().unwrap();
        let max_bytes = guard.config.clamped_max_frame_bytes();
        if frame_bytes.len() > max_bytes {
            return Err(TransportError::FrameTooLarge {
                declared: frame_bytes.len(),
                max: max_bytes,
            });
        }
        guard.latest_frame = Some(frame_bytes.to_vec());
        let now_ms = guard.now_ms(now);
        let addrs: Vec<SocketAddr> = guard.peers.keys().copied().collect();
        let mut sent_peers = 0usize;

        for addr in addrs {
            let state = &mut *guard;
            if let Some(peer) = state.peers.get_mut(&addr) {
                let nvd_pkt = peer.session.encode_frame(frame_bytes, now_ms);
                let kind = DeltaPacketRef::parse(&nvd_pkt)
                    .map(|p| p.header().kind)
                    .unwrap_or(PacketKind::Keyframe);
                if UdpServerInner::send_nvd_to_peer(
                    &self.socket,
                    addr,
                    peer,
                    &mut state.metrics,
                    &nvd_pkt,
                    now,
                )
                .is_ok()
                {
                    state.metrics.snapshot.frames_sent += 1;
                    state.metrics.snapshot.raw_bytes_sent += frame_bytes.len() as u64;
                    match kind {
                        PacketKind::Keyframe => state.metrics.snapshot.keyframes_sent += 1,
                        PacketKind::XorRleDelta | PacketKind::CommandOpDelta => {
                            state.metrics.snapshot.deltas_sent += 1
                        }
                        _ => {}
                    }
                    sent_peers += 1;
                }
            }
        }
        Ok(sent_peers)
    }

    pub fn metrics(&self) -> PairingMetrics {
        let mut guard = self.inner.0.lock().unwrap();
        guard.metrics.snapshot.connected_peers = guard.peers.len();
        guard.metrics.snapshot(Instant::now())
    }

    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for UdpDeltaServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn udp_server_loop(
    socket: UdpSocket,
    inner: Arc<(Mutex<UdpServerInner>, Condvar)>,
    shutdown: Arc<AtomicBool>,
) {
    let mut buf = [0u8; 2048];
    let mut last_tick = Instant::now();

    while !shutdown.load(Ordering::Relaxed) {
        match socket.recv_from(&mut buf) {
            Ok((n, src)) => {
                let now = Instant::now();
                let mut guard = inner.0.lock().unwrap();
                guard.metrics.record_rx(now, n);
                let packet = &buf[..n];
                let env = match TransportEnvelopeRef::parse(packet) {
                    Ok(e) => e,
                    Err(_) => {
                        guard.metrics.snapshot.malformed_packets_dropped += 1;
                        continue;
                    }
                };

                match env.kind {
                    TransportMsgKind::Hello => {
                        let Ok(hello) = HelloPayload::decode(env.payload) else {
                            guard.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        };
                        if hello.schema_version != guard.config.schema_version {
                            guard.metrics.snapshot.rejected_handshakes += 1;
                            let rej = encode_reject(
                                RejectReason::SchemaVersionMismatch,
                                guard.config.schema_version,
                            );
                            if socket.send_to(&rej, src).is_ok() {
                                guard.metrics.record_tx(now, rej.len());
                            }
                            continue;
                        }
                        if !ct_eq_16(&hello.token_proof, &guard.expected_proof) {
                            guard.metrics.snapshot.rejected_handshakes += 1;
                            let rej = encode_reject(
                                RejectReason::InvalidPairingToken,
                                guard.config.schema_version,
                            );
                            if socket.send_to(&rej, src).is_ok() {
                                guard.metrics.record_tx(now, rej.len());
                            }
                            continue;
                        }

                        // Valid handshake! Derive deterministic session_id.
                        let server_nonce = next_nonce(hello.client_nonce ^ 0x55AA_55AA);
                        let session_id = derive_session_id(
                            &guard.config.pairing_token,
                            guard.config.schema_version,
                            hello.client_nonce,
                            server_nonce,
                        );
                        let window = guard
                            .config
                            .clamped_window()
                            .min(usize::from(hello.max_window).max(4));
                        let is_reconnect =
                            guard.peers.contains_key(&src) || guard.seen_peers_total > 0;
                        guard.seen_peers_total += 1;
                        if is_reconnect {
                            guard.metrics.snapshot.reconnects += 1;
                        }

                        let has_latest = guard.latest_frame.is_some();
                        let resync_required = has_latest;
                        let peer_state = UdpPeerState {
                            session_id,
                            session: SyncDeltaSession::new(window),
                            frag_cache: VecDeque::new(),
                            last_activity: now,
                            last_ping_sent: now,
                        };
                        guard.peers.insert(src, peer_state);
                        guard.metrics.snapshot.session_id = session_id;
                        guard.metrics.snapshot.connected_peers = guard.peers.len();

                        let ack = HelloAckPayload {
                            schema_version: guard.config.schema_version,
                            server_max_window: window as u16,
                            server_seq: 0,
                            server_nonce,
                            resync_required,
                        };
                        let ack_pkt =
                            encode_envelope(TransportMsgKind::HelloAck, session_id, &ack.encode());
                        if socket.send_to(&ack_pkt, src).is_ok() {
                            guard.metrics.record_tx(now, ack_pkt.len());
                        }

                        // Kick off immediate RTT probe.
                        let ts_us = guard.now_us(now);
                        let ping = encode_ping_or_pong(TransportMsgKind::Ping, session_id, ts_us);
                        if socket.send_to(&ping, src).is_ok() {
                            guard.metrics.record_tx(now, ping.len());
                        }

                        // If server already has live state, immediately send a resync Keyframe.
                        if let Some(latest) = guard.latest_frame.clone() {
                            let now_ms = guard.now_ms(now);
                            let state = &mut *guard;
                            if let Some(peer) = state.peers.get_mut(&src) {
                                let kf = peer.session.encode_keyframe(&latest, now_ms);
                                if UdpServerInner::send_nvd_to_peer(
                                    &socket,
                                    src,
                                    peer,
                                    &mut state.metrics,
                                    &kf,
                                    now,
                                )
                                .is_ok()
                                {
                                    state.metrics.snapshot.frames_sent += 1;
                                    state.metrics.snapshot.keyframes_sent += 1;
                                    state.metrics.snapshot.keyframe_resyncs += 1;
                                    state.metrics.snapshot.raw_bytes_sent += latest.len() as u64;
                                }
                            }
                        }

                        inner.1.notify_all();
                    }
                    TransportMsgKind::DeltaSingle => {
                        let now_ms = guard.now_ms(now);
                        let state = &mut *guard;
                        let Some(peer) = state.peers.get_mut(&src) else {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        };
                        if peer.session_id != env.session_id {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        }
                        peer.last_activity = now;

                        let Ok(parsed) = DeltaPacketRef::parse(env.payload) else {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        };
                        match parsed.header().kind {
                            PacketKind::Ack => {
                                if peer.session.handle_ack(env.payload).is_ok() {
                                    state.metrics.snapshot.acks_received += 1;
                                }
                            }
                            PacketKind::Nack => {
                                state.metrics.snapshot.nacks_received += 1;
                                if let Ok(Some(retx)) = peer.session.handle_nack(env.payload, now_ms)
                                {
                                    let is_kf = DeltaPacketRef::parse(&retx)
                                        .map(|p| p.header().kind == PacketKind::Keyframe)
                                        .unwrap_or(false);
                                    if UdpServerInner::send_nvd_to_peer(
                                        &socket,
                                        src,
                                        peer,
                                        &mut state.metrics,
                                        &retx,
                                        now,
                                    )
                                    .is_ok()
                                    {
                                        state.metrics.snapshot.retransmits_sent += 1;
                                        if is_kf {
                                            state.metrics.snapshot.keyframe_resyncs += 1;
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Ignore unexpected data packets from receiver.
                            }
                        }
                    }
                    TransportMsgKind::FragNack => {
                        let state = &mut *guard;
                        let Some(peer) = state.peers.get_mut(&src) else {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        };
                        if peer.session_id != env.session_id {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        }
                        peer.last_activity = now;
                        if let Ok((frag_msg_id, missing_indices)) = decode_frag_nack(env.payload) {
                            state.metrics.snapshot.nacks_received += 1;
                            for idx in missing_indices {
                                if let Some(dgram) = peer.lookup_frag(frag_msg_id, idx) {
                                    if socket.send_to(dgram, src).is_ok() {
                                        state.metrics.record_tx(now, dgram.len());
                                        state.metrics.snapshot.retransmits_sent += 1;
                                    }
                                }
                            }
                        } else {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                        }
                    }
                    TransportMsgKind::KeyframeRequest => {
                        let now_ms = guard.now_ms(now);
                        let latest_opt = guard.latest_frame.clone();
                        let state = &mut *guard;
                        let Some(peer) = state.peers.get_mut(&src) else {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        };
                        if peer.session_id != env.session_id {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        }
                        if decode_keyframe_request(env.payload).is_err() {
                            state.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        }
                        peer.last_activity = now;
                        if let Some(latest) = latest_opt {
                            let kf = peer.session.encode_keyframe(&latest, now_ms);
                            if UdpServerInner::send_nvd_to_peer(
                                &socket,
                                src,
                                peer,
                                &mut state.metrics,
                                &kf,
                                now,
                            )
                            .is_ok()
                            {
                                state.metrics.snapshot.frames_sent += 1;
                                state.metrics.snapshot.keyframes_sent += 1;
                                state.metrics.snapshot.keyframe_resyncs += 1;
                                state.metrics.snapshot.raw_bytes_sent += latest.len() as u64;
                            }
                        }
                    }
                    TransportMsgKind::Ping => {
                        let Some(peer) = guard.peers.get_mut(&src) else {
                            continue;
                        };
                        if peer.session_id != env.session_id {
                            continue;
                        }
                        peer.last_activity = now;
                        if let Ok(ts_us) = decode_timestamp_us(env.payload) {
                            let pong =
                                encode_ping_or_pong(TransportMsgKind::Pong, env.session_id, ts_us);
                            if socket.send_to(&pong, src).is_ok() {
                                guard.metrics.record_tx(now, pong.len());
                            }
                        }
                    }
                    TransportMsgKind::Pong => {
                        let now_us = guard.now_us(now);
                        let valid_peer = guard
                            .peers
                            .get_mut(&src)
                            .map(|p| {
                                if p.session_id == env.session_id {
                                    p.last_activity = now;
                                    true
                                } else {
                                    false
                                }
                            })
                            .unwrap_or(false);
                        if valid_peer {
                            if let Ok(sent_us) = decode_timestamp_us(env.payload) {
                                let rtt = now_us.saturating_sub(sent_us);
                                guard.metrics.record_rtt_us(rtt);
                            }
                        }
                    }
                    TransportMsgKind::Bye => {
                        let matches = guard
                            .peers
                            .get(&src)
                            .map(|p| p.session_id == env.session_id)
                            .unwrap_or(false);
                        if matches {
                            guard.peers.remove(&src);
                            guard.metrics.snapshot.connected_peers = guard.peers.len();
                        }
                    }
                    TransportMsgKind::HelloAck
                    | TransportMsgKind::Reject
                    | TransportMsgKind::DeltaFrag => {
                        guard.metrics.snapshot.malformed_packets_dropped += 1;
                    }
                }
            }
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }

        // Periodic RTO retransmit & heartbeat pump.
        let now = Instant::now();
        if now.saturating_duration_since(last_tick) >= Duration::from_millis(8) {
            last_tick = now;
            let mut guard = inner.0.lock().unwrap();
            let base_rto_ms = guard.config.rto_ms;
            let effective_rto_ms = guard
                .metrics
                .snapshot
                .smoothed_rtt_us
                .map(|us| base_rto_ms.max((us / 1000).saturating_mul(3).saturating_add(25)))
                .unwrap_or(base_rto_ms);
            let hb_interval = Duration::from_millis(guard.config.heartbeat_interval_ms);
            let now_ms = guard.now_ms(now);
            let now_us = guard.now_us(now);
            let addrs: Vec<SocketAddr> = guard.peers.keys().copied().collect();

            for addr in addrs {
                let state = &mut *guard;
                if let Some(peer) = state.peers.get_mut(&addr) {
                    let retransmits = peer.session.poll_retransmits(now_ms, effective_rto_ms);
                    for retx in retransmits {
                        if UdpServerInner::send_nvd_to_peer(
                            &socket,
                            addr,
                            peer,
                            &mut state.metrics,
                            &retx,
                            now,
                        )
                        .is_ok()
                        {
                            state.metrics.snapshot.retransmits_sent += 1;
                        }
                    }

                    if now.saturating_duration_since(peer.last_ping_sent) >= hb_interval {
                        peer.last_ping_sent = now;
                        let ping =
                            encode_ping_or_pong(TransportMsgKind::Ping, peer.session_id, now_us);
                        if socket.send_to(&ping, addr).is_ok() {
                            state.metrics.record_tx(now, ping.len());
                        }
                    }
                }
            }
        }
    }
}

// -----------------------------------------------------------------------------
// UdpDeltaClient
// -----------------------------------------------------------------------------

struct UdpClientShared {
    config: PairingConfig,
    started_at: Instant,
    session_id: u32,
    session: SyncDeltaSession,
    metrics: MetricsTracker,
    connected: bool,
}

impl UdpClientShared {
    fn now_us(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_micros() as u64
    }
}

/// Low-latency UDP delta client with pairing handshake retry, MTU-safe fragment reassembly,
/// automatic SACK/NACK/FragNack feedback, RTT tracking, and graceful reconnect.
pub struct UdpDeltaClient {
    server_addr: SocketAddr,
    socket: Option<UdpSocket>,
    shared: Arc<Mutex<UdpClientShared>>,
    frame_rx: mpsc::Receiver<(u32, Vec<u8>)>,
    frame_tx: mpsc::Sender<(u32, Vec<u8>)>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl UdpDeltaClient {
    /// Binds a local ephemeral UDP socket and performs the deterministic pairing handshake.
    pub fn connect(server_addr: SocketAddr, config: PairingConfig) -> Result<Self, TransportError> {
        let (frame_tx, frame_rx) = mpsc::channel();
        let metrics = MetricsTracker::new(
            config.schema_version,
            Duration::from_millis(config.metrics_window_ms),
        );
        let shared = Arc::new(Mutex::new(UdpClientShared {
            session: SyncDeltaSession::new(config.clamped_window()),
            config,
            started_at: Instant::now(),
            session_id: 0,
            metrics,
            connected: false,
        }));

        let mut client = Self {
            server_addr,
            socket: None,
            shared,
            frame_rx,
            frame_tx,
            shutdown: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        client.establish_connection(false)?;
        Ok(client)
    }

    fn establish_connection(&mut self, is_reconnect: bool) -> Result<u32, TransportError> {
        let bind_addr = if self.server_addr.is_ipv6() {
            "[::1]:0"
        } else {
            "127.0.0.1:0"
        };
        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_read_timeout(Some(Duration::from_millis(20)))?;

        let (config, client_last_seq) = {
            let guard = self.shared.lock().unwrap();
            (guard.config.clone(), guard.session.last_applied_seq())
        };

        let client_nonce = next_nonce(u64::from(client_last_seq) ^ 0x1357_9BDF);
        let hello = HelloPayload {
            schema_version: config.schema_version,
            max_window: config.clamped_window() as u16,
            client_last_seq,
            client_nonce,
            token_proof: derive_token_proof(&config.pairing_token, config.schema_version),
        };
        let hello_pkt = encode_envelope(TransportMsgKind::Hello, 0, &hello.encode());

        let deadline = Instant::now() + Duration::from_millis(config.handshake_timeout_ms);
        let mut last_send: Option<Instant> = None;
        let mut buf = [0u8; 2048];
        let negotiated_session_id;
        let negotiated_window;

        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(TransportError::HandshakeTimeout);
            }
            let should_send = last_send
                .map(|t| now.saturating_duration_since(t) >= Duration::from_millis(25))
                .unwrap_or(true);
            if should_send {
                socket.send_to(&hello_pkt, self.server_addr)?;
                self.shared.lock().unwrap().metrics.record_tx(now, hello_pkt.len());
                last_send = Some(now);
            }

            match socket.recv_from(&mut buf) {
                Ok((n, _src)) => {
                    let rx_now = Instant::now();
                    self.shared.lock().unwrap().metrics.record_rx(rx_now, n);
                    let Ok(env) = TransportEnvelopeRef::parse(&buf[..n]) else {
                        continue;
                    };
                    match env.kind {
                        TransportMsgKind::HelloAck => {
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
                            negotiated_session_id = env.session_id;
                            negotiated_window = usize::from(ack.server_max_window).max(4);
                            break;
                        }
                        TransportMsgKind::Reject => {
                            let (reason, expected_schema) = decode_reject(env.payload)?;
                            return Err(reject_to_error(
                                reason,
                                expected_schema,
                                config.schema_version,
                            ));
                        }
                        _ => {}
                    }
                }
                Err(err)
                    if err.kind() == io::ErrorKind::WouldBlock
                        || err.kind() == io::ErrorKind::TimedOut => {}
                Err(err) => return Err(TransportError::Io(err)),
            }
        }

        // Update shared session state for the newly negotiated session epoch.
        {
            let mut guard = self.shared.lock().unwrap();
            if is_reconnect || guard.session_id != negotiated_session_id {
                guard.session = SyncDeltaSession::new(negotiated_window);
            }
            guard.session_id = negotiated_session_id;
            guard.connected = true;
            guard.metrics.snapshot.session_id = negotiated_session_id;
            guard.metrics.snapshot.connected_peers = 1;
            if is_reconnect {
                guard.metrics.snapshot.reconnects += 1;
            }
            // Immediate RTT probe from client side too.
            let now = Instant::now();
            let ping = encode_ping_or_pong(
                TransportMsgKind::Ping,
                negotiated_session_id,
                guard.now_us(now),
            );
            let _ = socket.send_to(&ping, self.server_addr);
            guard.metrics.record_tx(now, ping.len());
        }

        socket.set_read_timeout(Some(Duration::from_millis(5)))?;
        let worker_sock = socket.try_clone()?;
        self.socket = Some(socket);
        self.shutdown = Arc::new(AtomicBool::new(false));

        let worker_shared = Arc::clone(&self.shared);
        let worker_tx = self.frame_tx.clone();
        let worker_shutdown = Arc::clone(&self.shutdown);
        let server_addr = self.server_addr;

        self.worker = Some(
            thread::Builder::new()
                .name("nve-udp-delta-client".into())
                .spawn(move || {
                    udp_client_loop(
                        worker_sock,
                        server_addr,
                        worker_shared,
                        worker_tx,
                        worker_shutdown,
                    );
                })?,
        );

        Ok(negotiated_session_id)
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.socket.as_ref().and_then(|s| s.local_addr().ok())
    }

    pub fn server_addr(&self) -> SocketAddr {
        self.server_addr
    }

    pub fn session_id(&self) -> u32 {
        self.shared.lock().unwrap().session_id
    }

    pub fn is_connected(&self) -> bool {
        self.shared.lock().unwrap().connected
    }

    pub fn recv_frame(&self, timeout: Duration) -> Result<Option<(u32, Vec<u8>)>, TransportError> {
        match self.frame_rx.recv_timeout(timeout) {
            Ok(item) => Ok(Some(item)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(TransportError::Disconnected),
        }
    }

    pub fn try_recv_frame(&self) -> Option<(u32, Vec<u8>)> {
        self.frame_rx.try_recv().ok()
    }

    pub fn request_keyframe(&self) -> Result<(), TransportError> {
        let Some(socket) = self.socket.as_ref() else {
            return Err(TransportError::Disconnected);
        };
        let now = Instant::now();
        let mut guard = self.shared.lock().unwrap();
        let pkt = encode_keyframe_request(guard.session_id, guard.session.last_applied_seq(), 1);
        socket.send_to(&pkt, self.server_addr)?;
        guard.metrics.record_tx(now, pkt.len());
        guard.metrics.snapshot.keyframe_resyncs += 1;
        Ok(())
    }

    pub fn send_ping(&self) -> Result<(), TransportError> {
        let Some(socket) = self.socket.as_ref() else {
            return Err(TransportError::Disconnected);
        };
        let now = Instant::now();
        let mut guard = self.shared.lock().unwrap();
        let ping = encode_ping_or_pong(
            TransportMsgKind::Ping,
            guard.session_id,
            guard.now_us(now),
        );
        socket.send_to(&ping, self.server_addr)?;
        guard.metrics.record_tx(now, ping.len());
        Ok(())
    }

    /// Gracefully sends `Bye` and stops the client's receiver loop while allowing [`reconnect`].
    pub fn disconnect(&mut self) {
        if let Some(socket) = self.socket.as_ref() {
            let sid = self.shared.lock().unwrap().session_id;
            let bye = encode_envelope(TransportMsgKind::Bye, sid, &[]);
            let _ = socket.send_to(&bye, self.server_addr);
        }
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        self.socket = None;
        let mut guard = self.shared.lock().unwrap();
        guard.connected = false;
        guard.metrics.snapshot.connected_peers = 0;
    }

    /// Gracefully reconnects to the server, negotiating a fresh session and triggering automatic
    /// keyframe resync if server state has moved ahead.
    pub fn reconnect(&mut self) -> Result<u32, TransportError> {
        self.disconnect();
        self.establish_connection(true)
    }

    pub fn metrics(&self) -> PairingMetrics {
        self.shared.lock().unwrap().metrics.snapshot(Instant::now())
    }

    pub fn shutdown(&mut self) {
        self.disconnect();
    }
}

impl Drop for UdpDeltaClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

fn udp_client_loop(
    socket: UdpSocket,
    server_addr: SocketAddr,
    shared: Arc<Mutex<UdpClientShared>>,
    frame_tx: mpsc::Sender<(u32, Vec<u8>)>,
    shutdown: Arc<AtomicBool>,
) {
    let max_frame_bytes = shared.lock().unwrap().config.clamped_max_frame_bytes();
    let mut reassembler = FragReassembler::new(max_frame_bytes);
    let mut nack_cooldowns: HashMap<u32, Instant> = HashMap::new();
    let mut last_keyframe_req: Option<Instant> = None;
    let mut last_tick = Instant::now();
    let mut last_ping_sent = Instant::now();
    let mut buf = [0u8; 2048];

    while !shutdown.load(Ordering::Relaxed) {
        match socket.recv_from(&mut buf) {
            Ok((n, _src)) => {
                let now = Instant::now();
                let mut guard = shared.lock().unwrap();
                guard.metrics.record_rx(now, n);
                let session_id = guard.session_id;
                let env = match TransportEnvelopeRef::parse(&buf[..n]) {
                    Ok(e) => e,
                    Err(_) => {
                        guard.metrics.snapshot.malformed_packets_dropped += 1;
                        continue;
                    }
                };
                if env.session_id != session_id {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                }

                match env.kind {
                    TransportMsgKind::DeltaSingle => {
                        handle_udp_nvd_packet(
                            &socket,
                            server_addr,
                            &mut guard,
                            env.payload,
                            now,
                            &frame_tx,
                            &mut nack_cooldowns,
                            &mut last_keyframe_req,
                        );
                    }
                    TransportMsgKind::DeltaFrag => match reassembler.push(env.payload, now) {
                        Ok((Some(reassembled), _)) => {
                            handle_udp_nvd_packet(
                                &socket,
                                server_addr,
                                &mut guard,
                                &reassembled,
                                now,
                                &frame_tx,
                                &mut nack_cooldowns,
                                &mut last_keyframe_req,
                            );
                        }
                        Ok((None, Some((msg_id, missing)))) => {
                            let fnack = encode_frag_nack(session_id, msg_id, &missing);
                            if socket.send_to(&fnack, server_addr).is_ok() {
                                guard.metrics.record_tx(now, fnack.len());
                                guard.metrics.snapshot.nacks_sent += 1;
                            }
                        }
                        Ok((None, None)) => {}
                        Err(_) => {
                            guard.metrics.snapshot.malformed_packets_dropped += 1;
                        }
                    },
                    TransportMsgKind::Ping => {
                        if let Ok(ts_us) = decode_timestamp_us(env.payload) {
                            let pong =
                                encode_ping_or_pong(TransportMsgKind::Pong, session_id, ts_us);
                            if socket.send_to(&pong, server_addr).is_ok() {
                                guard.metrics.record_tx(now, pong.len());
                            }
                        }
                    }
                    TransportMsgKind::Pong => {
                        if let Ok(sent_us) = decode_timestamp_us(env.payload) {
                            let rtt = guard.now_us(now).saturating_sub(sent_us);
                            guard.metrics.record_rtt_us(rtt);
                        }
                    }
                    TransportMsgKind::Bye => {
                        guard.connected = false;
                        guard.metrics.snapshot.connected_peers = 0;
                        break;
                    }
                    _ => {
                        guard.metrics.snapshot.malformed_packets_dropped += 1;
                    }
                }
            }
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }

        // Periodic gap NACK, fragment NACK, and heartbeat timer.
        let now = Instant::now();
        if now.saturating_duration_since(last_tick) >= Duration::from_millis(10) {
            last_tick = now;
            let mut guard = shared.lock().unwrap();
            let session_id = guard.session_id;
            let hb_interval = Duration::from_millis(guard.config.heartbeat_interval_ms);

            // Re-emit FragNacks for stalled partial fragmented keyframes.
            for (msg_id, missing) in reassembler.poll_stale_nacks(now, 15) {
                let fnack = encode_frag_nack(session_id, msg_id, &missing);
                if socket.send_to(&fnack, server_addr).is_ok() {
                    guard.metrics.record_tx(now, fnack.len());
                    guard.metrics.snapshot.nacks_sent += 1;
                }
            }

            // Re-emit NACK for head-of-line gap if reorder buffer is stalled.
            if let Some(missing_seq) = guard.session.next_missing_seq() {
                let can_send = nack_cooldowns
                    .get(&missing_seq)
                    .map(|t| now.saturating_duration_since(*t) >= Duration::from_millis(18))
                    .unwrap_or(true);
                if can_send {
                    nack_cooldowns.insert(missing_seq, now);
                    let nack_nvd = guard.session.build_nack_packet(missing_seq);
                    let env = encode_envelope(TransportMsgKind::DeltaSingle, session_id, &nack_nvd);
                    if socket.send_to(&env, server_addr).is_ok() {
                        guard.metrics.record_tx(now, env.len());
                        guard.metrics.snapshot.nacks_sent += 1;
                    }
                }
            }

            // Periodic heartbeat Ping for continuous RTT tracking.
            if now.saturating_duration_since(last_ping_sent) >= hb_interval {
                last_ping_sent = now;
                let ping = encode_ping_or_pong(
                    TransportMsgKind::Ping,
                    session_id,
                    guard.now_us(now),
                );
                if socket.send_to(&ping, server_addr).is_ok() {
                    guard.metrics.record_tx(now, ping.len());
                }
            }

            // Prune stale cooldown entries.
            if nack_cooldowns.len() > 128 {
                nack_cooldowns
                    .retain(|_, ts| now.saturating_duration_since(*ts) < Duration::from_secs(2));
            }
        }
    }
    reassembler.clear();
}

#[allow(clippy::too_many_arguments)]
fn handle_udp_nvd_packet(
    socket: &UdpSocket,
    server_addr: SocketAddr,
    guard: &mut UdpClientShared,
    nvd_bytes: &[u8],
    now: Instant,
    frame_tx: &mpsc::Sender<(u32, Vec<u8>)>,
    nack_cooldowns: &mut HashMap<u32, Instant>,
    last_keyframe_req: &mut Option<Instant>,
) {
    let session_id = guard.session_id;
    let Ok(parsed) = DeltaPacketRef::parse(nvd_bytes) else {
        guard.metrics.snapshot.malformed_packets_dropped += 1;
        return;
    };
    let hdr = parsed.header();

    // Detect window overflow (> max_window dropped packets) -> trigger automatic keyframe resync!
    let last_applied = guard.session.last_applied_seq();
    let max_win = guard.session.max_window() as u32;
    if hdr.kind != PacketKind::Keyframe
        && hdr.seq > last_applied
        && hdr.seq.saturating_sub(last_applied) > max_win
    {
        let can_req = last_keyframe_req
            .map(|t| now.saturating_duration_since(t) >= Duration::from_millis(25))
            .unwrap_or(true);
        if can_req {
            *last_keyframe_req = Some(now);
            let req = encode_keyframe_request(session_id, last_applied, 2);
            if socket.send_to(&req, server_addr).is_ok() {
                guard.metrics.record_tx(now, req.len());
                guard.metrics.snapshot.keyframe_resyncs += 1;
            }
        }
        return;
    }

    match guard.session.receive_packet(nvd_bytes) {
        Ok(outcome) => {
            for (seq, frame) in outcome.applied_frames {
                nack_cooldowns.remove(&seq);
                guard.metrics.snapshot.frames_received += 1;
                guard.metrics.snapshot.raw_bytes_received += frame.len() as u64;
                let _ = frame_tx.send((seq, frame));
            }
            if let Some(ack_nvd) = outcome.ack_packet {
                let ack_env = encode_envelope(TransportMsgKind::DeltaSingle, session_id, &ack_nvd);
                if socket.send_to(&ack_env, server_addr).is_ok() {
                    guard.metrics.record_tx(now, ack_env.len());
                    guard.metrics.snapshot.acks_sent += 1;
                }
            }
            for nack_nvd in outcome.nack_packets {
                let missing_seq = DeltaPacketRef::parse(&nack_nvd)
                    .ok()
                    .and_then(|p| {
                        (p.payload().len() == 4)
                            .then(|| u32::from_le_bytes(p.payload().try_into().unwrap()))
                    })
                    .unwrap_or(0);
                // RFC 9002 packet-threshold: avoid spurious NACKs on 1-packet reordering;
                // fire immediately once >= 2 later packets have arrived past `missing_seq`
                // (smaller gaps are recovered by the 18ms periodic gap-NACK timer if truly lost).
                if hdr.seq.saturating_sub(missing_seq) < 2 {
                    continue;
                }
                let can_send = nack_cooldowns
                    .get(&missing_seq)
                    .map(|t| now.saturating_duration_since(*t) >= Duration::from_millis(25))
                    .unwrap_or(true);
                if can_send {
                    nack_cooldowns.insert(missing_seq, now);
                    let nack_env =
                        encode_envelope(TransportMsgKind::DeltaSingle, session_id, &nack_nvd);
                    if socket.send_to(&nack_env, server_addr).is_ok() {
                        guard.metrics.record_tx(now, nack_env.len());
                        guard.metrics.snapshot.nacks_sent += 1;
                    }
                }
            }
        }
        Err(SyncDeltaError::BaseSequenceMismatch { .. }) => {
            // Base mismatch -> request immediate keyframe resync!
            let can_req = last_keyframe_req
                .map(|t| now.saturating_duration_since(t) >= Duration::from_millis(25))
                .unwrap_or(true);
            if can_req {
                *last_keyframe_req = Some(now);
                let req = encode_keyframe_request(session_id, guard.session.last_applied_seq(), 3);
                if socket.send_to(&req, server_addr).is_ok() {
                    guard.metrics.record_tx(now, req.len());
                    guard.metrics.snapshot.keyframe_resyncs += 1;
                }
            }
        }
        Err(_) => {
            guard.metrics.snapshot.malformed_packets_dropped += 1;
        }
    }
}

// =============================================================================
// Zero-Dependency RFC 6455 WebSocket Codec (SHA-1 + Base64 + Frame Codec)
// =============================================================================

/// Pure-Rust SHA-1 (RFC 3174) for RFC 6455 `Sec-WebSocket-Accept` computation.
fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 80];
    for chunk in padded.chunks_exact(64) {
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDCu32),
                _ => (b ^ c ^ d, 0xCA62_C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, &val) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
    }
    out
}

/// Pure-Rust RFC 4648 standard Base64 encoder.
fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 { u32::from(chunk[1]) } else { 0 };
        let b2 = if chunk.len() > 2 { u32::from(chunk[2]) } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Computes RFC 6455 `Sec-WebSocket-Accept` from `Sec-WebSocket-Key`.
pub fn compute_ws_accept_key(client_key: &str) -> String {
    let mut concat = String::with_capacity(client_key.len() + WS_GUID.len());
    concat.push_str(client_key.trim());
    concat.push_str(WS_GUID);
    base64_encode(&sha1_digest(concat.as_bytes()))
}

/// RFC 6455 WebSocket frame opcode.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsOpcode {
    Continuation = 0x0,
    Text = 0x1,
    Binary = 0x2,
    Close = 0x8,
    Ping = 0x9,
    Pong = 0xA,
}

impl WsOpcode {
    pub const fn from_u8(v: u8) -> Result<Self, TransportError> {
        match v {
            0x0 => Ok(Self::Continuation),
            0x1 => Ok(Self::Text),
            0x2 => Ok(Self::Binary),
            0x8 => Ok(Self::Close),
            0x9 => Ok(Self::Ping),
            0xA => Ok(Self::Pong),
            _ => Err(TransportError::WebSocketProtocol("unsupported opcode")),
        }
    }
}

/// Decoded RFC 6455 WebSocket frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsFrame {
    pub fin: bool,
    pub opcode: WsOpcode,
    pub masked: bool,
    pub payload: Vec<u8>,
}

/// Encodes one RFC 6455 WebSocket frame. Pass `Some(mask_key)` for client-to-server frames and
/// `None` for server-to-client frames.
pub fn encode_ws_frame(
    fin: bool,
    opcode: WsOpcode,
    payload: &[u8],
    mask_key: Option<[u8; 4]>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + payload.len());
    let b0 = (if fin { 0x80 } else { 0 }) | (opcode as u8);
    out.push(b0);

    let mask_bit = if mask_key.is_some() { 0x80u8 } else { 0u8 };
    let len = payload.len();
    if len <= 125 {
        out.push(mask_bit | (len as u8));
    } else if len <= u16::MAX as usize {
        out.push(mask_bit | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(mask_bit | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }

    if let Some(mask) = mask_key {
        out.extend_from_slice(&mask);
        for (i, &b) in payload.iter().enumerate() {
            out.push(b ^ mask[i & 3]);
        }
    } else {
        out.extend_from_slice(payload);
    }
    out
}

/// Decodes one RFC 6455 WebSocket frame from a byte slice, returning `(frame, bytes_consumed)`.
/// Enforces `max_payload_len` BEFORE allocating to reject OOM bombs.
pub fn decode_ws_frame_slice(
    bytes: &[u8],
    max_payload_len: usize,
) -> Result<(WsFrame, usize), TransportError> {
    if bytes.len() < 2 {
        return Err(TransportError::TruncatedFrame);
    }
    let fin = (bytes[0] & 0x80) != 0;
    if (bytes[0] & 0x70) != 0 {
        return Err(TransportError::WebSocketProtocol(
            "non-zero RSV bits without extension",
        ));
    }
    let opcode = WsOpcode::from_u8(bytes[0] & 0x0F)?;
    let masked = (bytes[1] & 0x80) != 0;
    let len_code = bytes[1] & 0x7F;
    let mut pos = 2usize;

    let payload_len: usize = match len_code {
        0..=125 => usize::from(len_code),
        126 => {
            if bytes.len() < pos + 2 {
                return Err(TransportError::TruncatedFrame);
            }
            let n = u16::from_be_bytes(bytes[pos..pos + 2].try_into().unwrap());
            pos += 2;
            usize::from(n)
        }
        127 => {
            if bytes.len() < pos + 8 {
                return Err(TransportError::TruncatedFrame);
            }
            let n_u64 = u64::from_be_bytes(bytes[pos..pos + 8].try_into().unwrap());
            pos += 8;
            if (n_u64 >> 63) != 0 {
                return Err(TransportError::WebSocketProtocol(
                    "MSB set on 64-bit WebSocket payload length",
                ));
            }
            usize::try_from(n_u64).map_err(|_| TransportError::FrameTooLarge {
                declared: usize::MAX,
                max: max_payload_len,
            })?
        }
        _ => unreachable!(),
    };

    if payload_len > max_payload_len {
        return Err(TransportError::FrameTooLarge {
            declared: payload_len,
            max: max_payload_len,
        });
    }

    let mask_key = if masked {
        if bytes.len() < pos + 4 {
            return Err(TransportError::TruncatedFrame);
        }
        let m: [u8; 4] = bytes[pos..pos + 4].try_into().unwrap();
        pos += 4;
        Some(m)
    } else {
        None
    };

    if bytes.len().saturating_sub(pos) < payload_len {
        return Err(TransportError::TruncatedFrame);
    }
    let mut payload = bytes[pos..pos + payload_len].to_vec();
    if let Some(mask) = mask_key {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i & 3];
        }
    }
    pos += payload_len;
    Ok((
        WsFrame {
            fin,
            opcode,
            masked,
            payload,
        },
        pos,
    ))
}

// =============================================================================
// Framed Binary Stream Transport (TcpDeltaServer & TcpDeltaClient)
// =============================================================================

/// Stream framing mode used on a TCP connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFraming {
    /// 4-byte little-endian length prefix + `NVT1` transport message.
    RawLengthPrefixed,
    /// RFC 6455 HTTP Upgrade + binary WebSocket frames carrying `NVT1` transport messages.
    WebSocketBinary,
}

enum StreamReadOutcome {
    Frame(Vec<u8>),
    Idle,
    Closed,
}

fn read_exact_interruptible(
    stream: &mut TcpStream,
    buf: &mut [u8],
    shutdown: &AtomicBool,
    allow_idle_on_zero: bool,
    stall_timeout: Duration,
) -> Result<bool, TransportError> {
    let mut offset = 0usize;
    let mut started_at: Option<Instant> = None;

    while offset < buf.len() {
        if shutdown.load(Ordering::Relaxed) {
            return Err(TransportError::Disconnected);
        }
        match stream.read(&mut buf[offset..]) {
            Ok(0) => {
                if offset == 0 {
                    return Ok(false);
                }
                return Err(TransportError::TruncatedFrame);
            }
            Ok(n) => {
                offset += n;
                if started_at.is_none() {
                    started_at = Some(Instant::now());
                }
            }
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut =>
            {
                if offset == 0 && allow_idle_on_zero {
                    return Ok(false);
                }
                let start = *started_at.get_or_insert_with(Instant::now);
                if Instant::now().saturating_duration_since(start) >= stall_timeout {
                    return Err(TransportError::TruncatedFrame);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(TransportError::Io(err)),
        }
    }
    Ok(true)
}

fn write_framed_message(
    stream: &mut TcpStream,
    framing: StreamFraming,
    is_client: bool,
    payload: &[u8],
) -> Result<usize, TransportError> {
    match framing {
        StreamFraming::RawLengthPrefixed => {
            let len_u32 = u32::try_from(payload.len()).map_err(|_| TransportError::FrameTooLarge {
                declared: payload.len(),
                max: u32::MAX as usize,
            })?;
            let mut framed = Vec::with_capacity(4 + payload.len());
            framed.extend_from_slice(&len_u32.to_le_bytes());
            framed.extend_from_slice(payload);
            stream.write_all(&framed)?;
            Ok(framed.len())
        }
        StreamFraming::WebSocketBinary => {
            let mask = is_client.then(|| {
                let n = next_nonce(payload.len() as u64);
                (n as u32).to_le_bytes()
            });
            let ws_bytes = encode_ws_frame(true, WsOpcode::Binary, payload, mask);
            stream.write_all(&ws_bytes)?;
            Ok(ws_bytes.len())
        }
    }
}

fn read_framed_message(
    stream: &mut TcpStream,
    write_stream: &Mutex<TcpStream>,
    framing: StreamFraming,
    is_client: bool,
    max_frame_bytes: usize,
    shutdown: &AtomicBool,
) -> Result<(StreamReadOutcome, usize), TransportError> {
    let stall_timeout = Duration::from_millis(500);
    match framing {
        StreamFraming::RawLengthPrefixed => {
            let mut len_buf = [0u8; 4];
            let got =
                read_exact_interruptible(stream, &mut len_buf, shutdown, true, stall_timeout)?;
            if !got {
                // Distinguish EOF from idle timeout by peeking 1 byte? No: if stream closed,
                // read_exact_interruptible saw Ok(0). Check via non-blocking? Better: inline first read.
                return Ok((StreamReadOutcome::Idle, 0));
            }
            let declared = u32::from_le_bytes(len_buf) as usize;
            if declared == 0 {
                return Err(TransportError::MalformedFrame("zero-length TCP frame"));
            }
            if declared > max_frame_bytes {
                return Err(TransportError::FrameTooLarge {
                    declared,
                    max: max_frame_bytes,
                });
            }
            let mut payload = vec![0u8; declared];
            let ok = read_exact_interruptible(stream, &mut payload, shutdown, false, stall_timeout)?;
            if !ok {
                return Err(TransportError::TruncatedFrame);
            }
            Ok((StreamReadOutcome::Frame(payload), 4 + declared))
        }
        StreamFraming::WebSocketBinary => {
            let mut assembled: Option<Vec<u8>> = None;
            let mut total_wire = 0usize;
            loop {
                let mut hdr = [0u8; 2];
                let allow_idle = assembled.is_none();
                let got = read_exact_interruptible(
                    stream,
                    &mut hdr,
                    shutdown,
                    allow_idle,
                    stall_timeout,
                )?;
                if !got {
                    return Ok((StreamReadOutcome::Idle, 0));
                }
                total_wire += 2;
                let fin = (hdr[0] & 0x80) != 0;
                if (hdr[0] & 0x70) != 0 {
                    return Err(TransportError::WebSocketProtocol("non-zero RSV bits"));
                }
                let opcode = WsOpcode::from_u8(hdr[0] & 0x0F)?;
                let masked = (hdr[1] & 0x80) != 0;
                let len_code = hdr[1] & 0x7F;

                let payload_len: usize = match len_code {
                    0..=125 => usize::from(len_code),
                    126 => {
                        let mut b = [0u8; 2];
                        read_exact_interruptible(stream, &mut b, shutdown, false, stall_timeout)?;
                        total_wire += 2;
                        usize::from(u16::from_be_bytes(b))
                    }
                    127 => {
                        let mut b = [0u8; 8];
                        read_exact_interruptible(stream, &mut b, shutdown, false, stall_timeout)?;
                        total_wire += 8;
                        let n = u64::from_be_bytes(b);
                        if (n >> 63) != 0 {
                            return Err(TransportError::WebSocketProtocol(
                                "MSB set on 64-bit WebSocket length",
                            ));
                        }
                        usize::try_from(n).map_err(|_| TransportError::FrameTooLarge {
                            declared: usize::MAX,
                            max: max_frame_bytes,
                        })?
                    }
                    _ => unreachable!(),
                };

                let acc_len = assembled.as_ref().map_or(0, Vec::len);
                if payload_len > max_frame_bytes || acc_len.saturating_add(payload_len) > max_frame_bytes
                {
                    return Err(TransportError::FrameTooLarge {
                        declared: acc_len.saturating_add(payload_len),
                        max: max_frame_bytes,
                    });
                }

                let mask = if masked {
                    let mut m = [0u8; 4];
                    read_exact_interruptible(stream, &mut m, shutdown, false, stall_timeout)?;
                    total_wire += 4;
                    Some(m)
                } else {
                    None
                };

                let mut payload = vec![0u8; payload_len];
                if payload_len > 0 {
                    read_exact_interruptible(
                        stream,
                        &mut payload,
                        shutdown,
                        false,
                        stall_timeout,
                    )?;
                    total_wire += payload_len;
                    if let Some(m) = mask {
                        for (i, b) in payload.iter_mut().enumerate() {
                            *b ^= m[i & 3];
                        }
                    }
                }

                match opcode {
                    WsOpcode::Close => {
                        return Ok((StreamReadOutcome::Closed, total_wire));
                    }
                    WsOpcode::Ping => {
                        let pong_mask = is_client.then(|| (next_nonce(7) as u32).to_le_bytes());
                        let pong = encode_ws_frame(true, WsOpcode::Pong, &payload, pong_mask);
                        if let Ok(mut w) = write_stream.lock() {
                            let _ = w.write_all(&pong);
                        }
                    }
                    WsOpcode::Pong => {}
                    WsOpcode::Binary | WsOpcode::Text => {
                        if fin && assembled.is_none() {
                            return Ok((StreamReadOutcome::Frame(payload), total_wire));
                        }
                        assembled.get_or_insert_with(Vec::new).extend_from_slice(&payload);
                        if fin {
                            return Ok((
                                StreamReadOutcome::Frame(assembled.unwrap_or_default()),
                                total_wire,
                            ));
                        }
                    }
                    WsOpcode::Continuation => {
                        let Some(acc) = assembled.as_mut() else {
                            return Err(TransportError::WebSocketProtocol(
                                "unexpected continuation frame",
                            ));
                        };
                        acc.extend_from_slice(&payload);
                        if fin {
                            return Ok((
                                StreamReadOutcome::Frame(assembled.unwrap_or_default()),
                                total_wire,
                            ));
                        }
                    }
                }
            }
        }
    }
}

/// Checks whether the initial byte of a raw/WS stream is EOF vs idle timeout by reading the first
/// byte directly, avoiding spinning on a closed TCP socket.
fn read_framed_message_eof_aware(
    stream: &mut TcpStream,
    write_stream: &Mutex<TcpStream>,
    framing: StreamFraming,
    is_client: bool,
    max_frame_bytes: usize,
    shutdown: &AtomicBool,
) -> Result<(StreamReadOutcome, usize), TransportError> {
    let mut peek_one = [0u8; 1];
    match stream.peek(&mut peek_one) {
        Ok(0) => return Ok((StreamReadOutcome::Closed, 0)),
        Ok(_) => {}
        Err(err)
            if err.kind() == io::ErrorKind::WouldBlock
                || err.kind() == io::ErrorKind::TimedOut =>
        {
            return Ok((StreamReadOutcome::Idle, 0));
        }
        Err(err) if err.kind() == io::ErrorKind::Interrupted => {
            return Ok((StreamReadOutcome::Idle, 0));
        }
        Err(err) => return Err(TransportError::Io(err)),
    }
    read_framed_message(
        stream,
        write_stream,
        framing,
        is_client,
        max_frame_bytes,
        shutdown,
    )
}

fn read_http_headers_until_crlfcrlf(
    stream: &mut TcpStream,
    timeout: Duration,
    shutdown: &AtomicBool,
) -> Result<String, TransportError> {
    let deadline = Instant::now() + timeout;
    let mut buf = Vec::with_capacity(512);
    let mut byte = [0u8; 1];
    while Instant::now() < deadline {
        if shutdown.load(Ordering::Relaxed) {
            return Err(TransportError::Disconnected);
        }
        match stream.read(&mut byte) {
            Ok(0) => return Err(TransportError::TruncatedFrame),
            Ok(1) => {
                buf.push(byte[0]);
                if buf.len() > MAX_WS_UPGRADE_HEADER_BYTES {
                    return Err(TransportError::FrameTooLarge {
                        declared: buf.len(),
                        max: MAX_WS_UPGRADE_HEADER_BYTES,
                    });
                }
                if buf.ends_with(b"\r\n\r\n") {
                    return String::from_utf8(buf)
                        .map_err(|_| TransportError::WebSocketProtocol("non-UTF8 HTTP headers"));
                }
            }
            Ok(_) => unreachable!(),
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut => {}
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(TransportError::Io(err)),
        }
    }
    Err(TransportError::HandshakeTimeout)
}

fn server_handle_ws_upgrade(
    stream: &mut TcpStream,
    timeout: Duration,
    shutdown: &AtomicBool,
) -> Result<(), TransportError> {
    let req = read_http_headers_until_crlfcrlf(stream, timeout, shutdown)?;
    let mut key: Option<&str> = None;
    let mut has_upgrade = false;
    for line in req.lines().skip(1) {
        if let Some((name, val)) = line.split_once(':') {
            let name_lower = name.trim().to_ascii_lowercase();
            let val_trim = val.trim();
            if name_lower == "sec-websocket-key" {
                key = Some(val_trim);
            } else if name_lower == "upgrade" && val_trim.eq_ignore_ascii_case("websocket") {
                has_upgrade = true;
            }
        }
    }
    let Some(client_key) = key.filter(|k| !k.is_empty() && has_upgrade) else {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
        return Err(TransportError::WebSocketProtocol(
            "missing Upgrade: websocket or Sec-WebSocket-Key",
        ));
    };
    let accept = compute_ws_accept_key(client_key);
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream.write_all(resp.as_bytes())?;
    Ok(())
}

fn client_perform_ws_upgrade(
    stream: &mut TcpStream,
    host: &str,
    path: &str,
    timeout: Duration,
    shutdown: &AtomicBool,
) -> Result<(), TransportError> {
    let nonce_a = next_nonce(0xABCD_1234).to_le_bytes();
    let nonce_b = next_nonce(0x5678_EF01).to_le_bytes();
    let mut key_raw = [0u8; 16];
    key_raw[0..8].copy_from_slice(&nonce_a);
    key_raw[8..16].copy_from_slice(&nonce_b);
    let client_key = base64_encode(&key_raw);
    let expected_accept = compute_ws_accept_key(&client_key);

    let req = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {client_key}\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n"
    );
    stream.write_all(req.as_bytes())?;

    let resp = read_http_headers_until_crlfcrlf(stream, timeout, shutdown)?;
    let first_line = resp.lines().next().unwrap_or("");
    if !first_line.contains("101") {
        return Err(TransportError::WebSocketProtocol(
            "server did not respond with HTTP 101 Switching Protocols",
        ));
    }
    let mut got_accept = None;
    for line in resp.lines().skip(1) {
        if let Some((name, val)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("sec-websocket-accept") {
                got_accept = Some(val.trim());
            }
        }
    }
    if got_accept != Some(expected_accept.as_str()) {
        return Err(TransportError::WebSocketProtocol(
            "invalid Sec-WebSocket-Accept header",
        ));
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// TcpDeltaServer
// -----------------------------------------------------------------------------

struct TcpPeerEntry {
    id: u64,
    session_id: u32,
    framing: StreamFraming,
    write_stream: Arc<Mutex<TcpStream>>,
    session: SyncDeltaSession,
}

struct TcpServerInner {
    config: PairingConfig,
    expected_proof: [u8; 16],
    started_at: Instant,
    next_peer_id: u64,
    seen_peers_total: u64,
    peers: HashMap<u64, TcpPeerEntry>,
    latest_frame: Option<Vec<u8>>,
    metrics: MetricsTracker,
}

impl TcpServerInner {
    fn now_ms(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_millis() as u64
    }

    fn now_us(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_micros() as u64
    }
}

/// Framed binary TCP + auto-detected RFC 6455 binary WebSocket delta server.
pub struct TcpDeltaServer {
    local_addr: SocketAddr,
    inner: Arc<(Mutex<TcpServerInner>, Condvar)>,
    shutdown: Arc<AtomicBool>,
    accept_worker: Option<JoinHandle<()>>,
    peer_workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl TcpDeltaServer {
    pub fn bind(addr: impl ToSocketAddrs, config: PairingConfig) -> Result<Self, TransportError> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let local_addr = listener.local_addr()?;

        let expected_proof = derive_token_proof(&config.pairing_token, config.schema_version);
        let metrics = MetricsTracker::new(
            config.schema_version,
            Duration::from_millis(config.metrics_window_ms),
        );
        let inner = Arc::new((
            Mutex::new(TcpServerInner {
                config,
                expected_proof,
                started_at: Instant::now(),
                next_peer_id: 1,
                seen_peers_total: 0,
                peers: HashMap::new(),
                latest_frame: None,
                metrics,
            }),
            Condvar::new(),
        ));
        let shutdown = Arc::new(AtomicBool::new(false));
        let peer_workers = Arc::new(Mutex::new(Vec::new()));

        let acc_inner = Arc::clone(&inner);
        let acc_shutdown = Arc::clone(&shutdown);
        let acc_workers = Arc::clone(&peer_workers);
        let accept_worker = thread::Builder::new()
            .name("nve-tcp-delta-accept".into())
            .spawn(move || {
                tcp_accept_loop(listener, acc_inner, acc_shutdown, acc_workers);
            })?;

        Ok(Self {
            local_addr,
            inner,
            shutdown,
            accept_worker: Some(accept_worker),
            peer_workers,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn peer_count(&self) -> usize {
        self.inner.0.lock().unwrap().peers.len()
    }

    pub fn wait_for_peers(&self, min_peers: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut guard = self.inner.0.lock().unwrap();
        while guard.peers.len() < min_peers {
            let rem = deadline.saturating_duration_since(Instant::now());
            if rem.is_zero() {
                return false;
            }
            let (g, _) = self.inner.1.wait_timeout(guard, rem).unwrap();
            guard = g;
        }
        true
    }

    /// Broadcasts `frame_bytes` over every paired TCP / WebSocket connection.
    pub fn send_frame(&self, frame_bytes: &[u8]) -> Result<usize, TransportError> {
        let now = Instant::now();
        let mut guard = self.inner.0.lock().unwrap();
        let max_bytes = guard.config.clamped_max_frame_bytes();
        if frame_bytes.len() > max_bytes {
            return Err(TransportError::FrameTooLarge {
                declared: frame_bytes.len(),
                max: max_bytes,
            });
        }
        guard.latest_frame = Some(frame_bytes.to_vec());
        let now_ms = guard.now_ms(now);
        let peer_ids: Vec<u64> = guard.peers.keys().copied().collect();
        let mut dead_peers = Vec::new();
        let mut sent = 0usize;

        for pid in peer_ids {
            let state = &mut *guard;
            if let Some(peer) = state.peers.get_mut(&pid) {
                let nvd_pkt = peer.session.encode_frame(frame_bytes, now_ms);
                let kind = DeltaPacketRef::parse(&nvd_pkt)
                    .map(|p| p.header().kind)
                    .unwrap_or(PacketKind::Keyframe);
                let env = encode_envelope(TransportMsgKind::DeltaSingle, peer.session_id, &nvd_pkt);
                let write_res = peer
                    .write_stream
                    .lock()
                    .ok()
                    .and_then(|mut ws| write_framed_message(&mut ws, peer.framing, false, &env).ok());
                if let Some(wire_len) = write_res {
                    state.metrics.record_tx(now, wire_len);
                    state.metrics.snapshot.frames_sent += 1;
                    state.metrics.snapshot.raw_bytes_sent += frame_bytes.len() as u64;
                    match kind {
                        PacketKind::Keyframe => state.metrics.snapshot.keyframes_sent += 1,
                        PacketKind::XorRleDelta | PacketKind::CommandOpDelta => {
                            state.metrics.snapshot.deltas_sent += 1
                        }
                        _ => {}
                    }
                    sent += 1;
                } else {
                    dead_peers.push(pid);
                }
            }
        }
        for dead in dead_peers {
            guard.peers.remove(&dead);
        }
        guard.metrics.snapshot.connected_peers = guard.peers.len();
        Ok(sent)
    }

    pub fn metrics(&self) -> PairingMetrics {
        let mut guard = self.inner.0.lock().unwrap();
        guard.metrics.snapshot.connected_peers = guard.peers.len();
        guard.metrics.snapshot(Instant::now())
    }

    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Shut down all active peer streams so reader threads exit immediately.
        {
            let guard = self.inner.0.lock().unwrap();
            for peer in guard.peers.values() {
                if let Ok(ws) = peer.write_stream.lock() {
                    let _ = ws.shutdown(Shutdown::Both);
                }
            }
        }
        if let Some(handle) = self.accept_worker.take() {
            let _ = handle.join();
        }
        let workers: Vec<JoinHandle<()>> = {
            let mut w = self.peer_workers.lock().unwrap();
            std::mem::take(&mut *w)
        };
        for handle in workers {
            let _ = handle.join();
        }
    }
}

impl Drop for TcpDeltaServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn tcp_accept_loop(
    listener: TcpListener,
    inner: Arc<(Mutex<TcpServerInner>, Condvar)>,
    shutdown: Arc<AtomicBool>,
    peer_workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
) {
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _addr)) => {
                let p_inner = Arc::clone(&inner);
                let p_shutdown = Arc::clone(&shutdown);
                if let Ok(handle) = thread::Builder::new()
                    .name("nve-tcp-delta-peer".into())
                    .spawn(move || {
                        tcp_server_peer_worker(stream, p_inner, p_shutdown);
                    })
                {
                    let mut workers = peer_workers.lock().unwrap();
                    // Reap finished threads to avoid unbounded handle growth on reconnects.
                    let mut i = 0;
                    while i < workers.len() {
                        if workers[i].is_finished() {
                            let h = workers.swap_remove(i);
                            let _ = h.join();
                        } else {
                            i += 1;
                        }
                    }
                    workers.push(handle);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    }
}

fn detect_stream_framing(
    stream: &TcpStream,
    timeout: Duration,
    shutdown: &AtomicBool,
) -> Result<StreamFraming, TransportError> {
    let deadline = Instant::now() + timeout;
    let mut prefix = [0u8; 4];
    let mut partial_tries = 0u32;

    while Instant::now() < deadline {
        if shutdown.load(Ordering::Relaxed) {
            return Err(TransportError::Disconnected);
        }
        match stream.peek(&mut prefix) {
            Ok(0) => return Err(TransportError::TruncatedFrame),
            Ok(n) if n >= 4 => {
                if &prefix[..4] == b"GET " {
                    return Ok(StreamFraming::WebSocketBinary);
                }
                return Ok(StreamFraming::RawLengthPrefixed);
            }
            Ok(n) => {
                if b"GET ".starts_with(&prefix[..n]) && partial_tries < 4 {
                    partial_tries += 1;
                    thread::sleep(Duration::from_millis(4));
                    continue;
                }
                return Ok(StreamFraming::RawLengthPrefixed);
            }
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut => {}
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(TransportError::Io(err)),
        }
    }
    Err(TransportError::HandshakeTimeout)
}

fn tcp_server_peer_worker(
    mut read_stream: TcpStream,
    inner: Arc<(Mutex<TcpServerInner>, Condvar)>,
    shutdown: Arc<AtomicBool>,
) {
    let _ = read_stream.set_nonblocking(false);
    let _ = read_stream.set_nodelay(true);
    let _ = read_stream.set_read_timeout(Some(Duration::from_millis(10)));

    let (config, expected_proof) = {
        let guard = inner.0.lock().unwrap();
        (guard.config.clone(), guard.expected_proof)
    };
    let handshake_timeout = Duration::from_millis(config.handshake_timeout_ms);
    let max_frame_bytes = config.clamped_max_frame_bytes();

    // 1. Auto-detect RawLengthPrefixed vs RFC 6455 WebSocket upgrade.
    let framing = match detect_stream_framing(&read_stream, handshake_timeout, &shutdown) {
        Ok(f) => f,
        Err(_) => {
            inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
            return;
        }
    };
    if framing == StreamFraming::WebSocketBinary
        && server_handle_ws_upgrade(&mut read_stream, handshake_timeout, &shutdown).is_err()
    {
        inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
        return;
    }

    let Ok(write_clone) = read_stream.try_clone() else {
        return;
    };
    let write_stream = Arc::new(Mutex::new(write_clone));

    // 2. Receive and validate Hello frame within handshake_timeout.
    let deadline = Instant::now() + handshake_timeout;
    let hello_bytes = loop {
        if shutdown.load(Ordering::Relaxed) || Instant::now() >= deadline {
            return;
        }
        match read_framed_message_eof_aware(
            &mut read_stream,
            &write_stream,
            framing,
            false,
            max_frame_bytes,
            &shutdown,
        ) {
            Ok((StreamReadOutcome::Frame(bytes), wire_len)) => {
                inner.0.lock().unwrap().metrics.record_rx(Instant::now(), wire_len);
                break bytes;
            }
            Ok((StreamReadOutcome::Idle, _)) => continue,
            Ok((StreamReadOutcome::Closed, _)) => {
                inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
                return;
            }
            Err(_) => {
                inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
                return;
            }
        }
    };

    let env = match TransportEnvelopeRef::parse(&hello_bytes) {
        Ok(e) if e.kind == TransportMsgKind::Hello => e,
        _ => {
            inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
            return;
        }
    };
    let Ok(hello) = HelloPayload::decode(env.payload) else {
        inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
        return;
    };

    if hello.schema_version != config.schema_version {
        let rej = encode_reject(RejectReason::SchemaVersionMismatch, config.schema_version);
        if let Ok(mut ws) = write_stream.lock() {
            let _ = write_framed_message(&mut ws, framing, false, &rej);
        }
        let mut guard = inner.0.lock().unwrap();
        guard.metrics.snapshot.rejected_handshakes += 1;
        guard.metrics.record_tx(Instant::now(), rej.len());
        return;
    }
    if !ct_eq_16(&hello.token_proof, &expected_proof) {
        let rej = encode_reject(RejectReason::InvalidPairingToken, config.schema_version);
        if let Ok(mut ws) = write_stream.lock() {
            let _ = write_framed_message(&mut ws, framing, false, &rej);
        }
        let mut guard = inner.0.lock().unwrap();
        guard.metrics.snapshot.rejected_handshakes += 1;
        guard.metrics.record_tx(Instant::now(), rej.len());
        return;
    }

    // 3. Register peer, send HelloAck + initial Ping + automatic Keyframe resync if state exists.
    let now = Instant::now();
    let server_nonce = next_nonce(hello.client_nonce ^ 0xCAFE_BABE);
    let session_id = derive_session_id(
        &config.pairing_token,
        config.schema_version,
        hello.client_nonce,
        server_nonce,
    );
    let window = config
        .clamped_window()
        .min(usize::from(hello.max_window).max(4));

    let peer_id = {
        let mut guard = inner.0.lock().unwrap();
        let pid = guard.next_peer_id;
        guard.next_peer_id += 1;
        let is_reconnect = guard.seen_peers_total > 0;
        guard.seen_peers_total += 1;
        if is_reconnect {
            guard.metrics.snapshot.reconnects += 1;
        }

        let has_latest = guard.latest_frame.is_some();
        let mut peer_session = SyncDeltaSession::new(window);

        let ack = HelloAckPayload {
            schema_version: config.schema_version,
            server_max_window: window as u16,
            server_seq: 0,
            server_nonce,
            resync_required: has_latest,
        };
        let ack_env = encode_envelope(TransportMsgKind::HelloAck, session_id, &ack.encode());
        if let Ok(mut ws) = write_stream.lock() {
            if let Ok(n) = write_framed_message(&mut ws, framing, false, &ack_env) {
                guard.metrics.record_tx(now, n);
            }
            let ping = encode_ping_or_pong(TransportMsgKind::Ping, session_id, guard.now_us(now));
            if let Ok(n) = write_framed_message(&mut ws, framing, false, &ping) {
                guard.metrics.record_tx(now, n);
            }
            if let Some(latest) = guard.latest_frame.clone() {
                let kf = peer_session.encode_keyframe(&latest, guard.now_ms(now));
                let kf_env = encode_envelope(TransportMsgKind::DeltaSingle, session_id, &kf);
                if let Ok(n) = write_framed_message(&mut ws, framing, false, &kf_env) {
                    guard.metrics.record_tx(now, n);
                    guard.metrics.snapshot.frames_sent += 1;
                    guard.metrics.snapshot.keyframes_sent += 1;
                    guard.metrics.snapshot.keyframe_resyncs += 1;
                    guard.metrics.snapshot.raw_bytes_sent += latest.len() as u64;
                }
            }
        }

        guard.peers.insert(
            pid,
            TcpPeerEntry {
                id: pid,
                session_id,
                framing,
                write_stream: Arc::clone(&write_stream),
                session: peer_session,
            },
        );
        guard.metrics.snapshot.session_id = session_id;
        guard.metrics.snapshot.connected_peers = guard.peers.len();
        inner.1.notify_all();
        pid
    };

    // 4. Active peer loop: process incoming ACK/NACK/KeyframeRequest/Ping/Pong/Bye.
    let hb_interval = Duration::from_millis(config.heartbeat_interval_ms);
    let mut last_ping_sent = Instant::now();

    while !shutdown.load(Ordering::Relaxed) {
        match read_framed_message_eof_aware(
            &mut read_stream,
            &write_stream,
            framing,
            false,
            max_frame_bytes,
            &shutdown,
        ) {
            Ok((StreamReadOutcome::Frame(bytes), wire_len)) => {
                let now = Instant::now();
                let mut guard = inner.0.lock().unwrap();
                guard.metrics.record_rx(now, wire_len);
                let Ok(env) = TransportEnvelopeRef::parse(&bytes) else {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                };
                if env.session_id != session_id {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                }
                match env.kind {
                    TransportMsgKind::DeltaSingle => {
                        let now_ms = guard.now_ms(now);
                        let state = &mut *guard;
                        if let Some(peer) = state.peers.get_mut(&peer_id) {
                            if let Ok(parsed) = DeltaPacketRef::parse(env.payload) {
                                match parsed.header().kind {
                                    PacketKind::Ack => {
                                        if peer.session.handle_ack(env.payload).is_ok() {
                                            state.metrics.snapshot.acks_received += 1;
                                        }
                                    }
                                    PacketKind::Nack => {
                                        state.metrics.snapshot.nacks_received += 1;
                                        if let Ok(Some(retx)) =
                                            peer.session.handle_nack(env.payload, now_ms)
                                        {
                                            let retx_env = encode_envelope(
                                                TransportMsgKind::DeltaSingle,
                                                session_id,
                                                &retx,
                                            );
                                            if let Ok(mut ws) = peer.write_stream.lock() {
                                                if let Ok(n) = write_framed_message(
                                                    &mut ws,
                                                    peer.framing,
                                                    false,
                                                    &retx_env,
                                                ) {
                                                    state.metrics.record_tx(now, n);
                                                    state.metrics.snapshot.retransmits_sent += 1;
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            } else {
                                state.metrics.snapshot.malformed_packets_dropped += 1;
                            }
                        }
                    }
                    TransportMsgKind::KeyframeRequest => {
                        if decode_keyframe_request(env.payload).is_err() {
                            guard.metrics.snapshot.malformed_packets_dropped += 1;
                            continue;
                        }
                        let now_ms = guard.now_ms(now);
                        let latest_opt = guard.latest_frame.clone();
                        let state = &mut *guard;
                        if let (Some(peer), Some(latest)) =
                            (state.peers.get_mut(&peer_id), latest_opt)
                        {
                            let kf = peer.session.encode_keyframe(&latest, now_ms);
                            let kf_env =
                                encode_envelope(TransportMsgKind::DeltaSingle, session_id, &kf);
                            if let Ok(mut ws) = peer.write_stream.lock() {
                                if let Ok(n) =
                                    write_framed_message(&mut ws, peer.framing, false, &kf_env)
                                {
                                    state.metrics.record_tx(now, n);
                                    state.metrics.snapshot.frames_sent += 1;
                                    state.metrics.snapshot.keyframes_sent += 1;
                                    state.metrics.snapshot.keyframe_resyncs += 1;
                                    state.metrics.snapshot.raw_bytes_sent += latest.len() as u64;
                                }
                            }
                        }
                    }
                    TransportMsgKind::Ping => {
                        if let Ok(ts_us) = decode_timestamp_us(env.payload) {
                            let pong =
                                encode_ping_or_pong(TransportMsgKind::Pong, session_id, ts_us);
                            if let Ok(mut ws) = write_stream.lock() {
                                if let Ok(n) = write_framed_message(&mut ws, framing, false, &pong)
                                {
                                    guard.metrics.record_tx(now, n);
                                }
                            }
                        }
                    }
                    TransportMsgKind::Pong => {
                        if let Ok(sent_us) = decode_timestamp_us(env.payload) {
                            let rtt = guard.now_us(now).saturating_sub(sent_us);
                            guard.metrics.record_rtt_us(rtt);
                        }
                    }
                    TransportMsgKind::Bye => break,
                    _ => {
                        guard.metrics.snapshot.malformed_packets_dropped += 1;
                    }
                }
            }
            Ok((StreamReadOutcome::Idle, _)) => {
                let now = Instant::now();
                if now.saturating_duration_since(last_ping_sent) >= hb_interval {
                    last_ping_sent = now;
                    let mut guard = inner.0.lock().unwrap();
                    let ping =
                        encode_ping_or_pong(TransportMsgKind::Ping, session_id, guard.now_us(now));
                    if let Ok(mut ws) = write_stream.lock() {
                        if let Ok(n) = write_framed_message(&mut ws, framing, false, &ping) {
                            guard.metrics.record_tx(now, n);
                        } else {
                            break;
                        }
                    }
                }
            }
            Ok((StreamReadOutcome::Closed, _)) => break,
            Err(_) => {
                inner.0.lock().unwrap().metrics.snapshot.malformed_packets_dropped += 1;
                break;
            }
        }
    }

    let mut guard = inner.0.lock().unwrap();
    if let Some(removed) = guard.peers.remove(&peer_id) {
        let _ = removed.id;
    }
    guard.metrics.snapshot.connected_peers = guard.peers.len();
}

// -----------------------------------------------------------------------------
// TcpDeltaClient
// -----------------------------------------------------------------------------

struct TcpClientShared {
    config: PairingConfig,
    started_at: Instant,
    session_id: u32,
    session: SyncDeltaSession,
    metrics: MetricsTracker,
    connected: bool,
}

impl TcpClientShared {
    fn now_us(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_micros() as u64
    }
}

/// Framed binary TCP / RFC 6455 binary WebSocket delta client with mid-stream reconnect and
/// automatic keyframe resync.
pub struct TcpDeltaClient {
    server_addr: SocketAddr,
    framing: StreamFraming,
    ws_path: String,
    write_stream: Option<Arc<Mutex<TcpStream>>>,
    shared: Arc<Mutex<TcpClientShared>>,
    frame_rx: mpsc::Receiver<(u32, Vec<u8>)>,
    frame_tx: mpsc::Sender<(u32, Vec<u8>)>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl TcpDeltaClient {
    /// Connects using compact 4-byte length-prefixed binary framing.
    pub fn connect(server_addr: SocketAddr, config: PairingConfig) -> Result<Self, TransportError> {
        Self::connect_with_framing(server_addr, StreamFraming::RawLengthPrefixed, "/", config)
    }

    /// Connects using an RFC 6455 HTTP/1.1 WebSocket upgrade and masked binary WebSocket frames.
    pub fn connect_websocket(
        server_addr: SocketAddr,
        path: &str,
        config: PairingConfig,
    ) -> Result<Self, TransportError> {
        Self::connect_with_framing(server_addr, StreamFraming::WebSocketBinary, path, config)
    }

    fn connect_with_framing(
        server_addr: SocketAddr,
        framing: StreamFraming,
        ws_path: &str,
        config: PairingConfig,
    ) -> Result<Self, TransportError> {
        let (frame_tx, frame_rx) = mpsc::channel();
        let metrics = MetricsTracker::new(
            config.schema_version,
            Duration::from_millis(config.metrics_window_ms),
        );
        let shared = Arc::new(Mutex::new(TcpClientShared {
            session: SyncDeltaSession::new(config.clamped_window()),
            config,
            started_at: Instant::now(),
            session_id: 0,
            metrics,
            connected: false,
        }));
        let mut client = Self {
            server_addr,
            framing,
            ws_path: ws_path.to_string(),
            write_stream: None,
            shared,
            frame_rx,
            frame_tx,
            shutdown: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        client.establish_connection(false)?;
        Ok(client)
    }

    fn establish_connection(&mut self, is_reconnect: bool) -> Result<u32, TransportError> {
        let (config, client_last_seq) = {
            let guard = self.shared.lock().unwrap();
            (guard.config.clone(), guard.session.last_applied_seq())
        };
        let handshake_timeout = Duration::from_millis(config.handshake_timeout_ms);
        let max_frame_bytes = config.clamped_max_frame_bytes();

        let mut read_stream = TcpStream::connect_timeout(&self.server_addr, handshake_timeout)?;
        read_stream.set_nodelay(true)?;
        read_stream.set_read_timeout(Some(Duration::from_millis(10)))?;

        let shutdown = Arc::new(AtomicBool::new(false));
        if self.framing == StreamFraming::WebSocketBinary {
            let host = self.server_addr.to_string();
            client_perform_ws_upgrade(
                &mut read_stream,
                &host,
                &self.ws_path,
                handshake_timeout,
                &shutdown,
            )?;
        }

        let write_stream = Arc::new(Mutex::new(read_stream.try_clone()?));

        // Send Hello.
        let client_nonce = next_nonce(u64::from(client_last_seq) ^ 0x2468_ACE0);
        let hello = HelloPayload {
            schema_version: config.schema_version,
            max_window: config.clamped_window() as u16,
            client_last_seq,
            client_nonce,
            token_proof: derive_token_proof(&config.pairing_token, config.schema_version),
        };
        let hello_env = encode_envelope(TransportMsgKind::Hello, 0, &hello.encode());
        {
            let mut ws = write_stream.lock().unwrap();
            let n = write_framed_message(&mut ws, self.framing, true, &hello_env)?;
            self.shared.lock().unwrap().metrics.record_tx(Instant::now(), n);
        }

        // Wait for HelloAck or Reject.
        let deadline = Instant::now() + handshake_timeout;
        let (negotiated_sid, negotiated_window) = loop {
            if Instant::now() >= deadline {
                return Err(TransportError::HandshakeTimeout);
            }
            match read_framed_message_eof_aware(
                &mut read_stream,
                &write_stream,
                self.framing,
                true,
                max_frame_bytes,
                &shutdown,
            )? {
                (StreamReadOutcome::Frame(bytes), wire_len) => {
                    self.shared
                        .lock()
                        .unwrap()
                        .metrics
                        .record_rx(Instant::now(), wire_len);
                    let env = TransportEnvelopeRef::parse(&bytes)?;
                    match env.kind {
                        TransportMsgKind::HelloAck => {
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
                            break (env.session_id, usize::from(ack.server_max_window).max(4));
                        }
                        TransportMsgKind::Reject => {
                            let (reason, expected_schema) = decode_reject(env.payload)?;
                            return Err(reject_to_error(
                                reason,
                                expected_schema,
                                config.schema_version,
                            ));
                        }
                        _ => return Err(TransportError::MalformedFrame("unexpected handshake reply")),
                    }
                }
                (StreamReadOutcome::Idle, _) => continue,
                (StreamReadOutcome::Closed, _) => return Err(TransportError::Disconnected),
            }
        };

        {
            let mut guard = self.shared.lock().unwrap();
            if is_reconnect || guard.session_id != negotiated_sid {
                guard.session = SyncDeltaSession::new(negotiated_window);
            }
            guard.session_id = negotiated_sid;
            guard.connected = true;
            guard.metrics.snapshot.session_id = negotiated_sid;
            guard.metrics.snapshot.connected_peers = 1;
            if is_reconnect {
                guard.metrics.snapshot.reconnects += 1;
            }
            let now = Instant::now();
            let ping = encode_ping_or_pong(
                TransportMsgKind::Ping,
                negotiated_sid,
                guard.now_us(now),
            );
            if let Ok(mut ws) = write_stream.lock() {
                if let Ok(n) = write_framed_message(&mut ws, self.framing, true, &ping) {
                    guard.metrics.record_tx(now, n);
                }
            }
        }

        self.write_stream = Some(Arc::clone(&write_stream));
        self.shutdown = Arc::clone(&shutdown);

        let worker_shared = Arc::clone(&self.shared);
        let worker_tx = self.frame_tx.clone();
        let worker_framing = self.framing;
        self.worker = Some(
            thread::Builder::new()
                .name("nve-tcp-delta-client".into())
                .spawn(move || {
                    tcp_client_loop(
                        read_stream,
                        write_stream,
                        worker_framing,
                        worker_shared,
                        worker_tx,
                        shutdown,
                    );
                })?,
        );

        Ok(negotiated_sid)
    }

    pub fn framing(&self) -> StreamFraming {
        self.framing
    }

    pub fn session_id(&self) -> u32 {
        self.shared.lock().unwrap().session_id
    }

    pub fn is_connected(&self) -> bool {
        self.shared.lock().unwrap().connected
    }

    pub fn recv_frame(&self, timeout: Duration) -> Result<Option<(u32, Vec<u8>)>, TransportError> {
        match self.frame_rx.recv_timeout(timeout) {
            Ok(item) => Ok(Some(item)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(TransportError::Disconnected),
        }
    }

    pub fn try_recv_frame(&self) -> Option<(u32, Vec<u8>)> {
        self.frame_rx.try_recv().ok()
    }

    pub fn request_keyframe(&self) -> Result<(), TransportError> {
        let Some(ws_arc) = self.write_stream.as_ref() else {
            return Err(TransportError::Disconnected);
        };
        let now = Instant::now();
        let mut guard = self.shared.lock().unwrap();
        let pkt = encode_keyframe_request(guard.session_id, guard.session.last_applied_seq(), 1);
        let mut ws = ws_arc.lock().unwrap();
        let n = write_framed_message(&mut ws, self.framing, true, &pkt)?;
        guard.metrics.record_tx(now, n);
        guard.metrics.snapshot.keyframe_resyncs += 1;
        Ok(())
    }

    pub fn send_ping(&self) -> Result<(), TransportError> {
        let Some(ws_arc) = self.write_stream.as_ref() else {
            return Err(TransportError::Disconnected);
        };
        let now = Instant::now();
        let mut guard = self.shared.lock().unwrap();
        let ping = encode_ping_or_pong(
            TransportMsgKind::Ping,
            guard.session_id,
            guard.now_us(now),
        );
        let mut ws = ws_arc.lock().unwrap();
        let n = write_framed_message(&mut ws, self.framing, true, &ping)?;
        guard.metrics.record_tx(now, n);
        Ok(())
    }

    pub fn disconnect(&mut self) {
        if let Some(ws_arc) = self.write_stream.take() {
            let sid = self.shared.lock().unwrap().session_id;
            let bye = encode_envelope(TransportMsgKind::Bye, sid, &[]);
            if let Ok(mut ws) = ws_arc.lock() {
                let _ = write_framed_message(&mut ws, self.framing, true, &bye);
                let _ = ws.shutdown(Shutdown::Both);
            }
        }
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        let mut guard = self.shared.lock().unwrap();
        guard.connected = false;
        guard.metrics.snapshot.connected_peers = 0;
    }

    /// Reconnects to the server and automatically resyncs via server Keyframe.
    pub fn reconnect(&mut self) -> Result<u32, TransportError> {
        self.disconnect();
        self.establish_connection(true)
    }

    pub fn metrics(&self) -> PairingMetrics {
        self.shared.lock().unwrap().metrics.snapshot(Instant::now())
    }

    pub fn shutdown(&mut self) {
        self.disconnect();
    }
}

impl Drop for TcpDeltaClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

fn tcp_client_loop(
    mut read_stream: TcpStream,
    write_stream: Arc<Mutex<TcpStream>>,
    framing: StreamFraming,
    shared: Arc<Mutex<TcpClientShared>>,
    frame_tx: mpsc::Sender<(u32, Vec<u8>)>,
    shutdown: Arc<AtomicBool>,
) {
    let (max_frame_bytes, hb_interval) = {
        let guard = shared.lock().unwrap();
        (
            guard.config.clamped_max_frame_bytes(),
            Duration::from_millis(guard.config.heartbeat_interval_ms),
        )
    };
    let mut last_ping_sent = Instant::now();

    while !shutdown.load(Ordering::Relaxed) {
        match read_framed_message_eof_aware(
            &mut read_stream,
            &write_stream,
            framing,
            true,
            max_frame_bytes,
            &shutdown,
        ) {
            Ok((StreamReadOutcome::Frame(bytes), wire_len)) => {
                let now = Instant::now();
                let mut guard = shared.lock().unwrap();
                guard.metrics.record_rx(now, wire_len);
                let session_id = guard.session_id;
                let Ok(env) = TransportEnvelopeRef::parse(&bytes) else {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                };
                if env.session_id != session_id {
                    guard.metrics.snapshot.malformed_packets_dropped += 1;
                    continue;
                }
                match env.kind {
                    TransportMsgKind::DeltaSingle => {
                        match guard.session.receive_packet(env.payload) {
                            Ok(outcome) => {
                                for (seq, frame) in outcome.applied_frames {
                                    guard.metrics.snapshot.frames_received += 1;
                                    guard.metrics.snapshot.raw_bytes_received += frame.len() as u64;
                                    let _ = frame_tx.send((seq, frame));
                                }
                                if let Some(ack_nvd) = outcome.ack_packet {
                                    let ack_env = encode_envelope(
                                        TransportMsgKind::DeltaSingle,
                                        session_id,
                                        &ack_nvd,
                                    );
                                    if let Ok(mut ws) = write_stream.lock() {
                                        if let Ok(n) =
                                            write_framed_message(&mut ws, framing, true, &ack_env)
                                        {
                                            guard.metrics.record_tx(now, n);
                                            guard.metrics.snapshot.acks_sent += 1;
                                        }
                                    }
                                }
                            }
                            Err(SyncDeltaError::BaseSequenceMismatch { .. }) => {
                                let req = encode_keyframe_request(
                                    session_id,
                                    guard.session.last_applied_seq(),
                                    3,
                                );
                                if let Ok(mut ws) = write_stream.lock() {
                                    if let Ok(n) =
                                        write_framed_message(&mut ws, framing, true, &req)
                                    {
                                        guard.metrics.record_tx(now, n);
                                        guard.metrics.snapshot.keyframe_resyncs += 1;
                                    }
                                }
                            }
                            Err(_) => {
                                guard.metrics.snapshot.malformed_packets_dropped += 1;
                            }
                        }
                    }
                    TransportMsgKind::Ping => {
                        if let Ok(ts_us) = decode_timestamp_us(env.payload) {
                            let pong =
                                encode_ping_or_pong(TransportMsgKind::Pong, session_id, ts_us);
                            if let Ok(mut ws) = write_stream.lock() {
                                if let Ok(n) = write_framed_message(&mut ws, framing, true, &pong) {
                                    guard.metrics.record_tx(now, n);
                                }
                            }
                        }
                    }
                    TransportMsgKind::Pong => {
                        if let Ok(sent_us) = decode_timestamp_us(env.payload) {
                            let rtt = guard.now_us(now).saturating_sub(sent_us);
                            guard.metrics.record_rtt_us(rtt);
                        }
                    }
                    TransportMsgKind::Bye => break,
                    _ => {
                        guard.metrics.snapshot.malformed_packets_dropped += 1;
                    }
                }
            }
            Ok((StreamReadOutcome::Idle, _)) => {
                let now = Instant::now();
                if now.saturating_duration_since(last_ping_sent) >= hb_interval {
                    last_ping_sent = now;
                    let mut guard = shared.lock().unwrap();
                    let ping = encode_ping_or_pong(
                        TransportMsgKind::Ping,
                        guard.session_id,
                        guard.now_us(now),
                    );
                    if let Ok(mut ws) = write_stream.lock() {
                        if let Ok(n) = write_framed_message(&mut ws, framing, true, &ping) {
                            guard.metrics.record_tx(now, n);
                        } else {
                            break;
                        }
                    }
                }
            }
            Ok((StreamReadOutcome::Closed, _)) => break,
            Err(_) => {
                shared
                    .lock()
                    .unwrap()
                    .metrics
                    .snapshot
                    .malformed_packets_dropped += 1;
                break;
            }
        }
    }

    let mut guard = shared.lock().unwrap();
    guard.connected = false;
    guard.metrics.snapshot.connected_peers = 0;
}

// =============================================================================
// Unit Tests (RFC 6455 vectors, MTU packetization, token derivation)
// =============================================================================

#[cfg(test)]
mod tests {
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
}
