//! Portable `FetchConfig` and `CredentialsMode` configuration (compiles and tests on both native
//! and `wasm32`).

use nve_rpc::{
    format_rpc_path_into, validate_bearer_token, CborLimits, TransportError, DEFAULT_RPC_PREFIX,
};

/// Default browser `fetch` timeout in milliseconds (`30s`).
pub const DEFAULT_FETCH_TIMEOUT_MS: u32 = 30_000;

/// Browser Fetch `credentials` mode.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CredentialsMode {
    /// Sends `HttpOnly; SameSite=Lax` cookies only to the same origin (recommended default).
    #[default]
    SameOrigin = 0,
    /// Sends cookies on cross-origin CORS requests (requires server `CorsPolicy::allow_credentials(true)`).
    Include = 1,
    /// Never sends cookies (recommended when using `Authorization: Bearer`).
    Omit = 2,
}

impl CredentialsMode {
    #[inline]
    pub const fn as_js_str(self) -> &'static str {
        match self {
            Self::SameOrigin => "same-origin",
            Self::Include => "include",
            Self::Omit => "omit",
        }
    }
}

/// Configuration for [`crate::FetchTransport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchConfig {
    /// Base origin (empty string `""` for relative same-origin `/rpc/...` calls).
    pub base_url: String,
    /// RPC path prefix (default: `"/rpc"`).
    pub rpc_prefix: String,
    /// Browser `fetch` credentials policy (default: [`CredentialsMode::SameOrigin`]).
    pub credentials: CredentialsMode,
    /// End-to-end request + response body timeout in milliseconds (`0` disables timeout).
    pub timeout_ms: u32,
    /// Hard maximum response body size enforced in JS BEFORE copying into WASM linear memory
    /// (default: `4 MiB`, aligned with [`CborLimits::DEFAULT`]).
    pub max_response_bytes: usize,
    /// Optional validated `Authorization: Bearer <token>` header value.
    pub(crate) auth_header: Option<String>,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self::same_origin()
    }
}

impl FetchConfig {
    /// Same-origin relative configuration (`POST /rpc/<Procedure::NAME>`, `credentials: "same-origin"`).
    pub fn same_origin() -> Self {
        Self {
            base_url: String::new(),
            rpc_prefix: DEFAULT_RPC_PREFIX.to_owned(),
            credentials: CredentialsMode::SameOrigin,
            timeout_ms: DEFAULT_FETCH_TIMEOUT_MS,
            max_response_bytes: CborLimits::DEFAULT.max_bytes,
            auth_header: None,
        }
    }

    /// Explicit base URL configuration (e.g. `"http://localhost:3000"` for cross-origin CORS dev).
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        let mut cfg = Self::same_origin();
        cfg.base_url = base_url.into().trim().trim_end_matches('/').to_owned();
        cfg
    }

    pub fn with_rpc_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.rpc_prefix = prefix.into();
        self
    }

    #[inline]
    pub fn with_credentials(mut self, credentials: CredentialsMode) -> Self {
        self.credentials = credentials;
        self
    }

    #[inline]
    pub fn with_timeout_ms(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    #[inline]
    pub fn with_max_response_bytes(mut self, max_bytes: usize) -> Self {
        self.max_response_bytes = max_bytes;
        self
    }

    /// Attaches a validated `Authorization: Bearer <token>` header (and sets `credentials` to
    /// `Omit` by default unless overridden afterward).
    pub fn with_bearer_token(mut self, token: &str) -> Result<Self, TransportError> {
        validate_bearer_token(token)?;
        let mut hdr = String::with_capacity(7 + token.len());
        hdr.push_str("Bearer ");
        hdr.push_str(token);
        self.auth_header = Some(hdr);
        self.credentials = CredentialsMode::Omit;
        Ok(self)
    }

    /// Formats the target fetch URL for `proc_name`.
    pub fn build_url(&self, proc_name: &str) -> String {
        let mut out =
            String::with_capacity(self.base_url.len() + self.rpc_prefix.len() + proc_name.len() + 2);
        out.push_str(&self.base_url);
        format_rpc_path_into(&self.rpc_prefix, proc_name, &mut out);
        out
    }

    #[inline]
    pub fn auth_header(&self) -> Option<&str> {
        self.auth_header.as_deref()
    }
}
