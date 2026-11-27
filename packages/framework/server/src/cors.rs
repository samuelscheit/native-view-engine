//! Opt-in Cross-Origin Resource Sharing (`CorsPolicy`) for preflighted binary RPC endpoints.
//!
//! Security invariants:
//! - Disabled by default (`ServerConfig::cors == None`): browser calls remain strictly same-origin.
//! - Never emits `Access-Control-Allow-Origin: *`; every allowed origin is pre-canonicalized at
//!   startup and echoed explicitly, always paired with `Vary: Origin`.
//! - Applies ONLY to non-simple `POST /rpc/*` (`application/cbor`) and `OPTIONS` preflights —
//!   NEVER to `FormRoute` (`application/x-www-form-urlencoded`), which browsers send without
//!   triggering a CORS preflight.
//! - Always exposes `nve-error-kind` and `nve-invalidate` so cross-origin `FetchTransport` can
//!   classify domain errors and apply cache invalidations.

use bytes::Bytes;
use http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE,
    ACCESS_CONTROL_REQUEST_METHOD, CONTENT_TYPE, COOKIE, ORIGIN, VARY,
};
use http::{HeaderMap, HeaderName, HeaderValue, Method, Response, StatusCode};
use nve_rpc::{RpcStatus, HEADER_ERROR_KIND, HEADER_INVALIDATE};

use crate::auth::extract_bearer_token_strict;
use crate::csrf::{canonicalize_origin, origin_matches_canonical, single_header_str};
use crate::response::{protocol_error_response, PreflightError};

const HDR_SEC_FETCH_SITE: HeaderName = HeaderName::from_static("sec-fetch-site");

const EXPOSE_HEADERS_VALUE: &str = const_expose_headers();
const ALLOW_METHODS_VALUE: &str = "POST, OPTIONS";
const ALLOW_HEADERS_VALUE: &str = "content-type, authorization";
const VARY_PREFLIGHT_VALUE: &str =
    "Origin, Access-Control-Request-Method, Access-Control-Request-Headers";

const fn const_expose_headers() -> &'static str {
    // Keep in sync with `nve_rpc::HEADER_ERROR_KIND` & `nve_rpc::HEADER_INVALIDATE`.
    assert!(HEADER_ERROR_KIND.len() == 14);
    assert!(HEADER_INVALIDATE.len() == 14);
    "nve-error-kind, nve-invalidate"
}

/// Opt-in CORS policy for cross-origin browser RPC deployments (e.g. separate dev-server ports).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorsPolicy {
    allowed_origins: Vec<String>,
    allowed_header_values: Vec<HeaderValue>,
    allow_credentials: bool,
    max_age_secs: u32,
    max_age_header: HeaderValue,
}

impl CorsPolicy {
    /// Creates a CORS policy trusting only the listed canonical origins (e.g.
    /// `"http://localhost:8080"`). Panics on malformed or empty origin lists.
    pub fn allow_origins<I, S>(origins: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::try_allow_origins(origins).expect("invalid CorsPolicy origin")
    }

    /// Fallible version of [`Self::allow_origins`], normalizing each origin and pre-validating
    /// its [`HeaderValue`] at startup for zero per-response allocation.
    pub fn try_allow_origins<I, S>(origins: I) -> Result<Self, &'static str>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut allowed_origins = Vec::new();
        let mut allowed_header_values = Vec::new();
        for raw in origins {
            let raw_s = raw.into();
            let canonical = canonicalize_origin(&raw_s)?;
            if !allowed_origins.iter().any(|existing| existing == &canonical) {
                let hdr = HeaderValue::from_str(&canonical)
                    .map_err(|_| "canonical CORS origin is not a valid HTTP header value")?;
                allowed_origins.push(canonical);
                allowed_header_values.push(hdr);
            }
        }
        if allowed_origins.is_empty() {
            return Err("CorsPolicy::allow_origins requires at least one origin");
        }
        Ok(Self {
            allowed_origins,
            allowed_header_values,
            allow_credentials: true,
            max_age_secs: 600,
            max_age_header: HeaderValue::from_static("600"),
        })
    }

    /// Controls `Access-Control-Allow-Credentials: true` (default: `true`).
    #[inline]
    pub fn allow_credentials(mut self, allow: bool) -> Self {
        self.allow_credentials = allow;
        self
    }

    /// Returns whether `Access-Control-Allow-Credentials: true` is enabled.
    #[inline]
    pub fn allows_credentials(&self) -> bool {
        self.allow_credentials
    }

    /// Controls `Access-Control-Max-Age` in seconds (default: `600`).
    #[inline]
    pub fn max_age_secs(mut self, secs: u32) -> Self {
        self.max_age_secs = secs;
        self.max_age_header = HeaderValue::from_str(&secs.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("600"));
        self
    }

    #[inline]
    pub fn allowed_origins(&self) -> &[String] {
        &self.allowed_origins
    }

    /// Checks whether `headers` carries a single valid `Origin` matching an entry in this policy.
    /// Returns `Err` on duplicate or non-UTF-8 `Origin` headers.
    pub(crate) fn match_origin(&self, headers: &HeaderMap) -> Result<Option<usize>, &'static str> {
        let Some(origin_val) = single_header_str(headers, ORIGIN)? else {
            return Ok(None);
        };
        let trimmed = origin_val.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
            return Ok(None);
        }
        Ok(self
            .allowed_origins
            .iter()
            .position(|allowed| origin_matches_canonical(allowed, trimmed)))
    }

    /// Validates an `OPTIONS` CORS preflight request and returns the matching `origin_idx`.
    pub(crate) fn check_preflight(&self, headers: &HeaderMap) -> Result<usize, &'static str> {
        let origin_idx = self
            .match_origin(headers)?
            .ok_or("CORS preflight Origin is not allowed")?;
        let req_method = single_header_str(headers, ACCESS_CONTROL_REQUEST_METHOD)?
            .ok_or("CORS preflight requires Access-Control-Request-Method")?;
        if !req_method.trim().eq_ignore_ascii_case(Method::POST.as_str()) {
            return Err("CORS preflight only permits POST");
        }
        Ok(origin_idx)
    }

    /// Builds a `204 No Content` CORS preflight response for a validated `origin_idx`.
    pub(crate) fn preflight_response(&self, origin_idx: usize) -> Response<Bytes> {
        let Some(origin_hdr) = self.allowed_header_values.get(origin_idx) else {
            return protocol_error_response(RpcStatus::Internal, "invalid CORS origin index");
        };
        let mut resp = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header(ACCESS_CONTROL_ALLOW_ORIGIN, origin_hdr.clone())
            .header(ACCESS_CONTROL_ALLOW_METHODS, ALLOW_METHODS_VALUE)
            .header(ACCESS_CONTROL_ALLOW_HEADERS, ALLOW_HEADERS_VALUE)
            .header(ACCESS_CONTROL_EXPOSE_HEADERS, EXPOSE_HEADERS_VALUE)
            .header(ACCESS_CONTROL_MAX_AGE, self.max_age_header.clone())
            .header(VARY, VARY_PREFLIGHT_VALUE)
            .body(Bytes::new())
            .expect("static CORS preflight headers build cleanly");
        if self.allow_credentials {
            resp.headers_mut().insert(
                ACCESS_CONTROL_ALLOW_CREDENTIALS,
                HeaderValue::from_static("true"),
            );
        }
        resp
    }

    /// Attaches CORS response headers for an already-validated `origin_idx` with zero string
    /// allocation.
    pub(crate) fn apply_origin_headers(&self, origin_idx: usize, resp: &mut Response<Bytes>) {
        let Some(origin_hdr) = self.allowed_header_values.get(origin_idx) else {
            return;
        };
        let headers = resp.headers_mut();
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin_hdr.clone());
        headers.insert(
            ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static(EXPOSE_HEADERS_VALUE),
        );
        if self.allow_credentials {
            headers.insert(
                ACCESS_CONTROL_ALLOW_CREDENTIALS,
                HeaderValue::from_static("true"),
            );
        }
        append_vary_origin(headers);
    }

    /// Validates non-simple binary RPC headers for an admitted cross-origin `origin_idx`,
    /// stripping `Cookie` when Bearer auth is present, `allow_credentials(false)`, or
    /// `CsrfPolicy::bearer_only()`.
    pub(crate) fn check_rpc_headers(
        &self,
        headers: &mut HeaderMap,
        expected_content_type: &str,
        origin_idx: usize,
        csrf_allows_cookies: bool,
    ) -> Result<(), PreflightError> {
        let raw_ct = single_header_str(headers, CONTENT_TYPE)
            .map_err(|_| {
                PreflightError::protocol(
                    RpcStatus::UnsupportedMediaType,
                    "duplicate or invalid Content-Type header",
                )
                .with_cors_origin(Some(origin_idx))
            })?
            .ok_or_else(|| {
                PreflightError::protocol(RpcStatus::UnsupportedMediaType, "missing Content-Type header")
                    .with_cors_origin(Some(origin_idx))
            })?;
        let essence = raw_ct.split(';').next().unwrap_or("").trim();
        if !essence.eq_ignore_ascii_case(expected_content_type) {
            return Err(PreflightError::protocol(
                RpcStatus::UnsupportedMediaType,
                "unexpected Content-Type media type",
            )
            .with_cors_origin(Some(origin_idx)));
        }
        let _ = single_header_str(headers, &HDR_SEC_FETCH_SITE).map_err(|_| {
            PreflightError::protocol(
                RpcStatus::PermissionDenied,
                "duplicate or invalid Sec-Fetch-Site header",
            )
            .with_cors_origin(Some(origin_idx))
        })?;
        match extract_bearer_token_strict(headers) {
            Ok(Some(_)) => {
                headers.remove(COOKIE);
            }
            Err(_) => {
                return Err(PreflightError::protocol(
                    RpcStatus::PermissionDenied,
                    "duplicate or invalid Authorization header",
                )
                .with_cors_origin(Some(origin_idx)));
            }
            Ok(None) => {
                if !self.allow_credentials || !csrf_allows_cookies {
                    headers.remove(COOKIE);
                }
            }
        }
        Ok(())
    }

    /// Attaches CORS response headers if `req_headers` contains a single allowed `Origin`.
    pub(crate) fn apply_if_allowed(&self, req_headers: &HeaderMap, resp: &mut Response<Bytes>) {
        if let Ok(Some(idx)) = self.match_origin(req_headers) {
            self.apply_origin_headers(idx, resp);
        }
    }
}

fn append_vary_origin(headers: &mut HeaderMap) {
    let already_has_origin = headers.get_all(VARY).iter().any(|val| {
        val.to_str().is_ok_and(|s| {
            s.split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("origin"))
        })
    });
    if !already_has_origin {
        headers.append(VARY, HeaderValue::from_static("Origin"));
    }
}
