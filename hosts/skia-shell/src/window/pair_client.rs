//! Live `NveWireFrame` v2 thin-client pairing adapter (`PairedRemoteApp`) connecting
//! `nve-skia-shell` directly to a remote `nve pair` host via [`nve_wire::ReconnectingWireClient`]
//! over QUIC (`nve-wire-2`), HTTP/3 WebTransport (`h3`), TLS WebSocket (`wss://`), or
//! Happy-Eyeballs auto-fallback with automatic exponential-backoff reconnect & keyframe recovery.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nve_abi::{EventBatchBuf, FrameId, SurfaceId, ValidatedEvents};
use nve_store::wire::WireDecoder;
use nve_wire::{
    parse_wire_uri, NveWireClientConfig, NveWireKind, NveWireMetrics, NveWireTransportKind,
    NveWireUri, ReconnectConfig, ReconnectingWireClient, WireInboundItem, WireResyncState,
    DEFAULT_WIRE_WSS_PATH,
};

use super::{DesktopApp, DesktopHostError};

/// Default WebSocket / WebTransport path served by `nve pair`.
pub const DEFAULT_PAIR_WS_PATH: &str = DEFAULT_WIRE_WSS_PATH;

/// Maximum queued wire frames retained between ticks before triggering keyframe resync.
const MAX_PENDING_REMOTE_FRAMES: usize = 256;

/// Connection parameters for live thin-client pairing mode (`--pair-connect`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairConnectConfig {
    /// Target `nve://pair?...` URI, `quic://`, `https://`, `wss://`, `ws://`, or `host:port`.
    pub addr: String,
    /// Shared Bearer token printed by `nve pair`.
    pub token: String,
    /// Wire transport protocol (`auto`, `quic`, `webtransport`, `wss`, or `ws`).
    pub transport: NveWireTransportKind,
    /// Optional pinned SHA-256 leaf certificate digest (auto-parsed from `nve://pair?...`).
    pub cert_sha256: Option<[u8; 32]>,
    /// WebSocket / WebTransport path.
    pub ws_path: String,
    /// Maximum time to wait for Frame 1 on initial connect / `--screenshot`.
    pub initial_frame_timeout: Duration,
}

impl PairConnectConfig {
    pub fn new(
        addr: impl Into<String>,
        token: impl Into<String>,
        transport: NveWireTransportKind,
    ) -> Self {
        Self {
            addr: addr.into(),
            token: token.into(),
            transport,
            cert_sha256: None,
            ws_path: DEFAULT_PAIR_WS_PATH.to_string(),
            initial_frame_timeout: Duration::from_secs(5),
        }
    }

    #[cfg(test)]
    pub fn with_cert_sha256(mut self, sha256: [u8; 32]) -> Self {
        self.cert_sha256 = Some(sha256);
        self
    }

    pub fn resolve_uri(&self) -> Result<NveWireUri, DesktopHostError> {
        let mut uri = parse_wire_uri(&self.addr).map_err(|e| {
            DesktopHostError::Pairing(format!("invalid pair endpoint '{}': {e}", self.addr))
        })?;
        if self.transport != NveWireTransportKind::Auto {
            uri.transport = self.transport;
        }
        if !self.token.trim().is_empty() {
            uri.token = self.token.clone();
        }
        if let Some(hash) = self.cert_sha256 {
            uri.cert_sha256 = Some(hash);
        }
        if uri.path.is_empty() {
            uri.path = self.ws_path.clone();
        }
        Ok(uri)
    }
}

struct QueuedRemoteFrame {
    session_id: u64,
    is_keyframe: bool,
    is_frame1: bool,
    payload: Vec<u8>,
}

/// Live thin-client [`DesktopApp`] backed directly by [`ReconnectingWireClient`].
///
/// Queues ordered frames in a bounded [`VecDeque`] and shares [`WireResyncState`] with
/// `nve-runtime` so incremental `CommandDelta` batches apply sequentially and recover cleanly on
/// reconnect, overflow, or decode failure.
pub struct PairedRemoteApp {
    client: ReconnectingWireClient,
    endpoint_label: String,
    pending_frames: VecDeque<QueuedRemoteFrame>,
    has_seen_frame: bool,
    resync: WireResyncState,
    last_dequeued_session_id: u64,
    last_dequeued_resets: bool,
}

impl PairedRemoteApp {
    /// Connects to a remote `nve pair` server over QUIC/WebTransport/WSS/WS using the shared
    /// single-thread [`ReconnectingWireClient`].
    pub fn connect(config: PairConnectConfig) -> Result<Self, DesktopHostError> {
        let uri = config.resolve_uri()?;
        if uri.token.trim().is_empty() {
            return Err(DesktopHostError::Pairing(
                "missing --pair-token for --pair-connect".to_string(),
            ));
        }

        let mut wire_cfg = NveWireClientConfig::new(uri.token.clone(), "nve-skia-shell");
        wire_cfg.cert_sha256 = uri.cert_sha256;

        let client =
            ReconnectingWireClient::connect_sync(uri, wire_cfg, ReconnectConfig::default(), None)
                .map_err(|e| {
                    DesktopHostError::Pairing(format!(
                        "wire pair connect to {} failed: {e}",
                        config.addr
                    ))
                })?;

        Ok(Self {
            client,
            endpoint_label: config.addr,
            pending_frames: VecDeque::new(),
            has_seen_frame: false,
            resync: WireResyncState::new(),
            last_dequeued_session_id: 0,
            last_dequeued_resets: false,
        })
    }

    #[cfg(test)]
    pub fn spawn_for_test(config: PairConnectConfig) -> Result<Self, DesktopHostError> {
        let uri = config.resolve_uri()?;
        if uri.token.trim().is_empty() {
            return Err(DesktopHostError::Pairing(
                "missing --pair-token for --pair-connect".to_string(),
            ));
        }
        let mut wire_cfg = NveWireClientConfig::new(uri.token.clone(), "nve-skia-shell");
        wire_cfg.cert_sha256 = uri.cert_sha256;
        let client = ReconnectingWireClient::spawn(
            uri,
            wire_cfg,
            ReconnectConfig {
                initial_backoff: Duration::from_millis(20),
                max_backoff: Duration::from_millis(40),
            },
            None,
        )
        .map_err(|e| DesktopHostError::Pairing(format!("wire spawn failed: {e}")))?;
        Ok(Self {
            client,
            endpoint_label: config.addr,
            pending_frames: VecDeque::new(),
            has_seen_frame: false,
            resync: WireResyncState::new(),
            last_dequeued_session_id: 0,
            last_dequeued_resets: false,
        })
    }

    #[cfg(test)]
    pub fn auth_rejections(&self) -> u64 {
        self.client.auth_rejections()
    }

    #[cfg(test)]
    pub fn transport(&self) -> NveWireTransportKind {
        self.client.transport_kind()
    }

    #[cfg(test)]
    pub fn session_id(&self) -> u32 {
        self.client.session_id()
    }

    pub fn is_connected(&self) -> bool {
        self.client.is_connected()
    }

    #[cfg(test)]
    pub fn frames_received(&self) -> u64 {
        let m = self.client.metrics();
        m.command_deltas_received.saturating_add(m.keyframes_received)
    }

    #[cfg(test)]
    pub fn last_seq(&self) -> u32 {
        self.client.metrics().last_sequence
    }

    pub fn metrics(&self) -> NveWireMetrics {
        self.client.metrics()
    }

    /// Processes one ordered [`WireInboundItem`], returning `Some(sequence)` when a scene frame
    /// was enqueued.
    fn ingest_item(&mut self, item: WireInboundItem) -> Option<u32> {
        match item {
            WireInboundItem::SessionStarted {
                session_id,
                reconnected,
            } => {
                if reconnected {
                    self.pending_frames.clear();
                }
                self.resync
                    .on_session_started(session_id, reconnected, self.has_seen_frame);
                None
            }
            WireInboundItem::Overflowed { session_id } => {
                self.pending_frames.clear();
                let client = &self.client;
                self.resync
                    .on_overflowed(session_id, || client.request_keyframe().is_ok());
                None
            }
            WireInboundItem::Frame { session_id, frame } => {
                if !matches!(
                    frame.kind,
                    NveWireKind::CommandDelta | NveWireKind::SceneKeyframe
                ) || frame.payload.is_empty()
                {
                    return None;
                }
                let is_keyframe = frame.kind == NveWireKind::SceneKeyframe;
                let is_frame1 =
                    WireDecoder::validate_header(&frame.payload).is_ok_and(|(num, _)| num == 1);
                let has_queued_resync_base = self
                    .pending_frames
                    .front()
                    .is_some_and(|f| (f.is_keyframe || f.is_frame1) && f.session_id == session_id);
                if !self.resync.should_apply(is_keyframe, is_frame1) && !has_queued_resync_base {
                    return None;
                }
                if is_keyframe || is_frame1 {
                    self.pending_frames.clear();
                } else if self.pending_frames.len() >= MAX_PENDING_REMOTE_FRAMES {
                    self.pending_frames.clear();
                    let client = &self.client;
                    self.resync
                        .on_overflowed(session_id, || client.request_keyframe().is_ok());
                    return None;
                }
                let seq = frame.sequence;
                self.pending_frames.push_back(QueuedRemoteFrame {
                    session_id,
                    is_keyframe,
                    is_frame1,
                    payload: frame.payload,
                });
                self.has_seen_frame = true;
                Some(seq)
            }
        }
    }

    pub fn wait_for_frame(
        &mut self,
        timeout: Duration,
    ) -> Result<(u32, Vec<u8>), DesktopHostError> {
        let deadline = Instant::now() + timeout;
        let mut last_seq = self.client.metrics().last_sequence;
        while self.pending_frames.is_empty() {
            let now = Instant::now();
            if now >= deadline {
                return Err(DesktopHostError::Pairing(format!(
                    "timed out waiting for paired wire frame from {}",
                    self.endpoint_label
                )));
            }
            let Some(item) = self.client.recv_item_timeout(deadline - now) else {
                continue;
            };
            if let Some(seq) = self.ingest_item(item) {
                last_seq = seq;
            }
        }
        while let Some(item) = self.client.try_recv_item() {
            if let Some(seq) = self.ingest_item(item) {
                last_seq = seq;
            }
        }
        let latest = self
            .pending_frames
            .back()
            .map(|f| f.payload.clone())
            .unwrap_or_default();
        Ok((last_seq, latest))
    }

    fn drain_incoming_frames(&mut self) {
        while let Some(item) = self.client.try_recv_item() {
            let _ = self.ingest_item(item);
        }
    }

    pub fn disconnect(&mut self) {
        self.client.shutdown();
    }
}

impl Drop for PairedRemoteApp {
    fn drop(&mut self) {
        self.disconnect();
    }
}

impl DesktopApp for PairedRemoteApp {
    fn generation(&self) -> u64 {
        let m = self.client.metrics();
        m.command_deltas_received
            .saturating_add(m.keyframes_received)
            .max(1)
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        // Queued wire apps deliver all frames strictly via `take_queued_wire_frame`.
        &[]
    }

    fn uses_queued_wire_frames(&self) -> bool {
        true
    }

    fn take_queued_wire_frame(&mut self) -> Option<(bool, Vec<u8>)> {
        self.drain_incoming_frames();
        let q = self.pending_frames.pop_front()?;
        self.last_dequeued_session_id = q.session_id;
        self.last_dequeued_resets = q.is_keyframe || q.is_frame1;
        Some((q.is_keyframe, q.payload))
    }

    fn on_queued_wire_frame_applied(&mut self, is_keyframe: bool) {
        if is_keyframe || self.last_dequeued_resets {
            self.resync.mark_keyframe_applied();
        }
    }

    fn on_queued_wire_frame_error(&mut self) {
        self.pending_frames.clear();
        let sid = if self.last_dequeued_session_id != 0 {
            self.last_dequeued_session_id
        } else {
            self.client.active_session_id()
        };
        let client = &self.client;
        self.resync
            .on_decode_error(sid, || client.request_keyframe().is_ok());
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        if events.is_empty() || !self.client.is_connected() {
            return false;
        }
        let mut buf = EventBatchBuf::with_capacity(events.len(), 0);
        buf.extend_from_validated(events);
        self.client.send_events(buf.to_wire_bytes()).is_ok()
    }

    fn apply_hot_patch(&mut self, _patch_nvp1: &[u8]) -> bool {
        false
    }

    fn reload(
        &mut self,
        _new_dylib_path: &Path,
        _surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError> {
        Err(DesktopHostError::Reload(
            "paired remote session does not support local dylib reload".into(),
        ))
    }

    fn install_wake_callback(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.client.set_wake_callback(Some(wake));
    }

    fn has_pending_remote_frame(&self) -> bool {
        !self.pending_frames.is_empty() || self.client.has_pending_item()
    }

    fn remote_status_banner(&self) -> Option<String> {
        let m = self.metrics();
        let transport = self.client.transport_kind();
        if self.client.auth_rejections() > 0 {
            let reason = self
                .client
                .last_error()
                .unwrap_or_else(|| "unauthorized".to_owned());
            return Some(format!(
                "Paired {} [AUTH REJECTED] · {reason}",
                transport.as_str(),
            ));
        }
        let state = if self.is_connected() {
            "LIVE"
        } else {
            "RECONNECTING"
        };
        Some(format!(
            "Paired {} [{}] · seq #{} · {} B rx",
            transport.as_str(),
            state,
            m.last_sequence,
            m.wire_bytes_rx
        ))
    }
}
