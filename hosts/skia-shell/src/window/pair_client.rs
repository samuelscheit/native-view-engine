//! Live `NveWireFrame` v2 thin-client pairing adapter (`PairedRemoteApp`) connecting
//! `nve-skia-shell` directly to a remote `nve pair` host via [`nve_wire::ReconnectingWireClient`]
//! over QUIC (`nve-wire-2`), HTTP/3 WebTransport (`h3`), TLS WebSocket (`wss://`), or
//! Happy-Eyeballs auto-fallback with automatic exponential-backoff reconnect & keyframe recovery.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nve_abi::{direct_event_to_native, encode_events, FrameId, SurfaceId};
use nve_store::wire::WireDecoder;
use nve_wire::{
    parse_wire_uri, NveWireClientConfig, NveWireKind, NveWireMetrics, NveWireTransportKind,
    NveWireUri, ReconnectConfig, ReconnectingWireClient, WireInboundItem, DEFAULT_WIRE_WSS_PATH,
};

use super::{DesktopApp, DesktopHostError};

/// Default WebSocket / WebTransport path served by `nve pair`.
pub const DEFAULT_PAIR_WS_PATH: &str = DEFAULT_WIRE_WSS_PATH;

/// Maximum queued wire frames retained between ticks before triggering keyframe resync.
const MAX_PENDING_REMOTE_FRAMES: usize = 256;

/// Transport protocol used by `--pair-connect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PairTransport {
    /// Happy-Eyeballs: QUIC first, racing `wss://` after 250ms.
    #[default]
    Auto,
    /// Native raw QUIC (`nve-wire-2` ALPN).
    Quic,
    /// HTTP/3 WebTransport (`h3` ALPN).
    WebTransport,
    /// TLS WebSocket (`wss://`).
    Wss,
    /// Localhost plain WebSocket (`ws://`).
    Ws,
}

impl PairTransport {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let s = raw.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("auto") {
            Ok(Self::Auto)
        } else if s.eq_ignore_ascii_case("quic") || s.eq_ignore_ascii_case("udp") {
            Ok(Self::Quic)
        } else if s.eq_ignore_ascii_case("wt")
            || s.eq_ignore_ascii_case("webtransport")
            || s.eq_ignore_ascii_case("h3")
        {
            Ok(Self::WebTransport)
        } else if s.eq_ignore_ascii_case("wss") || s.eq_ignore_ascii_case("tls") {
            Ok(Self::Wss)
        } else if s.eq_ignore_ascii_case("ws")
            || s.eq_ignore_ascii_case("tcp")
            || s.eq_ignore_ascii_case("websocket")
        {
            Ok(Self::Ws)
        } else {
            Err(format!(
                "invalid --pair-transport '{s}' (expected one of: auto, quic, webtransport, wss, ws)"
            ))
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Quic => "QUIC",
            Self::WebTransport => "WT/H3",
            Self::Wss => "WSS",
            Self::Ws => "WS",
        }
    }

    const fn to_wire_kind(self) -> NveWireTransportKind {
        match self {
            Self::Auto => NveWireTransportKind::Auto,
            Self::Quic => NveWireTransportKind::Quic,
            Self::WebTransport => NveWireTransportKind::WebTransport,
            Self::Wss => NveWireTransportKind::Wss,
            Self::Ws => NveWireTransportKind::Ws,
        }
    }

    const fn from_wire_kind(kind: NveWireTransportKind) -> Self {
        match kind {
            NveWireTransportKind::Auto => Self::Auto,
            NveWireTransportKind::Quic => Self::Quic,
            NveWireTransportKind::WebTransport => Self::WebTransport,
            NveWireTransportKind::Wss => Self::Wss,
            NveWireTransportKind::Ws => Self::Ws,
        }
    }
}

/// Connection parameters for live thin-client pairing mode (`--pair-connect`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairConnectConfig {
    /// Target `nve://pair?...` URI, `quic://`, `https://`, `wss://`, `ws://`, or `host:port`.
    pub addr: String,
    /// Shared Bearer token printed by `nve pair`.
    pub token: String,
    /// Wire transport protocol (`auto`, `quic`, `webtransport`, `wss`, or `ws`).
    pub transport: PairTransport,
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
        transport: PairTransport,
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
        if self.transport != PairTransport::Auto {
            uri.transport = self.transport.to_wire_kind();
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

/// Live thin-client [`DesktopApp`] backed directly by [`ReconnectingWireClient`].
///
/// Queues ordered `(is_keyframe, payload)` frames in a bounded [`VecDeque`] so incremental
/// `CommandDelta` batches are applied sequentially without dropping intermediate mutations.
pub struct PairedRemoteApp {
    client: ReconnectingWireClient,
    endpoint_label: String,
    pending_frames: VecDeque<(bool, Vec<u8>)>,
    last_rendered: Vec<u8>,
    has_seen_frame: bool,
    waiting_for_keyframe: bool,
    resync_epoch: u64,
    keyframe_requested_key: Option<(u64, u64)>,
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
            last_rendered: Vec::new(),
            has_seen_frame: false,
            waiting_for_keyframe: false,
            resync_epoch: 0,
            keyframe_requested_key: None,
        })
    }

    #[cfg(test)]
    pub fn transport(&self) -> PairTransport {
        PairTransport::from_wire_kind(self.client.transport_kind())
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

    fn request_keyframe_once(&mut self, session_id: u64) {
        self.waiting_for_keyframe = true;
        let key = (session_id, self.resync_epoch);
        if self.keyframe_requested_key != Some(key) {
            self.keyframe_requested_key = Some(key);
            let _ = self.client.request_keyframe();
        }
    }

    /// Processes one ordered [`WireInboundItem`], returning `Some(sequence)` when a scene frame
    /// was enqueued.
    fn ingest_item(&mut self, item: WireInboundItem) -> Option<u32> {
        match item {
            WireInboundItem::SessionStarted {
                session_id,
                reconnected,
            } => {
                if reconnected && (self.has_seen_frame || self.waiting_for_keyframe) {
                    self.waiting_for_keyframe = true;
                    self.keyframe_requested_key = Some((session_id, self.resync_epoch));
                }
                None
            }
            WireInboundItem::Overflowed { session_id } => {
                self.pending_frames.clear();
                self.request_keyframe_once(session_id);
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
                if self.waiting_for_keyframe && !is_keyframe && !is_frame1 {
                    return None;
                }
                if is_keyframe || is_frame1 {
                    self.pending_frames.clear();
                    if self.waiting_for_keyframe {
                        self.waiting_for_keyframe = false;
                        self.resync_epoch = self.resync_epoch.wrapping_add(1);
                    }
                } else if self.pending_frames.len() >= MAX_PENDING_REMOTE_FRAMES {
                    self.pending_frames.clear();
                    self.request_keyframe_once(session_id);
                    return None;
                }
                let seq = frame.sequence;
                self.pending_frames.push_back((is_keyframe, frame.payload));
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
            .map(|(_, bytes)| bytes.clone())
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
        self.drain_incoming_frames();
        if let Some((_, bytes)) = self.pending_frames.pop_front() {
            self.last_rendered = bytes;
            &self.last_rendered
        } else {
            &[]
        }
    }

    fn take_queued_wire_frame(&mut self) -> Option<(bool, Vec<u8>)> {
        self.drain_incoming_frames();
        self.pending_frames.pop_front()
    }

    fn dispatch_events(&mut self, events: nve_abi::NveEventBatchRef<'_>) -> bool {
        if events.is_empty() || !self.client.is_connected() {
            return false;
        }
        let owned: Vec<_> = events.iter().filter_map(direct_event_to_native).collect();
        if owned.is_empty() {
            return false;
        }
        self.client.send_events(encode_events(&owned)).is_ok()
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
        let state = if self.is_connected() {
            "LIVE"
        } else {
            "RECONNECTING"
        };
        let transport = PairTransport::from_wire_kind(self.client.transport_kind());
        Some(format!(
            "Paired {} [{}] · seq #{} · {} B rx",
            transport.label(),
            state,
            m.last_sequence,
            m.wire_bytes_rx
        ))
    }
}
