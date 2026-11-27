//! Transport-agnostic bytes-only RPC seam (`Transport`, `call`) with `MaybeSend`/`MaybeSync`
//! portability across multi-threaded Tokio and single-threaded `wasm32` / `wasm32-wasip2`.

use core::future::Future;
use core::pin::Pin;

use bytes::Bytes;

use crate::cbor::{CborCodec, RpcCodec};
use crate::procedure::{Effect, EffectKind, ProcId, Procedure};
use crate::status::{
    decode_protocol_error_json, CodecError, RpcError, RpcStatus, TransportError, ERROR_KIND_DOMAIN,
    ERROR_KIND_FORM_VALIDATION, ERROR_KIND_PROTOCOL,
};
use crate::tag::{parse_invalidate_header, InvalidateParse, Tag64};

/// Default RPC HTTP path prefix (`POST /rpc/<Procedure::NAME>`).
pub const DEFAULT_RPC_PREFIX: &str = "/rpc";

/// `Send` on native targets, unconstrained on `wasm32` (browser `JsFuture` / single-threaded WASI).
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}

/// `Send` on native targets, unconstrained on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// `Sync` on native targets, unconstrained on `wasm32`.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync + ?Sized> MaybeSync for T {}

/// `Sync` on native targets, unconstrained on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSync for T {}

/// Boxed future bounded by `Send` on native and `?Send` on `wasm32`.
#[cfg(not(target_arch = "wasm32"))]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Boxed future bounded by `Send` on native and `?Send` on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Wire-level call descriptor passed to a [`Transport`].
///
/// Procedure names are always compile-time constants (`P::NAME: &'static str`), allowing
/// transports and query engines to spawn `'static` async tasks without cloning route names.
#[derive(Debug, Clone)]
pub struct RawCall {
    pub proc_id: ProcId,
    pub name: &'static str,
    pub effect: Effect,
    pub content_type: &'static str,
    pub body: Bytes,
}

/// Wire classification derived from `http_status` and the `Nve-Error-Kind` response header.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReplyKind {
    #[default]
    Ok = 0,
    Domain = 1,
    FormValidation = 2,
    Protocol = 3,
}

/// Wire-level reply returned by a [`Transport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawReply {
    pub http_status: u16,
    pub kind: ReplyKind,
    pub invalidated_tags: Vec<Tag64>,
    /// True when the server emitted `Nve-Invalidate: *` (or overflowed/malformed the header),
    /// instructing client caches to invalidate all entries rather than dropping tags silently.
    pub invalidate_all: bool,
    pub body: Bytes,
}

/// Decoded typed RPC reply carrying both the value and server-reported cache invalidations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallReply<T> {
    pub value: T,
    pub invalidated_tags: Vec<Tag64>,
    pub invalidate_all: bool,
    pub raw_body: Bytes,
}

/// Bytes-only RPC transport seam implemented by HTTP clients (`nve-http-hyper`, `nve-http-web`),
/// in-process `DirectTransport` (SSR/tests), and future `nve-wire` adapters.
pub trait Transport: MaybeSend + MaybeSync + 'static {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>>;
}

/// Classifies an HTTP RPC response from raw status + header bytes with zero `http` crate
/// dependency — shared by `nve-server`, `nve-http-hyper`, and `nve-http-web` (`FetchTransport`).
///
/// Any malformed, non-UTF-8, or overflowing `Nve-Invalidate` header safely collapses to
/// `invalidate_all = true` so clients never fail or retry an already-committed server mutation.
pub fn classify_http_reply<'h, I>(
    http_status: u16,
    error_kind: Option<&[u8]>,
    invalidate_headers: I,
    body: Bytes,
) -> RawReply
where
    I: IntoIterator<Item = &'h [u8]>,
{
    let kind = match error_kind {
        Some(raw) if raw.trim_ascii().eq_ignore_ascii_case(ERROR_KIND_DOMAIN.as_bytes()) => {
            ReplyKind::Domain
        }
        Some(raw)
            if raw
                .trim_ascii()
                .eq_ignore_ascii_case(ERROR_KIND_FORM_VALIDATION.as_bytes()) =>
        {
            ReplyKind::FormValidation
        }
        Some(raw) if raw.trim_ascii().eq_ignore_ascii_case(ERROR_KIND_PROTOCOL.as_bytes()) => {
            ReplyKind::Protocol
        }
        _ if (200..300).contains(&http_status) => ReplyKind::Ok,
        _ => ReplyKind::Protocol,
    };

    let mut invalidated_tags = Vec::new();
    let mut invalidate_all = false;
    for raw in invalidate_headers {
        let Ok(header_str) = core::str::from_utf8(raw) else {
            invalidate_all = true;
            invalidated_tags.clear();
            break;
        };
        match parse_invalidate_header(header_str, &mut invalidated_tags) {
            Ok(InvalidateParse::Tags) => {}
            Ok(InvalidateParse::All) | Err(_) => {
                invalidate_all = true;
                invalidated_tags.clear();
                break;
            }
        }
    }

    RawReply {
        http_status,
        kind,
        invalidated_tags,
        invalidate_all,
        body,
    }
}

/// Appends the canonical HTTP path for `proc_name` under `prefix` into `out` without extra
/// intermediate allocations.
pub fn format_rpc_path_into(prefix: &str, proc_name: &str, out: &mut String) {
    let clean_prefix = prefix.trim().trim_end_matches('/');
    let clean_name = proc_name.trim_start_matches('/');
    if !clean_prefix.is_empty() {
        if !clean_prefix.starts_with('/') {
            out.push('/');
        }
        out.push_str(clean_prefix);
    }
    out.push('/');
    out.push_str(clean_name);
}

/// Formats the canonical HTTP path for `proc_name` under `prefix` (e.g. `"/rpc/issues.v1.Issues/List"`).
pub fn format_rpc_path(prefix: &str, proc_name: &str) -> String {
    let mut out = String::with_capacity(prefix.len() + proc_name.len() + 2);
    format_rpc_path_into(prefix, proc_name, &mut out);
    out
}

/// Invokes procedure `P` over `transport` using the default [`CborCodec`].
pub async fn call<P>(
    transport: &(impl Transport + ?Sized),
    args: &P::Args,
) -> Result<CallReply<P::Output>, RpcError<P::Error>>
where
    P: Procedure,
    CborCodec: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
{
    call_with_codec::<P, CborCodec>(transport, &CborCodec::default(), args).await
}

/// Encodes `args` for procedure `P` using `codec`, verifying `P::NAME` and `P::ID` at compile time.
pub fn encode_args<P, C>(codec: &C, args: &P::Args) -> Result<Bytes, CodecError>
where
    P: Procedure,
    C: RpcCodec<P::Args>,
{
    const {
        crate::procedure::assert_valid_procedure_name(P::NAME);
        assert!(
            P::ID.raw() == ProcId::of(P::NAME).raw(),
            "Procedure::ID must match ProcId::of(Procedure::NAME)"
        );
    }

    let mut buf = Vec::with_capacity(64);
    codec.encode_into(args, &mut buf)?;
    Ok(Bytes::from(buf))
}

/// Decodes a [`RawReply`] for procedure `P` using `codec`.
pub fn decode_reply<P, C>(
    codec: &C,
    reply: RawReply,
) -> Result<CallReply<P::Output>, RpcError<P::Error>>
where
    P: Procedure,
    C: RpcCodec<P::Output> + RpcCodec<P::Error>,
{
    match reply.kind {
        // Check `Domain` BEFORE `(200..300)` so a misconfigured server returning HTTP 200
        // with `Nve-Error-Kind: domain` can never decode `P::Error` bytes as `P::Output`.
        ReplyKind::Domain => {
            let error = codec.decode_bounded(&reply.body)?;
            let status = if (200..300).contains(&reply.http_status) {
                RpcStatus::Unprocessable
            } else {
                RpcStatus::from_http_status(reply.http_status)
            };
            Err(RpcError::Domain { status, error })
        }
        ReplyKind::Protocol => {
            let (status, message) = decode_protocol_error_json(reply.http_status, &reply.body);
            let status = if status == RpcStatus::Ok {
                RpcStatus::Internal
            } else {
                status
            };
            Err(RpcError::Protocol { status, message })
        }
        ReplyKind::FormValidation => {
            let message = core::str::from_utf8(&reply.body)
                .ok()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("form validation failed")
                .to_owned();
            Err(RpcError::Protocol {
                status: RpcStatus::Unprocessable,
                message,
            })
        }
        ReplyKind::Ok => {
            if (200..300).contains(&reply.http_status) {
                let value = codec.decode_bounded(&reply.body)?;
                Ok(CallReply {
                    value,
                    invalidated_tags: reply.invalidated_tags,
                    invalidate_all: reply.invalidate_all,
                    raw_body: reply.body,
                })
            } else {
                let (status, message) = decode_protocol_error_json(reply.http_status, &reply.body);
                Err(RpcError::Protocol { status, message })
            }
        }
    }
}

/// Invokes procedure `P` over `transport` using any pluggable [`RpcCodec`] `C`.
pub async fn call_with_codec<P, C>(
    transport: &(impl Transport + ?Sized),
    codec: &C,
    args: &P::Args,
) -> Result<CallReply<P::Output>, RpcError<P::Error>>
where
    P: Procedure,
    C: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
{
    let body = encode_args::<P, C>(codec, args)?;
    let reply = transport
        .call_raw(RawCall {
            proc_id: P::ID,
            name: P::NAME,
            effect: P::Effect::EFFECT,
            content_type: C::CONTENT_TYPE,
            body,
        })
        .await?;
    decode_reply::<P, C>(codec, reply)
}
