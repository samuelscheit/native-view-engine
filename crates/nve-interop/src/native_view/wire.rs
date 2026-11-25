//! Shared binary wire primitives (`NVNP` / `NVNC` constants, typed errors, `NativeValueRef`, and
//! scalar/string/byte read-write helpers) for zero-JSON native view codecs.

use std::fmt;

/// Magic header for binary native view prop buffers (`"NVNP"`).
pub const NATIVE_PROPS_MAGIC: [u8; 4] = *b"NVNP";
/// Current binary native view prop format version.
pub const NATIVE_PROPS_VERSION: u8 = 1;
/// Header size for `NVNP` (`magic[4] + version[1] + field_count[2]`).
pub const NATIVE_PROPS_HEADER_SIZE: usize = 7;

/// Magic header for binary native view command payloads (`"NVNC"`).
pub const NATIVE_CMD_MAGIC: [u8; 4] = *b"NVNC";
/// Current binary native view command format version.
pub const NATIVE_CMD_VERSION: u8 = 1;
/// Header size for `NVNC` (`magic[4] + version[1] + opcode[2] + arg_count[1]`).
pub const NATIVE_CMD_HEADER_SIZE: usize = 8;

/// Standard `nve-abi` `SetRawProp` key carrying the concrete native view kind string.
pub const NATIVE_VIEW_KIND_PROP_KEY: &str = "nve.native_kind";
/// Standard `nve-abi` `SetRawProp` key carrying the binary `NVNP` encoded prop bytes.
pub const NATIVE_VIEW_PROPS_PROP_KEY: &str = "nve.native_props";

/// Maximum permitted binary fields per native view prop payload (guards against OOM bombs).
pub const MAX_NATIVE_PROP_FIELDS: usize = 1024;
/// Maximum permitted arguments per imperative native view command.
pub const MAX_NATIVE_CMD_ARGS: usize = 64;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueTag {
    Null = 0,
    Bool = 1,
    U32 = 2,
    I64 = 3,
    U64 = 4,
    F32 = 5,
    F64 = 6,
    Str = 7,
    Bytes = 8,
}

impl ValueTag {
    #[inline]
    fn from_u8(v: u8) -> Result<Self, NativeViewCodecError> {
        match v {
            0 => Ok(Self::Null),
            1 => Ok(Self::Bool),
            2 => Ok(Self::U32),
            3 => Ok(Self::I64),
            4 => Ok(Self::U64),
            5 => Ok(Self::F32),
            6 => Ok(Self::F64),
            7 => Ok(Self::Str),
            8 => Ok(Self::Bytes),
            other => Err(NativeViewCodecError::InvalidTag(other)),
        }
    }
}

/// Typed errors returned when encoding, decoding, or dispatching native view props and commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeViewCodecError {
    Truncated,
    InvalidMagic([u8; 4]),
    UnsupportedVersion(u8),
    InvalidTag(u8),
    InvalidBool(u8),
    InvalidEnum(&'static str, u32),
    InvalidUtf8,
    MissingField(u16),
    MissingArg(usize),
    UnexpectedArgCount { expected: usize, actual: usize },
    TypeMismatch { expected: &'static str },
    TrailingBytes(usize),
    UnknownCommandOpcode(u16),
    TooManyFields(usize),
    PayloadTooLarge,
    UnboundHandle,
}

impl fmt::Display for NativeViewCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("truncated native view binary buffer"),
            Self::InvalidMagic(m) => write!(f, "invalid native view magic: {m:?}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported native view codec version: {v}"),
            Self::InvalidTag(t) => write!(f, "invalid native view value tag: {t}"),
            Self::InvalidBool(b) => write!(f, "invalid native view bool byte: {b}"),
            Self::InvalidEnum(ty, v) => write!(f, "invalid {ty} discriminant: {v}"),
            Self::InvalidUtf8 => f.write_str("invalid UTF-8 in native view binary string"),
            Self::MissingField(id) => write!(f, "missing required native prop field {id}"),
            Self::MissingArg(idx) => write!(f, "missing required native command arg {idx}"),
            Self::UnexpectedArgCount { expected, actual } => {
                write!(f, "expected {expected} command args, got {actual}")
            }
            Self::TypeMismatch { expected } => {
                write!(f, "native view binary type mismatch (expected {expected})")
            }
            Self::TrailingBytes(n) => {
                write!(f, "{n} unexpected trailing bytes in native view buffer")
            }
            Self::UnknownCommandOpcode(op) => write!(f, "unknown native command opcode: {op}"),
            Self::TooManyFields(n) => write!(f, "too many native view fields/args: {n}"),
            Self::PayloadTooLarge => f.write_str("native view binary payload exceeds u32 length"),
            Self::UnboundHandle => f.write_str("NativeViewHandle is not bound to a mounted node"),
        }
    }
}

impl std::error::Error for NativeViewCodecError {}

/// Zero-copy borrowed scalar/string/byte value decoded from an `NVNP` or `NVNC` buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NativeValueRef<'a> {
    Null,
    Bool(bool),
    U32(u32),
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
    Str(&'a str),
    Bytes(&'a [u8]),
}

impl<'a> NativeValueRef<'a> {
    #[inline]
    pub fn as_bool(self) -> Result<bool, NativeViewCodecError> {
        match self {
            Self::Bool(v) => Ok(v),
            _ => Err(NativeViewCodecError::TypeMismatch { expected: "bool" }),
        }
    }

    #[inline]
    pub fn as_u32(self) -> Result<u32, NativeViewCodecError> {
        match self {
            Self::U32(v) => Ok(v),
            _ => Err(NativeViewCodecError::TypeMismatch { expected: "u32" }),
        }
    }

    #[inline]
    pub fn as_opt_u32(self) -> Result<Option<u32>, NativeViewCodecError> {
        match self {
            Self::Null => Ok(None),
            Self::U32(v) => Ok(Some(v)),
            _ => Err(NativeViewCodecError::TypeMismatch {
                expected: "Option<u32>",
            }),
        }
    }

    #[inline]
    pub fn as_i64(self) -> Result<i64, NativeViewCodecError> {
        match self {
            Self::I64(v) => Ok(v),
            _ => Err(NativeViewCodecError::TypeMismatch { expected: "i64" }),
        }
    }

    #[inline]
    pub fn as_u64(self) -> Result<u64, NativeViewCodecError> {
        match self {
            Self::U64(v) => Ok(v),
            _ => Err(NativeViewCodecError::TypeMismatch { expected: "u64" }),
        }
    }

    #[inline]
    pub fn as_f32(self) -> Result<f32, NativeViewCodecError> {
        match self {
            Self::F32(v) => Ok(v),
            _ => Err(NativeViewCodecError::TypeMismatch { expected: "f32" }),
        }
    }

    #[inline]
    pub fn as_f64(self) -> Result<f64, NativeViewCodecError> {
        match self {
            Self::F64(v) => Ok(v),
            _ => Err(NativeViewCodecError::TypeMismatch { expected: "f64" }),
        }
    }

    #[inline]
    pub fn as_str(self) -> Result<&'a str, NativeViewCodecError> {
        match self {
            Self::Str(s) => Ok(s),
            _ => Err(NativeViewCodecError::TypeMismatch { expected: "str" }),
        }
    }

    #[inline]
    pub fn as_bytes(self) -> Result<&'a [u8], NativeViewCodecError> {
        match self {
            Self::Bytes(b) => Ok(b),
            _ => Err(NativeViewCodecError::TypeMismatch { expected: "bytes" }),
        }
    }
}

#[inline]
pub(crate) fn write_value(
    buf: &mut Vec<u8>,
    value: NativeValueRef<'_>,
) -> Result<(), NativeViewCodecError> {
    match value {
        NativeValueRef::Null => buf.push(ValueTag::Null as u8),
        NativeValueRef::Bool(v) => {
            buf.push(ValueTag::Bool as u8);
            buf.push(u8::from(v));
        }
        NativeValueRef::U32(v) => {
            buf.push(ValueTag::U32 as u8);
            buf.extend_from_slice(&v.to_le_bytes());
        }
        NativeValueRef::I64(v) => {
            buf.push(ValueTag::I64 as u8);
            buf.extend_from_slice(&v.to_le_bytes());
        }
        NativeValueRef::U64(v) => {
            buf.push(ValueTag::U64 as u8);
            buf.extend_from_slice(&v.to_le_bytes());
        }
        NativeValueRef::F32(v) => {
            buf.push(ValueTag::F32 as u8);
            buf.extend_from_slice(&v.to_le_bytes());
        }
        NativeValueRef::F64(v) => {
            buf.push(ValueTag::F64 as u8);
            buf.extend_from_slice(&v.to_le_bytes());
        }
        NativeValueRef::Str(s) => {
            let b = s.as_bytes();
            let len = u32::try_from(b.len()).map_err(|_| NativeViewCodecError::PayloadTooLarge)?;
            buf.push(ValueTag::Str as u8);
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(b);
        }
        NativeValueRef::Bytes(b) => {
            let len = u32::try_from(b.len()).map_err(|_| NativeViewCodecError::PayloadTooLarge)?;
            buf.push(ValueTag::Bytes as u8);
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(b);
        }
    }
    Ok(())
}

#[inline]
pub(crate) fn read_value<'a>(
    bytes: &'a [u8],
    pos: &mut usize,
) -> Result<NativeValueRef<'a>, NativeViewCodecError> {
    let raw_tag = *bytes.get(*pos).ok_or(NativeViewCodecError::Truncated)?;
    *pos += 1;
    let tag = ValueTag::from_u8(raw_tag)?;
    match tag {
        ValueTag::Null => Ok(NativeValueRef::Null),
        ValueTag::Bool => {
            let b = *bytes.get(*pos).ok_or(NativeViewCodecError::Truncated)?;
            *pos += 1;
            match b {
                0 => Ok(NativeValueRef::Bool(false)),
                1 => Ok(NativeValueRef::Bool(true)),
                other => Err(NativeViewCodecError::InvalidBool(other)),
            }
        }
        ValueTag::U32 => {
            let slice = take_slice(bytes, pos, 4)?;
            Ok(NativeValueRef::U32(u32::from_le_bytes(
                slice.try_into().unwrap(),
            )))
        }
        ValueTag::I64 => {
            let slice = take_slice(bytes, pos, 8)?;
            Ok(NativeValueRef::I64(i64::from_le_bytes(
                slice.try_into().unwrap(),
            )))
        }
        ValueTag::U64 => {
            let slice = take_slice(bytes, pos, 8)?;
            Ok(NativeValueRef::U64(u64::from_le_bytes(
                slice.try_into().unwrap(),
            )))
        }
        ValueTag::F32 => {
            let slice = take_slice(bytes, pos, 4)?;
            Ok(NativeValueRef::F32(f32::from_le_bytes(
                slice.try_into().unwrap(),
            )))
        }
        ValueTag::F64 => {
            let slice = take_slice(bytes, pos, 8)?;
            Ok(NativeValueRef::F64(f64::from_le_bytes(
                slice.try_into().unwrap(),
            )))
        }
        ValueTag::Str => {
            let len_slice = take_slice(bytes, pos, 4)?;
            let len = u32::from_le_bytes(len_slice.try_into().unwrap()) as usize;
            let raw = take_slice(bytes, pos, len)?;
            let s = std::str::from_utf8(raw).map_err(|_| NativeViewCodecError::InvalidUtf8)?;
            Ok(NativeValueRef::Str(s))
        }
        ValueTag::Bytes => {
            let len_slice = take_slice(bytes, pos, 4)?;
            let len = u32::from_le_bytes(len_slice.try_into().unwrap()) as usize;
            let raw = take_slice(bytes, pos, len)?;
            Ok(NativeValueRef::Bytes(raw))
        }
    }
}

#[inline]
pub(crate) fn take_slice<'a>(
    bytes: &'a [u8],
    pos: &mut usize,
    len: usize,
) -> Result<&'a [u8], NativeViewCodecError> {
    let end = pos.checked_add(len).ok_or(NativeViewCodecError::Truncated)?;
    let slice = bytes.get(*pos..end).ok_or(NativeViewCodecError::Truncated)?;
    *pos = end;
    Ok(slice)
}
