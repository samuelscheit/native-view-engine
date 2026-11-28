//! Unified QUIC (`nve-wire-2`) + HTTP/3 WebTransport (`h3`) + TLS WebSocket (`wss://`)
//! Happy-Eyeballs wire transport for Native View Engine.
//!
//! Feature split:
//! - `client`: [`NveWireClient`], [`ReconnectingWireClient`], client TLS SHA-256 pinning (embedded in `nve-runtime`).
//! - `server`: [`NveWireServer`], [`NveDevCertificate`], Bearer token generation (`nve pair` CLI / dev servers).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod auth;
#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
mod client_ws;
pub mod framing;
#[cfg(feature = "client")]
pub mod reconnect;
#[cfg(feature = "client")]
mod reconnect_loop;
#[cfg(feature = "client")]
pub mod resync;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
mod server_peers;
pub mod tls;

use std::fmt;
#[cfg(feature = "server")]
use std::net::SocketAddr;
use std::time::Duration;

#[cfg(feature = "server")]
pub use auth::generate_bearer_token;
pub use auth::verify_bearer_token;
#[cfg(feature = "client")]
pub use client::NveWireClient;
pub use nve_wire_core::*;
#[cfg(feature = "client")]
pub use reconnect::{
    ReconnectConfig, ReconnectingWireClient, WireInboundItem, WireWakeCallback,
};
#[cfg(feature = "client")]
pub use resync::WireResyncState;
#[cfg(feature = "server")]
pub use server::NveWireServer;
#[cfg(feature = "server")]
pub use tls::NveDevCertificate;
pub use tls::{
    ensure_crypto_provider, DEV_CERT_VALIDITY_DAYS, NVE_WIRE_QUIC_ALPN_BYTES, WEBTRANSPORT_ALPN,
};

/// Default Happy-Eyeballs delay before racing `wss://` alongside QUIC (`250ms`).
pub const DEFAULT_HAPPY_EYEBALLS_DELAY: Duration = Duration::from_millis(250);

/// 1-byte `KeyframeRequest` payload marker identifying an automatic reconnect-init request
/// (deduplicated against `register_peer`'s initial send; empty payload = explicit recovery).
pub(crate) const KEYFRAME_REQ_RECONNECT_INIT: u8 = 1;

/// Errors returned by `nve-wire` server/client transport operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NveWireTransportError {
    Wire(NveWireError),
    Tls(String),
    Auth(String),
    Connect(String),
    Backpressure(String),
    Io(String),
}

impl From<NveWireError> for NveWireTransportError {
    fn from(e: NveWireError) -> Self {
        Self::Wire(e)
    }
}

impl fmt::Display for NveWireTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(e) => write!(f, "wire protocol error: {e}"),
            Self::Tls(msg) => write!(f, "TLS error: {msg}"),
            Self::Auth(msg) => write!(f, "authentication error: {msg}"),
            Self::Connect(msg) => write!(f, "connect error: {msg}"),
            Self::Backpressure(msg) => write!(f, "wire backpressure: {msg}"),
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for NveWireTransportError {}

/// Configuration for launching an [`NveWireServer`].
#[cfg(feature = "server")]
#[derive(Debug, Clone)]
pub struct NveWireServerConfig {
    pub quic_bind_addr: SocketAddr,
    pub wss_bind_addr: SocketAddr,
    pub bearer_token: String,
    pub cert_sans: Vec<String>,
    pub max_frame_bytes: usize,
    /// Allow plain `ws://` strictly on loopback peers (`127.0.0.1` / `::1`) for local browser dev
    /// when a self-signed WSS certificate is not trusted in the OS store.
    pub allow_plain_loopback_ws: bool,
}

#[cfg(feature = "server")]
impl NveWireServerConfig {
    pub fn new(
        quic_bind_addr: SocketAddr,
        wss_bind_addr: SocketAddr,
        bearer_token: impl Into<String>,
    ) -> Self {
        Self {
            quic_bind_addr,
            wss_bind_addr,
            bearer_token: bearer_token.into(),
            cert_sans: Vec::new(),
            max_frame_bytes: DEFAULT_MAX_WIRE_FRAME_BYTES,
            allow_plain_loopback_ws: true,
        }
    }
}

/// Configuration for connecting an [`NveWireClient`].
#[cfg(feature = "client")]
#[derive(Debug, Clone)]
pub struct NveWireClientConfig {
    pub bearer_token: String,
    pub client_name: String,
    pub server_name: String,
    pub cert_sha256: Option<[u8; 32]>,
    pub max_frame_bytes: usize,
    pub connect_timeout: Duration,
    pub fallback_delay: Duration,
}

#[cfg(feature = "client")]
impl NveWireClientConfig {
    pub fn new(bearer_token: impl Into<String>, client_name: impl Into<String>) -> Self {
        Self {
            bearer_token: bearer_token.into(),
            client_name: client_name.into(),
            server_name: "localhost".to_owned(),
            cert_sha256: None,
            max_frame_bytes: DEFAULT_MAX_WIRE_FRAME_BYTES,
            connect_timeout: Duration::from_secs(5),
            fallback_delay: DEFAULT_HAPPY_EYEBALLS_DELAY,
        }
    }

    pub fn with_cert_sha256(mut self, sha256: [u8; 32]) -> Self {
        self.cert_sha256 = Some(sha256);
        self
    }
}

/// Inbound frame received by [`NveWireServer`] from an authenticated peer.
#[cfg(feature = "server")]
#[derive(Debug, Clone)]
pub struct NveWireInboundMessage {
    pub session_id: u32,
    pub transport: NveWireTransportKind,
    pub frame: NveWireFrame,
}

/// Aggregate telemetry snapshot from [`NveWireServer`].
#[cfg(feature = "server")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NveWireServerMetrics {
    pub connected_peers: usize,
    pub frames_sent: u64,
    pub datagrams_sent: u64,
    pub frames_received: u64,
    pub wire_bytes_tx: u64,
    pub wire_bytes_rx: u64,
    pub rejected_handshakes: u64,
}

/// Validates a pairing hostname or bracketed IPv6 literal.
#[cfg(feature = "client")]
pub fn is_valid_pair_host(host: &str) -> bool {
    if host.is_empty() || host.bytes().any(|b| b.is_ascii_whitespace()) {
        return false;
    }
    let bare = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    if bare.is_empty() {
        return false;
    }
    if host.starts_with('[') || host.ends_with(']') || bare.contains(':') {
        return bare.parse::<std::net::Ipv6Addr>().is_ok();
    }
    bare.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !bare.starts_with('.')
        && !bare.ends_with('.')
        && !bare.starts_with('-')
        && !bare.ends_with('-')
}

/// Shared pairing URI parser and validator used by both `nve-runtime` and desktop/native shells.
///
/// Parses `addr` via [`parse_wire_uri`], applies optional transport/token/cert/path overrides,
/// validates host, non-zero port, non-empty Bearer token, and (when `require_cert_pin` is set)
/// TLS SHA-256 certificate pinning for encrypted transports.
#[cfg(feature = "client")]
pub fn resolve_pair_uri(
    addr: &str,
    token_override: &str,
    transport_override: Option<NveWireTransportKind>,
    cert_sha256_override: Option<[u8; 32]>,
    default_ws_path: Option<&str>,
    require_cert_pin: bool,
) -> Result<NveWireUri, NveWireTransportError> {
    let trimmed_addr = addr.trim();
    if trimmed_addr.is_empty() {
        return Err(NveWireTransportError::Connect(
            "pairing URI must not be empty".to_owned(),
        ));
    }
    let mut uri = parse_wire_uri(trimmed_addr)?;
    if let Some(transport) = transport_override
        && transport != NveWireTransportKind::Auto
    {
        uri.transport = transport;
    }
    if !is_valid_pair_host(&uri.host) {
        return Err(NveWireTransportError::Connect(
            "invalid pairing URI host".to_owned(),
        ));
    }
    if uri.quic_port == Some(0) || uri.wss_port == Some(0) {
        return Err(NveWireTransportError::Connect(
            "pairing URI port must be non-zero".to_owned(),
        ));
    }
    if uri.quic_port.is_none() && uri.wss_port.is_none() {
        return Err(NveWireTransportError::Connect(
            "pairing URI must specify host and port".to_owned(),
        ));
    }
    let trimmed_token = token_override.trim();
    if !trimmed_token.is_empty() {
        uri.token = trimmed_token.to_owned();
    }
    if uri.token.trim().is_empty() {
        return Err(NveWireTransportError::Auth(
            "missing pairing token".to_owned(),
        ));
    }
    if let Some(hash) = cert_sha256_override {
        uri.cert_sha256 = Some(hash);
    }
    if uri.path.is_empty()
        && let Some(path) = default_ws_path
    {
        uri.path = path.to_owned();
    }
    let requires_pin = matches!(
        uri.transport,
        NveWireTransportKind::Quic
            | NveWireTransportKind::WebTransport
            | NveWireTransportKind::Wss
            | NveWireTransportKind::Auto
    );
    if require_cert_pin && requires_pin && uri.cert_sha256.is_none() {
        return Err(NveWireTransportError::Tls(
            "TLS-pinned pairing transport requires cert_sha256".to_owned(),
        ));
    }
    Ok(uri)
}

/// Connection parameters for live thin-client pairing mode (`--pair-connect`).
#[cfg(feature = "client")]
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

#[cfg(feature = "client")]
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
            ws_path: DEFAULT_WIRE_WSS_PATH.to_owned(),
            initial_frame_timeout: Duration::from_secs(5),
        }
    }

    pub fn with_cert_sha256(mut self, sha256: [u8; 32]) -> Self {
        self.cert_sha256 = Some(sha256);
        self
    }

    pub fn resolve_uri(&self) -> Result<NveWireUri, NveWireTransportError> {
        resolve_pair_uri(
            &self.addr,
            &self.token,
            Some(self.transport),
            self.cert_sha256,
            Some(&self.ws_path),
            false,
        )
    }
}
