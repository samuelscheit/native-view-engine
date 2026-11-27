//! Transport-agnostic bytes-only RPC seam (`Transport`, `call`) with `MaybeSend`/`MaybeSync`
//! portability across multi-threaded Tokio and single-threaded `wasm32` / `wasm32-wasip2`.

use core::future::Future;
use core::pin::Pin;

use bytes::Bytes;

use crate::cbor::{CborCodec, RpcCodec};
use crate::procedure::{Effect, EffectKind, ProcId, Procedure};
use crate::status::{decode_protocol_error_json, RpcError, RpcStatus, TransportError};
use crate::tag::Tag64;

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
#[derive(Debug, Clone)]
pub struct RawCall<'a> {
    pub proc_id: ProcId,
    pub name: &'a str,
    pub effect: Effect,
    pub content_type: &'static str,
    pub body: Bytes,
}

/// Wire-level reply returned by a [`Transport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawReply {
    pub http_status: u16,
    pub is_domain_error: bool,
    pub is_protocol_error: bool,
    pub invalidated_tags: Vec<Tag64>,
    /// True when the server emitted `Nve-Invalidate: *` (or overflowed the explicit tag cap),
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

/// Bytes-only RPC transport seam implemented by HTTP clients (`nve-http-hyper`, browser `fetch`),
/// in-process `DirectTransport` (SSR/tests), and future `nve-wire` adapters.
pub trait Transport: MaybeSend + MaybeSync + 'static {
    fn call_raw<'a>(&'a self, req: RawCall<'a>) -> BoxFuture<'a, Result<RawReply, TransportError>>;
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
    const {
        crate::procedure::assert_valid_procedure_name(P::NAME);
        assert!(
            P::ID.raw() == ProcId::of(P::NAME).raw(),
            "Procedure::ID must match ProcId::of(Procedure::NAME)"
        );
    }

    let mut buf = Vec::with_capacity(64);
    codec.encode_into(args, &mut buf)?;

    let reply = transport
        .call_raw(RawCall {
            proc_id: P::ID,
            name: P::NAME,
            effect: P::Effect::EFFECT,
            content_type: C::CONTENT_TYPE,
            body: Bytes::from(buf),
        })
        .await?;

    // Check `is_domain_error` BEFORE `(200..300)` so a misconfigured server returning HTTP 200
    // with `Nve-Error-Kind: domain` can never decode `P::Error` bytes as `P::Output`.
    if reply.is_domain_error {
        let error = codec.decode_bounded(&reply.body)?;
        let status = if (200..300).contains(&reply.http_status) {
            RpcStatus::Unprocessable
        } else {
            RpcStatus::from_http_status(reply.http_status)
        };
        return Err(RpcError::Domain { status, error });
    }

    if reply.is_protocol_error {
        let (status, message) = decode_protocol_error_json(reply.http_status, &reply.body);
        let status = if status == RpcStatus::Ok {
            RpcStatus::Internal
        } else {
            status
        };
        return Err(RpcError::Protocol { status, message });
    }

    if (200..300).contains(&reply.http_status) {
        let value = codec.decode_bounded(&reply.body)?;
        return Ok(CallReply {
            value,
            invalidated_tags: reply.invalidated_tags,
            invalidate_all: reply.invalidate_all,
            raw_body: reply.body,
        });
    }

    let (status, message) = decode_protocol_error_json(reply.http_status, &reply.body);
    Err(RpcError::Protocol { status, message })
}
