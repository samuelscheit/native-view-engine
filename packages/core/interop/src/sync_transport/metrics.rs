//! Live sliding-window bandwidth telemetry, EWMA RTT µs tracking, event counters,
//! and shared client/handshake session helpers reused by UDP and TCP/WS transports.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::sync_delta::{PacketKind, SyncDeltaSession};

use super::protocol::{
    ct_eq_16, derive_session_id, encode_envelope, next_nonce, HelloAckPayload, HelloPayload,
    PairingConfig, RejectReason, TransportError, TransportMsgKind,
};

#[derive(Debug, Clone)]
struct SlidingWindowMeter {
    window: Duration,
    samples: VecDeque<(Instant, usize)>,
    total_bytes: usize,
}

impl SlidingWindowMeter {
    fn new(window: Duration) -> Self {
        Self { window, samples: VecDeque::with_capacity(128), total_bytes: 0 }
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
        (self.total_bytes as f64 / secs, self.samples.len() as f64 / secs)
    }
}

/// Snapshot of live pairing handshake, reliability, bandwidth, input-event, and RTT telemetry.
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
    pub events_rx: u64,
    pub events_tx: u64,
    pub event_bytes_rx: u64,
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
    pub fn tx_kbps(&self) -> f64 { self.wire_bytes_per_sec_tx / 1024.0 }

    /// Current sliding-window inbound wire bandwidth in KiB/s.
    pub fn rx_kbps(&self) -> f64 { self.wire_bytes_per_sec_rx / 1024.0 }

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
pub(crate) struct MetricsTracker {
    pub snapshot: PairingMetrics,
    tx_meter: SlidingWindowMeter,
    rx_meter: SlidingWindowMeter,
}

impl MetricsTracker {
    pub(crate) fn new(schema_version: u16, window: Duration) -> Self {
        let snapshot = PairingMetrics {
            schema_version,
            ..PairingMetrics::default()
        };
        Self {
            snapshot,
            tx_meter: SlidingWindowMeter::new(window),
            rx_meter: SlidingWindowMeter::new(window),
        }
    }

    pub(crate) fn record_tx(&mut self, now: Instant, wire_bytes: usize) {
        self.snapshot.wire_bytes_sent += wire_bytes as u64;
        self.snapshot.packets_sent += 1;
        self.tx_meter.record(now, wire_bytes);
    }

    pub(crate) fn record_rx(&mut self, now: Instant, wire_bytes: usize) {
        self.snapshot.wire_bytes_received += wire_bytes as u64;
        self.snapshot.packets_received += 1;
        self.rx_meter.record(now, wire_bytes);
    }

    pub(crate) fn record_frame_sent(&mut self, kind: PacketKind, raw_bytes: usize, is_resync: bool) {
        self.snapshot.frames_sent += 1;
        self.snapshot.raw_bytes_sent += raw_bytes as u64;
        match kind {
            PacketKind::Keyframe => self.snapshot.keyframes_sent += 1,
            PacketKind::XorRleDelta | PacketKind::CommandOpDelta => self.snapshot.deltas_sent += 1,
            _ => {}
        }
        if is_resync {
            self.snapshot.keyframe_resyncs += 1;
        }
    }

    pub(crate) fn record_event_rx(&mut self, payload_bytes: usize) {
        self.snapshot.events_rx += 1;
        self.snapshot.event_bytes_rx += payload_bytes as u64;
    }

    pub(crate) fn record_event_tx(&mut self) {
        self.snapshot.events_tx += 1;
    }

    pub(crate) fn record_rtt_us(&mut self, rtt_us: u64) {
        self.snapshot.last_rtt_us = Some(rtt_us);
        self.snapshot.rtt_samples += 1;
        self.snapshot.min_rtt_us = Some(self.snapshot.min_rtt_us.map_or(rtt_us, |p| p.min(rtt_us)));
        self.snapshot.smoothed_rtt_us = Some(
            self.snapshot
                .smoothed_rtt_us
                .map_or(rtt_us, |srtt| (srtt * 7 + rtt_us) / 8),
        );
    }

    pub(crate) fn snapshot(&mut self, now: Instant) -> PairingMetrics {
        let (tx_bps, tx_pps) = self.tx_meter.rates(now);
        let (rx_bps, rx_pps) = self.rx_meter.rates(now);
        self.snapshot.wire_bytes_per_sec_tx = tx_bps;
        self.snapshot.packets_per_sec_tx = tx_pps;
        self.snapshot.wire_bytes_per_sec_rx = rx_bps;
        self.snapshot.packets_per_sec_rx = rx_pps;
        self.snapshot.clone()
    }
}

/// Shared client session + telemetry state reused by both `UdpDeltaClient` and `TcpDeltaClient`.
pub(crate) struct ClientSharedState {
    pub config: PairingConfig,
    pub started_at: Instant,
    pub session_id: u32,
    pub session: SyncDeltaSession,
    pub metrics: MetricsTracker,
    pub connected: bool,
}

impl ClientSharedState {
    pub(crate) fn new(config: PairingConfig) -> Self {
        let metrics = MetricsTracker::new(
            config.schema_version,
            Duration::from_millis(config.metrics_window_ms),
        );
        let session = SyncDeltaSession::new(config.clamped_window());
        Self {
            config,
            started_at: Instant::now(),
            session_id: 0,
            session,
            metrics,
            connected: false,
        }
    }

    pub(crate) fn now_us(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_micros() as u64
    }

    pub(crate) fn on_connected(&mut self, sid: u32, window: usize, is_reconnect: bool) {
        if is_reconnect || self.session_id != sid {
            self.session = SyncDeltaSession::new(window);
        }
        self.session_id = sid;
        self.connected = true;
        self.metrics.snapshot.session_id = sid;
        self.metrics.snapshot.connected_peers = 1;
        if is_reconnect {
            self.metrics.snapshot.reconnects += 1;
        }
    }

    pub(crate) fn on_disconnected(&mut self) {
        self.connected = false;
        self.metrics.snapshot.connected_peers = 0;
    }
}

pub(crate) fn recv_frame_timeout(
    rx: &mpsc::Receiver<(u32, Vec<u8>)>,
    timeout: Duration,
) -> Result<Option<(u32, Vec<u8>)>, TransportError> {
    match rx.recv_timeout(timeout) {
        Ok(item) => Ok(Some(item)),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(TransportError::Disconnected),
    }
}

/// Outcome of validating a client `Hello` payload on the server side.
pub(crate) struct AcceptedHello {
    pub session_id: u32,
    pub window: usize,
    pub ack_envelope: Vec<u8>,
}

/// Validates a client `Hello`, returning either `(session_id, window, ack_envelope)` or a `RejectReason`.
pub(crate) fn accept_server_hello(
    hello: HelloPayload,
    config: &PairingConfig,
    expected_proof: &[u8; 16],
    nonce_salt: u64,
    resync_required: bool,
) -> Result<AcceptedHello, RejectReason> {
    if hello.schema_version != config.schema_version {
        return Err(RejectReason::SchemaVersionMismatch);
    }
    if !ct_eq_16(&hello.token_proof, expected_proof) {
        return Err(RejectReason::InvalidPairingToken);
    }
    let server_nonce = next_nonce(hello.client_nonce ^ nonce_salt);
    let session_id = derive_session_id(
        &config.pairing_token,
        config.schema_version,
        hello.client_nonce,
        server_nonce,
    );
    let window = config.clamped_window().min(usize::from(hello.max_window).max(4));
    let ack = HelloAckPayload {
        schema_version: config.schema_version,
        server_max_window: window as u16,
        server_seq: 0,
        server_nonce,
        resync_required,
    };
    let ack_envelope = encode_envelope(TransportMsgKind::HelloAck, session_id, &ack.encode());
    Ok(AcceptedHello { session_id, window, ack_envelope })
}
