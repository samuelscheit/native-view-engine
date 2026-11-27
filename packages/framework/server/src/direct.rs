//! In-process [`nve_rpc::Transport`] (`DirectTransport`) for SSR loaders and integration tests.

use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderName, HeaderValue, Method, Request};
use nve_rpc::{
    format_rpc_path, BoxFuture, CborCodec, CodecFormat, MaybeSend, MaybeSync, RawCall, RawReply,
    Transport, TransportError,
};

use crate::auth::{make_bearer_header, AuthResolver};
use crate::response::parse_rpc_http_reply;
use crate::server::Server;

#[derive(Clone)]
enum DirectMode<U> {
    /// Trusted in-process SSR mode: skips browser CSRF checks for read-only
    /// [`nve_rpc::Effect::Query`] calls so SSR loaders can forward navigation `Cookie` +
    /// `Sec-Fetch-Site: cross-site` headers safely. Rejects [`nve_rpc::Effect::Action`] unless
    /// explicitly enabled via [`DirectTransport::allow_trusted_actions`].
    Trusted {
        principal_override: Option<U>,
        allow_actions: bool,
    },
    /// Strict HTTP mode: routes every call through [`Server::handle_rpc_request`] with full CSRF,
    /// `Content-Type`, and [`AuthResolver`] checks.
    StrictHttp,
}

/// In-process [`Transport`] executing directly against a [`Server`].
///
/// Use [`Self::new`] (trusted SSR mode, read-only queries by default) or [`Self::strict_http`]
/// (full HTTP CSRF/auth simulation). Because the mode is mutually exclusive by construction,
/// pre-resolved principals can never be silently dropped under strict HTTP mode.
pub struct DirectTransport<S, U, A, C = CborCodec> {
    server: Server<S, U, A, C>,
    default_headers: HeaderMap,
    mode: DirectMode<U>,
}

impl<S, U: Clone, A, C> Clone for DirectTransport<S, U, A, C> {
    fn clone(&self) -> Self {
        Self {
            server: self.server.clone(),
            default_headers: self.default_headers.clone(),
            mode: self.mode.clone(),
        }
    }
}

impl<S, U, A, C> DirectTransport<S, U, A, C>
where
    S: Clone + MaybeSend + MaybeSync + 'static,
    U: Clone + MaybeSend + MaybeSync + 'static,
    A: AuthResolver<S, U>,
    C: Clone + MaybeSend + MaybeSync + 'static,
{
    /// Creates a trusted in-process transport for SSR loaders (read-only queries allowed without
    /// browser CSRF checks; actions require [`Self::allow_trusted_actions`] or [`Self::strict_http`]).
    pub fn new(server: Server<S, U, A, C>) -> Self {
        Self {
            server,
            default_headers: HeaderMap::new(),
            mode: DirectMode::Trusted {
                principal_override: None,
                allow_actions: false,
            },
        }
    }

    /// Creates a trusted in-process transport backed by an already-authenticated principal,
    /// bypassing [`AuthResolver`].
    pub fn with_principal(server: Server<S, U, A, C>, principal: U) -> Self {
        Self {
            server,
            default_headers: HeaderMap::new(),
            mode: DirectMode::Trusted {
                principal_override: Some(principal),
                allow_actions: false,
            },
        }
    }

    /// Creates a strict HTTP in-process transport that routes calls through
    /// [`Server::handle_rpc_request`] with identical CSRF, `Content-Type`, and auth enforcement
    /// as live HTTP requests.
    pub fn strict_http(server: Server<S, U, A, C>) -> Self {
        Self {
            server,
            default_headers: HeaderMap::new(),
            mode: DirectMode::StrictHttp,
        }
    }

    /// Opts a trusted in-process transport into executing state-mutating [`nve_rpc::Effect::Action`]
    /// procedures without browser CSRF checks (e.g. trusted backend jobs or tests).
    ///
    /// Has no effect on [`Self::strict_http`] transports, which always enforce CSRF on actions.
    pub fn allow_trusted_actions(mut self, allow: bool) -> Self {
        if let DirectMode::Trusted { allow_actions, .. } = &mut self.mode {
            *allow_actions = allow;
        }
        self
    }

    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.default_headers.insert(name, value);
        self
    }

    pub fn with_bearer_token(mut self, token: &str) -> Result<Self, TransportError> {
        let val = make_bearer_header(token)?;
        self.default_headers.insert(http::header::AUTHORIZATION, val);
        Ok(self)
    }
}

impl<S, U, A, C> Transport for DirectTransport<S, U, A, C>
where
    S: Clone + MaybeSend + MaybeSync + 'static,
    U: Clone + MaybeSend + MaybeSync + 'static,
    A: AuthResolver<S, U>,
    C: CodecFormat,
{
    fn call_raw<'a>(&'a self, req: RawCall<'a>) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        let proc_name = req.name;
        let content_type = req.content_type;
        let body = req.body;

        Box::pin(async move {
            let resp = match &self.mode {
                DirectMode::StrictHttp => {
                    let path = format_rpc_path(self.server.rpc_prefix(), proc_name);
                    let mut http_req = Request::builder()
                        .method(Method::POST)
                        .uri(&path)
                        .body(body)
                        .map_err(|err| TransportError::new(err.to_string()))?;
                    *http_req.headers_mut() = self.default_headers.clone();
                    http_req
                        .headers_mut()
                        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
                    self.server.handle_rpc_request(http_req).await
                }
                DirectMode::Trusted {
                    principal_override,
                    allow_actions,
                } => {
                    self.server
                        .execute_trusted_direct(
                            proc_name,
                            content_type,
                            self.default_headers.clone(),
                            principal_override.clone(),
                            *allow_actions,
                            body,
                        )
                        .await?
                }
            };

            let (parts, body) = resp.into_parts();
            parse_rpc_http_reply(parts.status, &parts.headers, body)
        })
    }
}
