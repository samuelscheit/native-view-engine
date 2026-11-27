//! Standalone composable CSRF policy, strict Content-Type checks, session cookie helper,
//! and same-origin redirect validation.

use core::fmt;

use http::header::{AsHeaderName, CONTENT_TYPE, COOKIE, ORIGIN};
use http::{HeaderMap, HeaderName, Method};
use nve_rpc::RpcStatus;

use crate::auth::extract_bearer_token_strict;

const HDR_SEC_FETCH_SITE: HeaderName = HeaderName::from_static("sec-fetch-site");

/// Outcome returned when [`CsrfPolicy`] admits a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsrfVerdict {
    /// True when browser Origin/Fetch-Metadata checks were bypassed because a valid single
    /// `Authorization: Bearer` header was present. Callers MUST strip `Cookie` before running
    /// authentication or handlers whenever this is `true`.
    pub bypassed_via_bearer: bool,
}

/// Failure returned by [`CsrfPolicy`] checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRejection {
    pub status: RpcStatus,
    pub reason: &'static str,
}

impl PolicyRejection {
    #[inline]
    pub const fn forbidden(reason: &'static str) -> Self {
        Self {
            status: RpcStatus::PermissionDenied,
            reason,
        }
    }

    #[inline]
    pub const fn unsupported_media_type(reason: &'static str) -> Self {
        Self {
            status: RpcStatus::UnsupportedMediaType,
            reason,
        }
    }
}

impl fmt::Display for PolicyRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.status.code_str(), self.reason)
    }
}

impl std::error::Error for PolicyRejection {}

/// Fail-closed CSRF and media-type policy layer kept strictly separate from [`crate::AuthResolver`].
///
/// Rules enforced on non-safe methods (`POST`, `PUT`, `PATCH`, `DELETE`):
/// 1. Duplicate single-value security headers (`Authorization`, `Sec-Fetch-Site`, `Origin`,
///    `Content-Type`) fail closed immediately (`403` / `415`).
/// 2. A single `Authorization: Bearer <token>` request bypasses browser Origin checks and signals
///    [`CsrfVerdict::bypassed_via_bearer`] so [`crate::Server`] strips `Cookie` before auth.
/// 3. `Sec-Fetch-Site`, when present, MUST be `"same-origin"` or `"none"`; `"cross-site"`,
///    `"same-site"` (sibling subdomains), and unknown values fail closed (`403`).
/// 4. `Origin`, when present, MUST NOT be `"null"` and MUST match a canonical configured origin
///    in `allowed_origins` (case-insensitive scheme/host, default `:80`/`:443` normalized, never
///    compared against the untrusted `Host` header).
/// 5. When BOTH `Sec-Fetch-Site` and `Origin` are absent AND a `Cookie` header is present,
///    fail closed (`403`).
/// 6. Binary RPC endpoints additionally require an exact media type essence match (e.g.
///    `application/cbor`), blocking CORS simple-request smuggling via `text/plain`.
#[derive(Debug, Clone)]
pub struct CsrfPolicy {
    allowed_origins: Vec<String>,
}

impl CsrfPolicy {
    /// Creates a policy trusting only the listed canonical same-origin entries (e.g.
    /// `"https://app.example.com"`). Entries represent this server's own canonical origin(s):
    /// `Sec-Fetch-Site: same-site` (sibling subdomains) and `cross-site` are rejected by default.
    ///
    /// Panics if any entry is malformed (missing `http://`/`https://`, contains a path/query/userinfo,
    /// or has an invalid port). Use [`Self::try_allow_origins`] for fallible construction from runtime
    /// configuration, or [`Self::bearer_only`] for token-only APIs.
    pub fn allow_origins<I, S>(origins: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::try_allow_origins(origins).expect("invalid CsrfPolicy origin")
    }

    /// Fallible version of [`Self::allow_origins`] validating and normalizing each origin at startup.
    pub fn try_allow_origins<I, S>(origins: I) -> Result<Self, &'static str>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut allowed_origins = Vec::new();
        for raw in origins {
            let raw_s = raw.into();
            let canonical = canonicalize_origin(&raw_s)?;
            if !allowed_origins.iter().any(|existing| existing == &canonical) {
                allowed_origins.push(canonical);
            }
        }
        if allowed_origins.is_empty() {
            return Err("allow_origins requires at least one origin; use CsrfPolicy::bearer_only() for token-only APIs");
        }
        Ok(Self { allowed_origins })
    }

    /// Creates a policy with an empty browser origin allowlist: only `Authorization: Bearer`
    /// requests (or non-cookie server-to-server requests with no `Origin` header) are admitted.
    #[inline]
    pub const fn bearer_only() -> Self {
        Self {
            allowed_origins: Vec::new(),
        }
    }

    /// Returns the configured canonical allowed origins.
    #[inline]
    pub fn allowed_origins(&self) -> &[String] {
        &self.allowed_origins
    }

    /// Returns `true` when cookie-authenticated browser origins are configured (`false` for
    /// [`Self::bearer_only`]).
    #[inline]
    pub fn allows_cookies(&self) -> bool {
        !self.allowed_origins.is_empty()
    }

    /// Validates origin & fetch-metadata rules for a state-changing request without allocating.
    pub fn check_origin(
        &self,
        method: &Method,
        headers: &HeaderMap,
    ) -> Result<CsrfVerdict, PolicyRejection> {
        if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
            return Ok(CsrfVerdict {
                bypassed_via_bearer: false,
            });
        }

        match extract_bearer_token_strict(headers) {
            Ok(Some(_)) => {
                return Ok(CsrfVerdict {
                    bypassed_via_bearer: true,
                });
            }
            Err(_) => {
                return Err(PolicyRejection::forbidden(
                    "duplicate or invalid Authorization header",
                ));
            }
            Ok(None) => {}
        }

        if self.allowed_origins.is_empty() && headers.contains_key(COOKIE) {
            return Err(PolicyRejection::forbidden(
                "cookie-authenticated POST rejected by bearer-only policy",
            ));
        }

        let sec_fetch_site = single_header_str(headers, &HDR_SEC_FETCH_SITE)
            .map_err(|_| PolicyRejection::forbidden("duplicate or invalid Sec-Fetch-Site header"))?;

        if let Some(site) = sec_fetch_site {
            let trimmed = site.trim();
            if !(trimmed.eq_ignore_ascii_case("same-origin")
                || trimmed.eq_ignore_ascii_case("none"))
            {
                return Err(PolicyRejection::forbidden(
                    "rejected by Sec-Fetch-Site policy",
                ));
            }
        }

        let origin = single_header_str(headers, ORIGIN)
            .map_err(|_| PolicyRejection::forbidden("duplicate or invalid Origin header"))?;

        if let Some(origin_val) = origin {
            let trimmed = origin_val.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
                return Err(PolicyRejection::forbidden("rejected null or empty Origin"));
            }
            if !self
                .allowed_origins
                .iter()
                .any(|allowed| origin_matches_canonical(allowed, trimmed))
            {
                return Err(PolicyRejection::forbidden(
                    "Origin does not match configured canonical origins",
                ));
            }
        }

        if sec_fetch_site.is_none() && origin.is_none() && headers.contains_key(COOKIE) {
            return Err(PolicyRejection::forbidden(
                "cookie-authenticated POST requires Origin or Sec-Fetch-Site",
            ));
        }

        Ok(CsrfVerdict {
            bypassed_via_bearer: false,
        })
    }

    /// Validates both origin rules AND a single exact `Content-Type` essence (e.g.
    /// `"application/cbor"`). Zero heap allocations.
    pub fn check_content_type_and_origin(
        &self,
        method: &Method,
        headers: &HeaderMap,
        expected_media_type: &str,
    ) -> Result<CsrfVerdict, PolicyRejection> {
        let verdict = self.check_origin(method, headers)?;
        let raw_ct = single_header_str(headers, CONTENT_TYPE)
            .map_err(|_| {
                PolicyRejection::unsupported_media_type("duplicate or invalid Content-Type header")
            })?
            .ok_or_else(|| {
                PolicyRejection::unsupported_media_type("missing Content-Type header")
            })?;
        let essence = raw_ct.split(';').next().unwrap_or("").trim();
        if !essence.eq_ignore_ascii_case(expected_media_type) {
            return Err(PolicyRejection::unsupported_media_type(
                "unexpected Content-Type media type",
            ));
        }
        Ok(verdict)
    }
}

#[inline]
pub(crate) fn single_header_str<K: AsHeaderName>(
    headers: &HeaderMap,
    name: K,
) -> Result<Option<&str>, &'static str> {
    let mut iter = headers.get_all(name).iter();
    let Some(first) = iter.next() else {
        return Ok(None);
    };
    if iter.next().is_some() {
        return Err("duplicate header");
    }
    first.to_str().map(Some).map_err(|_| "non-utf8 header")
}

/// Normalizes and validates an origin string (`scheme://host[:port]` with default `:80`/`:443`
/// stripped and ASCII lowercased).
pub(crate) fn canonicalize_origin(raw: &str) -> Result<String, &'static str> {
    let trimmed = raw.trim().trim_end_matches('/');
    let (scheme, rest, default_port) = if let Some(rest) = strip_prefix_ignore_ascii_case(trimmed, "https://") {
        ("https://", rest, "443")
    } else if let Some(rest) = strip_prefix_ignore_ascii_case(trimmed, "http://") {
        ("http://", rest, "80")
    } else {
        return Err("origin must start with http:// or https://");
    };

    if rest.is_empty()
        || rest
            .bytes()
            .any(|b| b <= 0x20 || b >= 0x7f || matches!(b, b'/' | b'?' | b'#' | b'@' | b'\\'))
    {
        return Err("origin must contain only scheme://host[:port] with no path, query, or userinfo");
    }

    let (host, port) = split_host_port(rest)?;
    if host.is_empty() {
        return Err("origin host must not be empty");
    }

    let default_port_num: u16 = default_port.parse().unwrap_or(0);
    let parsed_port = match port {
        Some(p) => Some(p.parse::<u16>().map_err(|_| "invalid port in origin")?),
        None => None,
    };
    let normalized_port = parsed_port.filter(|&num| num != default_port_num);
    let port_str = normalized_port.map(|n| n.to_string());
    let mut out = String::with_capacity(
        scheme.len() + host.len() + port_str.as_ref().map_or(0, |p| 1 + p.len()),
    );
    out.push_str(scheme);
    for b in host.bytes() {
        // allow(to_ascii_lowercase): single byte by value, zero heap allocation
        out.push(b.to_ascii_lowercase() as char);
    }
    if let Some(p) = port_str {
        out.push(':');
        out.push_str(&p);
    }
    Ok(out)
}

/// Zero-allocation comparison between a pre-canonicalized allowed origin and an incoming
/// `Origin` header (normalizing optional default `:80`/`:443` ports and ASCII case).
pub(crate) fn origin_matches_canonical(allowed: &str, incoming: &str) -> bool {
    let incoming = incoming.trim_end_matches('/');
    if allowed.eq_ignore_ascii_case(incoming) {
        return true;
    }
    // Handle browser/proxy sending explicit default port (`:443` for https, `:80` for http).
    if let Some(without_port) = incoming.strip_suffix(":443")
        && strip_prefix_ignore_ascii_case(without_port, "https://").is_some()
    {
        return allowed.eq_ignore_ascii_case(without_port);
    }
    if let Some(without_port) = incoming.strip_suffix(":80")
        && strip_prefix_ignore_ascii_case(without_port, "http://").is_some()
    {
        return allowed.eq_ignore_ascii_case(without_port);
    }
    false
}

fn split_host_port(authority: &str) -> Result<(&str, Option<&str>), &'static str> {
    if let Some(after_bracket) = authority.strip_prefix('[') {
        let end = after_bracket
            .find(']')
            .ok_or("unclosed IPv6 bracket in origin")?;
        if end == 0 {
            return Err("empty IPv6 host in origin");
        }
        let host = &authority[..end + 2];
        let tail = &after_bracket[end + 1..];
        if tail.is_empty() {
            return Ok((host, None));
        }
        let port = tail
            .strip_prefix(':')
            .ok_or("unexpected trailing bytes after IPv6 origin host")?;
        validate_port(port)?;
        return Ok((host, Some(port)));
    }

    if let Some((host, port)) = authority.rsplit_once(':') {
        if host.contains(':') {
            return Err("IPv6 origin host must be enclosed in brackets");
        }
        validate_port(port)?;
        Ok((host, Some(port)))
    } else {
        Ok((authority, None))
    }
}

fn validate_port(port: &str) -> Result<(), &'static str> {
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return Err("invalid port in origin");
    }
    let parsed: u16 = port.parse().map_err(|_| "origin port out of u16 range")?;
    if parsed == 0 {
        return Err("origin port must be non-zero");
    }
    Ok(())
}

#[inline]
fn strip_prefix_ignore_ascii_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
    {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Formats a hardened `Set-Cookie` header value with explicit `HttpOnly; SameSite=Lax; Path=/`
/// (avoiding Chrome's 2-minute implicit Lax+POST window).
///
/// For production HTTPS deployments (`secure = true`), prefer prefixing the cookie name with
/// `"__Host-"` (e.g. `"__Host-nve_session"`) so browsers enforce `Secure`, `Path=/`, and no
/// `Domain` attribute against sibling-subdomain cookie tossing.
pub fn build_session_cookie(
    name: &str,
    value: &str,
    secure: bool,
    max_age_secs: Option<u64>,
) -> Result<String, &'static str> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err("invalid cookie name");
    }
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| (0x21..=0x7e).contains(&b) && !matches!(b, b';' | b',' | b'"' | b'\\'))
    {
        return Err("invalid cookie value");
    }
    let mut out = format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax");
    if secure {
        out.push_str("; Secure");
    }
    if let Some(max_age) = max_age_secs {
        use core::fmt::Write as _;
        let _ = write!(out, "; Max-Age={max_age}");
    }
    Ok(out)
}

/// Validates a server-selected same-origin relative redirect target (`303 See Other`), rejecting
/// protocol-relative URLs (`//evil.com`), backslash tricks (`/\evil.com`), absolute schemes,
/// spaces, control characters (CRLF header injection), and non-ASCII bytes (`>= 0x80`).
pub fn validate_same_origin_redirect(location: &str) -> Result<&str, &'static str> {
    if !location.starts_with('/') {
        return Err("redirect target must start with '/'");
    }
    if location.starts_with("//") || location.starts_with("/\\") {
        return Err("redirect target must be same-origin relative");
    }
    if !location.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err("redirect target must contain only visible ASCII URI characters");
    }
    Ok(location)
}
