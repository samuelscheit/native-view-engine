//! Normalized, query/anchor-safe documentation route type implementing `nve_router::Routable`.

use std::fmt;

use nve_mdx::normalize_route;
use nve_router::{QueryParams, Routable};

use crate::config::normalize_base_path;

/// Canonical documentation page route (e.g. `"/"`, `"/getting-started/quickstart"`).
///
/// Paths are normalized via [`nve_mdx::normalize_route`], stripping query parameters,
/// `#fragment` anchors, and trailing slashes while guaranteeing a leading `'/'`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocsRoute(pub String);

impl Default for DocsRoute {
    fn default() -> Self {
        Self::root()
    }
}

impl DocsRoute {
    /// Root documentation route `"/"`.
    pub fn root() -> Self {
        Self("/".to_string())
    }

    /// Construct a normalized `DocsRoute` from any raw path or URI string.
    pub fn new(raw: impl AsRef<str>) -> Self {
        Self(normalize_route(raw.as_ref()))
    }

    /// Borrow the normalized route path (always starts with `'/'`).
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Format a full `href` with optional deployment `base_path` (e.g. `"/docs/quickstart"`).
    pub fn href(&self, base_path: &str) -> String {
        let norm = normalize_route(&self.0);
        let base = normalize_base_path(base_path);
        if base.is_empty() {
            norm
        } else if norm == "/" {
            base
        } else {
            format!("{base}{norm}")
        }
    }

    /// Parse a link target into an optional internal route and optional heading anchor.
    ///
    /// Returns `None` for external links (`http://`, `https://`, `mailto:`, `tel:`, `data:`).
    pub fn parse_link_target(raw: &str) -> Option<(Option<DocsRoute>, Option<String>)> {
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("http://")
            || trimmed.starts_with("https://")
            || trimmed.starts_with("mailto:")
            || trimmed.starts_with("tel:")
            || trimmed.starts_with("data:")
        {
            return None;
        }
        if let Some(anchor) = trimmed.strip_prefix('#') {
            let clean_anchor = (!anchor.is_empty()).then(|| anchor.to_string());
            return Some((None, clean_anchor));
        }
        if trimmed.starts_with('/') {
            let (path_part, anchor_part) = match trimmed.split_once('#') {
                Some((p, a)) => (p, (!a.is_empty()).then(|| a.to_string())),
                None => (trimmed, None),
            };
            return Some((Some(DocsRoute::new(path_part)), anchor_part));
        }
        None
    }
}

impl fmt::Display for DocsRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for DocsRoute {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for DocsRoute {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl Routable for DocsRoute {
    fn parse_path(path: &str, _query: &QueryParams) -> Option<Self> {
        Some(Self::new(path))
    }

    fn to_path(&self) -> String {
        normalize_route(&self.0)
    }
}
