//! Zero-copy / low-allocation RFC 3986 query string, fragment, and percent-encoding codec.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

/// Error returned when RFC 3986 percent-decoding encounters malformed escapes or invalid UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PercentDecodeError {
    /// A `%` escape did not have two following hex characters at byte offset `usize`.
    TruncatedEscape(usize),
    /// A `%` escape contained a non-hex character at byte offset `usize`.
    InvalidHex(usize),
    /// Decoded byte sequence was not valid UTF-8.
    InvalidUtf8,
}

impl fmt::Display for PercentDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedEscape(idx) => {
                write!(f, "truncated percent-escape at byte offset {idx}")
            }
            Self::InvalidHex(idx) => write!(f, "invalid hex digit in percent-escape at byte {idx}"),
            Self::InvalidUtf8 => write!(f, "percent-decoded bytes are not valid UTF-8"),
        }
    }
}

impl std::error::Error for PercentDecodeError {}

/// Returns `true` if `b` is an RFC 3986 §2.3 unreserved character (`ALPHA / DIGIT / "-" / "." / "_" / "~"`).
#[inline]
pub const fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

#[inline]
const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// RFC 3986 percent-encodes `input`, returning `Cow::Borrowed` with zero allocation when all bytes
/// are already unreserved.
pub fn percent_encode(input: &str) -> Cow<'_, str> {
    if input.bytes().all(is_unreserved) {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len() + 12);
    percent_encode_into(input, &mut out);
    Cow::Owned(out)
}

/// Appends the RFC 3986 percent-encoded form of `input` into `dst`.
pub fn percent_encode_into(input: &str, dst: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for b in input.bytes() {
        if is_unreserved(b) {
            dst.push(b as char);
        } else {
            dst.push('%');
            dst.push(HEX[(b >> 4) as usize] as char);
            dst.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
}

/// Strictly decodes RFC 3986 `%XX` escapes in `input`, returning `Cow::Borrowed` with zero
/// allocation when no `%` is present.
pub fn percent_decode(input: &str) -> Result<Cow<'_, str>, PercentDecodeError> {
    let bytes = input.as_bytes();
    let Some(first_pct) = bytes.iter().position(|&b| b == b'%') else {
        return Ok(Cow::Borrowed(input));
    };

    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[..first_pct]);
    let mut i = first_pct;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(PercentDecodeError::TruncatedEscape(i));
            }
            let hi = hex_nibble(bytes[i + 1]).ok_or(PercentDecodeError::InvalidHex(i + 1))?;
            let lo = hex_nibble(bytes[i + 2]).ok_or(PercentDecodeError::InvalidHex(i + 2))?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }

    String::from_utf8(out)
        .map(Cow::Owned)
        .map_err(|_| PercentDecodeError::InvalidUtf8)
}

/// Zero-copy view over a URI split into path, optional raw query string, and optional raw fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UriParts<'a> {
    pub path: &'a str,
    pub raw_query: Option<&'a str>,
    pub raw_fragment: Option<&'a str>,
}

/// Splits `raw` into `(path, ?query, #fragment)` with zero heap allocations.
pub fn split_uri(raw: &str) -> UriParts<'_> {
    let (before_hash, raw_fragment) = match raw.split_once('#') {
        Some((head, frag)) => (head, Some(frag)),
        None => (raw, None),
    };
    let (path, raw_query) = match before_hash.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (before_hash, None),
    };
    UriParts {
        path,
        raw_query,
        raw_fragment,
    }
}

/// Zero-allocation iterator over raw `(key, value)` pairs in a query string.
pub struct RawQueryIter<'a> {
    rest: &'a str,
}

impl<'a> RawQueryIter<'a> {
    pub fn new(raw_query: &'a str) -> Self {
        let trimmed = raw_query.strip_prefix('?').unwrap_or(raw_query);
        let without_frag = trimmed.split_once('#').map_or(trimmed, |(q, _)| q);
        Self { rest: without_frag }
    }
}

impl<'a> Iterator for RawQueryIter<'a> {
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.rest.is_empty() {
                return None;
            }
            let (pair, tail) = match self.rest.split_once('&') {
                Some((head, rest)) => (head, rest),
                None => (self.rest, ""),
            };
            self.rest = tail;
            if pair.is_empty() {
                continue;
            }
            return Some(match pair.split_once('=') {
                Some((k, v)) => (k, v),
                None => (pair, ""),
            });
        }
    }
}

/// Compact, order-preserving query parameters and optional `#fragment`.
///
/// Uses a flat `Vec<(String, String)>` instead of a `HashMap` to avoid hashing overhead for typical
/// small query sets while preserving multi-value parameter ordering.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryParams {
    entries: Vec<(String, String)>,
    fragment: Option<String>,
}

impl QueryParams {
    /// Creates an empty parameter set with no fragment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses a raw query string (optionally starting with `?` and/or ending with `#fragment`).
    pub fn parse(raw: &str) -> Result<Self, PercentDecodeError> {
        let (query_part, frag_part) = match raw.split_once('#') {
            Some((q, f)) => (q, Some(f)),
            None => (raw, None),
        };
        let query_part = query_part.strip_prefix('?').unwrap_or(query_part);
        let opt_query = if query_part.is_empty() {
            None
        } else {
            Some(query_part)
        };
        Self::parse_parts(opt_query, frag_part)
    }

    /// Splits a relative or path-only URI (`"/issues/42?tab=comments#top"`) into `(path, QueryParams)`.
    pub fn from_uri(uri: &str) -> Result<(&str, Self), PercentDecodeError> {
        let parts = split_uri(uri);
        let params = Self::parse_parts(parts.raw_query, parts.raw_fragment)?;
        Ok((parts.path, params))
    }

    /// Parses optional raw query and raw fragment components with strict RFC 3986 decoding.
    pub fn parse_parts(
        raw_query: Option<&str>,
        raw_fragment: Option<&str>,
    ) -> Result<Self, PercentDecodeError> {
        let mut entries = Vec::new();
        if let Some(q) = raw_query {
            for (k, v) in RawQueryIter::new(q) {
                let dk = percent_decode(k)?.into_owned();
                let dv = percent_decode(v)?.into_owned();
                entries.push((dk, dv));
            }
        }
        let fragment = match raw_fragment {
            Some(f) if !f.is_empty() => Some(percent_decode(f)?.into_owned()),
            Some(_) => Some(String::new()),
            None => None,
        };
        Ok(Self { entries, fragment })
    }

    /// Appends a `(key, value)` query parameter.
    pub fn push(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.entries.push((key.into(), value.into()));
    }

    /// Replaces the first matching `key`, or appends if absent.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        if let Some((_, existing)) = self.entries.iter_mut().find(|(k, _)| k == &key) {
            *existing = value;
        } else {
            self.entries.push((key, value));
        }
    }

    /// Builder helper to append a `(key, value)` pair.
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.push(key, value);
        self
    }

    /// Builder helper to attach a `#fragment`.
    pub fn with_fragment(mut self, fragment: impl Into<String>) -> Self {
        self.fragment = Some(fragment.into());
        self
    }

    /// Sets or clears the `#fragment`.
    pub fn set_fragment(&mut self, fragment: Option<String>) {
        self.fragment = fragment;
    }

    /// Returns the decoded `#fragment`, if present.
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }

    /// Returns `true` if there are no query parameters (ignores fragment).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of query key-value entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns the first value associated with `key`, if any.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Returns `true` if at least one entry matches `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    /// Iterates over all values associated with `key` in insertion order.
    pub fn get_all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> {
        self.entries
            .iter()
            .filter(move |(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Parses the first value of `key` via [`FromStr`], returning `None` if absent or unparseable.
    pub fn get_parse<T: FromStr>(&self, key: &str) -> Option<T> {
        self.get(key)?.parse::<T>().ok()
    }

    /// Parses the first value of `key` via [`FromStr`], distinguishing absence (`None`) from a parse
    /// error (`Some(Err(_))`).
    pub fn get_parse_result<T: FromStr>(&self, key: &str) -> Option<Result<T, T::Err>> {
        self.get(key).map(str::parse::<T>)
    }

    /// Iterates over all `(key, value)` pairs in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Encodes all query parameters using RFC 3986 percent-encoding (without leading `?`).
    pub fn encode_query(&self) -> String {
        let mut out = String::new();
        for (idx, (k, v)) in self.entries.iter().enumerate() {
            if idx > 0 {
                out.push('&');
            }
            percent_encode_into(k, &mut out);
            out.push('=');
            percent_encode_into(v, &mut out);
        }
        out
    }

    /// Appends `?query` and/or `#fragment` (RFC 3986 encoded) onto `path`.
    pub fn append_to_path(&self, path: &str) -> String {
        if self.entries.is_empty() && self.fragment.is_none() {
            return path.to_owned();
        }
        let mut out = String::with_capacity(path.len() + self.entries.len() * 16 + 8);
        out.push_str(path);
        if !self.entries.is_empty() {
            out.push('?');
            for (idx, (k, v)) in self.entries.iter().enumerate() {
                if idx > 0 {
                    out.push('&');
                }
                percent_encode_into(k, &mut out);
                out.push('=');
                percent_encode_into(v, &mut out);
            }
        }
        if let Some(frag) = &self.fragment {
            out.push('#');
            percent_encode_into(frag, &mut out);
        }
        out
    }
}
