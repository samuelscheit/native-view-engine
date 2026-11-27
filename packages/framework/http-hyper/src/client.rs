//! Native plain HTTP/1.1 `HyperTransport` implementing `nve_rpc::Transport` for Skia, Desktop,
//! Headless, and integration tests.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::sync::Arc;

use bytes::Bytes;
use http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN};
use http::{HeaderMap, HeaderName, HeaderValue, Method, Request};
use http_body_util::Full;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioTimer};
use nve_rpc::{
    classify_http_reply, format_rpc_path_into, validate_bearer_token, BoxFuture, CborLimits,
    RawCall, RawReply, Transport, TransportError, DEFAULT_RPC_PREFIX, HEADER_ERROR_KIND,
    HEADER_INVALIDATE,
};

use crate::body::{collect_body_limited, BodyReadError};

const HDR_ERROR_KIND: HeaderName = HeaderName::from_static(HEADER_ERROR_KIND);
const HDR_INVALIDATE: HeaderName = HeaderName::from_static(HEADER_INVALIDATE);

/// Default maximum HTTP response size accepted by [`HyperTransport`], aligned with
/// [`CborLimits::DEFAULT`]'s `max_bytes` (`4 MiB`).
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = CborLimits::DEFAULT.max_bytes;

/// Default TCP connect timeout for [`HyperTransport`].
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default end-to-end request + response body timeout for [`HyperTransport`].
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Default idle keep-alive pool eviction timeout.
pub const DEFAULT_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Default maximum idle keep-alive sockets cached per host.
pub const DEFAULT_POOL_MAX_IDLE_PER_HOST: usize = 32;

/// Native Hyper plain HTTP/1.1 (`http://`) client implementing [`nve_rpc::Transport`].
///
/// Configured with `TCP_NODELAY`, bounded idle pool timer, connect timeout, and per-call request
/// timeout. For TLS/QUIC over the public internet, pair with an HTTPS reverse proxy or Slice 2's
/// `nve-wire` transport.
#[derive(Clone)]
enum HyperRuntime {
    Handle(tokio::runtime::Handle),
    Owned(Arc<tokio::runtime::Runtime>),
}

impl HyperRuntime {
    #[inline]
    fn handle(&self) -> tokio::runtime::Handle {
        match self {
            Self::Handle(h) => h.clone(),
            Self::Owned(rt) => rt.handle().clone(),
        }
    }
}

struct AbortOnDropJoinHandle<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDropJoinHandle<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T> Future for AbortOnDropJoinHandle<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx)
    }
}

#[derive(Clone)]
pub struct HyperTransport {
    client: Client<HttpConnector, Full<Bytes>>,
    base_url: String,
    rpc_prefix: String,
    default_headers: HeaderMap,
    max_response_bytes: usize,
    request_timeout: Duration,
    runtime: Option<HyperRuntime>,
}

impl HyperTransport {
    /// Creates a transport targeting `base_url` (e.g. `"http://127.0.0.1:8080"`).
    ///
    /// When polled inside an existing Tokio runtime, uses `Handle::try_current()`. When polled
    /// from a non-Tokio UI thread (e.g. Skia's `winit` loop), attach a runtime via
    /// [`Self::with_runtime`] or [`Self::with_background_runtime`].
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into().trim().trim_end_matches('/').to_owned();
        let mut connector = HttpConnector::new();
        connector.set_nodelay(true);
        connector.set_connect_timeout(Some(DEFAULT_CONNECT_TIMEOUT));
        let client = Client::builder(TokioExecutor::new())
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(DEFAULT_POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(DEFAULT_POOL_MAX_IDLE_PER_HOST)
            .build(connector);
        Self {
            client,
            base_url,
            rpc_prefix: DEFAULT_RPC_PREFIX.to_owned(),
            default_headers: HeaderMap::new(),
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            runtime: None,
        }
    }

    /// Runs HTTP exchanges on an explicit Tokio runtime handle (safe to poll from non-Tokio UI
    /// threads such as Skia's `winit` event loop).
    pub fn with_runtime(mut self, handle: tokio::runtime::Handle) -> Self {
        self.runtime = Some(HyperRuntime::Handle(handle));
        self
    }

    /// Spawns a dedicated 1-thread background Tokio runtime owned by this transport (cleanly shut
    /// down when the last clone drops), enabling `HyperTransport` on non-Tokio UI threads.
    pub fn with_background_runtime(mut self) -> Result<Self, TransportError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("nve-http-hyper")
            .build()
            .map_err(|err| TransportError::config(format!("failed to start Tokio runtime: {err}")))?;
        self.runtime = Some(HyperRuntime::Owned(Arc::new(rt)));
        Ok(self)
    }

    pub fn with_rpc_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.rpc_prefix = prefix.into();
        self
    }

    pub fn with_max_response_bytes(mut self, limit: usize) -> Self {
        self.max_response_bytes = limit;
        self
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.default_headers.insert(name, value);
        self
    }

    pub fn with_bearer_token(mut self, token: &str) -> Result<Self, TransportError> {
        validate_bearer_token(token)?;
        let mut s = String::with_capacity(7 + token.len());
        s.push_str("Bearer ");
        s.push_str(token);
        let val = HeaderValue::from_str(&s)
            .map_err(|err| TransportError::config(format!("invalid bearer token: {err}")))?;
        self.default_headers.insert(AUTHORIZATION, val);
        Ok(self)
    }

    /// Attaches a session `Cookie` and matching `Origin` header (does not forge `Sec-Fetch-Site`).
    pub fn with_cookie_and_origin(
        mut self,
        cookie: &str,
        origin: &str,
    ) -> Result<Self, TransportError> {
        let cookie_val = HeaderValue::from_str(cookie)
            .map_err(|err| TransportError::config(format!("invalid cookie header: {err}")))?;
        let origin_val = HeaderValue::from_str(origin)
            .map_err(|err| TransportError::config(format!("invalid origin header: {err}")))?;
        self.default_headers.insert(COOKIE, cookie_val);
        self.default_headers.insert(ORIGIN, origin_val);
        Ok(self)
    }

    #[inline]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Transport for HyperTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        Box::pin(async move {
            if self
                .base_url
                .as_bytes()
                .get(..8)
                .is_some_and(|head| head.eq_ignore_ascii_case(b"https://"))
            {
                return Err(TransportError::config(
                    "HyperTransport uses plain http://; terminate TLS at a proxy or use a TLS/nve-wire transport",
                ));
            }

            let Some(handle) = self
                .runtime
                .as_ref()
                .map(HyperRuntime::handle)
                .or_else(|| tokio::runtime::Handle::try_current().ok())
            else {
                return Err(TransportError::config(
                    "HyperTransport requires a Tokio runtime; call with_runtime() or with_background_runtime()",
                ));
            };

            let mut url = String::with_capacity(
                self.base_url.len() + self.rpc_prefix.len() + req.name.len() + 2,
            );
            url.push_str(&self.base_url);
            format_rpc_path_into(&self.rpc_prefix, req.name, &mut url);

            let mut http_req = Request::builder()
                .method(Method::POST)
                .uri(&url)
                .body(Full::new(req.body))
                .map_err(|err| TransportError::config(format!("failed to build request: {err}")))?;
            *http_req.headers_mut() = self.default_headers.clone();
            http_req
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static(req.content_type));

            let client = self.client.clone();
            let max_response_bytes = self.max_response_bytes;
            let request_timeout = self.request_timeout;

            let join = handle.spawn(async move {
                let exchange = async {
                    let resp = client.request(http_req).await.map_err(|err| {
                        TransportError::network(format!("HTTP request failed: {err}"))
                    })?;

                    let (parts, body) = resp.into_parts();
                    let resp_bytes = collect_body_limited(body, max_response_bytes)
                        .await
                        .map_err(|err| match err {
                            BodyReadError::TooLarge { .. } => {
                                TransportError::response_too_large(err.to_string())
                            }
                            BodyReadError::Timeout { .. } => {
                                TransportError::timeout(err.to_string())
                            }
                            BodyReadError::Stream(_) => TransportError::network(err.to_string()),
                        })?;
                    let error_kind = parts.headers.get(&HDR_ERROR_KIND).map(HeaderValue::as_bytes);
                    let invalidate = parts
                        .headers
                        .get_all(&HDR_INVALIDATE)
                        .iter()
                        .map(HeaderValue::as_bytes);
                    Ok(classify_http_reply(
                        parts.status.as_u16(),
                        error_kind,
                        invalidate,
                        resp_bytes,
                    ))
                };

                match tokio::time::timeout(request_timeout, exchange).await {
                    Ok(res) => res,
                    Err(_) => Err(TransportError::timeout(format!(
                        "HTTP request timed out after {}ms",
                        request_timeout.as_millis()
                    ))),
                }
            });

            match AbortOnDropJoinHandle(join).await {
                Ok(res) => res,
                Err(err) if err.is_cancelled() => {
                    Err(TransportError::cancelled("HTTP request task cancelled"))
                }
                Err(err) => Err(TransportError::network(format!(
                    "HTTP request task failed: {err}"
                ))),
            }
        })
    }
}
