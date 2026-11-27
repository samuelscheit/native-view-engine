//! Server configuration (`ServerConfig`), validated progressive form routes (`FormRoute`),
//! and path prefix normalization helpers.

use nve_rpc::{Action, Procedure, DEFAULT_MAX_FORM_FIELDS, DEFAULT_RPC_PREFIX};

use crate::csrf::validate_same_origin_redirect;

/// Server-configured progressive HTML `<form method="post">` route bound to a registered
/// [`Action`] procedure and a validated same-origin `303 See Other` redirect target.
///
/// Fields are sealed behind [`FormRoute::new`] so unnormalized paths, non-action procedures, or
/// open-redirect targets cannot be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormRoute {
    path: String,
    proc_name: &'static str,
    success_redirect: String,
}

impl FormRoute {
    /// Creates a validated [`FormRoute`], panicking immediately if `path` or `success_redirect`
    /// is invalid.
    pub fn new<P: Procedure<Effect = Action>>(
        path: impl Into<String>,
        success_redirect: impl Into<String>,
    ) -> Self {
        let path = normalize_exact_path(&path.into());
        assert!(!path.is_empty(), "form route path must not be empty");
        let redirect = success_redirect.into();
        let _ = validate_same_origin_redirect(&redirect)
            .unwrap_or_else(|err| panic!("invalid FormRoute success_redirect `{redirect}`: {err}"));
        Self {
            path,
            proc_name: P::NAME,
            success_redirect: redirect,
        }
    }

    #[inline]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[inline]
    pub fn proc_name(&self) -> &'static str {
        self.proc_name
    }

    #[inline]
    pub fn success_redirect(&self) -> &str {
        &self.success_redirect
    }
}

/// Configuration for [`crate::Server`].
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// URL path prefix for RPC procedures (default: `"/rpc"`).
    pub rpc_prefix: String,
    /// Hard maximum incoming request body size in bytes (default: `1 MiB`).
    pub max_body_bytes: usize,
    /// Hard maximum form field pairs accepted in `application/x-www-form-urlencoded` bodies
    /// (default: `1024`).
    pub max_form_fields: usize,
    /// Optional server-owned progressive HTML form routes dispatched automatically by
    /// [`crate::Server::handle_request`] and [`crate::Server::preflight_request`].
    pub form_routes: Vec<FormRoute>,
}

impl ServerConfig {
    /// Appends a validated server-owned HTML `<form method="post">` route for action `P`.
    pub fn with_form_route<P: Procedure<Effect = Action>>(
        mut self,
        path: impl Into<String>,
        success_redirect: impl Into<String>,
    ) -> Self {
        self.form_routes.push(FormRoute::new::<P>(path, success_redirect));
        self
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            rpc_prefix: DEFAULT_RPC_PREFIX.to_owned(),
            max_body_bytes: 1024 * 1024,
            max_form_fields: DEFAULT_MAX_FORM_FIELDS,
            form_routes: Vec::new(),
        }
    }
}

pub(crate) fn normalize_prefix(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{trimmed}")
    }
}

pub(crate) fn normalize_exact_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let clean = trimmed.trim_end_matches('/');
    if clean.is_empty() {
        "/".to_owned()
    } else if clean.starts_with('/') {
        clean.to_owned()
    } else {
        format!("/{clean}")
    }
}

pub(crate) fn strip_rpc_prefix<'a>(path: &'a str, clean_prefix: &str) -> Option<&'a str> {
    let rest = if clean_prefix.is_empty() {
        path.strip_prefix('/')?
    } else {
        let after = path.strip_prefix(clean_prefix)?;
        after.strip_prefix('/')?
    };
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}
