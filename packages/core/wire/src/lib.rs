//! Unified QUIC (`nve-wire-2`) + HTTP/3 WebTransport (`h3`) + TLS WebSocket (`wss://`)
//! Happy-Eyeballs wire transport and C ABI for Native View Engine.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod auth;
pub mod c_api;
pub mod client;
mod client_ws;
pub mod framing;
pub mod server;
mod server_peers;
pub mod tls;

use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

pub use auth::{generate_bearer_token, verify_bearer_token};
pub use c_api::{
    nve_wire_client_connect_uri, nve_wire_client_destroy, nve_wire_client_is_connected,
    nve_wire_client_metrics, nve_wire_client_poll_frame, nve_wire_client_request_keyframe,
    nve_wire_client_send_datagram, nve_wire_client_send_events, NveWireClientHandle,
    NveWireMetricsC, NVE_WIRE_ERR_AUTH, NVE_WIRE_ERR_BUFFER_TOO_SMALL, NVE_WIRE_ERR_CONNECT,
    NVE_WIRE_ERR_EMPTY, NVE_WIRE_ERR_INVALID_ARG, NVE_WIRE_ERR_IO, NVE_WIRE_ERR_PANIC,
    NVE_WIRE_ERR_PARSE, NVE_WIRE_OK,
};
pub use client::NveWireClient;
pub use nve_wire_core::*;
pub use server::NveWireServer;
pub use tls::{
    ensure_crypto_provider, NveDevCertificate, DEV_CERT_VALIDITY_DAYS, NVE_WIRE_QUIC_ALPN_BYTES,
    WEBTRANSPORT_ALPN,
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
#[derive(Debug, Clone)]
pub struct NveWireInboundMessage {
    pub session_id: u32,
    pub transport: NveWireTransportKind,
    pub frame: NveWireFrame,
}

/// Aggregate telemetry snapshot from [`NveWireServer`].
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
