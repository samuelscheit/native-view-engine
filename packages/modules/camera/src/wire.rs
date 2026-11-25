//! Zero-JSON `NVNP` (native props) and `NVNC` (native commands) binary wire primitives
//! byte-for-byte aligned with `packages/core/interop` and `packages/modules/camera/cxx`.

use std::fmt;

pub const NATIVE_PROPS_MAGIC: [u8; 4] = *b"NVNP";
pub const NATIVE_PROPS_VERSION: u8 = 1;
pub const NATIVE_PROPS_HEADER_SIZE: usize = 7;
pub const MAX_NATIVE_PROP_FIELDS: usize = 1024;

pub const NATIVE_CMD_MAGIC: [u8; 4] = *b"NVNC";
pub const NATIVE_CMD_VERSION: u8 = 1;
pub const NATIVE_CMD_HEADER_SIZE: usize = 8;
pub const MAX_NATIVE_CMD_ARGS: usize = 64;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeValueTag {
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

impl NativeValueTag {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Null),
            1 => Some(Self::Bool),
            2 => Some(Self::U32),
            3 => Some(Self::I64),
            4 => Some(Self::U64),
            5 => Some(Self::F32),
            6 => Some(Self::F64),
            7 => Some(Self::Str),
            8 => Some(Self::Bytes),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraCodecError {
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
}

impl fmt::Display for CameraCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CameraCodecError {}

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
    pub fn as_bool(self) -> Result<bool, CameraCodecError> {
        match self {
            Self::Bool(v) => Ok(v),
            _ => Err(CameraCodecError::TypeMismatch { expected: "bool" }),
        }
    }

    pub fn as_u32(self) -> Result<u32, CameraCodecError> {
        match self {
            Self::U32(v) => Ok(v),
            _ => Err(CameraCodecError::TypeMismatch { expected: "u32" }),
        }
    }

    pub fn as_f32(self) -> Result<f32, CameraCodecError> {
        match self {
            Self::F32(v) => Ok(v),
            _ => Err(CameraCodecError::TypeMismatch { expected: "f32" }),
        }
    }

    pub fn as_str(self) -> Result<&'a str, CameraCodecError> {
        match self {
            Self::Str(s) => Ok(s),
            _ => Err(CameraCodecError::TypeMismatch { expected: "str" }),
        }
    }
}

fn encode_value(out: &mut Vec<u8>, value: NativeValueRef<'_>) -> Result<(), CameraCodecError> {
    match value {
        NativeValueRef::Null => out.push(NativeValueTag::Null as u8),
        NativeValueRef::Bool(b) => {
            out.push(NativeValueTag::Bool as u8);
            out.push(u8::from(b));
        }
        NativeValueRef::U32(v) => {
            out.push(NativeValueTag::U32 as u8);
            out.extend_from_slice(&v.to_le_bytes());
        }
        NativeValueRef::I64(v) => {
            out.push(NativeValueTag::I64 as u8);
            out.extend_from_slice(&v.to_le_bytes());
        }
        NativeValueRef::U64(v) => {
            out.push(NativeValueTag::U64 as u8);
            out.extend_from_slice(&v.to_le_bytes());
        }
        NativeValueRef::F32(v) => {
            out.push(NativeValueTag::F32 as u8);
            out.extend_from_slice(&v.to_le_bytes());
        }
        NativeValueRef::F64(v) => {
            out.push(NativeValueTag::F64 as u8);
            out.extend_from_slice(&v.to_le_bytes());
        }
        NativeValueRef::Str(s) => {
            let len = u32::try_from(s.len()).map_err(|_| CameraCodecError::PayloadTooLarge)?;
            out.push(NativeValueTag::Str as u8);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        NativeValueRef::Bytes(b) => {
            let len = u32::try_from(b.len()).map_err(|_| CameraCodecError::PayloadTooLarge)?;
            out.push(NativeValueTag::Bytes as u8);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(b);
        }
    }
    Ok(())
}

fn decode_value<'a>(buf: &'a [u8], pos: &mut usize) -> Result<NativeValueRef<'a>, CameraCodecError> {
    if *pos >= buf.len() {
        return Err(CameraCodecError::Truncated);
    }
    let raw_tag = buf[*pos];
    *pos += 1;
    let tag = NativeValueTag::from_u8(raw_tag).ok_or(CameraCodecError::InvalidTag(raw_tag))?;
    match tag {
        NativeValueTag::Null => Ok(NativeValueRef::Null),
        NativeValueTag::Bool => {
            if buf.len() - *pos < 1 {
                return Err(CameraCodecError::Truncated);
            }
            let b = buf[*pos];
            *pos += 1;
            match b {
                0 => Ok(NativeValueRef::Bool(false)),
                1 => Ok(NativeValueRef::Bool(true)),
                other => Err(CameraCodecError::InvalidBool(other)),
            }
        }
        NativeValueTag::U32 => {
            if buf.len() - *pos < 4 {
                return Err(CameraCodecError::Truncated);
            }
            let v = u32::from_le_bytes(buf[*pos..*pos + 4].try_into().unwrap());
            *pos += 4;
            Ok(NativeValueRef::U32(v))
        }
        NativeValueTag::I64 => {
            if buf.len() - *pos < 8 {
                return Err(CameraCodecError::Truncated);
            }
            let v = i64::from_le_bytes(buf[*pos..*pos + 8].try_into().unwrap());
            *pos += 8;
            Ok(NativeValueRef::I64(v))
        }
        NativeValueTag::U64 => {
            if buf.len() - *pos < 8 {
                return Err(CameraCodecError::Truncated);
            }
            let v = u64::from_le_bytes(buf[*pos..*pos + 8].try_into().unwrap());
            *pos += 8;
            Ok(NativeValueRef::U64(v))
        }
        NativeValueTag::F32 => {
            if buf.len() - *pos < 4 {
                return Err(CameraCodecError::Truncated);
            }
            let v = f32::from_le_bytes(buf[*pos..*pos + 4].try_into().unwrap());
            *pos += 4;
            Ok(NativeValueRef::F32(v))
        }
        NativeValueTag::F64 => {
            if buf.len() - *pos < 8 {
                return Err(CameraCodecError::Truncated);
            }
            let v = f64::from_le_bytes(buf[*pos..*pos + 8].try_into().unwrap());
            *pos += 8;
            Ok(NativeValueRef::F64(v))
        }
        NativeValueTag::Str => {
            if buf.len() - *pos < 4 {
                return Err(CameraCodecError::Truncated);
            }
            let len = u32::from_le_bytes(buf[*pos..*pos + 4].try_into().unwrap()) as usize;
            *pos += 4;
            if buf.len() - *pos < len {
                return Err(CameraCodecError::Truncated);
            }
            let s = std::str::from_utf8(&buf[*pos..*pos + len])
                .map_err(|_| CameraCodecError::InvalidUtf8)?;
            *pos += len;
            Ok(NativeValueRef::Str(s))
        }
        NativeValueTag::Bytes => {
            if buf.len() - *pos < 4 {
                return Err(CameraCodecError::Truncated);
            }
            let len = u32::from_le_bytes(buf[*pos..*pos + 4].try_into().unwrap()) as usize;
            *pos += 4;
            if buf.len() - *pos < len {
                return Err(CameraCodecError::Truncated);
            }
            let slice = &buf[*pos..*pos + len];
            *pos += len;
            Ok(NativeValueRef::Bytes(slice))
        }
    }
}

pub struct PropEncoder {
    buf: Vec<u8>,
    count: u16,
}

impl PropEncoder {
    pub fn new() -> Self {
        let mut buf = Vec::with_capacity(96);
        buf.extend_from_slice(&NATIVE_PROPS_MAGIC);
        buf.push(NATIVE_PROPS_VERSION);
        buf.extend_from_slice(&0u16.to_le_bytes());
        Self { buf, count: 0 }
    }

    pub fn field(&mut self, id: u16, value: NativeValueRef<'_>) -> Result<&mut Self, CameraCodecError> {
        if usize::from(self.count) >= MAX_NATIVE_PROP_FIELDS {
            return Err(CameraCodecError::TooManyFields(usize::from(self.count) + 1));
        }
        self.buf.extend_from_slice(&id.to_le_bytes());
        encode_value(&mut self.buf, value)?;
        self.count += 1;
        Ok(self)
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.buf[5..7].copy_from_slice(&self.count.to_le_bytes());
        self.buf
    }
}

impl Default for PropEncoder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PropDecoder<'a> {
    fields: Vec<(u16, NativeValueRef<'a>)>,
}

impl<'a> PropDecoder<'a> {
    pub fn parse(buf: &'a [u8]) -> Result<Self, CameraCodecError> {
        if buf.len() < NATIVE_PROPS_HEADER_SIZE {
            return Err(CameraCodecError::Truncated);
        }
        let magic: [u8; 4] = buf[0..4].try_into().unwrap();
        if magic != NATIVE_PROPS_MAGIC {
            return Err(CameraCodecError::InvalidMagic(magic));
        }
        if buf[4] != NATIVE_PROPS_VERSION {
            return Err(CameraCodecError::UnsupportedVersion(buf[4]));
        }
        let count = usize::from(u16::from_le_bytes([buf[5], buf[6]]));
        if count > MAX_NATIVE_PROP_FIELDS {
            return Err(CameraCodecError::TooManyFields(count));
        }
        let mut pos = NATIVE_PROPS_HEADER_SIZE;
        let mut fields = Vec::with_capacity(count);
        for _ in 0..count {
            if buf.len() - pos < 2 {
                return Err(CameraCodecError::Truncated);
            }
            let id = u16::from_le_bytes([buf[pos], buf[pos + 1]]);
            pos += 2;
            let val = decode_value(buf, &mut pos)?;
            fields.push((id, val));
        }
        if pos != buf.len() {
            return Err(CameraCodecError::TrailingBytes(buf.len() - pos));
        }
        Ok(Self { fields })
    }

    pub fn require(&self, id: u16) -> Result<NativeValueRef<'a>, CameraCodecError> {
        self.fields
            .iter()
            .rev()
            .find_map(|&(fid, val)| (fid == id).then_some(val))
            .ok_or(CameraCodecError::MissingField(id))
    }
}

pub struct CommandEncoder {
    buf: Vec<u8>,
    argc: u8,
}

impl CommandEncoder {
    pub fn new(opcode: u16) -> Self {
        let mut buf = Vec::with_capacity(32);
        buf.extend_from_slice(&NATIVE_CMD_MAGIC);
        buf.push(NATIVE_CMD_VERSION);
        buf.extend_from_slice(&opcode.to_le_bytes());
        buf.push(0);
        Self { buf, argc: 0 }
    }

    pub fn arg(&mut self, value: NativeValueRef<'_>) -> Result<&mut Self, CameraCodecError> {
        if usize::from(self.argc) >= MAX_NATIVE_CMD_ARGS {
            return Err(CameraCodecError::TooManyFields(usize::from(self.argc) + 1));
        }
        encode_value(&mut self.buf, value)?;
        self.argc += 1;
        Ok(self)
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.buf[7] = self.argc;
        self.buf
    }
}

pub struct CommandDecoder<'a> {
    opcode: u16,
    args: Vec<NativeValueRef<'a>>,
}

impl<'a> CommandDecoder<'a> {
    pub fn parse(buf: &'a [u8]) -> Result<Self, CameraCodecError> {
        if buf.len() < NATIVE_CMD_HEADER_SIZE {
            return Err(CameraCodecError::Truncated);
        }
        let magic: [u8; 4] = buf[0..4].try_into().unwrap();
        if magic != NATIVE_CMD_MAGIC {
            return Err(CameraCodecError::InvalidMagic(magic));
        }
        if buf[4] != NATIVE_CMD_VERSION {
            return Err(CameraCodecError::UnsupportedVersion(buf[4]));
        }
        let opcode = u16::from_le_bytes([buf[5], buf[6]]);
        let argc = usize::from(buf[7]);
        if argc > MAX_NATIVE_CMD_ARGS {
            return Err(CameraCodecError::TooManyFields(argc));
        }
        let mut pos = NATIVE_CMD_HEADER_SIZE;
        let mut args = Vec::with_capacity(argc);
        for _ in 0..argc {
            args.push(decode_value(buf, &mut pos)?);
        }
        if pos != buf.len() {
            return Err(CameraCodecError::TrailingBytes(buf.len() - pos));
        }
        Ok(Self { opcode, args })
    }

    pub const fn opcode(&self) -> u16 {
        self.opcode
    }

    pub fn expect_arg_count(&self, expected: usize) -> Result<(), CameraCodecError> {
        if self.args.len() == expected {
            Ok(())
        } else {
            Err(CameraCodecError::UnexpectedArgCount {
                expected,
                actual: self.args.len(),
            })
        }
    }

    pub fn require(&self, idx: usize) -> Result<NativeValueRef<'a>, CameraCodecError> {
        self.args
            .get(idx)
            .copied()
            .ok_or(CameraCodecError::MissingArg(idx))
    }
}
