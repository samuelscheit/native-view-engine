//! Typed zero-JSON `CameraPreview` (`"nve.camera.preview"`), `CameraPreviewProps` (`NVNP`),
//! `CameraPreviewCommand` (`NVNC`), and 40-byte `#[repr(C)]` `CameraFrameDescriptor` (`NVCF`).

use std::mem::{offset_of, size_of};

use crate::types::CameraFacing;
use crate::wire::{CameraCodecError, CommandDecoder, CommandEncoder, NativeValueRef, PropDecoder, PropEncoder};

pub const CAMERA_PREVIEW_KIND: &str = "nve.camera.preview";
pub const CAMERA_FRAME_MAGIC: [u8; 4] = *b"NVCF";
pub const CAMERA_FRAME_VERSION: u8 = 1;
pub const CAMERA_FRAME_DESCRIPTOR_SIZE: usize = 40;
pub const MAX_CAMERA_FPS: u16 = 240;

#[inline]
fn facing_from_preview_u32(v: u32) -> Result<CameraFacing, CameraCodecError> {
    u8::try_from(v)
        .ok()
        .and_then(CameraFacing::from_preview_u8)
        .ok_or(CameraCodecError::InvalidEnum("CameraFacing", v))
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TorchMode {
    #[default]
    Off = 0,
    On = 1,
    Auto = 2,
}

impl TorchMode {
    #[inline]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Off),
            1 => Some(Self::On),
            2 => Some(Self::Auto),
            _ => None,
        }
    }

    #[inline]
    pub fn from_u32(v: u32) -> Result<Self, CameraCodecError> {
        u8::try_from(v)
            .ok()
            .and_then(Self::from_u8)
            .ok_or(CameraCodecError::InvalidEnum("TorchMode", v))
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum StabilizationMode {
    #[default]
    Off = 0,
    Standard = 1,
    Cinematic = 2,
}

impl StabilizationMode {
    #[inline]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Off),
            1 => Some(Self::Standard),
            2 => Some(Self::Cinematic),
            _ => None,
        }
    }

    #[inline]
    pub fn from_u32(v: u32) -> Result<Self, CameraCodecError> {
        u8::try_from(v)
            .ok()
            .and_then(Self::from_u8)
            .ok_or(CameraCodecError::InvalidEnum("StabilizationMode", v))
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CameraPixelFormat {
    #[default]
    Rgba8888 = 0,
    Bgra8888 = 1,
    Nv12 = 2,
    Yuv420_888 = 3,
}

impl CameraPixelFormat {
    #[inline]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Rgba8888),
            1 => Some(Self::Bgra8888),
            2 => Some(Self::Nv12),
            3 => Some(Self::Yuv420_888),
            _ => None,
        }
    }

    #[inline]
    pub fn from_u32(v: u32) -> Result<Self, CameraCodecError> {
        u8::try_from(v)
            .ok()
            .and_then(Self::from_u8)
            .ok_or(CameraCodecError::InvalidEnum("CameraPixelFormat", v))
    }

    #[inline]
    pub const fn min_stride_bytes(self, width: u32) -> Option<u32> {
        width.checked_mul(match self {
            Self::Rgba8888 | Self::Bgra8888 => 4,
            Self::Nv12 | Self::Yuv420_888 => 1,
        })
    }
}

/// Canonical marker type for the `"nve.camera.preview"` native view.
pub struct CameraPreview;

impl CameraPreview {
    pub const KIND: &'static str = CAMERA_PREVIEW_KIND;
}

/// Declarative zero-JSON props (`NVNP`) for [`CameraPreview`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CameraPreviewProps {
    pub device_id: Option<String>,
    pub facing: CameraFacing,
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
            facing: CameraFacing::Back,
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
    pub const fn with_facing(mut self, facing: CameraFacing) -> Self { self.facing = facing; self }
    pub const fn with_torch(mut self, torch: TorchMode) -> Self { self.torch = torch; self }
    pub const fn with_zoom(mut self, zoom: f32) -> Self { self.zoom = zoom; self }
    pub const fn with_fps(mut self, fps: u16) -> Self { self.fps = fps; self }
    pub const fn with_hdr(mut self, hdr: bool) -> Self { self.hdr = hdr; self }
    pub const fn with_stabilization(mut self, mode: StabilizationMode) -> Self { self.stabilization = mode; self }
    pub const fn with_frame_processor(mut self, enabled: bool) -> Self { self.enable_frame_processor = enabled; self }
    pub const fn with_audio(mut self, enabled: bool) -> Self { self.audio_enabled = enabled; self }
    pub const fn with_active(mut self, active: bool) -> Self { self.active = active; self }

    pub fn validate(&self) -> Result<(), CameraCodecError> {
        if self.facing == CameraFacing::Unknown {
            return Err(CameraCodecError::InvalidEnum("CameraFacing", 0));
        }
        validate_zoom(self.zoom)?;
        validate_fps(u32::from(self.fps))?;
        Ok(())
    }

    pub fn to_prop_bytes(&self) -> Result<Vec<u8>, CameraCodecError> {
        self.validate()?;
        let mut enc = PropEncoder::new();
        match self.device_id.as_deref() {
            Some(id) => enc.field(Self::FIELD_DEVICE_ID, NativeValueRef::Str(id))?,
            None => enc.field(Self::FIELD_DEVICE_ID, NativeValueRef::Null)?,
        };
        enc.field(Self::FIELD_FACING, NativeValueRef::U32(self.facing as u32))?
            .field(Self::FIELD_TORCH, NativeValueRef::U32(self.torch as u32))?
            .field(Self::FIELD_ZOOM, NativeValueRef::F32(self.zoom))?
            .field(Self::FIELD_FPS, NativeValueRef::U32(u32::from(self.fps)))?
            .field(Self::FIELD_HDR, NativeValueRef::Bool(self.hdr))?
            .field(Self::FIELD_STABILIZATION, NativeValueRef::U32(self.stabilization as u32))?
            .field(Self::FIELD_ENABLE_FRAME_PROCESSOR, NativeValueRef::Bool(self.enable_frame_processor))?
            .field(Self::FIELD_AUDIO_ENABLED, NativeValueRef::Bool(self.audio_enabled))?
            .field(Self::FIELD_ACTIVE, NativeValueRef::Bool(self.active))?;
        Ok(enc.finish())
    }

    pub fn from_prop_bytes(bytes: &[u8]) -> Result<Self, CameraCodecError> {
        let dec = PropDecoder::parse(bytes)?;
        let device_id = match dec.require(Self::FIELD_DEVICE_ID)? {
            NativeValueRef::Null => None,
            NativeValueRef::Str(s) => Some(s.to_owned()),
            _ => return Err(CameraCodecError::TypeMismatch { expected: "Option<str>" }),
        };
        let facing = facing_from_preview_u32(dec.require(Self::FIELD_FACING)?.as_u32()?)?;
        let torch = TorchMode::from_u32(dec.require(Self::FIELD_TORCH)?.as_u32()?)?;
        let zoom = dec.require(Self::FIELD_ZOOM)?.as_f32()?;
        validate_zoom(zoom)?;
        let fps = validate_fps(dec.require(Self::FIELD_FPS)?.as_u32()?)?;
        let hdr = dec.require(Self::FIELD_HDR)?.as_bool()?;
        let stabilization = StabilizationMode::from_u32(dec.require(Self::FIELD_STABILIZATION)?.as_u32()?)?;
        let enable_frame_processor = dec.require(Self::FIELD_ENABLE_FRAME_PROCESSOR)?.as_bool()?;
        let audio_enabled = dec.require(Self::FIELD_AUDIO_ENABLED)?.as_bool()?;
        let active = dec.require(Self::FIELD_ACTIVE)?.as_bool()?;
        Ok(Self {
            device_id,
            facing,
            torch,
            zoom,
            fps,
            hdr,
            stabilization,
            enable_frame_processor,
            audio_enabled,
            active,
        })
    }
}

/// Typed imperative commands (`NVNC`) for [`CameraPreview`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CameraPreviewCommand {
    SwitchFacing(CameraFacing),
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

    pub const fn command_name(&self) -> &'static str {
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

    pub fn validate(&self) -> Result<(), CameraCodecError> {
        match self {
            Self::SwitchFacing(CameraFacing::Unknown) => {
                Err(CameraCodecError::InvalidEnum("CameraFacing", 0))
            }
            Self::SwitchFacing(_)
            | Self::SetTorch(_)
            | Self::StartRecording { .. }
            | Self::StopRecording => Ok(()),
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

    pub fn encode_command(&self) -> Result<Vec<u8>, CameraCodecError> {
        self.validate()?;
        match self {
            Self::SwitchFacing(facing) => {
                let mut enc = CommandEncoder::new(Self::OP_SWITCH_FACING);
                enc.arg(NativeValueRef::U32(*facing as u32))?;
                Ok(enc.finish())
            }
            Self::SetTorch(mode) => {
                let mut enc = CommandEncoder::new(Self::OP_SET_TORCH);
                enc.arg(NativeValueRef::U32(*mode as u32))?;
                Ok(enc.finish())
            }
            Self::SetZoom(zoom) => {
                let mut enc = CommandEncoder::new(Self::OP_SET_ZOOM);
                enc.arg(NativeValueRef::F32(*zoom))?;
                Ok(enc.finish())
            }
            Self::FocusPoint { x, y } => {
                let mut enc = CommandEncoder::new(Self::OP_FOCUS_POINT);
                enc.arg(NativeValueRef::F32(*x))?.arg(NativeValueRef::F32(*y))?;
                Ok(enc.finish())
            }
            Self::CapturePhoto { quality, flash } => {
                let mut enc = CommandEncoder::new(Self::OP_CAPTURE_PHOTO);
                enc.arg(NativeValueRef::U32(u32::from(*quality)))?
                    .arg(NativeValueRef::U32(u32::from(*flash)))?;
                Ok(enc.finish())
            }
            Self::StartRecording { output_path } => {
                let mut enc = CommandEncoder::new(Self::OP_START_RECORDING);
                enc.arg(NativeValueRef::Str(output_path))?;
                Ok(enc.finish())
            }
            Self::StopRecording => Ok(CommandEncoder::new(Self::OP_STOP_RECORDING).finish()),
        }
    }

    pub fn from_command_bytes(bytes: &[u8]) -> Result<Self, CameraCodecError> {
        let dec = CommandDecoder::parse(bytes)?;
        match dec.opcode() {
            Self::OP_SWITCH_FACING => {
                dec.expect_arg_count(1)?;
                Ok(Self::SwitchFacing(facing_from_preview_u32(
                    dec.require(0)?.as_u32()?,
                )?))
            }
            Self::OP_SET_TORCH => {
                dec.expect_arg_count(1)?;
                Ok(Self::SetTorch(TorchMode::from_u32(dec.require(0)?.as_u32()?)?))
            }
            Self::OP_SET_ZOOM => {
                dec.expect_arg_count(1)?;
                let zoom = dec.require(0)?.as_f32()?;
                validate_zoom(zoom)?;
                Ok(Self::SetZoom(zoom))
            }
            Self::OP_FOCUS_POINT => {
                dec.expect_arg_count(2)?;
                let x = dec.require(0)?.as_f32()?;
                let y = dec.require(1)?.as_f32()?;
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
                Ok(Self::StartRecording {
                    output_path: dec.require(0)?.as_str()?.to_owned(),
                })
            }
            Self::OP_STOP_RECORDING => {
                dec.expect_arg_count(0)?;
                Ok(Self::StopRecording)
            }
            other => Err(CameraCodecError::UnknownCommandOpcode(other)),
        }
    }
}

/// 40-byte `#[repr(C)]` wire layout for [`CameraFrameDescriptor`] (`NVCF`), locked at compile time.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraFrameWireLayout {
    pub magic: [u8; 4],
    pub version: u8,
    pub pixel_format: u8,
    pub rotation_deg: u16,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub reserved: u32,
    pub timestamp_ns: u64,
    pub native_handle_id: u64,
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

/// Zero-copy 40-byte binary frame descriptor (`NVCF`) matching `packages/modules/camera/cxx` & `android`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    pub fn validate(&self) -> Result<(), CameraCodecError> {
        if self.width == 0 {
            return Err(CameraCodecError::TypeMismatch { expected: "non-zero frame width" });
        }
        if self.height == 0 {
            return Err(CameraCodecError::TypeMismatch { expected: "non-zero frame height" });
        }
        if !matches!(self.rotation_deg, 0 | 90 | 180 | 270) {
            return Err(CameraCodecError::InvalidEnum(
                "CameraRotationDeg",
                u32::from(self.rotation_deg),
            ));
        }
        let min_stride = self
            .pixel_format
            .min_stride_bytes(self.width)
            .ok_or(CameraCodecError::PayloadTooLarge)?;
        if self.stride_bytes < min_stride {
            return Err(CameraCodecError::TypeMismatch {
                expected: "stride_bytes >= width * bytes_per_pixel",
            });
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<[u8; CAMERA_FRAME_DESCRIPTOR_SIZE], CameraCodecError> {
        let mut out = [0u8; CAMERA_FRAME_DESCRIPTOR_SIZE];
        self.encode_into(&mut out)?;
        Ok(out)
    }

    pub fn encode_into(&self, dst: &mut [u8; CAMERA_FRAME_DESCRIPTOR_SIZE]) -> Result<(), CameraCodecError> {
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

    pub fn to_bytes(&self) -> Result<Vec<u8>, CameraCodecError> {
        Ok(self.encode()?.to_vec())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CameraCodecError> {
        if bytes.len() < CAMERA_FRAME_DESCRIPTOR_SIZE {
            return Err(CameraCodecError::Truncated);
        }
        if bytes.len() > CAMERA_FRAME_DESCRIPTOR_SIZE {
            return Err(CameraCodecError::TrailingBytes(
                bytes.len() - CAMERA_FRAME_DESCRIPTOR_SIZE,
            ));
        }
        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != CAMERA_FRAME_MAGIC {
            return Err(CameraCodecError::InvalidMagic(magic));
        }
        if bytes[4] != CAMERA_FRAME_VERSION {
            return Err(CameraCodecError::UnsupportedVersion(bytes[4]));
        }
        let pixel_format = CameraPixelFormat::from_u32(u32::from(bytes[5]))?;
        let rotation_deg = u16::from_le_bytes([bytes[6], bytes[7]]);
        let width = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let height = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let stride_bytes = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let reserved = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        if reserved != 0 {
            return Err(CameraCodecError::InvalidEnum("CameraFrameReserved", reserved));
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
fn validate_zoom(zoom: f32) -> Result<(), CameraCodecError> {
    if zoom.is_finite() && zoom > 0.0 {
        Ok(())
    } else {
        Err(CameraCodecError::TypeMismatch {
            expected: "finite positive zoom",
        })
    }
}

#[inline]
fn validate_fps(fps: u32) -> Result<u16, CameraCodecError> {
    if (1..=u32::from(MAX_CAMERA_FPS)).contains(&fps) {
        Ok(fps as u16)
    } else {
        Err(CameraCodecError::InvalidEnum("CameraFps", fps))
    }
}

#[inline]
fn validate_unit_coord(v: f32, expected: &'static str) -> Result<(), CameraCodecError> {
    if v.is_finite() && (0.0..=1.0).contains(&v) {
        Ok(())
    } else {
        Err(CameraCodecError::TypeMismatch { expected })
    }
}

#[inline]
fn validate_quality(q: u32) -> Result<u8, CameraCodecError> {
    if q <= 100 {
        Ok(q as u8)
    } else {
        Err(CameraCodecError::InvalidEnum("PhotoQuality", q))
    }
}

#[inline]
fn validate_flash(f: u32) -> Result<u8, CameraCodecError> {
    if f <= 2 {
        Ok(f as u8)
    } else {
        Err(CameraCodecError::InvalidEnum("FlashMode", f))
    }
}
