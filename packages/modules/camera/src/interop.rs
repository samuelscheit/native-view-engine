//! Optional `nve-interop` [`NativeViewSpec`] trait implementations for [`CameraPreview`],
//! [`CameraPreviewProps`], and [`CameraPreviewCommand`].
//!
//! Delegates directly to the canonical zero-JSON `NVNP`/`NVNC` codecs in [`crate::spec`]
//! without duplicating wire layout or validation logic.

use nve_interop::native_view::{
    DecodeNativeCommand, DecodeNativeProps, EncodeNativeCommand, EncodeNativeProps,
    NativeCommandDecoder, NativeCommandEncoder, NativePropDecoder, NativePropEncoder,
    NativeViewCodecError, NativeViewSpec,
};

use crate::spec::{
    CameraPreview, CameraPreviewCommand, CameraPreviewProps, CAMERA_PREVIEW_KIND,
};
use crate::wire::CameraCodecError;

/// Maps a [`CameraCodecError`] onto [`NativeViewCodecError`] losslessly.
pub fn map_codec_error(err: CameraCodecError) -> NativeViewCodecError {
    match err {
        CameraCodecError::Truncated => NativeViewCodecError::Truncated,
        CameraCodecError::InvalidMagic(m) => NativeViewCodecError::InvalidMagic(m),
        CameraCodecError::UnsupportedVersion(v) => NativeViewCodecError::UnsupportedVersion(v),
        CameraCodecError::InvalidTag(t) => NativeViewCodecError::InvalidTag(t),
        CameraCodecError::InvalidBool(b) => NativeViewCodecError::InvalidBool(b),
        CameraCodecError::InvalidEnum(ty, v) => NativeViewCodecError::InvalidEnum(ty, v),
        CameraCodecError::InvalidUtf8 => NativeViewCodecError::InvalidUtf8,
        CameraCodecError::MissingField(id) => NativeViewCodecError::MissingField(id),
        CameraCodecError::MissingArg(idx) => NativeViewCodecError::MissingArg(idx),
        CameraCodecError::UnexpectedArgCount { expected, actual } => {
            NativeViewCodecError::UnexpectedArgCount { expected, actual }
        }
        CameraCodecError::TypeMismatch { expected } => {
            NativeViewCodecError::TypeMismatch { expected }
        }
        CameraCodecError::TrailingBytes(n) => NativeViewCodecError::TrailingBytes(n),
        CameraCodecError::UnknownCommandOpcode(op) => {
            NativeViewCodecError::UnknownCommandOpcode(op)
        }
        CameraCodecError::TooManyFields(n) => NativeViewCodecError::TooManyFields(n),
        CameraCodecError::PayloadTooLarge => NativeViewCodecError::PayloadTooLarge,
    }
}

impl From<CameraCodecError> for NativeViewCodecError {
    #[inline]
    fn from(err: CameraCodecError) -> Self {
        map_codec_error(err)
    }
}

impl EncodeNativeProps for CameraPreviewProps {
    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError> {
        let bytes = CameraPreviewProps::to_prop_bytes(self)?;
        let dec = NativePropDecoder::decode(&bytes)?;
        for &(field_id, value) in dec.fields() {
            enc.field(field_id, value)?;
        }
        Ok(())
    }

    fn to_prop_bytes(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        Ok(CameraPreviewProps::to_prop_bytes(self)?)
    }
}

impl DecodeNativeProps for CameraPreviewProps {
    fn decode_props(dec: &NativePropDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        let mut enc = NativePropEncoder::with_capacity(64);
        for &(field_id, value) in dec.fields() {
            enc.field(field_id, value)?;
        }
        Ok(CameraPreviewProps::from_prop_bytes(&enc.finish())?)
    }

    fn from_prop_bytes(bytes: &[u8]) -> Result<Self, NativeViewCodecError> {
        Ok(CameraPreviewProps::from_prop_bytes(bytes)?)
    }
}

impl EncodeNativeCommand for CameraPreviewCommand {
    #[inline]
    fn command_name(&self) -> &'static str {
        CameraPreviewCommand::command_name(self)
    }

    fn encode_command(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        Ok(CameraPreviewCommand::encode_command(self)?)
    }
}

impl DecodeNativeCommand for CameraPreviewCommand {
    fn decode_command(dec: &NativeCommandDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        let mut enc = NativeCommandEncoder::new(dec.opcode());
        for idx in 0..dec.arg_count() {
            enc.arg(dec.require(idx)?)?;
        }
        Ok(CameraPreviewCommand::from_command_bytes(&enc.finish())?)
    }

    fn from_command_bytes(bytes: &[u8]) -> Result<Self, NativeViewCodecError> {
        Ok(CameraPreviewCommand::from_command_bytes(bytes)?)
    }
}

impl NativeViewSpec for CameraPreview {
    const KIND: &'static str = CAMERA_PREVIEW_KIND;
    type Props = CameraPreviewProps;
    type Command = CameraPreviewCommand;
}
