//! Typed zero-serialization `CameraPreview` (`"nve.camera.preview"`), `CameraPreviewProps`,
//! `CameraPreviewCommand`, and zero-copy `#[repr(C)]` `CameraFrameDescriptor` /
//! `CameraFrameWireLayout` (`NVCF`).
//!
//! Reuses the single canonical `nve_components::native_view` typed & `NVNP`/`NVNC` wire codec.

use nve_components::native_view::{
    DecodeNativeCommand, DecodeNativeProps, EncodeNativeCommand, EncodeNativeProps,
    NativeCommand, NativeCommandSink, NativeCommandSource, NativePropSink, NativePropSource,
    NativeProps, NativeValueRef, NativeViewCodecError as CameraCodecError, NativeViewSpec,
};

pub use crate::frame::{
    CameraFrameDescriptor, CameraFrameProcessor, CameraFrameWireLayout,
    CAMERA_FRAME_DESCRIPTOR_SIZE, CAMERA_FRAME_MAGIC, CAMERA_FRAME_VERSION,
};
use crate::types::CameraFacing;

pub const CAMERA_PREVIEW_KIND: &str = "nve.camera.preview";
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

impl NativeViewSpec for CameraPreview {
    const KIND: &'static str = CAMERA_PREVIEW_KIND;
    type Props = CameraPreviewProps;
    type Command = CameraPreviewCommand;
}

/// Declarative zero-serialization props for [`CameraPreview`].
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

    #[inline]
    pub fn to_native_props(&self) -> Result<NativeProps, CameraCodecError> {
        EncodeNativeProps::to_native_props(self)
    }

    #[inline]
    pub fn from_native_props(props: &NativeProps) -> Result<Self, CameraCodecError> {
        DecodeNativeProps::from_native_props(props)
    }

    #[inline]
    pub fn to_prop_bytes(&self) -> Result<Vec<u8>, CameraCodecError> {
        EncodeNativeProps::to_prop_bytes(self)
    }

    #[inline]
    pub fn from_prop_bytes(bytes: &[u8]) -> Result<Self, CameraCodecError> {
        DecodeNativeProps::from_prop_bytes(bytes)
    }
}

impl EncodeNativeProps for CameraPreviewProps {
    fn write_props<S: NativePropSink>(&self, sink: &mut S) -> Result<(), CameraCodecError> {
        self.validate()?;
        sink.opt_str_field(Self::FIELD_DEVICE_ID, self.device_id.as_deref())?
            .u32_field(Self::FIELD_FACING, self.facing as u32)?
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
    fn read_props<S: NativePropSource>(src: &S) -> Result<Self, CameraCodecError> {
        let device_id = match src.require_field(Self::FIELD_DEVICE_ID)? {
            NativeValueRef::Null => None,
            NativeValueRef::Str(s) => Some(s.to_owned()),
            _ => return Err(CameraCodecError::TypeMismatch { expected: "Option<str>" }),
        };
        let facing = facing_from_preview_u32(src.require_u32_field(Self::FIELD_FACING)?)?;
        let torch = TorchMode::from_u32(src.require_u32_field(Self::FIELD_TORCH)?)?;
        let zoom = src.require_f32_field(Self::FIELD_ZOOM)?;
        validate_zoom(zoom)?;
        let fps = validate_fps(src.require_u32_field(Self::FIELD_FPS)?)?;
        let hdr = src.require_bool_field(Self::FIELD_HDR)?;
        let stabilization =
            StabilizationMode::from_u32(src.require_u32_field(Self::FIELD_STABILIZATION)?)?;
        let enable_frame_processor = src.require_bool_field(Self::FIELD_ENABLE_FRAME_PROCESSOR)?;
        let audio_enabled = src.require_bool_field(Self::FIELD_AUDIO_ENABLED)?;
        let active = src.require_bool_field(Self::FIELD_ACTIVE)?;
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

/// Typed imperative commands for [`CameraPreview`].
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

    #[inline]
    pub fn to_native_command(&self) -> Result<NativeCommand, CameraCodecError> {
        EncodeNativeCommand::to_native_command(self)
    }

    #[inline]
    pub fn from_native_command(cmd: &NativeCommand) -> Result<Self, CameraCodecError> {
        DecodeNativeCommand::from_native_command(cmd)
    }

    #[inline]
    pub fn encode_command(&self) -> Result<Vec<u8>, CameraCodecError> {
        EncodeNativeCommand::encode_command(self)
    }

    #[inline]
    pub fn from_command_bytes(bytes: &[u8]) -> Result<Self, CameraCodecError> {
        DecodeNativeCommand::from_command_bytes(bytes)
    }
}

impl EncodeNativeCommand for CameraPreviewCommand {
    #[inline]
    fn command_name(&self) -> &'static str {
        CameraPreviewCommand::command_name(self)
    }

    fn command_opcode(&self) -> u16 {
        match self {
            Self::SwitchFacing(_) => Self::OP_SWITCH_FACING,
            Self::SetTorch(_) => Self::OP_SET_TORCH,
            Self::SetZoom(_) => Self::OP_SET_ZOOM,
            Self::FocusPoint { .. } => Self::OP_FOCUS_POINT,
            Self::CapturePhoto { .. } => Self::OP_CAPTURE_PHOTO,
            Self::StartRecording { .. } => Self::OP_START_RECORDING,
            Self::StopRecording => Self::OP_STOP_RECORDING,
        }
    }

    fn write_command_args<S: NativeCommandSink>(&self, sink: &mut S) -> Result<(), CameraCodecError> {
        self.validate()?;
        match self {
            Self::SwitchFacing(facing) => {
                sink.arg_u32(*facing as u32)?;
            }
            Self::SetTorch(mode) => {
                sink.arg_u32(*mode as u32)?;
            }
            Self::SetZoom(zoom) => {
                sink.arg_f32(*zoom)?;
            }
            Self::FocusPoint { x, y } => {
                sink.arg_f32(*x)?.arg_f32(*y)?;
            }
            Self::CapturePhoto { quality, flash } => {
                sink.arg_u32(u32::from(*quality))?.arg_u32(u32::from(*flash))?;
            }
            Self::StartRecording { output_path } => {
                sink.arg_str(output_path)?;
            }
            Self::StopRecording => {}
        }
        Ok(())
    }
}

impl DecodeNativeCommand for CameraPreviewCommand {
    fn read_command<S: NativeCommandSource>(src: &S) -> Result<Self, CameraCodecError> {
        match src.opcode() {
            Self::OP_SWITCH_FACING => {
                src.expect_arg_count(1)?;
                Ok(Self::SwitchFacing(facing_from_preview_u32(
                    src.require_u32_arg(0)?,
                )?))
            }
            Self::OP_SET_TORCH => {
                src.expect_arg_count(1)?;
                Ok(Self::SetTorch(TorchMode::from_u32(src.require_u32_arg(0)?)?))
            }
            Self::OP_SET_ZOOM => {
                src.expect_arg_count(1)?;
                let zoom = src.require_f32_arg(0)?;
                validate_zoom(zoom)?;
                Ok(Self::SetZoom(zoom))
            }
            Self::OP_FOCUS_POINT => {
                src.expect_arg_count(2)?;
                let x = src.require_f32_arg(0)?;
                let y = src.require_f32_arg(1)?;
                validate_unit_coord(x, "focus_point.x in 0.0..=1.0")?;
                validate_unit_coord(y, "focus_point.y in 0.0..=1.0")?;
                Ok(Self::FocusPoint { x, y })
            }
            Self::OP_CAPTURE_PHOTO => {
                src.expect_arg_count(2)?;
                let quality = validate_quality(src.require_u32_arg(0)?)?;
                let flash = validate_flash(src.require_u32_arg(1)?)?;
                Ok(Self::CapturePhoto { quality, flash })
            }
            Self::OP_START_RECORDING => {
                src.expect_arg_count(1)?;
                Ok(Self::StartRecording {
                    output_path: src.require_str_arg(0)?.to_owned(),
                })
            }
            Self::OP_STOP_RECORDING => {
                src.expect_arg_count(0)?;
                Ok(Self::StopRecording)
            }
            other => Err(CameraCodecError::UnknownCommandOpcode(other)),
        }
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
