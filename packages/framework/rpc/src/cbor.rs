//! Pluggable [`RpcCodec`] seam and default [`CborCodec`] (`minicbor`, `application/cbor`) with
//! a zero-allocation iterative RFC 8949 byte-slice validator guarding untrusted ingress.

use minicbor::{Decode, Decoder, Encode, Encoder};

use crate::status::CodecError;
use crate::transport::{MaybeSend, MaybeSync};

/// Standard RFC 8949 media type for CBOR RPC payloads.
pub const CONTENT_TYPE_CBOR: &str = "application/cbor";

/// Hard upper bound for stack-allocated container depth in [`validate_cbor`].
const MAX_STACK_DEPTH: usize = 128;

/// Maximum counter representable inside a packed 64-bit stack frame (`2^61 - 1`).
const MAX_FRAME_COUNT: u64 = (1u64 << 61) - 1;

const KIND_DEFINITE: u64 = 0 << 61;
const KIND_INDEF_ARRAY: u64 = 1 << 61;
const KIND_INDEF_MAP_KEY: u64 = 2 << 61;
const KIND_INDEF_MAP_VAL: u64 = 3 << 61;
const KIND_MASK: u64 = 3 << 61;
const COUNT_MASK: u64 = MAX_FRAME_COUNT;

/// Ingress resource limits enforced before `minicbor::decode` allocates any container or string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CborLimits {
    /// Maximum raw CBOR payload size in bytes.
    pub max_bytes: usize,
    /// Maximum container / tag nesting depth (capped at 128 on stack).
    pub max_depth: usize,
    /// Maximum declared or indefinite length of any single array or map.
    pub max_collection_len: usize,
    /// Maximum total CBOR data items across the entire payload.
    pub max_items: usize,
}

impl CborLimits {
    /// Default CBOR codec bounds (aligned with client `4 MiB` response cap; server ingress
    /// `ServerConfig::max_body_bytes` enforces the stricter default `1 MiB` request cap).
    pub const DEFAULT: Self = Self {
        max_bytes: 4 * 1024 * 1024,
        max_depth: 64,
        max_collection_len: 65_536,
        max_items: 262_144,
    };
}

impl Default for CborLimits {
    #[inline]
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Media-type identity for an [`RpcCodec`] implementation (non-generic so `C::CONTENT_TYPE`
/// resolves without turbofish or dummy `RpcCodec<()>` bounds).
pub trait CodecFormat: Clone + MaybeSend + MaybeSync + 'static {
    const CONTENT_TYPE: &'static str;
}

/// Pluggable binary payload codec trait (`CborCodec` by default; `prost`/`buffa` plug in cleanly).
pub trait RpcCodec<T>: CodecFormat {
    /// Encodes `value` into `buf` without clearing existing capacity (caller clears `buf` to reuse).
    fn encode_into(&self, value: &T, buf: &mut Vec<u8>) -> Result<(), CodecError>;

    /// Validates ingress bounds and decodes `bytes` into `T`.
    fn decode_bounded(&self, bytes: &[u8]) -> Result<T, CodecError>;
}

/// Default schema-evolvable CBOR payload codec backed by `minicbor` (`application/cbor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CborCodec {
    pub limits: CborLimits,
}

impl CborCodec {
    #[inline]
    pub const fn new(limits: CborLimits) -> Self {
        Self { limits }
    }
}

impl CodecFormat for CborCodec {
    const CONTENT_TYPE: &'static str = CONTENT_TYPE_CBOR;
}

impl<T> RpcCodec<T> for CborCodec
where
    T: Encode<()> + for<'b> Decode<'b, ()>,
{
    #[inline]
    fn encode_into(&self, value: &T, buf: &mut Vec<u8>) -> Result<(), CodecError> {
        let mut encoder = Encoder::new(buf);
        value
            .encode(&mut encoder, &mut ())
            .map_err(|err| CodecError::Encode(err.to_string()))
    }

    #[inline]
    fn decode_bounded(&self, bytes: &[u8]) -> Result<T, CodecError> {
        validate_cbor(bytes, self.limits)?;
        let mut decoder = Decoder::new(bytes);
        let value = decoder
            .decode::<T>()
            .map_err(|err| CodecError::Decode(err.to_string()))?;
        if decoder.position() != bytes.len() {
            return Err(CodecError::TrailingBytes(bytes.len() - decoder.position()));
        }
        Ok(value)
    }
}

/// Iterative, zero-allocation RFC 8949 CBOR ingress validator enforcing payload size, nesting
/// depth (including empty containers and CBOR tags), definite & indefinite collection length,
/// total item count, indefinite-container break parity, UTF-8 validity, and exact EOF before
/// `minicbor::decode` executes.
pub fn validate_cbor(bytes: &[u8], limits: CborLimits) -> Result<(), CodecError> {
    if bytes.len() > limits.max_bytes {
        return Err(CodecError::MaxBytesExceeded(limits.max_bytes));
    }
    if bytes.is_empty() {
        return Err(CodecError::Decode("empty CBOR payload".to_owned()));
    }

    let max_depth = limits.max_depth.min(MAX_STACK_DEPTH);
    if max_depth <= 32 {
        validate_cbor_stack::<32>(bytes, limits, max_depth)
    } else if max_depth <= 64 {
        validate_cbor_stack::<64>(bytes, limits, max_depth)
    } else {
        validate_cbor_stack::<MAX_STACK_DEPTH>(bytes, limits, max_depth)
    }
}

fn validate_cbor_stack<const N: usize>(
    bytes: &[u8],
    limits: CborLimits,
    max_depth: usize,
) -> Result<(), CodecError> {
    let mut stack = [0u64; N];
    let mut depth = 0usize;
    let mut total_items = 0usize;
    let mut root_complete = false;
    let mut pos = 0usize;
    let len = bytes.len();

    while !(depth == 0 && root_complete) {
        if pos >= len {
            if depth > 0 && (stack[depth - 1] & KIND_MASK) != KIND_DEFINITE {
                return Err(CodecError::UnclosedIndefinite);
            }
            return Err(CodecError::Decode("unexpected end of CBOR input".to_owned()));
        }

        let ib = bytes[pos];
        pos += 1;

        if ib == 0xff {
            if depth == 0 {
                return Err(CodecError::UnexpectedBreak);
            }
            match stack[depth - 1] & KIND_MASK {
                KIND_INDEF_ARRAY | KIND_INDEF_MAP_KEY => {
                    depth -= 1;
                    advance_parent(&mut stack, &mut depth, &mut root_complete);
                }
                KIND_INDEF_MAP_VAL => {
                    return Err(CodecError::Decode(
                        "unpaired key in indefinite-length CBOR map".to_owned(),
                    ));
                }
                _ => return Err(CodecError::UnexpectedBreak),
            }
            continue;
        }

        if depth > 0 {
            let top = stack[depth - 1];
            let kind = top & KIND_MASK;
            if kind == KIND_INDEF_ARRAY || kind == KIND_INDEF_MAP_KEY {
                let count = (top & COUNT_MASK) + 1;
                if count > limits.max_collection_len as u64 {
                    return Err(CodecError::MaxCollectionLenExceeded(
                        limits.max_collection_len,
                    ));
                }
                stack[depth - 1] = kind | count;
            }
        }

        total_items += 1;
        if total_items > limits.max_items {
            return Err(CodecError::MaxItemsExceeded(limits.max_items));
        }

        let major = ib >> 5;
        let ai = ib & 0x1f;

        match major {
            // Unsigned / negative integers.
            0 | 1 => {
                skip_arg_bytes(len, &mut pos, ai)?;
                advance_parent(&mut stack, &mut depth, &mut root_complete);
            }
            // Byte string (2) / UTF-8 text string (3).
            2 | 3 => {
                if ai == 31 {
                    loop {
                        if pos >= len {
                            return Err(CodecError::UnclosedIndefinite);
                        }
                        let cb = bytes[pos];
                        pos += 1;
                        if cb == 0xff {
                            break;
                        }
                        let chunk_major = cb >> 5;
                        let chunk_ai = cb & 0x1f;
                        if chunk_major != major || chunk_ai == 31 {
                            return Err(CodecError::Decode(
                                "invalid chunk inside indefinite CBOR string".to_owned(),
                            ));
                        }
                        let chunk_len = read_len_arg(bytes, &mut pos, chunk_ai)?;
                        if len - pos < chunk_len {
                            return Err(CodecError::Decode(
                                "truncated indefinite CBOR string chunk".to_owned(),
                            ));
                        }
                        if major == 3 && core::str::from_utf8(&bytes[pos..pos + chunk_len]).is_err()
                        {
                            return Err(CodecError::Decode(
                                "invalid UTF-8 in indefinite CBOR text string".to_owned(),
                            ));
                        }
                        pos += chunk_len;
                        total_items += 1;
                        if total_items > limits.max_items {
                            return Err(CodecError::MaxItemsExceeded(limits.max_items));
                        }
                    }
                } else {
                    let str_len = read_len_arg(bytes, &mut pos, ai)?;
                    if len - pos < str_len {
                        return Err(CodecError::Decode("truncated CBOR string".to_owned()));
                    }
                    if major == 3 && core::str::from_utf8(&bytes[pos..pos + str_len]).is_err() {
                        return Err(CodecError::Decode(
                            "invalid UTF-8 in CBOR text string".to_owned(),
                        ));
                    }
                    pos += str_len;
                }
                advance_parent(&mut stack, &mut depth, &mut root_complete);
            }
            // Array (definite or indefinite).
            4 => {
                if depth >= max_depth {
                    return Err(CodecError::MaxDepthExceeded(max_depth));
                }
                if ai == 31 {
                    stack[depth] = KIND_INDEF_ARRAY;
                    depth += 1;
                } else {
                    let count = read_u64_arg(bytes, &mut pos, ai)?;
                    let count = check_collection_len(count, limits.max_collection_len)?;
                    if count == 0 {
                        advance_parent(&mut stack, &mut depth, &mut root_complete);
                    } else {
                        stack[depth] = KIND_DEFINITE | count;
                        depth += 1;
                    }
                }
            }
            // Map (definite or indefinite).
            5 => {
                if depth >= max_depth {
                    return Err(CodecError::MaxDepthExceeded(max_depth));
                }
                if ai == 31 {
                    stack[depth] = KIND_INDEF_MAP_KEY;
                    depth += 1;
                } else {
                    let pairs = read_u64_arg(bytes, &mut pos, ai)?;
                    let pairs = check_collection_len(pairs, limits.max_collection_len)?;
                    let remaining = pairs
                        .checked_mul(2)
                        .filter(|&n| n <= MAX_FRAME_COUNT)
                        .ok_or(CodecError::MaxCollectionLenExceeded(
                            limits.max_collection_len,
                        ))?;
                    if remaining == 0 {
                        advance_parent(&mut stack, &mut depth, &mut root_complete);
                    } else {
                        stack[depth] = KIND_DEFINITE | remaining;
                        depth += 1;
                    }
                }
            }
            // Semantic tag (always wraps exactly 1 nested item).
            6 => {
                if depth >= max_depth {
                    return Err(CodecError::MaxDepthExceeded(max_depth));
                }
                skip_arg_bytes(len, &mut pos, ai)?;
                stack[depth] = KIND_DEFINITE | 1;
                depth += 1;
            }
            // Simple values and floats (`ai == 31` break handled above).
            7 => {
                if ai == 24 {
                    if pos >= len {
                        return Err(CodecError::Decode("unexpected end of CBOR input".to_owned()));
                    }
                    // RFC 8949 §3.3: 2-byte simple values 0xf8 0x00..=0x1f are ill-formed.
                    if bytes[pos] < 32 {
                        return Err(CodecError::Decode(
                            "ill-formed 2-byte CBOR simple value (< 32)".to_owned(),
                        ));
                    }
                    pos += 1;
                } else {
                    skip_arg_bytes(len, &mut pos, ai)?;
                }
                advance_parent(&mut stack, &mut depth, &mut root_complete);
            }
            _ => unreachable!("3-bit CBOR major type"),
        }
    }

    if pos != len {
        return Err(CodecError::TrailingBytes(len - pos));
    }
    Ok(())
}

#[inline]
fn skip_arg_bytes(total_len: usize, pos: &mut usize, ai: u8) -> Result<(), CodecError> {
    let extra = match ai {
        0..=23 => 0usize,
        24 => 1,
        25 => 2,
        26 => 4,
        27 => 8,
        _ => {
            return Err(CodecError::Decode(
                "reserved CBOR additional info".to_owned(),
            ));
        }
    };
    if total_len - *pos < extra {
        return Err(CodecError::Decode("unexpected end of CBOR input".to_owned()));
    }
    *pos += extra;
    Ok(())
}

#[inline]
fn read_u64_arg(bytes: &[u8], pos: &mut usize, ai: u8) -> Result<u64, CodecError> {
    let rem = &bytes[*pos..];
    match ai {
        0..=23 => Ok(ai as u64),
        24 => {
            if rem.is_empty() {
                return Err(CodecError::Decode("unexpected end of CBOR input".to_owned()));
            }
            *pos += 1;
            Ok(rem[0] as u64)
        }
        25 => {
            let chunk: [u8; 2] = rem
                .get(..2)
                .and_then(|s| s.try_into().ok())
                .ok_or_else(|| CodecError::Decode("unexpected end of CBOR input".to_owned()))?;
            *pos += 2;
            Ok(u16::from_be_bytes(chunk) as u64)
        }
        26 => {
            let chunk: [u8; 4] = rem
                .get(..4)
                .and_then(|s| s.try_into().ok())
                .ok_or_else(|| CodecError::Decode("unexpected end of CBOR input".to_owned()))?;
            *pos += 4;
            Ok(u32::from_be_bytes(chunk) as u64)
        }
        27 => {
            let chunk: [u8; 8] = rem
                .get(..8)
                .and_then(|s| s.try_into().ok())
                .ok_or_else(|| CodecError::Decode("unexpected end of CBOR input".to_owned()))?;
            *pos += 8;
            Ok(u64::from_be_bytes(chunk))
        }
        _ => Err(CodecError::Decode(
            "reserved CBOR additional info".to_owned(),
        )),
    }
}

#[inline]
fn read_len_arg(bytes: &[u8], pos: &mut usize, ai: u8) -> Result<usize, CodecError> {
    let raw = read_u64_arg(bytes, pos, ai)?;
    usize::try_from(raw)
        .map_err(|_| CodecError::Decode("CBOR length exceeds addressable size".to_owned()))
}

#[inline]
fn check_collection_len(len: u64, max_collection_len: usize) -> Result<u64, CodecError> {
    if len > max_collection_len as u64 || len > MAX_FRAME_COUNT {
        return Err(CodecError::MaxCollectionLenExceeded(max_collection_len));
    }
    Ok(len)
}

#[inline]
fn advance_parent<const N: usize>(stack: &mut [u64; N], depth: &mut usize, root_complete: &mut bool) {
    while *depth > 0 {
        let top = stack[*depth - 1];
        match top & KIND_MASK {
            KIND_DEFINITE => {
                let rem = (top & COUNT_MASK) - 1;
                if rem == 0 {
                    *depth -= 1;
                } else {
                    stack[*depth - 1] = KIND_DEFINITE | rem;
                    return;
                }
            }
            KIND_INDEF_ARRAY => return,
            KIND_INDEF_MAP_KEY => {
                stack[*depth - 1] = KIND_INDEF_MAP_VAL | (top & COUNT_MASK);
                return;
            }
            KIND_INDEF_MAP_VAL => {
                stack[*depth - 1] = KIND_INDEF_MAP_KEY | (top & COUNT_MASK);
                return;
            }
            _ => unreachable!("2-bit frame kind"),
        }
    }
    *root_complete = true;
}
