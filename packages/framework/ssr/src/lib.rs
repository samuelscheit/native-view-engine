#![cfg_attr(doctest, doc = include_str!("../README.md"))]
//! Streaming Server-Side Rendering (SSR) orchestrator for `native-view-engine`.
//!
//! Executes `!Send` [`nve_dioxus::DioxusSurface`] trees on a dedicated pool of single-threaded
//! Tokio `LocalSet` workers ([`SsrLocalPool`]), uniting:
//! - Adaptive Light-DOM fast path (`HttpBody::Full`) when zero `<QuerySuspense>` boundaries suspend,
//! - Out-of-order Declarative Shadow DOM `<slot>` streaming (`HttpBody::Stream`) with stateful
//!   Option 5 Hybrid CSS deduplication (`SsrStyleTracker`),
//! - Atomic per-slot `NVEQ` v2 query dehydration (`DehydrateCursor`) strictly preceding each slot
//!   DOM delta,
//! - Immediate `VirtualDom` and in-flight RPC cancellation on client disconnect,
//! - Strict zero-inline-JS CSP defaults plus per-request CSPRNG nonces ([`generate_csp_nonce`]).

mod options;
mod pool;
mod render;

use core::fmt;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use nve_backend_web::StreamingError;
use nve_query::DehydrateError;

pub use options::{
    SsrLocalPoolConfig, SsrPageOptions, StreamPlan, DEFAULT_CHANNEL_CAPACITY,
    DEFAULT_KEEPALIVE_INTERVAL, DEFAULT_QUEUE_CAPACITY_PER_WORKER, DEFAULT_TOTAL_DEADLINE,
    FAST_PATH_HTTP_HEADERS,
};
pub use pool::SsrLocalPool;

/// Errors produced by [`SsrLocalPool`], [`SsrLocalPool::render_streaming`], and
/// [`generate_csp_nonce`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsrError {
    /// Operating-system CSPRNG failure when generating a per-request CSP nonce.
    Entropy(String),
    /// Failed to initialize an SSR worker thread or its single-threaded Tokio runtime.
    PoolInit(String),
    /// All SSR worker queues are currently saturated (non-panicking backpressure).
    PoolOverloaded,
    /// The SSR worker pool has shut down or dropped the request before headers were emitted.
    PoolClosed,
    /// Failed to flush Dioxus mutations into `WebBackend` during pre-headers rendering.
    Render(String),
    /// Declarative Shadow DOM placement hygiene or streaming session error (raised before headers).
    Streaming(StreamingError),
    /// Failed to dehydrate query cache state into `NVEQ` v2 binary format.
    Dehydrate(DehydrateError),
    /// Failed to assemble the HTTP response status/headers.
    Http(String),
}

impl fmt::Display for SsrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entropy(msg) => write!(f, "CSPRNG entropy error: {msg}"),
            Self::PoolInit(msg) => write!(f, "SSR worker pool initialization failed: {msg}"),
            Self::PoolOverloaded => f.write_str("SSR worker pool queue is overloaded"),
            Self::PoolClosed => f.write_str("SSR worker pool is closed"),
            Self::Render(msg) => write!(f, "SSR pre-headers render failed: {msg}"),
            Self::Streaming(err) => write!(f, "SSR streaming error: {err}"),
            Self::Dehydrate(err) => write!(f, "SSR query dehydration error: {err}"),
            Self::Http(msg) => write!(f, "SSR HTTP response build error: {msg}"),
        }
    }
}

impl std::error::Error for SsrError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Streaming(err) => Some(err),
            Self::Dehydrate(err) => Some(err),
            _ => None,
        }
    }
}

impl From<StreamingError> for SsrError {
    #[inline]
    fn from(err: StreamingError) -> Self {
        Self::Streaming(err)
    }
}

impl From<DehydrateError> for SsrError {
    #[inline]
    fn from(err: DehydrateError) -> Self {
        Self::Dehydrate(err)
    }
}

/// Generates a fresh 128-bit cryptographic per-request CSP nonce encoded with Standard Base64
/// (24 ASCII characters), suitable for `Content-Security-Policy: style-src 'nonce-...'` and
/// [`SsrPageOptions::with_style_nonce`].
pub fn generate_csp_nonce() -> Result<String, SsrError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|err| SsrError::Entropy(err.to_string()))?;
    Ok(BASE64_STANDARD.encode(bytes))
}
