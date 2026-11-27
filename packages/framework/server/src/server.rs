//! Sans-I/O `Server<S, U, A, C>` request dispatcher with zero-body-read preflight and built-in
//! progressive form routing.

use core::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use http::header::COOKIE;
use http::{HeaderMap, Method, Request, Response};
use nve_rpc::{
    CborCodec, CodecFormat, Effect, FieldErrors, FormPairs, MaybeSend, MaybeSync, RpcCodec,
    RpcStatus, TransportError, CONTENT_TYPE_FORM_URLENCODED,
};

use crate::auth::AuthResolver;
use crate::config::{normalize_prefix, strip_rpc_prefix, FormRoute, ServerConfig};
use crate::csrf::CsrfPolicy;
use crate::registry::{Cx, FormHandlerOutcome, RpcRegistry, TagCollector};
use crate::response::{
    accepts_cbor, form_outcome_to_response, outcome_to_rpc_response, protocol_error_response,
    FormOutcome, PreflightError,
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
    entry_idx: usize,
    principal: U,
    headers: HeaderMap,
    kind: PreparedKind,
}

#[derive(Debug)]
enum PreparedKind {
    Rpc,
    Form { route_idx: usize },
}

struct ServerInner<S, U, A, C> {
    server_id: u64,
    state: S,
    registry: RpcRegistry<S, U, C>,
    auth: A,
    csrf: CsrfPolicy,
    config: ServerConfig,
    rpc_prefix_clean: String,
}

/// Sans-I/O RPC server handler (`Clone + Send + Sync + 'static` on native; `?Send` on `wasm32`).
///
/// Works identically inside `nve-http-hyper`, mounted in `axum::Router::nest_service` or
/// `route_service`, in-process via [`crate::DirectTransport`], or compiled to `wasm32-wasip2`.
pub struct Server<S, U, A, C = CborCodec> {
    inner: Arc<ServerInner<S, U, A, C>>,
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

        if parts.method != Method::POST {
            return Err(PreflightError::method_not_allowed(
                "RPC endpoints require HTTP POST",
            ));
        }

        let verdict = self
            .inner
            .csrf
            .check_content_type_and_origin(&parts.method, &parts.headers, C::CONTENT_TYPE)
            .map_err(|rej| PreflightError::protocol(rej.status, rej.reason))?;
        if verdict.bypassed_via_bearer {
            parts.headers.remove(COOKIE);
        }

        let Some(entry_idx) = self.inner.registry.lookup_index(proc_name) else {
            return Err(PreflightError::protocol(
                RpcStatus::NotFound,
                "unknown RPC procedure",
            ));
        };

        let principal = self.resolve_principal(&parts.headers).await?;
        Ok(PreparedRequest {
            server_id: self.inner.server_id,
            entry_idx,
            principal,
            headers: core::mem::take(&mut parts.headers),
            kind: PreparedKind::Rpc,
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
            entry_idx,
            principal,
            headers,
            kind: PreparedKind::Form { route_idx },
        })
    }

    /// Executes a request already validated by [`Self::preflight_request`] with its bounded `body`.
    pub async fn execute_prepared(
        &self,
        prepared: PreparedRequest<U>,
        body: Bytes,
    ) -> Response<Bytes>
    where
        C: CodecFormat + RpcCodec<FieldErrors>,
    {
        if prepared.server_id != self.inner.server_id {
            return protocol_error_response(
                RpcStatus::Internal,
                "PreparedRequest does not belong to this Server instance",
            );
        }
        if body.len() > self.inner.config.max_body_bytes {
            return protocol_error_response(
                RpcStatus::PayloadTooLarge,
                "request body exceeds configured max_body_bytes",
            );
        }

        match prepared.kind {
            PreparedKind::Rpc => {
                self.execute_rpc_entry(
                    prepared.entry_idx,
                    prepared.principal,
                    prepared.headers,
                    body,
                )
                .await
            }
            PreparedKind::Form { route_idx } => {
                let Some(route) = self.inner.config.form_routes.get(route_idx) else {
                    return protocol_error_response(
                        RpcStatus::Internal,
                        "invalid prepared form route index",
                    );
                };
                let wants_cbor = accepts_cbor(&prepared.headers, C::CONTENT_TYPE);
                let outcome = self
                    .execute_form_entry(
                        prepared.entry_idx,
                        prepared.principal,
                        prepared.headers,
                        body,
                        route.success_redirect(),
                    )
                    .await;
                form_outcome_to_response(self.inner.registry.codec(), outcome, wants_cbor)
            }
        }
    }

    /// Unified sans-I/O request handler dispatching both `/rpc/*` and registered `FormRoute`s.
    pub async fn handle_request(&self, req: Request<Bytes>) -> Response<Bytes>
    where
        C: CodecFormat + RpcCodec<FieldErrors>,
    {
        let (mut parts, body) = req.into_parts();
        let prepared = match self.preflight_request(&mut parts, false).await {
            Ok(p) => p,
            Err(err) => return err.into_response(),
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
            Err(err) => return err.into_response(),
        };
        if body.len() > self.inner.config.max_body_bytes {
            return protocol_error_response(
                RpcStatus::PayloadTooLarge,
                "request body exceeds configured max_body_bytes",
            );
        }
        self.execute_rpc_entry(
            prepared.entry_idx,
            prepared.principal,
            prepared.headers,
            body,
        )
        .await
    }

    /// Executes a progressive 0-JS HTML `<form method="post">` submission for a validated
    /// [`FormRoute`] and returns a typed [`FormOutcome`] (preserving submitted [`FormPairs`] +
    /// [`FieldErrors`] for SSR HTML re-rendering).
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

    pub(crate) async fn execute_trusted_direct(
        &self,
        proc_name: &str,
        content_type: &str,
        headers: HeaderMap,
        principal_override: Option<U>,
        allow_actions: bool,
        body: Bytes,
    ) -> Result<Response<Bytes>, TransportError>
    where
        C: CodecFormat,
    {
        if !content_type.eq_ignore_ascii_case(C::CONTENT_TYPE) {
            return Ok(protocol_error_response(
                RpcStatus::UnsupportedMediaType,
                "unsupported RPC Content-Type",
            ));
        }
        if body.len() > self.inner.config.max_body_bytes {
            return Ok(protocol_error_response(
                RpcStatus::PayloadTooLarge,
                "request body exceeds configured max_body_bytes",
            ));
        }
        let Some(entry_idx) = self.inner.registry.lookup_index(proc_name) else {
            return Ok(protocol_error_response(
                RpcStatus::NotFound,
                "unknown RPC procedure",
            ));
        };
        // Enforce Action guard against the registry's authoritative metadata rather than
        // trusting the caller-supplied `RawCall.effect`.
        if self.inner.registry.entry_at(entry_idx).effect == Effect::Action && !allow_actions {
            return Err(TransportError::new(
                "trusted DirectTransport blocks Action calls by default to prevent GET-triggered SSR mutations; use allow_trusted_actions(true) or DirectTransport::strict_http()",
            ));
        }
        let principal = match principal_override {
            Some(p) => p,
            None => match self.resolve_principal(&headers).await {
                Ok(p) => p,
                Err(err) => return Ok(err.into_response()),
            },
        };
        Ok(self
            .execute_rpc_entry(entry_idx, principal, headers, body)
            .await)
    }

    async fn resolve_principal(&self, headers: &HeaderMap) -> Result<U, PreflightError> {
        if let Some(res) = self.inner.auth.try_resolve_sync(&self.inner.state, headers) {
            return res.map_err(|err| PreflightError::protocol(err.status, err.message));
        }
        self.inner
            .auth
            .resolve(&self.inner.state, headers)
            .await
            .map_err(|err| PreflightError::protocol(err.status, err.message))
    }

    async fn execute_rpc_entry(
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
