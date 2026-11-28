//! Zero-unsafe, zero-runtime-dependency Tier-2 little-endian binary codec primitives
//! (`ByteReader`, `ByteWriter`, `SliceWriter`, `ByteSink`, `ByteBuf`, `CodecError`, `NveEncode`,
//! `NveDecode`) and `#[derive(NveCodec)]`.
//!
//! Sits strictly below `nve-abi` in the workspace dependency DAG (depends only on zero-dep
//! `nve-abi-macros`) so every core, framework, backend, and module crate shares one canonical,
//! bounds-checked, DoS-hardened Tier-2 codec.

#![forbid(unsafe_code)]
#![deny(clippy::let_underscore_must_use)]
#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod flat_op;
mod reader;
pub mod transport;
mod writer;

#[cfg(test)]
mod tests;

use core::fmt;
use core::ops::{Deref, DerefMut};

/// Derives [`NveEncode`] and [`NveDecode`] for Tier-2 little-endian structs and enums.
///
/// # Compile-Time Guards
///
/// Generics are rejected:
/// ```compile_fail
/// use nve_codec::NveCodec;
/// #[derive(NveCodec)]
/// struct BadGeneric<T: nve_codec::NveEncode + nve_codec::NveDecode>(T);
/// ```
///
/// `where` clauses are rejected:
/// ```compile_fail
/// use nve_codec::NveCodec;
/// #[derive(NveCodec)]
/// struct BadWhere(u8) where u8: Copy;
/// ```
///
/// Field-level `#[nve_codec(...)]` attributes are rejected:
/// ```compile_fail
/// use nve_codec::NveCodec;
/// #[derive(NveCodec)]
/// struct BadFieldAttr {
///     #[nve_codec(skip)]
///     a: u8,
/// }
/// ```
///
/// Conflicting `#[nve_codec(tag = ...)]` and `= discriminant` are rejected:
/// ```compile_fail
/// use nve_codec::NveCodec;
/// #[repr(u8)]
/// #[derive(NveCodec)]
/// enum ConflictingTag {
///     #[nve_codec(tag = 1)]
///     A(u8) = 2,
/// }
/// ```
///
/// Duplicate enum tags are rejected:
/// ```compile_fail
/// use nve_codec::NveCodec;
/// #[derive(NveCodec)]
/// enum DupTag {
///     #[nve_codec(tag = 1)]
///     A(u8),
///     #[nve_codec(tag = 1)]
///     B(u16),
/// }
/// ```
///
/// Out-of-range tags are rejected:
/// ```compile_fail
/// use nve_codec::NveCodec;
/// #[derive(NveCodec)]
/// enum OutOfRange {
///     #[nve_codec(tag = 256)]
///     TooBig(u8),
/// }
/// ```
///
/// Non-`u8` data-carrying enums are rejected:
/// ```compile_fail
/// use nve_codec::NveCodec;
/// #[repr(u16)]
/// #[derive(NveCodec)]
/// enum BadDataRepr {
///     A(u32),
/// }
/// ```
///
/// Unknown `#[nve_codec(...)]` keys are rejected:
/// ```compile_fail
/// use nve_codec::NveCodec;
/// #[derive(NveCodec)]
/// #[nve_codec(unknown_key = "x")]
/// struct BadKey(u8);
/// ```
///
/// Unit-only enums must use Rust discriminants instead of `#[nve_codec(tag = ...)]`:
/// ```compile_fail
/// use nve_codec::NveCodec;
/// #[repr(u8)]
/// #[derive(NveCodec)]
/// enum BadUnitTag {
///     #[nve_codec(tag = 7)]
///     A,
/// }
/// ```
///
/// Unsupported integer `#[repr(...)]` attributes are rejected:
/// ```compile_fail
/// use nve_codec::NveCodec;
/// #[repr(i8)]
/// #[derive(NveCodec)]
/// enum BadRepr {
///     A = 1,
/// }
/// ```
pub use flat_op::{
    FlatOpDesc, FlatOpFieldDesc, FlatOpFieldType, FlatOpScopeDesc, FlatOpStreamDesc,
    FloatConstraint,
};
pub use nve_abi_macros::{NveCodec, ReprEnum};
pub use reader::{ByteReader, DEFAULT_MAX_DEPTH};
pub use transport::{
    call_sync_decode, encode_f32_stack, encode_f64_stack, try_with_stack_writer, with_stack_writer,
    ByteTransport, StackWriter, TransportDoneFn, TransportError,
};
pub use writer::{try_write_fixed, write_fixed, write_fixed_into, ByteSink, ByteWriter, SliceWriter};

/// Default upper bound on element counts encoded/decoded by `NveEncode`/`NveDecode` for `Vec<T>`.
pub const DEFAULT_MAX_ITEMS: usize = 65_536;

/// Default upper bound on byte/string lengths encoded/decoded by `NveEncode`/`NveDecode` (`16 MiB`).
pub const DEFAULT_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Canonical quiet NaN bit pattern for `f32` (IEEE-754 little-endian deterministic encoding).
pub const CANONICAL_F32_NAN_BITS: u32 = 0x7FC0_0000;

/// Canonical quiet NaN bit pattern for `f64` (IEEE-754 little-endian deterministic encoding).
pub const CANONICAL_F64_NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

/// Canonicalizes an `f32` (`-0.0` -> `+0.0`, any `NaN` -> canonical quiet `NaN`).
#[inline]
pub const fn canonical_f32_bits(v: f32) -> u32 {
    if v.is_nan() {
        CANONICAL_F32_NAN_BITS
    } else if v == 0.0 {
        0
    } else {
        v.to_bits()
    }
}

/// Canonicalizes an `f64` (`-0.0` -> `+0.0`, any `NaN` -> canonical quiet `NaN`).
#[inline]
pub const fn canonical_f64_bits(v: f64) -> u64 {
    if v.is_nan() {
        CANONICAL_F64_NAN_BITS
    } else if v == 0.0 {
        0
    } else {
        v.to_bits()
    }
}

/// Typed, allocation-free error produced by [`ByteReader`], [`ByteWriter`], [`SliceWriter`],
/// and `#[derive(NveCodec)]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// Input ended before the required number of bytes could be read (or destination slice was full).
    Truncated { needed: usize, remaining: usize },
    /// Expected fixed magic prefix did not match.
    BadMagic,
    /// Unsupported wire or archive format version.
    BadVersion(u32),
    /// Unknown enum discriminant or tag (stored as `i64` so both `u32::MAX` and `i32::MIN` are exact).
    BadEnum { ty: &'static str, raw: i64 },
    /// Boolean byte was neither `0` nor `1`.
    BadBool(u8),
    /// Reserved / padding field was non-zero.
    NonZeroReserved(u64),
    /// Length or element count exceeded its configured upper bound or wire integer capacity.
    LenOverflow { len: usize, max: usize },
    /// Unexpected unread bytes remained at [`ByteReader::finish`] / [`SliceWriter::finish`].
    TrailingBytes(usize),
    /// Length-prefixed string slice was not valid UTF-8.
    InvalidUtf8,
    /// Recursive container nesting (`read_vec`/`read_opt`) exceeded `ByteReader`'s max depth.
    DepthExceeded { max: usize },
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, remaining } => {
                write!(f, "truncated binary input: needed {needed}B, remaining {remaining}B")
            }
            Self::BadMagic => f.write_str("invalid binary magic header"),
            Self::BadVersion(v) => write!(f, "unsupported binary format version {v}"),
            Self::BadEnum { ty, raw } => write!(f, "invalid enum discriminant {raw} for `{ty}`"),
            Self::BadBool(b) => write!(f, "invalid boolean byte {b} (expected 0 or 1)"),
            Self::NonZeroReserved(v) => write!(f, "non-zero reserved field {v:#x}"),
            Self::LenOverflow { len, max } => {
                write!(f, "binary length {len} exceeds maximum {max}")
            }
            Self::TrailingBytes(n) => write!(f, "unexpected {n} trailing byte(s)"),
            Self::InvalidUtf8 => f.write_str("invalid UTF-8 string in binary payload"),
            Self::DepthExceeded { max } => {
                write!(f, "binary container nesting exceeded maximum depth {max}")
            }
        }
    }
}

impl std::error::Error for CodecError {}

/// Bulk-encoded binary byte buffer (`u32` length prefix + raw bytes, capped at
/// [`DEFAULT_MAX_BYTES`] = 16 MiB and decoded in one slice copy rather than `Vec<u8>`'s
/// element-by-element loop).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ByteBuf(pub Vec<u8>);

impl ByteBuf {
    #[inline]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    #[inline]
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

impl Deref for ByteBuf {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl DerefMut for ByteBuf {
    #[inline]
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

impl AsRef<[u8]> for ByteBuf {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for ByteBuf {
    #[inline]
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

impl From<ByteBuf> for Vec<u8> {
    #[inline]
    fn from(b: ByteBuf) -> Self {
        b.0
    }
}

/// Trait for Tier-2 little-endian binary serialization into a [`ByteWriter`].
pub trait NveEncode {
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError>;

    #[inline]
    fn encode_to_vec(&self) -> Result<Vec<u8>, CodecError> {
        let mut w = ByteWriter::new();
        self.encode(&mut w)?;
        Ok(w.into_bytes())
    }
}

/// Trait for Tier-2 little-endian binary deserialization from a [`ByteReader`].
pub trait NveDecode: Sized {
    /// Conservative minimum encoded byte size used to bound vector pre-allocation.
    const MIN_ENCODED_BYTES: usize;

    fn decode(r: &mut ByteReader<'_>) -> Result<Self, CodecError>;

    #[inline]
    fn decode_exact(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut r = ByteReader::new(bytes);
        let value = Self::decode(&mut r)?;
        r.finish()?;
        Ok(value)
    }
}

impl NveEncode for () {
    #[inline]
    fn encode(&self, _w: &mut ByteWriter) -> Result<(), CodecError> {
        Ok(())
    }
}

impl NveDecode for () {
    const MIN_ENCODED_BYTES: usize = 0;

    #[inline]
    fn decode(_r: &mut ByteReader<'_>) -> Result<Self, CodecError> {
        Ok(())
    }
}

impl NveEncode for bool {
    #[inline]
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
        w.write_bool(*self);
        Ok(())
    }
}

impl NveDecode for bool {
    const MIN_ENCODED_BYTES: usize = 1;

    #[inline]
    fn decode(r: &mut ByteReader<'_>) -> Result<Self, CodecError> {
        r.read_bool()
    }
}

macro_rules! impl_prim_codec {
    ($($ty:ty => ($read:ident, $write:ident, $size:expr)),+ $(,)?) => {
        $(
            impl NveEncode for $ty {
                #[inline]
                fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
                    w.$write(*self);
                    Ok(())
                }
            }

            impl NveDecode for $ty {
                const MIN_ENCODED_BYTES: usize = $size;

                #[inline]
                fn decode(r: &mut ByteReader<'_>) -> Result<Self, CodecError> {
                    r.$read()
                }
            }
        )+
    };
}

impl_prim_codec!(
    u8 => (read_u8, write_u8, 1),
    i8 => (read_i8, write_i8, 1),
    u16 => (read_u16, write_u16, 2),
    i16 => (read_i16, write_i16, 2),
    u32 => (read_u32, write_u32, 4),
    i32 => (read_i32, write_i32, 4),
    u64 => (read_u64, write_u64, 8),
    i64 => (read_i64, write_i64, 8),
    f32 => (read_f32, write_f32, 4),
    f64 => (read_f64, write_f64, 8),
);

impl<const N: usize> NveEncode for [u8; N] {
    #[inline]
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
        w.write_raw(self);
        Ok(())
    }
}

impl<const N: usize> NveDecode for [u8; N] {
    const MIN_ENCODED_BYTES: usize = N;

    #[inline]
    fn decode(r: &mut ByteReader<'_>) -> Result<Self, CodecError> {
        r.read_array::<N>()
    }
}

impl NveEncode for &[u8] {
    #[inline]
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
        w.write_bytes_max(self, DEFAULT_MAX_BYTES)
    }
}

impl NveEncode for ByteBuf {
    #[inline]
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
        w.write_bytes_max(&self.0, DEFAULT_MAX_BYTES)
    }
}

impl NveDecode for ByteBuf {
    const MIN_ENCODED_BYTES: usize = 4;

    #[inline]
    fn decode(r: &mut ByteReader<'_>) -> Result<Self, CodecError> {
        Ok(Self(r.read_bytes(DEFAULT_MAX_BYTES)?.to_vec()))
    }
}

impl NveEncode for str {
    #[inline]
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
        w.write_str_max(self, DEFAULT_MAX_BYTES)
    }
}

impl NveEncode for &str {
    #[inline]
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
        w.write_str_max(self, DEFAULT_MAX_BYTES)
    }
}

impl NveEncode for String {
    #[inline]
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
        w.write_str_max(self, DEFAULT_MAX_BYTES)
    }
}

impl NveDecode for String {
    const MIN_ENCODED_BYTES: usize = 4;

    #[inline]
    fn decode(r: &mut ByteReader<'_>) -> Result<Self, CodecError> {
        r.read_string(DEFAULT_MAX_BYTES)
    }
}

impl<T: NveEncode> NveEncode for Option<T> {
    #[inline]
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
        match self {
            Some(v) => {
                w.write_bool(true);
                v.encode(w)
            }
            None => {
                w.write_bool(false);
                Ok(())
            }
        }
    }
}

impl<T: NveDecode> NveDecode for Option<T> {
    const MIN_ENCODED_BYTES: usize = 1;

    #[inline]
    fn decode(r: &mut ByteReader<'_>) -> Result<Self, CodecError> {
        r.read_opt(T::decode)
    }
}

impl<T: NveEncode> NveEncode for [T] {
    #[inline]
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
        w.write_len_max(self.len(), DEFAULT_MAX_ITEMS)?;
        for item in self {
            item.encode(w)?;
        }
        Ok(())
    }
}

impl<T: NveEncode> NveEncode for Vec<T> {
    #[inline]
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
        self.as_slice().encode(w)
    }
}

impl<T: NveDecode> NveDecode for Vec<T> {
    const MIN_ENCODED_BYTES: usize = 4;

    #[inline]
    fn decode(r: &mut ByteReader<'_>) -> Result<Self, CodecError> {
        r.read_vec(DEFAULT_MAX_ITEMS, T::MIN_ENCODED_BYTES, T::decode)
    }
}

impl<A: NveEncode, B: NveEncode> NveEncode for (A, B) {
    #[inline]
    fn encode(&self, w: &mut ByteWriter) -> Result<(), CodecError> {
        self.0.encode(w)?;
        self.1.encode(w)
    }
}

impl<A: NveDecode, B: NveDecode> NveDecode for (A, B) {
    const MIN_ENCODED_BYTES: usize = A::MIN_ENCODED_BYTES + B::MIN_ENCODED_BYTES;

    #[inline]
    fn decode(r: &mut ByteReader<'_>) -> Result<Self, CodecError> {
        Ok((A::decode(r)?, B::decode(r)?))
    }
}
