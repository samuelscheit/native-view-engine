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
pub use reconnect::{ReconnectConfig, ReconnectingWireClient, WireWakeCallback};
#[cfg(feature = "server")]
pub use server::NveWireServer;
#[cfg(feature = "server")]
pub use tls::NveDevCertificate;
pub use tls::{
    ensure_crypto_provider, DEV_CERT_VALIDITY_DAYS, NVE_WIRE_QUIC_ALPN_BYTES, WEBTRANSPORT_ALPN,
};

/// Default Happy-Eyeballs delay before racing `wss://` alongside QUIC (`250ms`).
pub const DEFAULT_HAPPY_EYEBALLS_DELAY: Duration = Duration::from_millis(250);

/// Errors returned by `nve-wire` server/client transport operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NveWireTransportError {
    Wire(NveWireError),
    Tls(String),
    Auth(String),
    Connect(String),
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
