#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod cbor;
pub mod form;
pub mod procedure;
pub mod status;
pub mod tag;
pub mod transport;

pub use bytes::Bytes;
pub use cbor::{validate_cbor, CborCodec, CborLimits, CodecFormat, RpcCodec, CONTENT_TYPE_CBOR};
pub use form::{
    decode_form_urlencoded_limited, FieldError, FieldErrors, FormDecode, FormPairs,
    CONTENT_TYPE_FORM_URLENCODED, DEFAULT_MAX_FORM_FIELDS,
};
pub use procedure::{
    assert_valid_procedure_name, is_valid_procedure_name, Action, Effect, EffectKind, ProcId,
    Procedure, Query, PROC_ID_SEED,
};
pub use status::{
    decode_protocol_error_json, encode_protocol_error_json, CodecError, DomainStatus, RpcError,
    RpcStatus, TransportError, CONTENT_TYPE_ERROR_JSON, ERROR_KIND_DOMAIN,
    ERROR_KIND_FORM_VALIDATION, ERROR_KIND_PROTOCOL, HEADER_ERROR_KIND, STATUS_TABLE,
};
pub use tag::{
    format_invalidate_header, format_invalidate_header_into, format_invalidate_sink,
    parse_invalidate_header, CacheTag, InvalidateParse, Tag64, TagSink, HEADER_INVALIDATE,
    INVALIDATE_ALL_WILDCARD, MAX_INVALIDATE_HEADER_SEGMENTS, MAX_INVALIDATE_HEADER_TAGS, TAG_SEED,
};
pub use transport::{
    call, call_with_codec, format_rpc_path, format_rpc_path_into, BoxFuture, CallReply, MaybeSend,
    MaybeSync, RawCall, RawReply, Transport, DEFAULT_RPC_PREFIX,
};
