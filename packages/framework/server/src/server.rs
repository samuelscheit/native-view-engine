//! Sans-I/O `Server<S, U, A, C>` request dispatcher with zero-body-read preflight and built-in
//! progressive form routing.

use core::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use http::header::COOKIE;
use http::{HeaderMap, Method, Request, Response};
use nve_rpc::{
    CborCodec, CodecFormat, Effect, FormPairs, MaybeSend, MaybeSync, RpcStatus,
    CONTENT_TYPE_FORM_URLENCODED,
};

use crate::auth::AuthResolver;
use crate::config::{normalize_prefix, strip_rpc_prefix, FormRoute, ServerConfig};
use crate::csrf::CsrfPolicy;
use crate::registry::{Cx, FormHandlerOutcome, RpcRegistry, TagCollector};
use crate::response::{
    form_outcome_to_response, outcome_to_rpc_response, protocol_error_response, FormOutcome,
    PreflightError,
};

static NEXT_SERVER_ID: AtomicU64 = AtomicU64::new(1);

/// Validated pre-body dispatch token returned by [`Server::preflight_request`].
///
/// Sealed to the originating [`Server`] instance and owns the sanitized [`HeaderMap`] (with
/// `Cookie` already stripped whenever `Authorization: Bearer` bypassed CSRF), so callers cannot
/// accidentally pass unstripped headers or cross-server tokens into [`Server::execute_prepared`].
#[derive(Debug)]
pub struct PreparedRequest<U> {
    server_id: u64,
    kind: PreparedKind<U>,
}

impl<U> PreparedRequest<U> {
    /// Returns `false` when the prepared response needs no HTTP request body (e.g. an `OPTIONS`
    /// CORS preflight) so streaming adapters can skip body polling entirely.
    #[inline]
    pub fn needs_body(&self) -> bool {
        !matches!(self.kind, PreparedKind::CorsPreflight { .. })
    }

    #[inline]
    fn cors_origin_idx(&self) -> Option<usize> {
        match &self.kind {
            PreparedKind::Rpc { cors_origin_idx, .. } => *cors_origin_idx,
            PreparedKind::CorsPreflight { origin_idx } => Some(*origin_idx),
            PreparedKind::Form { .. } => None,
        }
    }
}

#[derive(Debug)]
enum PreparedKind<U> {
    Rpc {
        entry_idx: usize,
        principal: U,
        headers: HeaderMap,
        cors_origin_idx: Option<usize>,
    },
    Form {
        entry_idx: usize,
        route_idx: usize,
        principal: U,
        headers: HeaderMap,
    },
    CorsPreflight {
        origin_idx: usize,
    },
}

pub(crate) struct ServerInner<S, U, A, C> {
    server_id: u64,
    state: S,
    pub(crate) registry: RpcRegistry<S, U, C>,
    auth: A,
    csrf: CsrfPolicy,
    pub(crate) config: ServerConfig,
    rpc_prefix_clean: String,
}

/// Sans-I/O RPC server handler (`Clone + Send + Sync + 'static` on native; `?Send` on `wasm32`).
///
/// Works identically inside `nve-http-hyper`, mounted in `axum::Router::nest_service` or
/// `route_service`, in-process via [`crate::DirectTransport`], or compiled to `wasm32-wasip2`.
pub struct Server<S, U, A, C = CborCodec> {
    pub(crate) inner: Arc<ServerInner<S, U, A, C>>,
}

impl<S, U, A, C> Clone for Server<S, U, A, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S, U, A, C> Server<S, U, A, C>
where
    S: Clone + MaybeSend + MaybeSync + 'static,
    U: Clone + MaybeSend + MaybeSync + 'static,
    A: AuthResolver<S, U>,
    C: Clone + MaybeSend + MaybeSync + 'static,
{
    pub fn new(
        state: S,
        registry: RpcRegistry<S, U, C>,
        auth: A,
        csrf: CsrfPolicy,
        config: ServerConfig,
    ) -> Self {
        for route in &config.form_routes {
            let idx = registry.lookup_index(route.proc_name()).unwrap_or_else(|| {
                panic!(
                    "FormRoute `{}` targets unregistered procedure `{}`",
                    route.path(),
                    route.proc_name()
                )
            });
            assert!(
                registry.entry_at(idx).supports_form(),
                "FormRoute `{}` targets procedure `{}` registered without `register_with_form`",
                route.path(),
                route.proc_name()
            );
        }

        let rpc_prefix_clean = normalize_prefix(&config.rpc_prefix);
        let server_id = NEXT_SERVER_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: Arc::new(ServerInner {
                server_id,
                state,
                registry,
                auth,
                csrf,
                config,
                rpc_prefix_clean,
            }),
        }
    }

    #[inline]
    pub fn config(&self) -> &ServerConfig {
        &self.inner.config
    }

    #[inline]
    pub fn rpc_prefix(&self) -> &str {
        &self.inner.rpc_prefix_clean
    }

    #[inline]
    pub fn state(&self) -> &S {
        &self.inner.state
    }

    #[inline]
    pub fn csrf(&self) -> &CsrfPolicy {
        &self.inner.csrf
    }

    #[inline]
    pub fn registry(&self) -> &RpcRegistry<S, U, C> {
        &self.inner.registry
    }

    /// Applies opt-in CORS response headers when `ServerConfig::cors` is enabled and `req_headers`
    /// contains a single allowed `Origin`. Safe to call on error responses (`401`, `404`, `413`,
    /// `408`) so browsers expose error statuses and `Nve-Error-Kind` instead of opaque `TypeError`.
    pub fn apply_cors_headers(&self, req_headers: &HeaderMap, resp: &mut Response<Bytes>) {
        if let Some(cors) = &self.inner.config.cors {
            cors.apply_if_allowed(req_headers, resp);
        }
    }

    /// Converts a [`PreflightError`] into an HTTP response, attaching opt-in CORS headers ONLY
    /// when the rejection occurred on an admitted CORS RPC route (never on `FormRoute`, non-RPC
    /// `404`s, or rejected `OPTIONS` preflights).
    pub fn preflight_error_response(&self, err: PreflightError) -> Response<Bytes> {
        let origin_idx = err.cors_origin_idx;
        let mut resp = err.into_response();
        if let (Some(cors), Some(idx)) = (&self.inner.config.cors, origin_idx) {
            cors.apply_origin_headers(idx, &mut resp);
        }
        resp
    }

    /// Applies opt-in CORS response headers carried on a validated [`PreparedRequest`] owned by
    /// this server (e.g. when a streaming adapter hits a `413` or `408` body read error).
    pub fn apply_prepared_cors_headers(
        &self,
        prepared: &PreparedRequest<U>,
        resp: &mut Response<Bytes>,
    ) {
        if prepared.server_id != self.inner.server_id {
            return;
        }
        if let (Some(cors), Some(origin_idx)) =
            (&self.inner.config.cors, prepared.cors_origin_idx())
        {
            cors.apply_origin_headers(origin_idx, resp);
        }
    }

    /// Resolves route, method (`POST`), CSRF, `Content-Type`, registry entry, and principal
    /// BEFORE reading the HTTP request body.
    ///
    /// When `nested_rpc` is `true` (e.g. `axum::Router::nest_service("/rpc", ...)`), the
    /// configured `rpc_prefix` has already been stripped from `parts.uri.path()`.
    pub async fn preflight_request(
        &self,
        parts: &mut http::request::Parts,
        nested_rpc: bool,
    ) -> Result<PreparedRequest<U>, PreflightError>
    where
        C: CodecFormat,
    {
        let path = parts.uri.path();

        if !nested_rpc
            && let Some((route_idx, route)) = self
                .inner
                .config
                .form_routes
                .iter()
                .enumerate()
                .find(|(_, r)| r.path() == path)
        {
            return self.preflight_form_parts(parts, route_idx, route).await;
        }

        self.preflight_rpc_parts(parts, nested_rpc).await
    }

    async fn preflight_rpc_parts(
        &self,
        parts: &mut http::request::Parts,
        nested_rpc: bool,
    ) -> Result<PreparedRequest<U>, PreflightError>
    where
        C: CodecFormat,
    {
        // Match route prefix BEFORE method check so `GET /favicon.ico` returns `404 Not Found`
        // rather than `405 Method Not Allowed (Allow: POST)` (RFC 9110 §15.5.6).
        let proc_name = if nested_rpc {
            strip_rpc_prefix(parts.uri.path(), "")
        } else {
            strip_rpc_prefix(parts.uri.path(), &self.inner.rpc_prefix_clean)
        };
        let Some(proc_name) = proc_name else {
            return Err(PreflightError::protocol(
                RpcStatus::NotFound,
                "unknown RPC route",
            ));
        };

        if parts.method == Method::OPTIONS
            && let Some(cors) = &self.inner.config.cors
        {
            if self.inner.registry.lookup_index(proc_name).is_none() {
                return Err(PreflightError::protocol(
                    RpcStatus::NotFound,
                    "unknown RPC procedure",
                ));
            }
            let origin_idx = cors
                .check_preflight(&parts.headers)
                .map_err(|reason| PreflightError::protocol(RpcStatus::PermissionDenied, reason))?;
            return Ok(PreparedRequest {
                server_id: self.inner.server_id,
                kind: PreparedKind::CorsPreflight { origin_idx },
            });
        }

        let cors_enabled = self.inner.config.cors.is_some();
        if parts.method != Method::POST {
            return Err(PreflightError::method_not_allowed(
                "RPC endpoints require HTTP POST",
                cors_enabled,
            ));
        }

        let cors_origin_idx = if let Some(cors) = &self.inner.config.cors {
            cors.match_origin(&parts.headers).map_err(|_| {
                PreflightError::protocol(
                    RpcStatus::PermissionDenied,
                    "duplicate or invalid Origin header",
                )
            })?
        } else {
            None
        };

        if let (Some(cors), Some(origin_idx)) = (&self.inner.config.cors, cors_origin_idx) {
            cors.check_rpc_headers(
                &mut parts.headers,
                C::CONTENT_TYPE,
                origin_idx,
                self.inner.csrf.allows_cookies(),
            )?;
        } else {
            let verdict = self
                .inner
                .csrf
                .check_content_type_and_origin(&parts.method, &parts.headers, C::CONTENT_TYPE)
                .map_err(|rej| PreflightError::protocol(rej.status, rej.reason))?;
            if verdict.bypassed_via_bearer {
                parts.headers.remove(COOKIE);
            }
        }

        let Some(entry_idx) = self.inner.registry.lookup_index(proc_name) else {
            return Err(
                PreflightError::protocol(RpcStatus::NotFound, "unknown RPC procedure")
                    .with_cors_origin(cors_origin_idx),
            );
        };

        let principal = self
            .resolve_principal(&parts.headers)
            .await
            .map_err(|err| err.with_cors_origin(cors_origin_idx))?;
        Ok(PreparedRequest {
            server_id: self.inner.server_id,
            kind: PreparedKind::Rpc {
                entry_idx,
                principal,
                headers: core::mem::take(&mut parts.headers),
                cors_origin_idx,
            },
        })
    }

    async fn preflight_form_route(
        &self,
        parts: &mut http::request::Parts,
        route: &FormRoute,
    ) -> Result<(usize, U, HeaderMap), PreflightError> {
        if parts.method != Method::POST {
            return Err(PreflightError::method_not_allowed(
                "form action requires HTTP POST",
                false,
            ));
        }
        let verdict = self
            .inner
            .csrf
            .check_content_type_and_origin(
                &parts.method,
                &parts.headers,
                CONTENT_TYPE_FORM_URLENCODED,
            )
            .map_err(|rej| PreflightError::protocol(rej.status, rej.reason))?;
        if verdict.bypassed_via_bearer {
            parts.headers.remove(COOKIE);
        }

        let Some(entry_idx) = self.inner.registry.lookup_index(route.proc_name()) else {
            return Err(PreflightError::protocol(
                RpcStatus::NotFound,
                "unknown form action procedure",
            ));
        };
        if !self.inner.registry.entry_at(entry_idx).supports_form() {
            return Err(PreflightError::protocol(
                RpcStatus::UnsupportedMediaType,
                "procedure does not accept HTML form submissions",
            ));
        }

        let principal = self.resolve_principal(&parts.headers).await?;
        Ok((entry_idx, principal, core::mem::take(&mut parts.headers)))
    }

    async fn preflight_form_parts(
        &self,
        parts: &mut http::request::Parts,
        route_idx: usize,
        route: &FormRoute,
    ) -> Result<PreparedRequest<U>, PreflightError> {
        let (entry_idx, principal, headers) = self.preflight_form_route(parts, route).await?;
        Ok(PreparedRequest {
            server_id: self.inner.server_id,
            kind: PreparedKind::Form {
                entry_idx,
                route_idx,
                principal,
                headers,
            },
        })
    }

    /// Executes a request already validated by [`Self::preflight_request`] with its bounded `body`.
    pub async fn execute_prepared(
        &self,
        prepared: PreparedRequest<U>,
        body: Bytes,
    ) -> Response<Bytes>
    where
        C: CodecFormat,
    {
        if prepared.server_id != self.inner.server_id {
            return protocol_error_response(
                RpcStatus::Internal,
                "PreparedRequest does not belong to this Server instance",
            );
        }

        if body.len() > self.inner.config.max_body_bytes && prepared.needs_body() {
            let mut resp = protocol_error_response(
                RpcStatus::PayloadTooLarge,
                "request body exceeds configured max_body_bytes",
            );
            self.apply_prepared_cors_headers(&prepared, &mut resp);
            return resp;
        }

        match prepared.kind {
            PreparedKind::CorsPreflight { origin_idx } => match &self.inner.config.cors {
                Some(cors) => cors.preflight_response(origin_idx),
                None => protocol_error_response(RpcStatus::Internal, "CORS policy unavailable"),
            },
            PreparedKind::Rpc {
                entry_idx,
                principal,
                headers,
                cors_origin_idx,
            } => {
                let mut resp = self
                    .execute_rpc_entry(entry_idx, principal, headers, body)
                    .await;
                if let (Some(cors), Some(origin_idx)) = (&self.inner.config.cors, cors_origin_idx) {
                    cors.apply_origin_headers(origin_idx, &mut resp);
                }
                resp
            }
            PreparedKind::Form {
                entry_idx,
                route_idx,
                principal,
                headers,
            } => {
                let Some(route) = self.inner.config.form_routes.get(route_idx) else {
                    return protocol_error_response(
                        RpcStatus::Internal,
                        "invalid prepared form route index",
                    );
                };
                let outcome = self
                    .execute_form_entry(
                        entry_idx,
                        principal,
                        headers,
                        body,
                        route.success_redirect(),
                    )
                    .await;
                form_outcome_to_response(outcome)
            }
        }
    }

    /// Unified sans-I/O request handler dispatching both `/rpc/*` and registered `FormRoute`s.
    pub async fn handle_request(&self, req: Request<Bytes>) -> Response<Bytes>
    where
        C: CodecFormat,
    {
        let (mut parts, body) = req.into_parts();
        let prepared = match self.preflight_request(&mut parts, false).await {
            Ok(p) => p,
            Err(err) => return self.preflight_error_response(err),
        };
        self.execute_prepared(prepared, body).await
    }

    /// Handles a binary RPC HTTP request (`POST <rpc_prefix>/<Procedure::NAME>`).
    pub async fn handle_rpc_request(&self, req: Request<Bytes>) -> Response<Bytes>
    where
        C: CodecFormat,
    {
        let (mut parts, body) = req.into_parts();
        let prepared = match self.preflight_rpc_parts(&mut parts, false).await {
            Ok(p) => p,
            Err(err) => return self.preflight_error_response(err),
        };
        self.execute_prepared(prepared, body).await
    }

    /// Executes a progressive 0-JS HTML `<form method="post">` submission for a validated
    /// [`FormRoute`] and returns a typed [`FormOutcome`] (preserving submitted [`FormPairs`] +
    /// [`nve_rpc::FieldErrors`] for SSR HTML re-rendering).
    pub async fn execute_form_action(
        &self,
        route: &FormRoute,
        req: Request<Bytes>,
    ) -> FormOutcome {
        let (mut parts, body) = req.into_parts();
        let (entry_idx, principal, headers) = match self.preflight_form_route(&mut parts, route).await
        {
            Ok(v) => v,
            Err(err) => return FormOutcome::ProtocolError(Box::new(err.into_response())),
        };
        if body.len() > self.inner.config.max_body_bytes {
            return FormOutcome::ProtocolError(Box::new(protocol_error_response(
                RpcStatus::PayloadTooLarge,
                "form body exceeds configured max_body_bytes",
            )));
        }
        self.execute_form_entry(entry_idx, principal, headers, body, route.success_redirect())
            .await
    }

    pub(crate) async fn resolve_principal(&self, headers: &HeaderMap) -> Result<U, PreflightError> {
        if let Some(res) = self.inner.auth.try_resolve_sync(&self.inner.state, headers) {
            return res.map_err(|err| PreflightError::protocol(err.status, err.message));
        }
        self.inner
            .auth
            .resolve(&self.inner.state, headers)
            .await
            .map_err(|err| PreflightError::protocol(err.status, err.message))
    }

    pub(crate) async fn execute_rpc_entry(
        &self,
        entry_idx: usize,
        principal: U,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response<Bytes>
    where
        C: CodecFormat,
    {
        let Some(entry) = self.inner.registry.entry_get(entry_idx) else {
            return protocol_error_response(
                RpcStatus::Internal,
                "invalid prepared RPC entry index",
            );
        };
        let tags = match entry.effect {
            Effect::Query => TagCollector::noop(),
            Effect::Action => TagCollector::active(),
        };
        let cx = Cx::new(self.inner.state.clone(), principal, headers, tags);
        let outcome = entry.call_rpc(cx, body).await;
        outcome_to_rpc_response::<C>(outcome)
    }

    async fn execute_form_entry(
        &self,
        entry_idx: usize,
        principal: U,
        headers: HeaderMap,
        body: Bytes,
        success_redirect: &str,
    ) -> FormOutcome {
        let Ok(body_str) = core::str::from_utf8(&body) else {
            return FormOutcome::ProtocolError(Box::new(protocol_error_response(
                RpcStatus::InvalidArgument,
                "form body is not valid UTF-8",
            )));
        };
        let form = match FormPairs::from_urlencoded_limited(
            body_str,
            self.inner.config.max_form_fields,
        ) {
            Ok(f) => f,
            Err(msg) => {
                return FormOutcome::ProtocolError(Box::new(protocol_error_response(
                    RpcStatus::PayloadTooLarge,
                    msg,
                )));
            }
        };

        let Some(entry) = self.inner.registry.entry_get(entry_idx) else {
            return FormOutcome::ProtocolError(Box::new(protocol_error_response(
                RpcStatus::Internal,
                "invalid prepared form entry index",
            )));
        };
        let cx = Cx::new(
            self.inner.state.clone(),
            principal,
            headers,
            TagCollector::active(),
        );
        let Some(fut) = entry.call_form(cx, form) else {
            return FormOutcome::ProtocolError(Box::new(protocol_error_response(
                RpcStatus::UnsupportedMediaType,
                "procedure does not accept HTML form submissions",
            )));
        };

        match fut.await {
            FormHandlerOutcome::Ok { invalidated } => FormOutcome::Redirect {
                location: success_redirect.to_owned(),
                invalidated,
            },
            FormHandlerOutcome::FormDecodeErr { errors, form } => {
                FormOutcome::ValidationFailed { errors, form }
            }
            FormHandlerOutcome::DomainErr { status, body } => {
                FormOutcome::DomainError { status, body }
            }
            FormHandlerOutcome::EncodeFailure(err) => FormOutcome::ProtocolError(Box::new(
                protocol_error_response(RpcStatus::Internal, &err.to_string()),
            )),
        }
    }
}
