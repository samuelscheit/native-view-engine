//! Deep-link URI matcher supporting custom URL schemes, universal links, web paths, trailing-slash
//! normalization, and base-prefix stripping.

use std::fmt;

use crate::query::{split_uri, PercentDecodeError, QueryParams};
use crate::route::{normalize_path, Routable};

/// Origin class matched by [`DeepLinkConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkOrigin {
    CustomScheme(String),
    UniversalLink { scheme: String, host: String },
    WebPath,
}

/// Successful deep-link parse result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepLinkMatch<R> {
    pub route: R,
    pub normalized_path: String,
    pub query: QueryParams,
    pub origin: DeepLinkOrigin,
}

/// Typed rejection reasons for malformed or mismatched deep-link URIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkError {
    EmptyUri,
    InvalidUri(&'static str),
    UnsupportedScheme(String),
    UnsupportedHost(String),
    PrefixMismatch,
    InvalidPercentEncoding(PercentDecodeError),
    UnmatchedRoute(String),
}

impl fmt::Display for DeepLinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyUri => write!(f, "deep-link URI is empty"),
            Self::InvalidUri(msg) => write!(f, "invalid deep-link URI: {msg}"),
            Self::UnsupportedScheme(s) => write!(f, "unsupported URI scheme: {s:?}"),
            Self::UnsupportedHost(h) => write!(f, "unsupported universal-link host: {h:?}"),
            Self::PrefixMismatch => write!(f, "URI path does not match required base prefix"),
            Self::InvalidPercentEncoding(e) => write!(f, "invalid percent-encoding: {e}"),
            Self::UnmatchedRoute(p) => write!(f, "no route matched normalized path {p:?}"),
        }
    }
}

impl std::error::Error for DeepLinkError {}

impl From<PercentDecodeError> for DeepLinkError {
    fn from(value: PercentDecodeError) -> Self {
        Self::InvalidPercentEncoding(value)
    }
}

/// Configuration for resolving external deep-link URIs into typed [`Routable`] values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepLinkConfig {
    schemes: Vec<String>,
    scheme_hosts: Vec<String>,
    universal_hosts: Vec<String>,
    allow_http_universal: bool,
    allow_web_paths: bool,
    base_prefix: Option<String>,
    require_base_prefix: bool,
}

impl Default for DeepLinkConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl DeepLinkConfig {
    /// Creates a config allowing web paths (`/issues/42`) and HTTPS universal links for any
    /// explicitly configured hosts.
    pub fn new() -> Self {
        Self {
            schemes: Vec::new(),
            scheme_hosts: Vec::new(),
            universal_hosts: Vec::new(),
            allow_http_universal: false,
            allow_web_paths: true,
            base_prefix: None,
            require_base_prefix: false,
        }
    }

    /// Registers a custom URL scheme (e.g. `"nve"` for `nve://app/issues/42`).
    #[expect(
        clippy::disallowed_methods,
        reason = "RFC 3986 deep-link config owns canonical lowercase scheme strings"
    )]
    pub fn with_scheme(mut self, scheme: impl AsRef<str>) -> Self {
        let lower = scheme.as_ref().trim().to_ascii_lowercase();
        if !lower.is_empty() && !self.schemes.contains(&lower) {
            self.schemes.push(lower);
        }
        self
    }

    /// Registers a host/authority prefix stripped for custom URL schemes (e.g. `"app"` so
    /// `nve://app/issues/42` resolves directly to `"/issues/42"`).
    #[expect(
        clippy::disallowed_methods,
        reason = "RFC 3986 deep-link config owns canonical lowercase scheme host strings"
    )]
    pub fn with_scheme_host(mut self, host: impl AsRef<str>) -> Self {
        let lower = host.as_ref().trim().to_ascii_lowercase();
        if !lower.is_empty() && !self.scheme_hosts.contains(&lower) {
            self.scheme_hosts.push(lower);
        }
        self
    }

    /// Registers an allowed universal-link hostname (e.g. `"app.example.com"`).
    #[expect(
        clippy::disallowed_methods,
        reason = "RFC 3986 deep-link config owns canonical lowercase universal host strings"
    )]
    pub fn with_universal_host(mut self, host: impl AsRef<str>) -> Self {
        let lower = host.as_ref().trim().to_ascii_lowercase();
        if !lower.is_empty() && !self.universal_hosts.contains(&lower) {
            self.universal_hosts.push(lower);
        }
        self
    }

    /// Enables or disables insecure `http://` universal links (disabled by default).
    pub fn allow_http(mut self, allow: bool) -> Self {
        self.allow_http_universal = allow;
        self
    }

    /// Enables or disables bare web paths starting with `'/'` (enabled by default).
    pub fn allow_web_paths(mut self, allow: bool) -> Self {
        self.allow_web_paths = allow;
        self
    }

    /// Configures a leading path prefix to strip at a segment boundary (e.g. `"/app"` turns
    /// `"/app/issues/42"` into `"/issues/42"`, while leaving `"/application/issues"` untouched).
    pub fn with_base_prefix(mut self, prefix: impl AsRef<str>) -> Self {
        let raw = prefix.as_ref().trim();
        if raw.is_empty() || raw == "/" {
            self.base_prefix = None;
        } else {
            let trimmed = raw.trim_matches('/');
            self.base_prefix = Some(format!("/{trimmed}"));
        }
        self
    }

    /// When `true` and a `base_prefix` is configured, URIs lacking that prefix fail with
    /// [`DeepLinkError::PrefixMismatch`] instead of falling through unchanged.
    pub fn require_base_prefix(mut self, required: bool) -> Self {
        self.require_base_prefix = required;
        self
    }

    /// Extracts `(normalized_path, QueryParams, DeepLinkOrigin)` without yet running `R::parse_path`.
    pub fn extract(&self, raw_uri: &str) -> Result<(String, QueryParams, DeepLinkOrigin), DeepLinkError> {
        if raw_uri.is_empty() {
            return Err(DeepLinkError::EmptyUri);
        }
        if raw_uri.bytes().any(|b| b <= 0x20 || b == 0x7f) {
            return Err(DeepLinkError::InvalidUri(
                "URI contains whitespace or control characters",
            ));
        }

        let extracted = if let Some(pos) = raw_uri.find("://") {
            self.extract_with_scheme(raw_uri, pos)?
        } else if raw_uri.starts_with('/') {
            if !self.allow_web_paths {
                return Err(DeepLinkError::UnsupportedScheme(String::new()));
            }
            let parts = split_uri(raw_uri);
            SchemeExtraction {
                raw_path: parts.path.to_owned(),
                raw_query: parts.raw_query,
                raw_fragment: parts.raw_fragment,
                origin: DeepLinkOrigin::WebPath,
            }
        } else {
            // Reject relative strings or bare scheme:opaque URIs such as `javascript:...`.
            if let Some((scheme, _)) = raw_uri.split_once(':')
                && is_valid_scheme(scheme)
            {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "cold deep-link error payload owns canonical lowercase scheme"
                )]
                let lower = scheme.to_ascii_lowercase();
                return Err(DeepLinkError::UnsupportedScheme(lower));
            }
            return Err(DeepLinkError::InvalidUri(
                "URI must start with '/' or '<scheme>://'",
            ));
        };

        let normalized = normalize_path(&extracted.raw_path)
            .ok_or(DeepLinkError::InvalidUri("malformed path segments"))?;
        let stripped = self.strip_prefix(&normalized)?;
        let query = QueryParams::parse_parts(extracted.raw_query, extracted.raw_fragment)?;
        Ok((stripped, query, extracted.origin))
    }

    /// Matches `raw_uri` against this config and parses it into route `R`.
    pub fn match_uri<R: Routable>(&self, raw_uri: &str) -> Result<DeepLinkMatch<R>, DeepLinkError> {
        let (normalized_path, query, origin) = self.extract(raw_uri)?;
        let route = R::parse_path(&normalized_path, &query)
            .ok_or_else(|| DeepLinkError::UnmatchedRoute(normalized_path.clone()))?;
        Ok(DeepLinkMatch {
            route,
            normalized_path,
            query,
            origin,
        })
    }

    fn extract_with_scheme<'a>(
        &self,
        raw_uri: &'a str,
        sep_pos: usize,
    ) -> Result<SchemeExtraction<'a>, DeepLinkError> {
        let scheme_raw = &raw_uri[..sep_pos];
        if !is_valid_scheme(scheme_raw) {
            return Err(DeepLinkError::InvalidUri("invalid RFC 3986 scheme"));
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "RFC 3986 DeepLinkOrigin owns canonical lowercase scheme"
        )]
        let scheme = scheme_raw.to_ascii_lowercase();
        let after_scheme = &raw_uri[sep_pos + 3..];

        // Split authority from path/query/fragment at the first '/', '?', or '#'.
        let split_idx = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        let authority = &after_scheme[..split_idx];
        let remainder = &after_scheme[split_idx..];
        let parts = split_uri(remainder);
        let path_part = if parts.path.is_empty() {
            "/"
        } else {
            parts.path
        };

        let is_universal = scheme == "https" || (scheme == "http" && self.allow_http_universal);
        if is_universal {
            let host = parse_host(authority)?;
            if !self.universal_hosts.iter().any(|h| h == &host) {
                return Err(DeepLinkError::UnsupportedHost(host));
            }
            return Ok(SchemeExtraction {
                raw_path: path_part.to_owned(),
                raw_query: parts.raw_query,
                raw_fragment: parts.raw_fragment,
                origin: DeepLinkOrigin::UniversalLink { scheme, host },
            });
        }

        if !self.schemes.iter().any(|s| s == &scheme) {
            return Err(DeepLinkError::UnsupportedScheme(scheme));
        }

        if authority.contains('@') {
            return Err(DeepLinkError::InvalidUri(
                "userinfo is not permitted in deep-link URIs",
            ));
        }

        let resolved_path = if authority.is_empty()
            || self
                .scheme_hosts
                .iter()
                .any(|h| h.eq_ignore_ascii_case(authority))
        {
                path_part.to_owned()
            } else if parts.path.is_empty() || parts.path == "/" {
                format!("/{authority}")
            } else {
                format!("/{authority}{path_part}")
            };

        Ok(SchemeExtraction {
            raw_path: resolved_path,
            raw_query: parts.raw_query,
            raw_fragment: parts.raw_fragment,
            origin: DeepLinkOrigin::CustomScheme(scheme),
        })
    }

    fn strip_prefix(&self, normalized_path: &str) -> Result<String, DeepLinkError> {
        let Some(prefix) = &self.base_prefix else {
            return Ok(normalized_path.to_owned());
        };
        if normalized_path == prefix {
            return Ok("/".to_owned());
        }
        if let Some(rest) = normalized_path.strip_prefix(prefix.as_str())
            && rest.starts_with('/')
        {
            return Ok(rest.to_owned());
        }
        if self.require_base_prefix {
            Err(DeepLinkError::PrefixMismatch)
        } else {
            Ok(normalized_path.to_owned())
        }
    }
}

struct SchemeExtraction<'a> {
    raw_path: String,
    raw_query: Option<&'a str>,
    raw_fragment: Option<&'a str>,
    origin: DeepLinkOrigin,
}

fn is_valid_scheme(scheme: &str) -> bool {
    let mut bytes = scheme.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

fn parse_host(authority: &str) -> Result<String, DeepLinkError> {
    if authority.is_empty() {
        return Err(DeepLinkError::InvalidUri("universal link missing host"));
    }
    if authority.contains('@') {
        return Err(DeepLinkError::InvalidUri(
            "userinfo is not permitted in universal links",
        ));
    }
    let host_part = if let Some((host, port)) = authority.rsplit_once(':') {
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(DeepLinkError::InvalidUri("invalid port in authority"));
        }
        host
    } else {
        authority
    };
    if host_part.is_empty() {
        return Err(DeepLinkError::InvalidUri("empty host in authority"));
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "RFC 3986 DeepLinkOrigin owns canonical lowercase host"
    )]
    Ok(host_part.to_ascii_lowercase())
}
