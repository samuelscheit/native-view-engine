//! Typed zero-JSON `CameraPreview` (`"nve.camera.preview"`) [`NativeViewSpec`] and
//! 40-byte little-endian zero-copy [`CameraFrameDescriptor`] (`NVCF`) wire codec.
//!
//! Enum discriminants align directly with `nve-sdk::camera` (`SERVICE_CAMERA = 0x0023`):
//! - [`CameraPreviewFacing`]: `Back = 1`, `Front = 2`, `External = 3` (matches `CameraFacing`).
//! - [`TorchMode`] / photo flash: `Off = 0`, `On = 1`, `Auto = 2` (matches `FlashMode`).

use std::mem::{offset_of, size_of};

use super::codec::{NativeCommandDecoder, NativePropDecoder, NativePropEncoder};
use super::handle::{
    cmd_encoder, DecodeNativeCommand, DecodeNativeProps, EncodeNativeCommand, EncodeNativeProps,
    NativeViewSpec,
};
use super::wire::{NativeValueRef, NativeViewCodecError};

/// Canonical native view kind string for [`CameraPreview`].
pub const CAMERA_PREVIEW_KIND: &str = "nve.camera.preview";
/// Magic header for zero-copy binary camera frame descriptors (`"NVCF"`).
pub const CAMERA_FRAME_MAGIC: [u8; 4] = *b"NVCF";
/// Current binary version for [`CameraFrameDescriptor`] (`NVCF`).
pub const CAMERA_FRAME_VERSION: u8 = 1;
/// Exact little-endian wire size in bytes of a [`CameraFrameDescriptor`] (`NVCF`) record.
pub const CAMERA_FRAME_DESCRIPTOR_SIZE: usize = 40;
/// Maximum permitted camera preview FPS (aligned with `nve-sdk::camera::CameraPreviewConfig`).
pub const MAX_CAMERA_FPS: u16 = 240;

// =============================================================================
// Enums
// =============================================================================

/// Physical lens facing direction for [`CameraPreview`] (`1..=3`, aligned with `nve-sdk`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CameraPreviewFacing {
    #[default]
    Back = 1,
    Front = 2,
    External = 3,
}

impl CameraPreviewFacing {
    #[inline]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v { 1 => Some(Self::Back), 2 => Some(Self::Front), 3 => Some(Self::External), _ => None }
    }
    #[inline]
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        u8::try_from(v).ok().and_then(Self::from_u8)
            .ok_or(NativeViewCodecError::InvalidEnum("CameraPreviewFacing", v))
    }
}

/// Continuous torch / flash illumination mode (`0..=2`, aligned with `nve_sdk::camera::FlashMode`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TorchMode { #[default] Off = 0, On = 1, Auto = 2 }

impl TorchMode {
    #[inline]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v { 0 => Some(Self::Off), 1 => Some(Self::On), 2 => Some(Self::Auto), _ => None }
    }
    #[inline]
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        u8::try_from(v).ok().and_then(Self::from_u8)
            .ok_or(NativeViewCodecError::InvalidEnum("TorchMode", v))
    }
}

/// Hardware video stabilization preset for live camera sessions.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StabilizationMode { #[default] Off = 0, Standard = 1, Cinematic = 2 }

impl StabilizationMode {
    #[inline]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v { 0 => Some(Self::Off), 1 => Some(Self::Standard), 2 => Some(Self::Cinematic), _ => None }
    }
    #[inline]
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        u8::try_from(v).ok().and_then(Self::from_u8)
            .ok_or(NativeViewCodecError::InvalidEnum("StabilizationMode", v))
    }
}

/// Pixel buffer memory layout carried by a zero-copy [`CameraFrameDescriptor`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CameraPixelFormat { #[default] Rgba8888 = 0, Bgra8888 = 1, Nv12 = 2, Yuv420_888 = 3 }

impl CameraPixelFormat {
    #[inline]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v { 0 => Some(Self::Rgba8888), 1 => Some(Self::Bgra8888), 2 => Some(Self::Nv12), 3 => Some(Self::Yuv420_888), _ => None }
    }
    #[inline]
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        u8::try_from(v).ok().and_then(Self::from_u8)
            .ok_or(NativeViewCodecError::InvalidEnum("CameraPixelFormat", v))
    }
    /// Minimum primary-plane row stride in bytes for a frame of `width` pixels.
    #[inline]
    pub const fn min_stride_bytes(self, width: u32) -> Option<u32> {
        width.checked_mul(match self { Self::Rgba8888 | Self::Bgra8888 => 4, Self::Nv12 | Self::Yuv420_888 => 1 })
    }
}

// =============================================================================
// CameraPreviewProps (`NVNP`)
// =============================================================================

/// Typed declarative props for [`CameraPreview`] (`KIND = "nve.camera.preview"`).
#[derive(Debug, Clone, PartialEq)]
pub struct CameraPreviewProps {
    pub device_id: Option<String>,
    pub facing: CameraPreviewFacing,
    pub torch: TorchMode,
    pub zoom: f32,
    pub fps: u16,
    pub hdr: bool,
    pub stabilization: StabilizationMode,
    pub enable_frame_processor: bool,
    pub audio_enabled: bool,
    pub active: bool,
}

impl Default for CameraPreviewProps {
    fn default() -> Self {
        Self {
            device_id: None,
            facing: CameraPreviewFacing::Back,
            torch: TorchMode::Off,
            zoom: 1.0,
            fps: 30,
            hdr: false,
            stabilization: StabilizationMode::Off,
            enable_frame_processor: false,
            audio_enabled: false,
            active: true,
        }
    }
}

impl CameraPreviewProps {
    pub const FIELD_DEVICE_ID: u16 = 1;
    pub const FIELD_FACING: u16 = 2;
    pub const FIELD_TORCH: u16 = 3;
    pub const FIELD_ZOOM: u16 = 4;
    pub const FIELD_FPS: u16 = 5;
    pub const FIELD_HDR: u16 = 6;
    pub const FIELD_STABILIZATION: u16 = 7;
    pub const FIELD_ENABLE_FRAME_PROCESSOR: u16 = 8;
    pub const FIELD_AUDIO_ENABLED: u16 = 9;
    pub const FIELD_ACTIVE: u16 = 10;

    pub fn new() -> Self { Self::default() }
    pub fn with_device_id(mut self, id: impl Into<String>) -> Self { self.device_id = Some(id.into()); self }
    pub const fn with_facing(mut self, facing: CameraPreviewFacing) -> Self { self.facing = facing; self }
    pub const fn with_torch(mut self, torch: TorchMode) -> Self { self.torch = torch; self }
    pub const fn with_zoom(mut self, zoom: f32) -> Self { self.zoom = zoom; self }
    pub const fn with_fps(mut self, fps: u16) -> Self { self.fps = fps; self }
    pub const fn with_hdr(mut self, hdr: bool) -> Self { self.hdr = hdr; self }
    pub const fn with_stabilization(mut self, mode: StabilizationMode) -> Self { self.stabilization = mode; self }
    pub const fn with_frame_processor(mut self, enabled: bool) -> Self { self.enable_frame_processor = enabled; self }
    pub const fn with_audio(mut self, enabled: bool) -> Self { self.audio_enabled = enabled; self }
    pub const fn with_active(mut self, active: bool) -> Self { self.active = active; self }

    /// Validates numeric invariants (`zoom` finite & positive, `fps` in `1..=240`).
    pub fn validate(&self) -> Result<(), NativeViewCodecError> {
        validate_zoom(self.zoom)?;
        validate_fps(u32::from(self.fps))?;
        Ok(())
    }
}

impl EncodeNativeProps for CameraPreviewProps {
    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError> {
        self.validate()?;
        match self.device_id.as_deref() {
            Some(id) => enc.str_field(Self::FIELD_DEVICE_ID, id)?,
            None => enc.field(Self::FIELD_DEVICE_ID, NativeValueRef::Null)?,
        };
        enc.u32_field(Self::FIELD_FACING, self.facing as u32)?
            .u32_field(Self::FIELD_TORCH, self.torch as u32)?
            .f32_field(Self::FIELD_ZOOM, self.zoom)?
            .u32_field(Self::FIELD_FPS, u32::from(self.fps))?
            .bool_field(Self::FIELD_HDR, self.hdr)?
            .u32_field(Self::FIELD_STABILIZATION, self.stabilization as u32)?
            .bool_field(Self::FIELD_ENABLE_FRAME_PROCESSOR, self.enable_frame_processor)?
            .bool_field(Self::FIELD_AUDIO_ENABLED, self.audio_enabled)?
            .bool_field(Self::FIELD_ACTIVE, self.active)?;
        Ok(())
    }
}

impl DecodeNativeProps for CameraPreviewProps {
    fn decode_props(dec: &NativePropDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        let device_id = match dec.require(Self::FIELD_DEVICE_ID)? {
            NativeValueRef::Null => None,
            NativeValueRef::Str(s) => Some(s.to_owned()),
            _ => return Err(NativeViewCodecError::TypeMismatch { expected: "Option<str>" }),
        };
        let facing = CameraPreviewFacing::from_u32(dec.require_u32(Self::FIELD_FACING)?)?;
        let torch = TorchMode::from_u32(dec.require_u32(Self::FIELD_TORCH)?)?;
        let zoom = dec.require_f32(Self::FIELD_ZOOM)?;
        validate_zoom(zoom)?;
        let fps = validate_fps(dec.require_u32(Self::FIELD_FPS)?)?;
        let hdr = dec.require_bool(Self::FIELD_HDR)?;
        let stabilization = StabilizationMode::from_u32(dec.require_u32(Self::FIELD_STABILIZATION)?)?;
        let enable_frame_processor = dec.require_bool(Self::FIELD_ENABLE_FRAME_PROCESSOR)?;
        let audio_enabled = dec.require_bool(Self::FIELD_AUDIO_ENABLED)?;
        let active = dec.require_bool(Self::FIELD_ACTIVE)?;
        Ok(Self {
            device_id, facing, torch, zoom, fps, hdr, stabilization,
            enable_frame_processor, audio_enabled, active,
        })
    }
}

// =============================================================================
// CameraPreviewCommand (`NVNC`)
// =============================================================================

/// Typed imperative commands for [`CameraPreview`].
#[derive(Debug, Clone, PartialEq)]
pub enum CameraPreviewCommand {
    SwitchFacing(CameraPreviewFacing),
    SetTorch(TorchMode),
    SetZoom(f32),
    FocusPoint { x: f32, y: f32 },
    CapturePhoto { quality: u8, flash: u8 },
    StartRecording { output_path: String },
    StopRecording,
}

impl CameraPreviewCommand {
    pub const OP_SWITCH_FACING: u16 = 1;
    pub const OP_SET_TORCH: u16 = 2;
    pub const OP_SET_ZOOM: u16 = 3;
    pub const OP_FOCUS_POINT: u16 = 4;
    pub const OP_CAPTURE_PHOTO: u16 = 5;
    pub const OP_START_RECORDING: u16 = 6;
    pub const OP_STOP_RECORDING: u16 = 7;

    /// Validates command argument invariants before encoding or after decoding.
    pub fn validate(&self) -> Result<(), NativeViewCodecError> {
        match self {
            Self::SwitchFacing(_) | Self::SetTorch(_) | Self::StartRecording { .. } | Self::StopRecording => Ok(()),
            Self::SetZoom(zoom) => validate_zoom(*zoom),
            Self::FocusPoint { x, y } => {
                validate_unit_coord(*x, "focus_point.x in 0.0..=1.0")?;
                validate_unit_coord(*y, "focus_point.y in 0.0..=1.0")
            }
            Self::CapturePhoto { quality, flash } => {
                validate_quality(u32::from(*quality))?;
                validate_flash(u32::from(*flash))?;
                Ok(())
            }
        }
    }
}

impl EncodeNativeCommand for CameraPreviewCommand {
    fn command_name(&self) -> &'static str {
        match self {
            Self::SwitchFacing(_) => "switch_facing",
            Self::SetTorch(_) => "set_torch",
            Self::SetZoom(_) => "set_zoom",
            Self::FocusPoint { .. } => "focus_point",
            Self::CapturePhoto { .. } => "capture_photo",
            Self::StartRecording { .. } => "start_recording",
            Self::StopRecording => "stop_recording",
        }
    }

    fn encode_command(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        self.validate()?;
        match self {
            Self::SwitchFacing(facing) => {
                let mut enc = cmd_encoder(Self::OP_SWITCH_FACING);
                enc.arg(NativeValueRef::U32(*facing as u32))?;
                Ok(enc.finish())
            }
            Self::SetTorch(mode) => {
                let mut enc = cmd_encoder(Self::OP_SET_TORCH);
                enc.arg(NativeValueRef::U32(*mode as u32))?;
                Ok(enc.finish())
            }
            Self::SetZoom(zoom) => {
                let mut enc = cmd_encoder(Self::OP_SET_ZOOM);
                enc.arg_f32(*zoom)?;
                Ok(enc.finish())
            }
            Self::FocusPoint { x, y } => {
                let mut enc = cmd_encoder(Self::OP_FOCUS_POINT);
                enc.arg_f32(*x)?.arg_f32(*y)?;
                Ok(enc.finish())
            }
            Self::CapturePhoto { quality, flash } => {
                let mut enc = cmd_encoder(Self::OP_CAPTURE_PHOTO);
                enc.arg(NativeValueRef::U32(u32::from(*quality)))?
                    .arg(NativeValueRef::U32(u32::from(*flash)))?;
                Ok(enc.finish())
            }
            Self::StartRecording { output_path } => {
                let mut enc = cmd_encoder(Self::OP_START_RECORDING);
                enc.arg_str(output_path)?;
                Ok(enc.finish())
            }
            Self::StopRecording => Ok(cmd_encoder(Self::OP_STOP_RECORDING).finish()),
        }
    }
}

impl DecodeNativeCommand for CameraPreviewCommand {
    fn decode_command(dec: &NativeCommandDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        match dec.opcode() {
            Self::OP_SWITCH_FACING => {
                dec.expect_arg_count(1)?;
                Ok(Self::SwitchFacing(CameraPreviewFacing::from_u32(dec.require(0)?.as_u32()?)?))
            }
            Self::OP_SET_TORCH => {
                dec.expect_arg_count(1)?;
                Ok(Self::SetTorch(TorchMode::from_u32(dec.require(0)?.as_u32()?)?))
            }
            Self::OP_SET_ZOOM => {
                dec.expect_arg_count(1)?;
                let zoom = dec.require_f32(0)?;
                validate_zoom(zoom)?;
                Ok(Self::SetZoom(zoom))
            }
            Self::OP_FOCUS_POINT => {
                dec.expect_arg_count(2)?;
                let (x, y) = (dec.require_f32(0)?, dec.require_f32(1)?);
                validate_unit_coord(x, "focus_point.x in 0.0..=1.0")?;
                validate_unit_coord(y, "focus_point.y in 0.0..=1.0")?;
                Ok(Self::FocusPoint { x, y })
            }
            Self::OP_CAPTURE_PHOTO => {
                dec.expect_arg_count(2)?;
                let quality = validate_quality(dec.require(0)?.as_u32()?)?;
                let flash = validate_flash(dec.require(1)?.as_u32()?)?;
                Ok(Self::CapturePhoto { quality, flash })
            }
            Self::OP_START_RECORDING => {
                dec.expect_arg_count(1)?;
                Ok(Self::StartRecording { output_path: dec.require_str(0)?.to_owned() })
            }
            Self::OP_STOP_RECORDING => {
                dec.expect_arg_count(0)?;
                Ok(Self::StopRecording)
            }
            other => Err(NativeViewCodecError::UnknownCommandOpcode(other)),
        }
    }
}

/// First-class pluggable native camera preview view (`KIND = "nve.camera.preview"`).
pub struct CameraPreview;

impl NativeViewSpec for CameraPreview {
    const KIND: &'static str = CAMERA_PREVIEW_KIND;
    type Props = CameraPreviewProps;
    type Command = CameraPreviewCommand;
}

// =============================================================================
// Zero-Copy Binary Frame Descriptor (`NVCF`, 40 bytes little-endian)
// =============================================================================

#[repr(C)]
struct CameraFrameWireLayout {
    magic: [u8; 4],        // 0..4
    version: u8,           // 4
    pixel_format: u8,      // 5
    rotation_deg: u16,     // 6..8
    width: u32,            // 8..12
    height: u32,           // 12..16
    stride_bytes: u32,     // 16..20
    reserved: u32,         // 20..24 (must be 0 in v1)
    timestamp_ns: u64,     // 24..32
    native_handle_id: u64, // 32..40
}

const _: () = {
    assert!(size_of::<CameraFrameWireLayout>() == CAMERA_FRAME_DESCRIPTOR_SIZE);
    assert!(offset_of!(CameraFrameWireLayout, magic) == 0);
    assert!(offset_of!(CameraFrameWireLayout, version) == 4);
    assert!(offset_of!(CameraFrameWireLayout, pixel_format) == 5);
    assert!(offset_of!(CameraFrameWireLayout, rotation_deg) == 6);
    assert!(offset_of!(CameraFrameWireLayout, width) == 8);
    assert!(offset_of!(CameraFrameWireLayout, height) == 12);
    assert!(offset_of!(CameraFrameWireLayout, stride_bytes) == 16);
    assert!(offset_of!(CameraFrameWireLayout, reserved) == 20);
    assert!(offset_of!(CameraFrameWireLayout, timestamp_ns) == 24);
    assert!(offset_of!(CameraFrameWireLayout, native_handle_id) == 32);
};

/// Zero-copy binary frame descriptor (`NVCF`, 40-byte little-endian) passed across the
/// host <-> Rust boundary for hardware frame processors (`CVPixelBuffer`, `AHardwareBuffer`,
/// GPU textures).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraFrameDescriptor {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub pixel_format: CameraPixelFormat,
    pub rotation_deg: u16,
    pub timestamp_ns: u64,
    pub native_handle_id: u64,
}

impl CameraFrameDescriptor {
    /// Validates dimensions, rotation (`0 | 90 | 180 | 270`), and minimum row stride.
    pub fn validate(&self) -> Result<(), NativeViewCodecError> {
        if self.width == 0 {
            return Err(NativeViewCodecError::TypeMismatch { expected: "non-zero frame width" });
        }
        if self.height == 0 {
            return Err(NativeViewCodecError::TypeMismatch { expected: "non-zero frame height" });
        }
        if !matches!(self.rotation_deg, 0 | 90 | 180 | 270) {
            return Err(NativeViewCodecError::InvalidEnum("CameraRotationDeg", u32::from(self.rotation_deg)));
        }
        let min_stride = self.pixel_format.min_stride_bytes(self.width)
            .ok_or(NativeViewCodecError::PayloadTooLarge)?;
        if self.stride_bytes < min_stride {
            return Err(NativeViewCodecError::TypeMismatch {
                expected: "stride_bytes >= width * bytes_per_pixel",
            });
        }
        Ok(())
    }

    /// Encodes into a fixed 40-byte stack buffer with zero heap allocation.
    pub fn encode(&self) -> Result<[u8; CAMERA_FRAME_DESCRIPTOR_SIZE], NativeViewCodecError> {
        let mut out = [0u8; CAMERA_FRAME_DESCRIPTOR_SIZE];
        self.encode_into(&mut out)?;
        Ok(out)
    }

    /// Encodes in-place into a 40-byte destination buffer.
    pub fn encode_into(&self, dst: &mut [u8; CAMERA_FRAME_DESCRIPTOR_SIZE]) -> Result<(), NativeViewCodecError> {
        self.validate()?;
        dst[0..4].copy_from_slice(&CAMERA_FRAME_MAGIC);
        dst[4] = CAMERA_FRAME_VERSION;
        dst[5] = self.pixel_format as u8;
        dst[6..8].copy_from_slice(&self.rotation_deg.to_le_bytes());
        dst[8..12].copy_from_slice(&self.width.to_le_bytes());
        dst[12..16].copy_from_slice(&self.height.to_le_bytes());
        dst[16..20].copy_from_slice(&self.stride_bytes.to_le_bytes());
        dst[20..24].copy_from_slice(&0u32.to_le_bytes());
        dst[24..32].copy_from_slice(&self.timestamp_ns.to_le_bytes());
        dst[32..40].copy_from_slice(&self.native_handle_id.to_le_bytes());
        Ok(())
    }

    /// Convenience helper returning an owned `Vec<u8>` of length 40.
    pub fn to_bytes(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        Ok(self.encode()?.to_vec())
    }

    /// Decodes and strictly validates a 40-byte little-endian `NVCF` frame descriptor.
    pub fn decode(bytes: &[u8]) -> Result<Self, NativeViewCodecError> {
        if bytes.len() < CAMERA_FRAME_DESCRIPTOR_SIZE {
            return Err(NativeViewCodecError::Truncated);
        }
        if bytes.len() > CAMERA_FRAME_DESCRIPTOR_SIZE {
            return Err(NativeViewCodecError::TrailingBytes(bytes.len() - CAMERA_FRAME_DESCRIPTOR_SIZE));
        }
        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != CAMERA_FRAME_MAGIC {
            return Err(NativeViewCodecError::InvalidMagic(magic));
        }
        if bytes[4] != CAMERA_FRAME_VERSION {
            return Err(NativeViewCodecError::UnsupportedVersion(bytes[4]));
        }
        let pixel_format = CameraPixelFormat::from_u32(u32::from(bytes[5]))?;
        let rotation_deg = u16::from_le_bytes([bytes[6], bytes[7]]);
        let width = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let height = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let stride_bytes = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let reserved = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        if reserved != 0 {
            return Err(NativeViewCodecError::InvalidEnum("CameraFrameReserved", reserved));
        }
        let desc = Self {
            width,
            height,
            stride_bytes,
            pixel_format,
            rotation_deg,
            timestamp_ns: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            native_handle_id: u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
        };
        desc.validate()?;
        Ok(desc)
    }
}

#[inline]
fn validate_zoom(zoom: f32) -> Result<(), NativeViewCodecError> {
    if zoom.is_finite() && zoom > 0.0 { Ok(()) }
    else { Err(NativeViewCodecError::TypeMismatch { expected: "finite positive zoom" }) }
}

#[inline]
fn validate_fps(fps: u32) -> Result<u16, NativeViewCodecError> {
    if (1..=u32::from(MAX_CAMERA_FPS)).contains(&fps) { Ok(fps as u16) }
    else { Err(NativeViewCodecError::InvalidEnum("CameraFps", fps)) }
}

#[inline]
fn validate_unit_coord(v: f32, expected: &'static str) -> Result<(), NativeViewCodecError> {
    if v.is_finite() && (0.0..=1.0).contains(&v) { Ok(()) }
    else { Err(NativeViewCodecError::TypeMismatch { expected }) }
}

#[inline]
fn validate_quality(q: u32) -> Result<u8, NativeViewCodecError> {
    if q <= 100 { Ok(q as u8) } else { Err(NativeViewCodecError::InvalidEnum("PhotoQuality", q)) }
}

#[inline]
fn validate_flash(f: u32) -> Result<u8, NativeViewCodecError> {
    if f <= 2 { Ok(f as u8) } else { Err(NativeViewCodecError::InvalidEnum("FlashMode", f)) }
}
