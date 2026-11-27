//! HTTP response builders, `FormOutcome` / `PreflightError`, and shared reply-header parsing
//! for `nve-server`.

use std::borrow::Cow;

use bytes::Bytes;
use http::header::{ACCEPT, ALLOW, CACHE_CONTROL, CONTENT_TYPE, LOCATION};
use http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode};
use nve_rpc::{
    encode_protocol_error_json, format_invalidate_sink, parse_invalidate_header, CodecError,
    CodecFormat, FieldErrors, FormPairs, InvalidateParse, RawReply, RpcCodec, RpcStatus, TagSink,
    TransportError, CONTENT_TYPE_ERROR_JSON, ERROR_KIND_DOMAIN, ERROR_KIND_FORM_VALIDATION,
    ERROR_KIND_PROTOCOL, HEADER_ERROR_KIND, HEADER_INVALIDATE,
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

/// Compact pre-body rejection returned by [`crate::Server::preflight_request`] (32 bytes,
/// avoiding inflating happy-path async state machines with a 144-byte `http::Response`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightError {
    pub status: RpcStatus,
    pub method_not_allowed: bool,
    pub message: Cow<'static, str>,
}

impl PreflightError {
    #[inline]
    pub(crate) fn protocol(status: RpcStatus, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            status,
            method_not_allowed: false,
            message: message.into(),
        }
    }

    #[inline]
    pub(crate) fn method_not_allowed(message: &'static str) -> Self {
        Self {
            status: RpcStatus::InvalidArgument,
            method_not_allowed: true,
            message: Cow::Borrowed(message),
        }
    }

    /// Converts this preflight rejection into a standard JSON protocol error HTTP response.
    pub fn into_response(self) -> Response<Bytes> {
        if self.method_not_allowed {
            method_not_allowed_response(&self.message)
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

pub(crate) fn form_outcome_to_response<C>(
    codec: &C,
    outcome: FormOutcome,
    wants_cbor: bool,
) -> Response<Bytes>
where
    C: CodecFormat + RpcCodec<FieldErrors>,
{
    match outcome {
        FormOutcome::Redirect {
            location,
            invalidated,
        } => {
            if wants_cbor {
                // Script-enhanced callers (`Accept: application/cbor`) receive `204 No Content`
                // with `Location` and `Nve-Invalidate` so browser `fetch` can read the headers
                // without opaque-redirect stripping.
                let mut builder = Response::builder()
                    .status(StatusCode::NO_CONTENT)
                    .header(LOCATION, location)
                    .header(CACHE_CONTROL, "no-store");
                if let Some(hdr) = format_invalidate_sink(&invalidated) {
                    builder = builder.header(&HDR_INVALIDATE, hdr);
                }
                builder.body(Bytes::new()).unwrap_or_else(|_| {
                    protocol_error_response(RpcStatus::Internal, "failed to build form response")
                })
            } else {
                Response::builder()
                    .status(StatusCode::SEE_OTHER)
                    .header(LOCATION, location)
                    .header(CACHE_CONTROL, "no-store")
                    .body(Bytes::new())
                    .unwrap_or_else(|_| {
                        protocol_error_response(RpcStatus::Internal, "failed to build redirect")
                    })
            }
        }
        FormOutcome::ValidationFailed { errors, .. } => {
            if wants_cbor {
                let mut buf = Vec::with_capacity(64);
                match codec.encode_into(&errors, &mut buf) {
                    Ok(()) => typed_error_response(
                        RpcStatus::Unprocessable,
                        C::CONTENT_TYPE,
                        ERROR_KIND_FORM_VALIDATION,
                        Bytes::from(buf),
                    ),
                    Err(_) => {
                        protocol_error_response(RpcStatus::Unprocessable, &errors.to_string())
                    }
                }
            } else {
                Response::builder()
                    .status(StatusCode::UNPROCESSABLE_ENTITY)
                    .header(CONTENT_TYPE, CONTENT_TYPE_TEXT_PLAIN_UTF8)
                    .header(&HDR_ERROR_KIND, ERROR_KIND_FORM_VALIDATION)
                    .header(CACHE_CONTROL, "no-store")
                    .body(Bytes::from(errors.to_string()))
                    .unwrap_or_else(|_| {
                        protocol_error_response(RpcStatus::Unprocessable, "validation failed")
                    })
            }
        }
        FormOutcome::DomainError { status, body } => {
            if wants_cbor {
                typed_error_response(status, C::CONTENT_TYPE, ERROR_KIND_DOMAIN, body)
            } else {
                let http_status = clamp_error_http_status(status);
                Response::builder()
                    .status(http_status)
                    .header(CONTENT_TYPE, CONTENT_TYPE_TEXT_PLAIN_UTF8)
                    .header(&HDR_ERROR_KIND, ERROR_KIND_DOMAIN)
                    .header(CACHE_CONTROL, "no-store")
                    .body(Bytes::from_static(status.code_str().as_bytes()))
                    .unwrap_or_else(|_| protocol_error_response(status, status.code_str()))
            }
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

fn method_not_allowed_response(message: &str) -> Response<Bytes> {
    let mut resp = protocol_error_response(RpcStatus::InvalidArgument, message);
    *resp.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
    resp.headers_mut()
        .insert(ALLOW, HeaderValue::from_static("POST"));
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
pub fn parse_rpc_http_reply(
    status: StatusCode,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<RawReply, TransportError> {
    let error_kind = headers.get(&HDR_ERROR_KIND).and_then(|v| v.to_str().ok());
    let is_domain_error = error_kind.is_some_and(|v| v.eq_ignore_ascii_case(ERROR_KIND_DOMAIN));
    let is_protocol_error = error_kind.is_some_and(|v| v.eq_ignore_ascii_case(ERROR_KIND_PROTOCOL));
    let mut invalidated_tags = Vec::new();
    let mut invalidate_all = false;
    for hdr in headers.get_all(&HDR_INVALIDATE) {
        let Ok(raw) = hdr.to_str() else {
            invalidate_all = true;
            invalidated_tags.clear();
            break;
        };
        match parse_invalidate_header(raw, &mut invalidated_tags) {
            Ok(InvalidateParse::Tags) => {}
            Ok(InvalidateParse::All) | Err(_) => {
                invalidate_all = true;
                invalidated_tags.clear();
                break;
            }
        }
    }
    Ok(RawReply {
        http_status: status.as_u16(),
        is_domain_error,
        is_protocol_error,
        invalidated_tags,
        invalidate_all,
        body,
    })
}

pub(crate) fn accepts_cbor(headers: &HeaderMap, codec_content_type: &str) -> bool {
    for val in headers.get_all(ACCEPT) {
        let Ok(raw) = val.to_str() else {
            continue;
        };
        for item in raw.split(',') {
            let mut parts = item.split(';');
            let essence = parts.next().unwrap_or("").trim();
            if !essence.eq_ignore_ascii_case(codec_content_type) {
                continue;
            }
            let rejected_by_q = parts.any(|param| {
                let Some((k, v)) = param.split_once('=') else {
                    return false;
                };
                k.trim().eq_ignore_ascii_case("q")
                    && v.trim().parse::<f32>().is_ok_and(|q| q <= 0.0)
            });
            if !rejected_by_q {
                return true;
            }
        }
    }
    false
}
