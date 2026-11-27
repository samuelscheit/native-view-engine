//! Composable Cookie and Bearer token credential extraction and `AuthResolver` seam.
//!
//! Security invariants:
//! - Duplicate `Authorization` headers fail closed.
//! - Duplicate cookies sharing `cookie_name` fail closed (defends against sibling-subdomain
//!   cookie tossing).
//! - When `Authorization: Bearer <token>` bypasses browser CSRF checks, [`crate::Server`] strips
//!   the `Cookie` header before invoking [`AuthResolver`] or handlers.

use core::fmt;

use http::header::{AUTHORIZATION, COOKIE};
use http::{HeaderMap, HeaderValue};
use nve_rpc::{BoxFuture, MaybeSend, MaybeSync, RpcStatus, TransportError};

/// Parsed authentication credential extracted from an incoming HTTP request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthCredential<'a> {
    /// `Authorization: Bearer <token>` (takes precedence over cookies).
    Bearer(&'a str),
    /// Named session cookie value from the `Cookie` header.
    Cookie(&'a str),
    /// No credential present (or ambiguous duplicate headers).
    None,
}

/// Extracts `Authorization: Bearer <token>` first; only falls back to `cookie_name` when no
/// `Authorization` header is present. Any non-Bearer, empty, malformed, or duplicate
/// `Authorization` header fails closed (`AuthCredential::None`) instead of falling back to an
/// ambient browser cookie. Zero heap allocations.
pub fn extract_credential<'a>(headers: &'a HeaderMap, cookie_name: &str) -> AuthCredential<'a> {
    if headers.contains_key(AUTHORIZATION) {
        return match extract_bearer_token_strict(headers) {
            Ok(Some(bearer)) => AuthCredential::Bearer(bearer),
            Ok(None) | Err(_) => AuthCredential::None,
        };
    }
    if !cookie_name.is_empty()
        && let Some(cookie) = extract_cookie_value(headers, cookie_name)
    {
        return AuthCredential::Cookie(cookie);
    }
    AuthCredential::None
}

/// Strictly extracts `Authorization: Bearer <token>`, failing closed (`Err`) when more than one
/// `Authorization` header is present or the header value is not valid UTF-8. Zero heap allocations.
pub fn extract_bearer_token_strict(headers: &HeaderMap) -> Result<Option<&str>, &'static str> {
    let mut iter = headers.get_all(AUTHORIZATION).iter();
    let Some(first) = iter.next() else {
        return Ok(None);
    };
    if iter.next().is_some() {
        return Err("multiple Authorization headers are not permitted");
    }
    let raw = first
        .to_str()
        .map_err(|_| "invalid Authorization header encoding")?;
    let trimmed = raw.trim();
    let Some((scheme, rest)) = trimmed.split_once(char::is_whitespace) else {
        return Ok(None);
    };
    if scheme.eq_ignore_ascii_case("bearer") {
        let token = rest.trim();
        if !token.is_empty() {
            return Ok(Some(token));
        }
    }
    Ok(None)
}

/// Extracts `cookie_name`'s value across `Cookie` headers without allocating, failing closed
/// (`None`) if `cookie_name` appears more than once (defends against sibling-subdomain cookie
/// tossing) or if the matching cookie's own value is empty or non-UTF-8.
///
/// Scans raw header bytes so unrelated UTF-8 sibling cookies on the same domain (e.g.
/// `theme=café`) do not lock out cookie authentication.
pub fn extract_cookie_value<'a>(headers: &'a HeaderMap, cookie_name: &str) -> Option<&'a str> {
    let target = cookie_name.as_bytes();
    let mut found: Option<&'a str> = None;
    for val in headers.get_all(COOKIE) {
        for part in val.as_bytes().split(|&b| b == b';') {
            let pair = trim_ascii_ows(part);
            let Some(eq_pos) = pair.iter().position(|&b| b == b'=') else {
                continue;
            };
            let key = trim_ascii_ows(&pair[..eq_pos]);
            if key == target {
                if found.is_some() {
                    return None;
                }
                let value_bytes = trim_ascii_ows(&pair[eq_pos + 1..]);
                if value_bytes.is_empty() {
                    return None;
                }
                let value = core::str::from_utf8(value_bytes).ok()?;
                found = Some(value);
            }
        }
    }
    found
}

#[inline]
fn trim_ascii_ows(mut bytes: &[u8]) -> &[u8] {
    while let Some((&first, rest)) = bytes.split_first() {
        if first.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    while let Some((&last, rest)) = bytes.split_last() {
        if last.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

/// Builds a validated `Authorization: Bearer <token>` header value shared across transports.
pub fn make_bearer_header(token: &str) -> Result<HeaderValue, TransportError> {
    if token.is_empty() || token.bytes().any(|b| b <= 0x20 || b >= 0x7f) {
        return Err(TransportError::new("invalid bearer token characters"));
    }
    let mut s = String::with_capacity(7 + token.len());
    s.push_str("Bearer ");
    s.push_str(token);
    HeaderValue::from_str(&s)
        .map_err(|err| TransportError::new(format!("invalid bearer token: {err}")))
}

/// Error returned by an [`AuthResolver`] when rejecting a request before handler execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthError {
    pub status: RpcStatus,
    pub message: String,
}

impl AuthError {
    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self {
            status: RpcStatus::Unauthenticated,
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: RpcStatus::PermissionDenied,
            message: message.into(),
        }
    }
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.status.code_str(), self.message)
    }
}

impl std::error::Error for AuthError {}

/// Pluggable principal resolver shared across HTTP RPC, 0-JS forms, and future transports.
pub trait AuthResolver<S, U>: MaybeSend + MaybeSync + 'static {
    /// Optional zero-allocation synchronous fast path (e.g. `AnonymousAuth` or in-memory token
    /// tables). When `Some` is returned, [`crate::Server`] skips boxing the async future.
    #[inline]
    fn try_resolve_sync(&self, _state: &S, _headers: &HeaderMap) -> Option<Result<U, AuthError>> {
        None
    }

    fn resolve<'a>(
        &'a self,
        state: &'a S,
        headers: &'a HeaderMap,
    ) -> BoxFuture<'a, Result<U, AuthError>>;
}

/// Default no-op auth resolver yielding `()` with zero allocations on the hot path.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnonymousAuth;

impl<S: MaybeSend + MaybeSync + 'static> AuthResolver<S, ()> for AnonymousAuth {
    #[inline]
    fn try_resolve_sync(&self, _state: &S, _headers: &HeaderMap) -> Option<Result<(), AuthError>> {
        Some(Ok(()))
    }

    fn resolve<'a>(
        &'a self,
        _state: &'a S,
        _headers: &'a HeaderMap,
    ) -> BoxFuture<'a, Result<(), AuthError>> {
        Box::pin(async { Ok(()) })
    }
}
