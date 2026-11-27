#![cfg_attr(all(doctest, feature = "server"), doc = include_str!("../README.md"))]

pub mod body;
#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "server")]
pub mod server;

pub use body::{
    collect_body_limited, collect_body_limited_timeout, BodyReadError, ChannelBody, HttpBody,
};
#[cfg(feature = "client")]
pub use client::{
    HyperTransport, DEFAULT_CONNECT_TIMEOUT, DEFAULT_MAX_RESPONSE_BYTES,
    DEFAULT_POOL_IDLE_TIMEOUT, DEFAULT_POOL_MAX_IDLE_PER_HOST, DEFAULT_REQUEST_TIMEOUT,
};
#[cfg(feature = "server")]
pub use server::{
    is_transient_connection_accept_error, serve_rpc_with_shutdown, serve_service_with_config,
    serve_service_with_shutdown, NveHyperService, ServeConfig, DEFAULT_ACCEPT_ERROR_BACKOFF,
    DEFAULT_BODY_READ_TIMEOUT, DEFAULT_GRACEFUL_SHUTDOWN_TIMEOUT, DEFAULT_HEADER_READ_TIMEOUT,
    DEFAULT_MAX_CONNECTIONS,
};
