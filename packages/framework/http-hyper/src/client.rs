//! Native plain HTTP/1.1 `HyperTransport` implementing `nve_rpc::Transport` for Skia, Desktop,
//! Headless, and integration tests.

use core::time::Duration;

use bytes::Bytes;
use http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN};
use http::{HeaderMap, HeaderName, HeaderValue, Method, Request};
use http_body_util::Full;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioTimer};
use nve_rpc::{
    format_rpc_path_into, BoxFuture, CborLimits, RawCall, RawReply, Transport, TransportError,
    DEFAULT_RPC_PREFIX,
};
use nve_server::{make_bearer_header, parse_rpc_http_reply};

use crate::body::collect_body_limited;

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
pub struct HyperTransport {
    client: Client<HttpConnector, Full<Bytes>>,
    base_url: String,
    rpc_prefix: String,
    default_headers: HeaderMap,
    max_response_bytes: usize,
    request_timeout: Duration,
}

impl HyperTransport {
    /// Creates a transport targeting `base_url` (e.g. `"http://127.0.0.1:8080"`).
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
        }
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
        let val = make_bearer_header(token)?;
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
            .map_err(|err| TransportError::new(format!("invalid cookie header: {err}")))?;
        let origin_val = HeaderValue::from_str(origin)
            .map_err(|err| TransportError::new(format!("invalid origin header: {err}")))?;
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
    fn call_raw<'a>(&'a self, req: RawCall<'a>) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        Box::pin(async move {
            if self
                .base_url
                .as_bytes()
                .get(..8)
                .is_some_and(|head| head.eq_ignore_ascii_case(b"https://"))
            {
                return Err(TransportError::new(
                    "HyperTransport uses plain http://; terminate TLS at a proxy or use a TLS/nve-wire transport",
                ));
            }

            let mut url =
                String::with_capacity(self.base_url.len() + self.rpc_prefix.len() + req.name.len() + 2);
            url.push_str(&self.base_url);
            format_rpc_path_into(&self.rpc_prefix, req.name, &mut url);

            let mut http_req = Request::builder()
                .method(Method::POST)
                .uri(&url)
                .body(Full::new(req.body))
                .map_err(|err| TransportError::new(format!("failed to build request: {err}")))?;
            *http_req.headers_mut() = self.default_headers.clone();
            http_req
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static(req.content_type));

            let exchange = async {
                let resp = self
                    .client
                    .request(http_req)
                    .await
                    .map_err(|err| TransportError::new(format!("HTTP request failed: {err}")))?;

                let (parts, body) = resp.into_parts();
                let resp_bytes = collect_body_limited(body, self.max_response_bytes)
                    .await
                    .map_err(|err| TransportError::new(err.to_string()))?;
                parse_rpc_http_reply(parts.status, &parts.headers, resp_bytes)
            };

            match tokio::time::timeout(self.request_timeout, exchange).await {
                Ok(res) => res,
                Err(_) => Err(TransportError::new(format!(
                    "HTTP request timed out after {}ms",
                    self.request_timeout.as_millis()
                ))),
            }
        })
    }
}
