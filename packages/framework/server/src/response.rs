//! HTTP response builders, `FormOutcome` / `PreflightError`, and shared reply-header parsing
//! for `nve-server`.

use std::borrow::Cow;

use bytes::Bytes;
use http::header::{ALLOW, CACHE_CONTROL, CONTENT_TYPE, LOCATION};
use http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode};
use nve_rpc::{
    classify_http_reply, encode_protocol_error_json, format_invalidate_sink, CodecError,
    CodecFormat, FieldErrors, FormPairs, RawReply, RpcStatus, TagSink,
    CONTENT_TYPE_ERROR_JSON, ERROR_KIND_DOMAIN, ERROR_KIND_FORM_VALIDATION, ERROR_KIND_PROTOCOL,
    HEADER_ERROR_KIND, HEADER_INVALIDATE,
};

use crate::registry::RpcHandlerOutcome;

const HDR_ERROR_KIND: HeaderName = HeaderName::from_static(HEADER_ERROR_KIND);
const HDR_INVALIDATE: HeaderName = HeaderName::from_static(HEADER_INVALIDATE);
const CONTENT_TYPE_TEXT_PLAIN_UTF8: &str = "text/plain; charset=utf-8";

/// Typed outcome returned by [`crate::Server::execute_form_action`] so SSR layers can re-render
/// HTML forms with submitted [`FormPairs`] and [`FieldErrors`] instead of emitting binary payloads.
#[derive(Debug)]
pub enum FormOutcome {
    Redirect {
        location: String,
        invalidated: TagSink,
    },
    ValidationFailed {
        errors: FieldErrors,
        form: FormPairs,
    },
    DomainError {
        status: RpcStatus,
        body: Bytes,
    },
    ProtocolError(Box<Response<Bytes>>),
}

/// Compact pre-body rejection returned by [`crate::Server::preflight_request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightError {
    pub status: RpcStatus,
    pub method_not_allowed: bool,
    pub(crate) allow_options: bool,
    pub(crate) cors_origin_idx: Option<usize>,
    pub message: Cow<'static, str>,
}

impl PreflightError {
    #[inline]
    pub(crate) fn protocol(status: RpcStatus, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            status,
            method_not_allowed: false,
            allow_options: false,
            cors_origin_idx: None,
            message: message.into(),
        }
    }

    #[inline]
    pub(crate) fn method_not_allowed(message: &'static str, allow_options: bool) -> Self {
        Self {
            status: RpcStatus::InvalidArgument,
            method_not_allowed: true,
            allow_options,
            cors_origin_idx: None,
            message: Cow::Borrowed(message),
        }
    }

    #[inline]
    pub(crate) fn with_cors_origin(mut self, origin_idx: Option<usize>) -> Self {
        self.cors_origin_idx = origin_idx;
        self
    }

    /// Converts this preflight rejection into a standard JSON protocol error HTTP response.
    pub fn into_response(self) -> Response<Bytes> {
        if self.method_not_allowed {
            method_not_allowed_response(&self.message, self.allow_options)
        } else {
            protocol_error_response(self.status, &self.message)
        }
    }
}

pub(crate) fn outcome_to_rpc_response<C: CodecFormat>(
    outcome: RpcHandlerOutcome,
) -> Response<Bytes> {
    match outcome {
        RpcHandlerOutcome::Ok { body, invalidated } => {
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, C::CONTENT_TYPE)
                .header(CACHE_CONTROL, "no-store");
            if let Some(hdr) = format_invalidate_sink(&invalidated) {
                builder = builder.header(&HDR_INVALIDATE, hdr);
            }
            builder.body(body).unwrap_or_else(|_| {
                protocol_error_response(RpcStatus::Internal, "failed to build RPC response")
            })
        }
        RpcHandlerOutcome::DomainErr { status, body } => {
            typed_error_response(status, C::CONTENT_TYPE, ERROR_KIND_DOMAIN, body)
        }
        RpcHandlerOutcome::BadInput(err @ CodecError::MaxBytesExceeded(_)) => {
            protocol_error_response(RpcStatus::PayloadTooLarge, &err.to_string())
        }
        RpcHandlerOutcome::BadInput(err) => {
            protocol_error_response(RpcStatus::InvalidArgument, &err.to_string())
        }
        RpcHandlerOutcome::EncodeFailure(err) => {
            protocol_error_response(RpcStatus::Internal, &err.to_string())
        }
    }
}

pub(crate) fn form_outcome_to_response(outcome: FormOutcome) -> Response<Bytes> {
    match outcome {
        FormOutcome::Redirect { location, .. } => Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(LOCATION, location)
            .header(CACHE_CONTROL, "no-store")
            .body(Bytes::new())
            .unwrap_or_else(|_| {
                protocol_error_response(RpcStatus::Internal, "failed to build redirect")
            }),
        FormOutcome::ValidationFailed { errors, .. } => Response::builder()
            .status(StatusCode::UNPROCESSABLE_ENTITY)
            .header(CONTENT_TYPE, CONTENT_TYPE_TEXT_PLAIN_UTF8)
            .header(&HDR_ERROR_KIND, ERROR_KIND_FORM_VALIDATION)
            .header(CACHE_CONTROL, "no-store")
            .body(Bytes::from(errors.to_string()))
            .unwrap_or_else(|_| {
                protocol_error_response(RpcStatus::Unprocessable, "validation failed")
            }),
        FormOutcome::DomainError { status, .. } => {
            let http_status = clamp_error_http_status(status);
            Response::builder()
                .status(http_status)
                .header(CONTENT_TYPE, CONTENT_TYPE_TEXT_PLAIN_UTF8)
                .header(&HDR_ERROR_KIND, ERROR_KIND_DOMAIN)
                .header(CACHE_CONTROL, "no-store")
                .body(Bytes::from_static(status.code_str().as_bytes()))
                .unwrap_or_else(|_| protocol_error_response(status, status.code_str()))
        }
        FormOutcome::ProtocolError(resp) => *resp,
    }
}

fn clamp_error_http_status(status: RpcStatus) -> StatusCode {
    let raw = status.http_status();
    if (400..600).contains(&raw) {
        StatusCode::from_u16(raw).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY)
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    }
}

fn typed_error_response(
    status: RpcStatus,
    content_type: &'static str,
    error_kind: &'static str,
    body: Bytes,
) -> Response<Bytes> {
    let http_status = clamp_error_http_status(status);
    Response::builder()
        .status(http_status)
        .header(CONTENT_TYPE, content_type)
        .header(&HDR_ERROR_KIND, error_kind)
        .header(CACHE_CONTROL, "no-store")
        .body(body)
        .unwrap_or_else(|_| {
            protocol_error_response(RpcStatus::Internal, "failed to build domain error response")
        })
}

fn method_not_allowed_response(message: &str, allow_options: bool) -> Response<Bytes> {
    let mut resp = protocol_error_response(RpcStatus::InvalidArgument, message);
    *resp.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
    let allow_val = if allow_options {
        HeaderValue::from_static("POST, OPTIONS")
    } else {
        HeaderValue::from_static("POST")
    };
    resp.headers_mut().insert(ALLOW, allow_val);
    resp
}

/// Builds a Connect-compatible JSON protocol error HTTP response without `serde_json`.
/// Clamps [`RpcStatus::Ok`] to [`RpcStatus::Internal`] (`500`) so protocol errors can never
/// emit HTTP `200 OK`.
pub fn protocol_error_response(status: RpcStatus, message: &str) -> Response<Bytes> {
    let safe_status = if status == RpcStatus::Ok {
        RpcStatus::Internal
    } else {
        status
    };
    let http_status =
        StatusCode::from_u16(safe_status.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = Bytes::from(encode_protocol_error_json(safe_status, message));
    Response::builder()
        .status(http_status)
        .header(CONTENT_TYPE, CONTENT_TYPE_ERROR_JSON)
        .header(&HDR_ERROR_KIND, ERROR_KIND_PROTOCOL)
        .header(CACHE_CONTROL, "no-store")
        .body(body)
        .expect("static headers and valid status code build cleanly")
}

/// Shared HTTP reply metadata parser used by both [`crate::DirectTransport`] and HTTP client
/// transports (e.g. `nve-http-hyper::HyperTransport`).
///
/// Any malformed or overflowing `Nve-Invalidate` header safely collapses to `invalidate_all = true`
/// rather than failing the call, preventing clients from retrying an already-committed mutation.
pub fn parse_rpc_http_reply(status: StatusCode, headers: &HeaderMap, body: Bytes) -> RawReply {
    let error_kind = headers.get(&HDR_ERROR_KIND).map(HeaderValue::as_bytes);
    let invalidate = headers.get_all(&HDR_INVALIDATE).iter().map(HeaderValue::as_bytes);
    classify_http_reply(status.as_u16(), error_kind, invalidate, body)
}
