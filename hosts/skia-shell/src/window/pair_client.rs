//! Live `NveWireFrame` v2 thin-client pairing adapter (`PairedRemoteApp`) connecting
//! `nve-skia-shell` directly to a remote `nve pair` host via [`nve_wire::ReconnectingWireClient`]
//! over QUIC (`nve-wire-2`), HTTP/3 WebTransport (`h3`), TLS WebSocket (`wss://`), or
//! Happy-Eyeballs auto-fallback with automatic exponential-backoff reconnect & keyframe recovery.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nve_abi::{direct_event_to_native, encode_events, FrameId, SurfaceId};
use nve_wire::{
    parse_wire_uri, NveWireClientConfig, NveWireKind, NveWireMetrics, NveWireTransportKind,
    NveWireUri, ReconnectConfig, ReconnectingWireClient, DEFAULT_WIRE_WSS_PATH,
};

use super::{DesktopApp, DesktopHostError};

/// Default WebSocket / WebTransport path served by `nve pair`.
pub const DEFAULT_PAIR_WS_PATH: &str = DEFAULT_WIRE_WSS_PATH;

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
pub struct PairedRemoteApp {
    client: ReconnectingWireClient,
    endpoint_label: String,
    latest_frame: Vec<u8>,
    has_unconsumed_frame: bool,
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
            latest_frame: Vec::new(),
            has_unconsumed_frame: false,
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

    pub fn wait_for_frame(
        &mut self,
        timeout: Duration,
    ) -> Result<(u32, Vec<u8>), DesktopHostError> {
        let deadline = Instant::now() + timeout;
        let first = loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(DesktopHostError::Pairing(format!(
                    "timed out waiting for paired wire frame from {}",
                    self.endpoint_label
                )));
            }
            let Some(frame) = self.client.recv_frame_timeout(deadline - now) else {
                continue;
            };
            if matches!(
                frame.kind,
                NveWireKind::CommandDelta | NveWireKind::SceneKeyframe
            ) && !frame.payload.is_empty()
            {
                break (frame.sequence, frame.payload);
            }
        };
        let mut latest = first;
        while let Some(newer) = self.client.try_recv_frame() {
            if matches!(
                newer.kind,
                NveWireKind::CommandDelta | NveWireKind::SceneKeyframe
            ) && !newer.payload.is_empty()
            {
                latest = (newer.sequence, newer.payload);
            }
        }
        self.latest_frame = latest.1.clone();
        self.has_unconsumed_frame = true;
        Ok(latest)
    }

    fn drain_incoming_frames(&mut self) -> bool {
        let mut got_new = false;
        while let Some(frame) = self.client.try_recv_frame() {
            if matches!(
                frame.kind,
                NveWireKind::CommandDelta | NveWireKind::SceneKeyframe
            ) && !frame.payload.is_empty()
            {
                self.latest_frame = frame.payload;
                self.has_unconsumed_frame = true;
                got_new = true;
            }
        }
        got_new
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
        if self.has_unconsumed_frame {
            self.has_unconsumed_frame = false;
            &self.latest_frame
        } else {
            &[]
        }
    }

    fn dispatch_events(&mut self, events: nve_abi::NveEventBatchRef<'_>) -> bool {
        if events.is_empty() || !self.client.is_connected() {
            return false;
        }
        let owned: Vec<_> = events.iter().filter_map(direct_event_to_native).collect();
        if owned.is_empty() {
            return false;
        }
        self.client.send_events(&encode_events(&owned)).is_ok()
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
        self.has_unconsumed_frame || self.client.has_pending_frame()
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
