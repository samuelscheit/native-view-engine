//! Type-safe [`Routable`] trait and pattern matching/formatting runtime for [`define_routes!`].

use std::fmt::Debug;

use crate::query::{percent_decode, percent_encode_into, QueryParams};

/// Implemented by route enums to bidirectionally convert between typed Rust variants and URI paths.
pub trait Routable: Clone + PartialEq + Eq + Debug + 'static {
    /// Parses a normalized path (without `?query`/`#fragment`) and its [`QueryParams`] into a route.
    fn parse_path(path: &str, query: &QueryParams) -> Option<Self>;

    /// Serializes this route back into its canonical path (including query/fragment if any).
    fn to_path(&self) -> String;

    /// Optional static human-readable title for accessibility and native navigation bars.
    fn title(&self) -> Option<&'static str> {
        None
    }

    /// Convenience helper parsing a full relative URI (`"/issues/42?tab=comments#top"`).
    fn parse_uri(uri: &str) -> Option<Self> {
        let (path, query) = QueryParams::from_uri(uri).ok()?;
        Self::parse_path(path, &query)
    }
}

/// Normalizes a URI path: requires leading `'/'`, collapses a single trailing `'/'` (except `"/"`),
/// and rejects empty strings or empty interior segments (`"//"`).
pub fn normalize_path(path: &str) -> Option<String> {
    let trimmed = strip_normalized_bounds(path)?;
    if trimmed.is_empty() {
        return Some("/".to_owned());
    }
    if trimmed.split('/').any(str::is_empty) {
        return None;
    }
    let mut out = String::with_capacity(trimmed.len() + 1);
    out.push('/');
    out.push_str(trimmed);
    Some(out)
}

#[inline]
fn strip_normalized_bounds(path: &str) -> Option<&str> {
    let rest = path.strip_prefix('/')?;
    if rest.is_empty() {
        return Some("");
    }
    let trimmed = rest.strip_suffix('/').unwrap_or(rest);
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

/// Internal runtime helpers used by [`crate::define_routes!`]. Not part of the semver-stable API.
#[doc(hidden)]
pub mod __private {
    use super::*;

    /// Captured dynamic path parameters from a matched pattern.
    #[derive(Debug, Default)]
    pub struct RouteMatch {
        params: Vec<(&'static str, String)>,
    }

    impl RouteMatch {
        #[inline]
        pub fn get(&self, name: &str) -> Option<&str> {
            self.params
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.as_str())
        }
    }

    /// Matches `pattern` (supporting static segments, `:param`, and trailing `*wildcard`) against
    /// `path`, performing strict RFC 3986 percent-decoding on every segment.
    pub fn match_route_pattern(pattern: &'static str, path: &str) -> Option<RouteMatch> {
        let pat_body = strip_normalized_bounds(pattern)?;
        let path_body = strip_normalized_bounds(path)?;

        if !path_body.is_empty() && path_body.split('/').any(str::is_empty) {
            return None;
        }

        let mut pat_iter = (!pat_body.is_empty()).then(|| pat_body.split('/'));
        let mut path_iter = (!path_body.is_empty()).then(|| path_body.split('/'));
        let mut params = Vec::new();

        loop {
            match pat_iter.as_mut().and_then(Iterator::next) {
                None => {
                    if path_iter.as_mut().and_then(Iterator::next).is_some() {
                        return None;
                    }
                    return Some(RouteMatch { params });
                }
                Some(pseg) if pseg.starts_with('*') => {
                    if pat_iter.as_mut().and_then(Iterator::next).is_some() {
                        return None;
                    }
                    let name = &pseg[1..];
                    if name.is_empty() {
                        return None;
                    }
                    let first = path_iter.as_mut().and_then(Iterator::next)?;
                    let mut joined = percent_decode(first).ok()?.into_owned();
                    if let Some(iter) = path_iter.as_mut() {
                        for seg in iter {
                            joined.push('/');
                            let decoded = percent_decode(seg).ok()?;
                            joined.push_str(&decoded);
                        }
                    }
                    params.push((name, joined));
                    return Some(RouteMatch { params });
                }
                Some(pseg) if pseg.starts_with(':') => {
                    let name = &pseg[1..];
                    if name.is_empty() {
                        return None;
                    }
                    let raw = path_iter.as_mut().and_then(Iterator::next)?;
                    let decoded = percent_decode(raw).ok()?.into_owned();
                    params.push((name, decoded));
                }
                Some(static_seg) => {
                    let raw = path_iter.as_mut().and_then(Iterator::next)?;
                    let decoded = percent_decode(raw).ok()?;
                    if decoded != static_seg {
                        return None;
                    }
                }
            }
        }
    }

    /// Formats a route pattern with percent-encoded `:param` / `*wildcard` values and optional
    /// query parameters + `#fragment`.
    pub fn format_route_pattern(
        pattern: &str,
        params: &[(&str, String)],
        query: &QueryParams,
    ) -> String {
        let pat_body = strip_normalized_bounds(pattern).unwrap_or("");
        let mut path = String::with_capacity(pattern.len() + 16);
        if pat_body.is_empty() {
            path.push('/');
        } else {
            for seg in pat_body.split('/') {
                if let Some(name) = seg.strip_prefix('*') {
                    if let Some((_, val)) = params.iter().find(|(k, _)| *k == name) {
                        let trimmed = val.trim_matches('/');
                        if trimmed.is_empty() {
                            path.push('/');
                        } else {
                            for part in trimmed.split('/').filter(|p| !p.is_empty()) {
                                path.push('/');
                                percent_encode_into(part, &mut path);
                            }
                        }
                    }
                } else if let Some(name) = seg.strip_prefix(':') {
                    path.push('/');
                    if let Some((_, val)) = params.iter().find(|(k, _)| *k == name) {
                        percent_encode_into(val, &mut path);
                    }
                } else {
                    path.push('/');
                    path.push_str(seg);
                }
            }
        }
        query.append_to_path(&path)
    }
}
