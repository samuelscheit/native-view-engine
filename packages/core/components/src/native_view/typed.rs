//! Zero-serialization in-process native view prop & command representations
//! (`NativePropValue`, `NativeProps`, `NativeCommand`, `TypedNativeViewProps`,
//! `TypedNativeViewCommand`) shared across `nve-dioxus`, `nve-interop`, `nve-camera`,
//! `nve-backend-headless`, `nve-backend-skia`, and `nve-backend-web`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use super::codec::{
    NativeCommandDecoder, NativeCommandEncoder, NativeCommandSink, NativeCommandSource,
    NativePropDecoder, NativePropEncoder, NativePropSink, NativePropSource,
};
use super::wire::{
    NativeValueRef, NativeViewCodecError, MAX_NATIVE_CMD_ARGS, MAX_NATIVE_PROP_FIELDS,
};

/// Owned typed native view prop/command value (zero binary serialization in-process).
#[derive(Debug, Clone, PartialEq)]
pub enum NativePropValue {
    Null,
    Bool(bool),
    U32(u32),
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
    Str(Arc<str>),
    Bytes(Arc<[u8]>),
}

impl NativePropValue {
    #[inline]
    pub fn from_ref(v: NativeValueRef<'_>) -> Self {
        match v {
            NativeValueRef::Null => Self::Null,
            NativeValueRef::Bool(b) => Self::Bool(b),
            NativeValueRef::U32(n) => Self::U32(n),
            NativeValueRef::I64(n) => Self::I64(n),
            NativeValueRef::U64(n) => Self::U64(n),
            NativeValueRef::F32(n) => Self::F32(n),
            NativeValueRef::F64(n) => Self::F64(n),
            NativeValueRef::Str(s) => Self::Str(Arc::from(s)),
            NativeValueRef::Bytes(b) => Self::Bytes(Arc::from(b)),
        }
    }

    #[inline]
    pub fn as_ref(&self) -> NativeValueRef<'_> {
        match self {
            Self::Null => NativeValueRef::Null,
            Self::Bool(b) => NativeValueRef::Bool(*b),
            Self::U32(n) => NativeValueRef::U32(*n),
            Self::I64(n) => NativeValueRef::I64(*n),
            Self::U64(n) => NativeValueRef::U64(*n),
            Self::F32(n) => NativeValueRef::F32(*n),
            Self::F64(n) => NativeValueRef::F64(*n),
            Self::Str(s) => NativeValueRef::Str(s.as_ref()),
            Self::Bytes(b) => NativeValueRef::Bytes(b.as_ref()),
        }
    }
}

/// One typed numeric-ID field inside an immutable [`NativeProps`] slice.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeProp {
    pub id: u16,
    pub value: NativePropValue,
}

/// Immutable, `O(1)`-clonable zero-serialization native view prop bag (`Arc<[NativeProp]>`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NativeProps {
    fields: Arc<[NativeProp]>,
}

impl NativeProps {
    #[inline]
    pub fn empty() -> Self {
        Self::default()
    }

    #[inline]
    pub fn from_vec(fields: Vec<NativeProp>) -> Self {
        Self {
            fields: Arc::from(fields),
        }
    }

    #[inline]
    pub fn builder() -> NativePropsBuilder {
        NativePropsBuilder::new()
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
    pub fn fields(&self) -> &[NativeProp] {
        &self.fields
    }

    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.fields, &other.fields)
    }

    /// Encodes these typed props into cross-language `NVNP` binary wire bytes on demand.
    pub fn to_nvnp_bytes(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        let mut enc = NativePropEncoder::with_capacity(self.fields.len() * 10 + 16);
        for f in self.fields.iter() {
            enc.field(f.id, f.value.as_ref())?;
        }
        Ok(enc.finish())
    }

    /// Decodes cross-language `NVNP` binary wire bytes into an owned [`NativeProps`].
    pub fn from_nvnp_bytes(bytes: &[u8]) -> Result<Self, NativeViewCodecError> {
        let dec = NativePropDecoder::decode(bytes)?;
        let fields = dec
            .fields()
            .iter()
            .map(|&(id, v)| NativeProp {
                id,
                value: NativePropValue::from_ref(v),
            })
            .collect();
        Ok(Self::from_vec(fields))
    }
}

impl NativePropSource for NativeProps {
    #[inline]
    fn get_field(&self, field_id: u16) -> Option<NativeValueRef<'_>> {
        self.fields
            .iter()
            .rev()
            .find(|f| f.id == field_id)
            .map(|f| f.value.as_ref())
    }
}

/// Fluent zero-serialization builder producing an immutable [`NativeProps`].
#[derive(Debug, Clone, Default)]
pub struct NativePropsBuilder {
    fields: Vec<NativeProp>,
}

impl NativePropsBuilder {
    #[inline]
    pub fn new() -> Self {
        Self {
            fields: Vec::with_capacity(8),
        }
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            fields: Vec::with_capacity(capacity),
        }
    }

    #[inline]
    pub fn finish(self) -> NativeProps {
        NativeProps::from_vec(self.fields)
    }
}

impl NativePropSink for NativePropsBuilder {
    fn field(
        &mut self,
        field_id: u16,
        value: NativeValueRef<'_>,
    ) -> Result<&mut Self, NativeViewCodecError> {
        if self.fields.len() >= MAX_NATIVE_PROP_FIELDS {
            return Err(NativeViewCodecError::TooManyFields(self.fields.len() + 1));
        }
        self.fields.push(NativeProp {
            id: field_id,
            value: NativePropValue::from_ref(value),
        });
        Ok(self)
    }
}

/// Immutable, `O(1)`-clonable zero-serialization imperative native view command.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeCommand {
    opcode: u16,
    command_name: &'static str,
    args: Arc<[NativePropValue]>,
}

impl NativeCommand {
    #[inline]
    pub fn new(opcode: u16, command_name: &'static str, args: Vec<NativePropValue>) -> Self {
        Self {
            opcode,
            command_name,
            args: Arc::from(args),
        }
    }

    #[inline]
    pub fn builder(opcode: u16, command_name: &'static str) -> NativeCommandBuilder {
        NativeCommandBuilder::new(opcode, command_name)
    }

    #[inline]
    pub fn opcode(&self) -> u16 {
        self.opcode
    }

    #[inline]
    pub fn command_name(&self) -> &'static str {
        self.command_name
    }

    #[inline]
    pub fn args(&self) -> &[NativePropValue] {
        &self.args
    }

    /// Encodes this typed command into cross-language `NVNC` binary wire bytes on demand.
    pub fn to_nvnc_bytes(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        let mut enc = NativeCommandEncoder::new(self.opcode);
        for arg in self.args.iter() {
            enc.arg(arg.as_ref())?;
        }
        Ok(enc.finish())
    }

    /// Decodes cross-language `NVNC` binary wire bytes into an owned [`NativeCommand`].
    pub fn from_nvnc_bytes(
        command_name: &'static str,
        bytes: &[u8],
    ) -> Result<Self, NativeViewCodecError> {
        let dec = NativeCommandDecoder::decode(bytes)?;
        let mut args = Vec::with_capacity(dec.arg_count());
        for i in 0..dec.arg_count() {
            args.push(NativePropValue::from_ref(dec.require(i)?));
        }
        Ok(Self::new(dec.opcode(), command_name, args))
    }
}

impl NativeCommandSource for NativeCommand {
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
        self.args.get(idx).map(NativePropValue::as_ref)
    }
}

/// Fluent zero-serialization builder producing an immutable [`NativeCommand`].
#[derive(Debug, Clone)]
pub struct NativeCommandBuilder {
    opcode: u16,
    command_name: &'static str,
    args: Vec<NativePropValue>,
}

impl NativeCommandBuilder {
    #[inline]
    pub fn new(opcode: u16, command_name: &'static str) -> Self {
        Self {
            opcode,
            command_name,
            args: Vec::with_capacity(4),
        }
    }

    #[inline]
    pub fn finish(self) -> NativeCommand {
        NativeCommand::new(self.opcode, self.command_name, self.args)
    }
}

impl NativeCommandSink for NativeCommandBuilder {
    fn arg(&mut self, value: NativeValueRef<'_>) -> Result<&mut Self, NativeViewCodecError> {
        if self.args.len() >= MAX_NATIVE_CMD_ARGS {
            return Err(NativeViewCodecError::TooManyFields(self.args.len() + 1));
        }
        self.args.push(NativePropValue::from_ref(value));
        Ok(self)
    }
}

/// Zero-serialization in-process native view props carrier holding both the canonical typed
/// [`NativeProps`] field slice and an optional concrete `Arc<T>` payload for 0-conversion
/// downcast in Rust backends (`HeadlessBackend`, `SkiaBackend`, `WebBackend`).
#[derive(Clone)]
pub struct TypedNativeViewProps {
    kind: Arc<str>,
    props: NativeProps,
    typed: Option<Arc<dyn Any + Send + Sync>>,
}

impl TypedNativeViewProps {
    #[inline]
    pub fn new(kind: impl Into<Arc<str>>, props: NativeProps) -> Self {
        Self {
            kind: kind.into(),
            props,
            typed: None,
        }
    }

    #[inline]
    pub fn with_typed<T: Any + Send + Sync>(
        kind: impl Into<Arc<str>>,
        props: NativeProps,
        typed: Arc<T>,
    ) -> Self {
        Self {
            kind: kind.into(),
            props,
            typed: Some(typed),
        }
    }

    /// Zero-serialization constructor directly from a [`super::specs::NativeViewSpec`] props value.
    #[inline]
    pub fn from_spec<S: super::specs::NativeViewSpec>(
        props: S::Props,
    ) -> Result<Self, NativeViewCodecError> {
        use super::specs::EncodeNativeProps;
        let kind: Arc<str> = Arc::from(S::resolved_kind(&props).as_ref());
        let native = props.to_native_props()?;
        Ok(Self::with_typed(kind, native, Arc::new(props)))
    }

    #[inline]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    #[inline]
    pub fn props(&self) -> &NativeProps {
        &self.props
    }

    #[inline]
    pub fn downcast_ref<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.typed.as_deref().and_then(|a| a.downcast_ref::<T>())
    }

    /// Encodes to cross-language `NVNP` bytes only when crossing an FFI wire boundary.
    #[inline]
    pub fn to_nvnp_bytes(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        self.props.to_nvnp_bytes()
    }
}

impl PartialEq for TypedNativeViewProps {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && (self.props.ptr_eq(&other.props) || self.props == other.props)
    }
}

impl fmt::Debug for TypedNativeViewProps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypedNativeViewProps")
            .field("kind", &self.kind)
            .field("props", &self.props)
            .field("has_typed", &self.typed.is_some())
            .finish()
    }
}

/// Zero-serialization in-process imperative native command carrying both the typed
/// [`NativeCommand`] arguments and an optional concrete `Arc<C>` payload.
#[derive(Clone)]
pub struct TypedNativeViewCommand {
    command: NativeCommand,
    typed: Option<Arc<dyn Any + Send + Sync>>,
}

impl TypedNativeViewCommand {
    #[inline]
    pub fn new(command: NativeCommand) -> Self {
        Self {
            command,
            typed: None,
        }
    }

    #[inline]
    pub fn with_typed<C: Any + Send + Sync>(command: NativeCommand, typed: Arc<C>) -> Self {
        Self {
            command,
            typed: Some(typed),
        }
    }

    /// Zero-serialization constructor directly from a [`super::specs::NativeViewSpec`] command.
    #[inline]
    pub fn from_spec<S: super::specs::NativeViewSpec>(
        cmd: S::Command,
    ) -> Result<Self, NativeViewCodecError> {
        use super::specs::EncodeNativeCommand;
        let native = cmd.to_native_command()?;
        Ok(Self::with_typed(native, Arc::new(cmd)))
    }

    #[inline]
    pub fn command(&self) -> &NativeCommand {
        &self.command
    }

    #[inline]
    pub fn command_name(&self) -> &'static str {
        self.command.command_name()
    }

    #[inline]
    pub fn downcast_ref<C: Any + Send + Sync>(&self) -> Option<&C> {
        self.typed.as_deref().and_then(|a| a.downcast_ref::<C>())
    }

    #[inline]
    pub fn to_nvnc_bytes(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        self.command.to_nvnc_bytes()
    }
}

impl PartialEq for TypedNativeViewCommand {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.command == other.command
    }
}

impl fmt::Debug for TypedNativeViewCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypedNativeViewCommand")
            .field("command", &self.command)
            .field("has_typed", &self.typed.is_some())
            .finish()
    }
}
