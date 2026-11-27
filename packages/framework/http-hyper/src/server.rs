//! Hyper HTTP/1.1 server adapter and `tower_service::Service` wrapper for `nve-server`.

use core::convert::Infallible;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::io;
use std::sync::Arc;

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Body;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use nve_rpc::{CborCodec, CodecFormat, RpcStatus};
use nve_server::{protocol_error_response, AuthResolver, Server};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use crate::body::{collect_body_limited_timeout, BodyReadError};

/// Default HTTP/1.1 header read timeout protecting against slowloris connections.
pub const DEFAULT_HEADER_READ_TIMEOUT: Duration = Duration::from_secs(15);

/// Default HTTP/1.1 body read timeout protecting against slow-body / stalled-chunked attacks.
pub const DEFAULT_BODY_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Default concurrent TCP connection limit guarding FD and task exhaustion.
pub const DEFAULT_MAX_CONNECTIONS: usize = 4096;

/// Default graceful shutdown drain timeout waiting for in-flight connections to close.
pub const DEFAULT_GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Default backoff after transient resource-exhaustion `accept()` errors (`EMFILE`/`ENFILE`).
pub const DEFAULT_ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(250);

/// Operational limits and timeouts for [`serve_service_with_config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServeConfig {
    pub header_read_timeout: Duration,
    pub max_connections: usize,
    pub graceful_shutdown_timeout: Duration,
    pub accept_error_backoff: Duration,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            header_read_timeout: DEFAULT_HEADER_READ_TIMEOUT,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            graceful_shutdown_timeout: DEFAULT_GRACEFUL_SHUTDOWN_TIMEOUT,
            accept_error_backoff: DEFAULT_ACCEPT_ERROR_BACKOFF,
        }
    }
}

/// `Clone + Send + Sync + 'static` adapter wrapping [`nve_server::Server`] for both
/// `hyper::service::Service` and `tower_service::Service` (mountable in Axum via
/// `Router::route_service` or `Router::nest_service`).
///
/// Runs route, method, CSRF, `Content-Type`, and auth preflight BEFORE reading any body frames.
pub struct NveHyperService<S, U, A, C = CborCodec> {
    server: Server<S, U, A, C>,
    nested_rpc: bool,
    body_read_timeout: Duration,
}

impl<S, U, A, C> Clone for NveHyperService<S, U, A, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            server: self.server.clone(),
            nested_rpc: self.nested_rpc,
            body_read_timeout: self.body_read_timeout,
        }
    }
}

impl<S, U, A, C> NveHyperService<S, U, A, C> {
    /// Creates a top-level service matching both `<rpc_prefix>/<Procedure::NAME>` and any
    /// server-configured [`nve_server::FormRoute`]s.
    #[inline]
    pub fn new(server: Server<S, U, A, C>) -> Self {
        Self {
            server,
            nested_rpc: false,
            body_read_timeout: DEFAULT_BODY_READ_TIMEOUT,
        }
    }

    /// Creates a prefix-stripped RPC service designed for `axum::Router::nest_service("/rpc", ...)`
    /// where the outer router strips the `/rpc` prefix before forwarding the URI.
    #[inline]
    pub fn nested(server: Server<S, U, A, C>) -> Self {
        Self {
            server,
            nested_rpc: true,
            body_read_timeout: DEFAULT_BODY_READ_TIMEOUT,
        }
    }

    /// Overrides the per-request body read deadline (default: 30s).
    #[inline]
    pub fn with_body_read_timeout(mut self, timeout: Duration) -> Self {
        self.body_read_timeout = timeout;
        self
    }

    #[inline]
    pub fn server(&self) -> &Server<S, U, A, C> {
        &self.server
    }
}

async fn dispatch_streaming_request<S, U, A, C, B>(
    server: Server<S, U, A, C>,
    nested_rpc: bool,
    body_read_timeout: Duration,
    req: Request<B>,
) -> Result<Response<Full<Bytes>>, Infallible>
where
    S: Clone + Send + Sync + 'static,
    U: Clone + Send + Sync + 'static,
    A: AuthResolver<S, U>,
    C: CodecFormat,
    B: Body<Data = Bytes> + Unpin + Send + 'static,
    B::Error: core::fmt::Display,
{
    let (mut parts, body) = req.into_parts();

    // 1. Preflight route, method, CSRF/CORS, Content-Type, and auth BEFORE polling any body bytes.
    let prepared = match server.preflight_request(&mut parts, nested_rpc).await {
        Ok(p) => p,
        Err(err) => {
            return Ok(server.preflight_error_response(err).map(Full::new));
        }
    };

    // 2. Skip body polling entirely for bodyless preflights (e.g. OPTIONS CORS).
    let bytes = if prepared.needs_body() {
        let max_bytes = server.config().max_body_bytes;
        match collect_body_limited_timeout(body, max_bytes, body_read_timeout).await {
            Ok(b) => b,
            Err(BodyReadError::TooLarge { .. }) => {
                let mut resp = protocol_error_response(
                    RpcStatus::PayloadTooLarge,
                    "request body exceeds configured max_body_bytes",
                );
                server.apply_prepared_cors_headers(&prepared, &mut resp);
                return Ok(resp.map(Full::new));
            }
            Err(BodyReadError::Timeout { .. }) => {
                let mut resp = protocol_error_response(
                    RpcStatus::RequestTimeout,
                    "request body read timed out",
                );
                server.apply_prepared_cors_headers(&prepared, &mut resp);
                return Ok(resp.map(Full::new));
            }
            Err(BodyReadError::Stream(msg)) => {
                let mut resp = protocol_error_response(RpcStatus::InvalidArgument, &msg);
                server.apply_prepared_cors_headers(&prepared, &mut resp);
                return Ok(resp.map(Full::new));
            }
        }
    } else {
        Bytes::new()
    };

    let resp = server.execute_prepared(prepared, bytes).await;
    Ok(resp.map(Full::new))
}

impl<S, U, A, C, B> hyper::service::Service<Request<B>> for NveHyperService<S, U, A, C>
where
    S: Clone + Send + Sync + 'static,
    U: Clone + Send + Sync + 'static,
    A: AuthResolver<S, U>,
    C: CodecFormat,
    B: Body<Data = Bytes> + Unpin + Send + 'static,
    B::Error: core::fmt::Display,
{
    type Response = Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<B>) -> Self::Future {
        Box::pin(dispatch_streaming_request(
            self.server.clone(),
            self.nested_rpc,
            self.body_read_timeout,
            req,
        ))
    }
}

impl<S, U, A, C, B> tower_service::Service<Request<B>> for NveHyperService<S, U, A, C>
where
    S: Clone + Send + Sync + 'static,
    U: Clone + Send + Sync + 'static,
    A: AuthResolver<S, U>,
    C: CodecFormat,
    B: Body<Data = Bytes> + Unpin + Send + 'static,
    B::Error: core::fmt::Display,
{
    type Response = Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    #[inline]
    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        Box::pin(dispatch_streaming_request(
            self.server.clone(),
            self.nested_rpc,
            self.body_read_timeout,
            req,
        ))
    }
}

/// Classifies `TcpListener::accept` errors: per-connection resets can be retried immediately,
/// while resource-exhaustion errors (`EMFILE`/`ENFILE`) require a short backoff so the server
/// never crashes on a transient accept spike.
pub fn is_transient_connection_accept_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

/// Serves any `hyper::service::Service` (including streaming response bodies) over HTTP/1.1 until
/// `shutdown` resolves, then gracefully drains active connections up to
/// `config.graceful_shutdown_timeout`.
pub async fn serve_service_with_config<Svc, ResBody, F>(
    listener: TcpListener,
    service: Svc,
    config: ServeConfig,
    shutdown: F,
) -> io::Result<()>
where
    ResBody: Body + Send + 'static,
    ResBody::Data: Send,
    ResBody::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    Svc: hyper::service::Service<
            Request<hyper::body::Incoming>,
            Response = Response<ResBody>,
            Error = Infallible,
        > + Clone
        + Send
        + 'static,
    Svc::Future: Send + 'static,
    F: Future<Output = ()>,
{
    let permits = Arc::new(Semaphore::new(config.max_connections.max(1)));
    let graceful = GracefulShutdown::new();
    tokio::pin!(shutdown);

    loop {
        let permit = tokio::select! {
            _ = &mut shutdown => break,
            res = Arc::clone(&permits).acquire_owned() => match res {
                Ok(p) => p,
                Err(_) => break,
            },
        };

        let accepted = tokio::select! {
            _ = &mut shutdown => {
                drop(permit);
                break;
            }
            res = listener.accept() => res,
        };

        let (stream, _peer) = match accepted {
            Ok(pair) => pair,
            Err(err) => {
                drop(permit);
                if is_transient_connection_accept_error(&err) {
                    continue;
                }
                tokio::select! {
                    _ = &mut shutdown => break,
                    _ = tokio::time::sleep(config.accept_error_backoff) => continue,
                }
            }
        };

        let _ = stream.set_nodelay(true);
        let io = TokioIo::new(stream);
        let svc = service.clone();
        let mut builder = http1::Builder::new();
        builder
            .timer(TokioTimer::new())
            .header_read_timeout(config.header_read_timeout);
        let conn = builder.serve_connection(io, svc);
        let watched = graceful.watch(conn);

        tokio::spawn(async move {
            let _permit = permit;
            let _ = watched.await;
        });
    }

    let _ = tokio::time::timeout(config.graceful_shutdown_timeout, graceful.shutdown()).await;
    Ok(())
}

/// Serves any `hyper::service::Service` over HTTP/1.1 with [`ServeConfig::default`] until
/// `shutdown` resolves, then gracefully drains active connections.
pub async fn serve_service_with_shutdown<Svc, ResBody, F>(
    listener: TcpListener,
    service: Svc,
    shutdown: F,
) -> io::Result<()>
where
    ResBody: Body + Send + 'static,
    ResBody::Data: Send,
    ResBody::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    Svc: hyper::service::Service<
            Request<hyper::body::Incoming>,
            Response = Response<ResBody>,
            Error = Infallible,
        > + Clone
        + Send
        + 'static,
    Svc::Future: Send + 'static,
    F: Future<Output = ()>,
{
    serve_service_with_config(listener, service, ServeConfig::default(), shutdown).await
}

/// Convenience wrapper serving [`NveHyperService`] directly on `listener` until `shutdown`.
pub async fn serve_rpc_with_shutdown<S, U, A, C, F>(
    listener: TcpListener,
    server: Server<S, U, A, C>,
    shutdown: F,
) -> io::Result<()>
where
    S: Clone + Send + Sync + 'static,
    U: Clone + Send + Sync + 'static,
    A: AuthResolver<S, U>,
    C: CodecFormat,
    F: Future<Output = ()>,
{
    serve_service_with_shutdown(listener, NveHyperService::new(server), shutdown).await
}
