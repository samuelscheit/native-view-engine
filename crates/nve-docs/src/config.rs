//! Site configuration and top-bar navigation sections for `nve-docs`.

use nve_mdx::normalize_route;

/// Top-level documentation section displayed in the sticky navbar tab bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocsNavSection {
    /// Stable identifier used for keys and `test_id` attributes.
    pub id: String,
    /// Human-readable tab label (e.g. `"Guides"`, `"Architecture"`, `"Hosts"`, `"Reference"`).
    pub label: String,
    /// Route prefix that activates this section (e.g. `"/guides"`, `"/architecture"`).
    pub prefix: String,
    /// Target route navigated to when clicking this section tab.
    pub default_route: String,
    /// Optional pill badge displayed next to the label.
    pub badge: Option<String>,
}

impl DocsNavSection {
    /// Create a new top-bar navigation section.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        prefix: impl Into<String>,
        default_route: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            prefix: normalize_route(&prefix.into()),
            default_route: normalize_route(&default_route.into()),
            badge: None,
        }
    }

    /// Attach an optional badge pill (e.g. `"New"` or `"v5"`).
    pub fn with_badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    /// Returns `true` when `route` falls under this section's prefix.
    pub fn matches_route(&self, route: &str) -> bool {
        let norm = normalize_route(route);
        if self.prefix == "/" {
            return norm == "/";
        }
        norm == self.prefix
            || norm
                .strip_prefix(&self.prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    }
}

/// Top-level configuration for a documentation site rendered by `nve-docs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocsSiteConfig {
    /// Display title rendered in the navbar and HTML `<title>`.
    pub site_title: String,
    /// Short subtitle or site tagline.
    pub site_tagline: String,
    /// Compact brand mark glyph or monogram rendered beside `site_title`.
    pub brand_mark: String,
    /// Optional GitHub repository URL for the navbar icon/link.
    pub github_repo_url: Option<String>,
    /// Optional base URL for "Edit this page on GitHub" links (joined with `page.source_path`).
    pub edit_base_url: Option<String>,
    /// Optional deployment base path (e.g. `""` or `"/docs"`).
    pub base_path: String,
    /// Whether the site defaults to Fumadocs/Linear dark mode when the host reports `Unspecified`.
    pub default_dark: bool,
    /// Top-bar section switcher tabs (e.g. `Guides`, `Architecture`, `Hosts`, `Reference`).
    pub sections: Vec<DocsNavSection>,
}

impl Default for DocsSiteConfig {
    fn default() -> Self {
        Self {
            site_title: "Documentation".to_string(),
            site_tagline: "Built with native-view-engine".to_string(),
            brand_mark: "NVE".to_string(),
            github_repo_url: None,
            edit_base_url: None,
            base_path: String::new(),
            default_dark: true,
            sections: Vec::new(),
        }
    }
}

impl DocsSiteConfig {
    /// Create a new dark-first site configuration with the given title.
    pub fn new(site_title: impl Into<String>) -> Self {
        Self {
            site_title: site_title.into(),
            ..Self::default()
        }
    }

    pub fn with_tagline(mut self, tagline: impl Into<String>) -> Self {
        self.site_tagline = tagline.into();
        self
    }

    pub fn with_brand_mark(mut self, mark: impl Into<String>) -> Self {
        self.brand_mark = mark.into();
        self
    }

    pub fn with_github_repo(mut self, url: impl Into<String>) -> Self {
        let s = url.into();
        self.github_repo_url = (!s.trim().is_empty()).then_some(s);
        self
    }

    pub fn with_edit_base_url(mut self, url: impl Into<String>) -> Self {
        let s = url.into();
        self.edit_base_url = (!s.trim().is_empty()).then_some(s);
        self
    }

    pub fn with_base_path(mut self, base_path: impl Into<String>) -> Self {
        self.base_path = normalize_base_path(&base_path.into());
        self
    }

    pub fn with_default_dark(mut self, default_dark: bool) -> Self {
        self.default_dark = default_dark;
        self
    }

    pub fn with_sections(mut self, sections: Vec<DocsNavSection>) -> Self {
        self.sections = sections;
        self
    }

    pub fn add_section(mut self, section: DocsNavSection) -> Self {
        self.sections.push(section);
        self
    }

    /// Build the "Edit this page" URL for a compiled page's `source_path`, if `edit_base_url` is set.
    pub fn edit_url_for(&self, source_path: &str) -> Option<String> {
        let base = self.edit_base_url.as_deref()?.trim_end_matches('/');
        let rel = source_path.trim_start_matches('/');
        if base.is_empty() || rel.is_empty() {
            return None;
        }
        Some(format!("{base}/{rel}"))
    }

    /// Resolve a site-internal route (and optional `#anchor`) against `base_path` for SSR `href`.
    pub fn resolve_href(&self, route_or_url: &str) -> String {
        let trimmed = route_or_url.trim();
        if trimmed.is_empty() {
            return if self.base_path.is_empty() {
                "/".to_string()
            } else {
                self.base_path.clone()
            };
        }
        if trimmed.starts_with('#')
            || trimmed.starts_with("http://")
            || trimmed.starts_with("https://")
            || trimmed.starts_with("mailto:")
        {
            return trimmed.to_string();
        }
        let (path_part, anchor) = match trimmed.split_once('#') {
            Some((p, a)) => (p, Some(a)),
            None => (trimmed, None),
        };
        let norm = normalize_route(path_part);
        let base = normalize_base_path(&self.base_path);
        let joined = if base.is_empty() {
            norm
        } else if norm == "/" {
            base
        } else {
            format!("{base}{norm}")
        };
        match anchor {
            Some(a) if !a.is_empty() => format!("{joined}#{a}"),
            _ => joined,
        }
    }

    /// Return the most specific matching top-bar section for `route`.
    pub fn active_section_for(&self, route: &str) -> Option<&DocsNavSection> {
        self.sections
            .iter()
            .filter(|s| s.matches_route(route))
            .max_by_key(|s| s.prefix.len())
    }
}

/// Normalize a deployment base path (`""` for root, or `"/docs"` with no trailing slash).
pub(crate) fn normalize_base_path(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{trimmed}")
    }
}
