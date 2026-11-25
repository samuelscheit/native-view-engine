//! Single-source cross-language `NVNP` (native view props) and `NVNC` (native view commands)
//! binary wire codecs plus unified sink/source traits shared by both in-process typed containers
//! and FFI wire buffers.

use super::wire::{
    read_value, take_slice, write_value, NativeValueRef, NativeViewCodecError, MAX_NATIVE_CMD_ARGS,
    MAX_NATIVE_PROP_FIELDS, NATIVE_CMD_HEADER_SIZE, NATIVE_CMD_MAGIC, NATIVE_CMD_VERSION,
    NATIVE_PROPS_HEADER_SIZE, NATIVE_PROPS_MAGIC, NATIVE_PROPS_VERSION,
};

// =============================================================================
// Unified Prop/Command Sink & Source Traits (Zero Duplication Between Typed & Wire)
// =============================================================================

/// Unified target for writing numeric-ID native view prop fields — implemented by both
/// in-process [`super::NativePropsBuilder`] (zero serialization) and FFI [`NativePropEncoder`].
pub trait NativePropSink {
    fn field(
        &mut self,
        field_id: u16,
        value: NativeValueRef<'_>,
    ) -> Result<&mut Self, NativeViewCodecError>;

    #[inline]
    fn bool_field(&mut self, field_id: u16, v: bool) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::Bool(v))
    }

    #[inline]
    fn u32_field(&mut self, field_id: u16, v: u32) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::U32(v))
    }

    #[inline]
    fn opt_u32_field(
        &mut self,
        field_id: u16,
        v: Option<u32>,
    ) -> Result<&mut Self, NativeViewCodecError> {
        match v {
            Some(n) => self.field(field_id, NativeValueRef::U32(n)),
            None => self.field(field_id, NativeValueRef::Null),
        }
    }

    #[inline]
    fn u64_field(&mut self, field_id: u16, v: u64) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::U64(v))
    }

    #[inline]
    fn f32_field(&mut self, field_id: u16, v: f32) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::F32(v))
    }

    #[inline]
    fn str_field(&mut self, field_id: u16, s: &str) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::Str(s))
    }

    #[inline]
    fn opt_str_field(
        &mut self,
        field_id: u16,
        s: Option<&str>,
    ) -> Result<&mut Self, NativeViewCodecError> {
        match s {
            Some(v) => self.field(field_id, NativeValueRef::Str(v)),
            None => self.field(field_id, NativeValueRef::Null),
        }
    }

    #[inline]
    fn bytes_field(&mut self, field_id: u16, b: &[u8]) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::Bytes(b))
    }
}

/// Unified source for reading numeric-ID native view prop fields — implemented by both
/// in-process [`super::NativeProps`] (zero decoding) and FFI [`NativePropDecoder`].
pub trait NativePropSource {
    fn get_field(&self, field_id: u16) -> Option<NativeValueRef<'_>>;

    #[inline]
    fn require_field(&self, field_id: u16) -> Result<NativeValueRef<'_>, NativeViewCodecError> {
        self.get_field(field_id)
            .ok_or(NativeViewCodecError::MissingField(field_id))
    }

    #[inline]
    fn require_bool_field(&self, field_id: u16) -> Result<bool, NativeViewCodecError> {
        self.require_field(field_id)?.as_bool()
    }

    #[inline]
    fn require_u32_field(&self, field_id: u16) -> Result<u32, NativeViewCodecError> {
        self.require_field(field_id)?.as_u32()
    }

    #[inline]
    fn require_opt_u32_field(&self, field_id: u16) -> Result<Option<u32>, NativeViewCodecError> {
        self.require_field(field_id)?.as_opt_u32()
    }

    #[inline]
    fn require_opt_str_field(&self, field_id: u16) -> Result<Option<&str>, NativeViewCodecError> {
        self.require_field(field_id)?.as_opt_str()
    }

    #[inline]
    fn require_f32_field(&self, field_id: u16) -> Result<f32, NativeViewCodecError> {
        self.require_field(field_id)?.as_f32()
    }

    #[inline]
    fn require_str_field(&self, field_id: u16) -> Result<&str, NativeViewCodecError> {
        self.require_field(field_id)?.as_str()
    }

    #[inline]
    fn require_bytes_field(&self, field_id: u16) -> Result<&[u8], NativeViewCodecError> {
        self.require_field(field_id)?.as_bytes()
    }
}

/// Unified target for writing imperative command arguments.
pub trait NativeCommandSink {
    fn arg(&mut self, value: NativeValueRef<'_>) -> Result<&mut Self, NativeViewCodecError>;

    #[inline]
    fn arg_u32(&mut self, v: u32) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::U32(v))
    }

    #[inline]
    fn arg_u64(&mut self, v: u64) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::U64(v))
    }

    #[inline]
    fn arg_f32(&mut self, v: f32) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::F32(v))
    }

    #[inline]
    fn arg_str(&mut self, s: &str) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::Str(s))
    }

    #[inline]
    fn arg_bool(&mut self, v: bool) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::Bool(v))
    }

    #[inline]
    fn arg_bytes(&mut self, b: &[u8]) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::Bytes(b))
    }
}

/// Unified source for reading imperative command opcode & arguments — implemented by both
/// in-process [`super::NativeCommand`] and FFI [`NativeCommandDecoder`].
pub trait NativeCommandSource {
    fn opcode(&self) -> u16;
    fn arg_count(&self) -> usize;
    fn get_arg(&self, idx: usize) -> Option<NativeValueRef<'_>>;

    #[inline]
    fn expect_arg_count(&self, expected: usize) -> Result<(), NativeViewCodecError> {
        let actual = self.arg_count();
        if actual != expected {
            return Err(NativeViewCodecError::UnexpectedArgCount { expected, actual });
        }
        Ok(())
    }

    #[inline]
    fn require_arg(&self, idx: usize) -> Result<NativeValueRef<'_>, NativeViewCodecError> {
        self.get_arg(idx).ok_or(NativeViewCodecError::MissingArg(idx))
    }

    #[inline]
    fn require_u32_arg(&self, idx: usize) -> Result<u32, NativeViewCodecError> {
        self.require_arg(idx)?.as_u32()
    }

    #[inline]
    fn require_u64_arg(&self, idx: usize) -> Result<u64, NativeViewCodecError> {
        self.require_arg(idx)?.as_u64()
    }

    #[inline]
    fn require_f32_arg(&self, idx: usize) -> Result<f32, NativeViewCodecError> {
        self.require_arg(idx)?.as_f32()
    }

    #[inline]
    fn require_str_arg(&self, idx: usize) -> Result<&str, NativeViewCodecError> {
        self.require_arg(idx)?.as_str()
    }
}

// =============================================================================
// Binary Native Props Encoder & Decoder (`NVNP`) — Cross-Language FFI Wire
// =============================================================================

/// Compact binary prop encoder writing numeric `field_id: u16` entries with zero JSON.
#[derive(Debug, Clone)]
pub struct NativePropEncoder {
    buf: Vec<u8>,
    field_count: usize,
}

impl Default for NativePropEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl NativePropEncoder {
    pub fn new() -> Self {
        Self::with_capacity(64)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let mut buf = Vec::with_capacity(capacity.max(NATIVE_PROPS_HEADER_SIZE));
        buf.extend_from_slice(&NATIVE_PROPS_MAGIC);
        buf.push(NATIVE_PROPS_VERSION);
        buf.extend_from_slice(&0u16.to_le_bytes());
        Self {
            buf,
            field_count: 0,
        }
    }

    pub fn field(
        &mut self,
        field_id: u16,
        value: NativeValueRef<'_>,
    ) -> Result<&mut Self, NativeViewCodecError> {
        if self.field_count >= MAX_NATIVE_PROP_FIELDS {
            return Err(NativeViewCodecError::TooManyFields(self.field_count + 1));
        }
        self.buf.extend_from_slice(&field_id.to_le_bytes());
        write_value(&mut self.buf, value)?;
        self.field_count += 1;
        Ok(self)
    }

    #[inline]
    pub fn bool_field(&mut self, field_id: u16, v: bool) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::Bool(v))
    }

    #[inline]
    pub fn u32_field(&mut self, field_id: u16, v: u32) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::U32(v))
    }

    #[inline]
    pub fn opt_u32_field(
        &mut self,
        field_id: u16,
        v: Option<u32>,
    ) -> Result<&mut Self, NativeViewCodecError> {
        match v {
            Some(n) => self.field(field_id, NativeValueRef::U32(n)),
            None => self.field(field_id, NativeValueRef::Null),
        }
    }

    #[inline]
    pub fn u64_field(&mut self, field_id: u16, v: u64) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::U64(v))
    }

    #[inline]
    pub fn f32_field(&mut self, field_id: u16, v: f32) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::F32(v))
    }

    #[inline]
    pub fn str_field(&mut self, field_id: u16, s: &str) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::Str(s))
    }

    #[inline]
    pub fn bytes_field(
        &mut self,
        field_id: u16,
        b: &[u8],
    ) -> Result<&mut Self, NativeViewCodecError> {
        self.field(field_id, NativeValueRef::Bytes(b))
    }

    pub fn finish(mut self) -> Vec<u8> {
        let count = self.field_count as u16;
        self.buf[5..7].copy_from_slice(&count.to_le_bytes());
        self.buf
    }
}

impl NativePropSink for NativePropEncoder {
    #[inline]
    fn field(
        &mut self,
        field_id: u16,
        value: NativeValueRef<'_>,
    ) -> Result<&mut Self, NativeViewCodecError> {
        NativePropEncoder::field(self, field_id, value)
    }
}

/// Validated zero-copy view over a binary `NVNP` native view prop buffer.
#[derive(Debug, Clone)]
pub struct NativePropDecoder<'a> {
    fields: Vec<(u16, NativeValueRef<'a>)>,
}

impl<'a> NativePropDecoder<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, NativeViewCodecError> {
        if bytes.len() < NATIVE_PROPS_HEADER_SIZE {
            return Err(NativeViewCodecError::Truncated);
        }
        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != NATIVE_PROPS_MAGIC {
            return Err(NativeViewCodecError::InvalidMagic(magic));
        }
        let version = bytes[4];
        if version != NATIVE_PROPS_VERSION {
            return Err(NativeViewCodecError::UnsupportedVersion(version));
        }
        let count = usize::from(u16::from_le_bytes([bytes[5], bytes[6]]));
        if count > MAX_NATIVE_PROP_FIELDS {
            return Err(NativeViewCodecError::TooManyFields(count));
        }
        let mut pos = NATIVE_PROPS_HEADER_SIZE;
        if count > (bytes.len() - pos) / 3 {
            return Err(NativeViewCodecError::Truncated);
        }
        let mut fields = Vec::with_capacity(count);
        for _ in 0..count {
            let id_slice = take_slice(bytes, &mut pos, 2)?;
            let field_id = u16::from_le_bytes(id_slice.try_into().unwrap());
            let val = read_value(bytes, &mut pos)?;
            fields.push((field_id, val));
        }
        if pos != bytes.len() {
            return Err(NativeViewCodecError::TrailingBytes(bytes.len() - pos));
        }
        Ok(Self { fields })
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    #[inline]
    pub fn fields(&self) -> &[(u16, NativeValueRef<'a>)] {
        &self.fields
    }

    #[inline]
    pub fn get(&self, field_id: u16) -> Option<NativeValueRef<'a>> {
        self.fields
            .iter()
            .rev()
            .find(|(id, _)| *id == field_id)
            .map(|(_, v)| *v)
    }

    #[inline]
    pub fn require(&self, field_id: u16) -> Result<NativeValueRef<'a>, NativeViewCodecError> {
        self.get(field_id)
            .ok_or(NativeViewCodecError::MissingField(field_id))
    }

    #[inline]
    pub fn require_bool(&self, field_id: u16) -> Result<bool, NativeViewCodecError> {
        self.require(field_id)?.as_bool()
    }

    #[inline]
    pub fn require_u32(&self, field_id: u16) -> Result<u32, NativeViewCodecError> {
        self.require(field_id)?.as_u32()
    }

    #[inline]
    pub fn require_opt_u32(&self, field_id: u16) -> Result<Option<u32>, NativeViewCodecError> {
        self.require(field_id)?.as_opt_u32()
    }

    #[inline]
    pub fn require_f32(&self, field_id: u16) -> Result<f32, NativeViewCodecError> {
        self.require(field_id)?.as_f32()
    }

    #[inline]
    pub fn require_str(&self, field_id: u16) -> Result<&'a str, NativeViewCodecError> {
        self.require(field_id)?.as_str()
    }
}

impl NativePropSource for NativePropDecoder<'_> {
    #[inline]
    fn get_field(&self, field_id: u16) -> Option<NativeValueRef<'_>> {
        self.get(field_id)
    }
}

// =============================================================================
// Binary Native Command Payload Encoder & Decoder (`NVNC`) — Cross-Language FFI
// =============================================================================

/// Compact binary imperative command payload encoder (`NVNC`), carried inside
/// `nve_abi::Command::DispatchCommand::payload` across FFI boundaries.
#[derive(Debug, Clone)]
pub struct NativeCommandEncoder {
    buf: Vec<u8>,
    arg_count: usize,
}

impl NativeCommandEncoder {
    pub fn new(opcode: u16) -> Self {
        let mut buf = Vec::with_capacity(32);
        buf.extend_from_slice(&NATIVE_CMD_MAGIC);
        buf.push(NATIVE_CMD_VERSION);
        buf.extend_from_slice(&opcode.to_le_bytes());
        buf.push(0);
        Self { buf, arg_count: 0 }
    }

    pub fn arg(&mut self, value: NativeValueRef<'_>) -> Result<&mut Self, NativeViewCodecError> {
        if self.arg_count >= MAX_NATIVE_CMD_ARGS {
            return Err(NativeViewCodecError::TooManyFields(self.arg_count + 1));
        }
        write_value(&mut self.buf, value)?;
        self.arg_count += 1;
        Ok(self)
    }

    #[inline]
    pub fn arg_u32(&mut self, v: u32) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::U32(v))
    }

    #[inline]
    pub fn arg_u64(&mut self, v: u64) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::U64(v))
    }

    #[inline]
    pub fn arg_f32(&mut self, v: f32) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::F32(v))
    }

    #[inline]
    pub fn arg_str(&mut self, s: &str) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::Str(s))
    }

    #[inline]
    pub fn arg_bool(&mut self, v: bool) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::Bool(v))
    }

    #[inline]
    pub fn arg_bytes(&mut self, b: &[u8]) -> Result<&mut Self, NativeViewCodecError> {
        self.arg(NativeValueRef::Bytes(b))
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.buf[7] = self.arg_count as u8;
        self.buf
    }
}

impl NativeCommandSink for NativeCommandEncoder {
    #[inline]
    fn arg(&mut self, value: NativeValueRef<'_>) -> Result<&mut Self, NativeViewCodecError> {
        NativeCommandEncoder::arg(self, value)
    }
}

/// Validated zero-copy decoder for binary `NVNC` imperative command payloads.
#[derive(Debug, Clone)]
pub struct NativeCommandDecoder<'a> {
    opcode: u16,
    args: Vec<NativeValueRef<'a>>,
}

impl<'a> NativeCommandDecoder<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, NativeViewCodecError> {
        if bytes.len() < NATIVE_CMD_HEADER_SIZE {
            return Err(NativeViewCodecError::Truncated);
        }
        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != NATIVE_CMD_MAGIC {
            return Err(NativeViewCodecError::InvalidMagic(magic));
        }
        let version = bytes[4];
        if version != NATIVE_CMD_VERSION {
            return Err(NativeViewCodecError::UnsupportedVersion(version));
        }
        let opcode = u16::from_le_bytes([bytes[5], bytes[6]]);
        let arg_count = usize::from(bytes[7]);
        if arg_count > MAX_NATIVE_CMD_ARGS {
            return Err(NativeViewCodecError::TooManyFields(arg_count));
        }
        let mut pos = NATIVE_CMD_HEADER_SIZE;
        if arg_count > bytes.len() - pos {
            return Err(NativeViewCodecError::Truncated);
        }
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            args.push(read_value(bytes, &mut pos)?);
        }
        if pos != bytes.len() {
            return Err(NativeViewCodecError::TrailingBytes(bytes.len() - pos));
        }
        Ok(Self { opcode, args })
    }

    #[inline]
    pub fn opcode(&self) -> u16 {
        self.opcode
    }

    #[inline]
    pub fn arg_count(&self) -> usize {
        self.args.len()
    }

    #[inline]
    pub fn expect_arg_count(&self, expected: usize) -> Result<(), NativeViewCodecError> {
        if self.args.len() != expected {
            return Err(NativeViewCodecError::UnexpectedArgCount {
                expected,
                actual: self.args.len(),
            });
        }
        Ok(())
    }

    #[inline]
    pub fn require(&self, idx: usize) -> Result<NativeValueRef<'a>, NativeViewCodecError> {
        self.args
            .get(idx)
            .copied()
            .ok_or(NativeViewCodecError::MissingArg(idx))
    }

    #[inline]
    pub fn require_str(&self, idx: usize) -> Result<&'a str, NativeViewCodecError> {
        self.require(idx)?.as_str()
    }

    #[inline]
    pub fn require_u32(&self, idx: usize) -> Result<u32, NativeViewCodecError> {
        self.require(idx)?.as_u32()
    }

    #[inline]
    pub fn require_u64(&self, idx: usize) -> Result<u64, NativeViewCodecError> {
        self.require(idx)?.as_u64()
    }

    #[inline]
    pub fn require_f32(&self, idx: usize) -> Result<f32, NativeViewCodecError> {
        self.require(idx)?.as_f32()
    }
}

impl NativeCommandSource for NativeCommandDecoder<'_> {
    #[inline]
    fn opcode(&self) -> u16 {
        self.opcode
    }

    #[inline]
    fn arg_count(&self) -> usize {
        self.args.len()
    }

    #[inline]
    fn get_arg(&self, idx: usize) -> Option<NativeValueRef<'_>> {
        self.args.get(idx).copied()
    }
}
