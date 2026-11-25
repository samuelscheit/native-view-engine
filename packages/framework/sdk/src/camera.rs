//! Camera & media capture capability (`SERVICE_CAMERA = 0x0023`) with zero-JSON binary codecs.

use crate::codec::{BinReader, BinWriter};
use crate::error::SdkError;
pub use crate::provider::PermissionStatus;
use crate::provider::HostServiceProvider;

/// Built-in service ID for camera enumeration, photo capture, and live preview sessions.
pub const SERVICE_CAMERA: u32 = 0x0023;

pub const METHOD_LIST_DEVICES: u32 = 1;
pub const METHOD_REQUEST_PERMISSION: u32 = 2;
pub const METHOD_CAPTURE_PHOTO: u32 = 3;
pub const METHOD_START_PREVIEW: u32 = 4;
pub const METHOD_STOP_PREVIEW: u32 = 5;

/// Opaque identifier for an active camera preview session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CameraSessionId(pub u64);

/// Physical lens facing direction of a camera device.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraFacing {
    Unknown = 0,
    Back = 1,
    Front = 2,
    External = 3,
}

impl CameraFacing {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Unknown),
            1 => Some(Self::Back),
            2 => Some(Self::Front),
            3 => Some(Self::External),
            _ => None,
        }
    }
}

/// Camera flash behavior during photo capture.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlashMode {
    Off = 0,
    On = 1,
    Auto = 2,
}

impl FlashMode {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Off),
            1 => Some(Self::On),
            2 => Some(Self::Auto),
            _ => None,
        }
    }
}

/// Encoded image format for captured media payloads.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaFormat {
    Jpeg = 0,
    Png = 1,
    Heif = 2,
    Webp = 3,
    Raw = 4,
}

impl MediaFormat {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Jpeg),
            1 => Some(Self::Png),
            2 => Some(Self::Heif),
            3 => Some(Self::Webp),
            4 => Some(Self::Raw),
            _ => None,
        }
    }
}

/// Hardware camera descriptor returned by [`CameraClient::list_devices`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDevice {
    pub id: String,
    pub name: String,
    pub facing: CameraFacing,
    pub has_flash: bool,
    pub max_width: u32,
    pub max_height: u32,
}

/// Options for [`CameraClient::capture_photo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraCaptureOptions {
    pub device_id: Option<String>,
    pub flash: FlashMode,
    pub format: MediaFormat,
    pub quality: u8,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
}

impl Default for CameraCaptureOptions {
    fn default() -> Self {
        Self {
            device_id: None,
            flash: FlashMode::Auto,
            format: MediaFormat::Jpeg,
            quality: 90,
            max_width: None,
            max_height: None,
        }
    }
}

/// Binary media payload and metadata returned by [`CameraClient::capture_photo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedMedia {
    pub format: MediaFormat,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
    pub file_path: Option<String>,
    pub timestamp_ms: u64,
}

/// Configuration for [`CameraClient::start_preview`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraPreviewConfig {
    pub device_id: Option<String>,
    pub width: u32,
    pub height: u32,
    pub target_fps: u16,
    pub mirror: bool,
}

impl Default for CameraPreviewConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            width: 1280,
            height: 720,
            target_fps: 30,
            mirror: false,
        }
    }
}

fn validate_capture_options(opts: &CameraCaptureOptions) -> Result<(), SdkError> {
    if opts.quality > 100 {
        return Err(SdkError::InvalidArg(format!(
            "camera capture quality {} must be in 0..=100",
            opts.quality
        )));
    }
    if opts.max_width == Some(0) || opts.max_height == Some(0) {
        return Err(SdkError::InvalidArg(
            "camera max_width/max_height must be non-zero when set".into(),
        ));
    }
    Ok(())
}

fn validate_preview_config(cfg: &CameraPreviewConfig) -> Result<(), SdkError> {
    if cfg.width == 0 || cfg.height == 0 {
        return Err(SdkError::InvalidArg(
            "camera preview width and height must be non-zero".into(),
        ));
    }
    if cfg.target_fps == 0 || cfg.target_fps > 240 {
        return Err(SdkError::InvalidArg(format!(
            "camera preview target_fps {} must be in 1..=240",
            cfg.target_fps
        )));
    }
    Ok(())
}

pub fn encode_devices_response(devices: &[CameraDevice]) -> Result<Vec<u8>, SdkError> {
    let count = u32::try_from(devices.len())
        .map_err(|_| SdkError::InvalidArg("camera device list exceeds u32::MAX".into()))?;
    let mut w = BinWriter::with_capacity(4 + devices.len() * 32);
    w.write_u32(count);
    for dev in devices {
        w.write_str(&dev.id)?;
        w.write_str(&dev.name)?;
        w.write_u8(dev.facing as u8);
        w.write_bool(dev.has_flash);
        w.write_u32(dev.max_width);
        w.write_u32(dev.max_height);
    }
    Ok(w.into_bytes())
}

pub fn decode_devices_response(bytes: &[u8]) -> Result<Vec<CameraDevice>, SdkError> {
    let mut r = BinReader::new(bytes);
    let count = r.read_seq_len(18)?;
    let mut devices = Vec::with_capacity(count);
    for _ in 0..count {
        let id = r.read_string()?;
        let name = r.read_string()?;
        let facing_raw = r.read_u8()?;
        let facing = CameraFacing::from_u8(facing_raw)
            .ok_or_else(|| SdkError::Codec(format!("invalid CameraFacing {facing_raw}")))?;
        let has_flash = r.read_bool()?;
        let max_width = r.read_u32()?;
        let max_height = r.read_u32()?;
        devices.push(CameraDevice {
            id,
            name,
            facing,
            has_flash,
            max_width,
            max_height,
        });
    }
    r.finish()?;
    Ok(devices)
}

pub fn encode_capture_options(opts: &CameraCaptureOptions) -> Result<Vec<u8>, SdkError> {
    validate_capture_options(opts)?;
    let mut w = BinWriter::with_capacity(24);
    w.write_opt_str(opts.device_id.as_deref())?;
    w.write_u8(opts.flash as u8);
    w.write_u8(opts.format as u8);
    w.write_u8(opts.quality);
    w.write_opt_u32(opts.max_width);
    w.write_opt_u32(opts.max_height);
    Ok(w.into_bytes())
}

pub fn decode_capture_options(bytes: &[u8]) -> Result<CameraCaptureOptions, SdkError> {
    let mut r = BinReader::new(bytes);
    let device_id = r.read_opt_string()?;
    let flash_raw = r.read_u8()?;
    let flash = FlashMode::from_u8(flash_raw)
        .ok_or_else(|| SdkError::Codec(format!("invalid FlashMode {flash_raw}")))?;
    let format_raw = r.read_u8()?;
    let format = MediaFormat::from_u8(format_raw)
        .ok_or_else(|| SdkError::Codec(format!("invalid MediaFormat {format_raw}")))?;
    let quality = r.read_u8()?;
    let max_width = r.read_opt_u32()?;
    let max_height = r.read_opt_u32()?;
    r.finish()?;
    let opts = CameraCaptureOptions {
        device_id,
        flash,
        format,
        quality,
        max_width,
        max_height,
    };
    validate_capture_options(&opts)?;
    Ok(opts)
}

pub fn encode_captured_media(media: &CapturedMedia) -> Result<Vec<u8>, SdkError> {
    let mut w = BinWriter::with_capacity(24 + media.bytes.len());
    w.write_u8(media.format as u8);
    w.write_u32(media.width);
    w.write_u32(media.height);
    w.write_bytes(&media.bytes)?;
    w.write_opt_str(media.file_path.as_deref())?;
    w.write_u64(media.timestamp_ms);
    Ok(w.into_bytes())
}

pub fn decode_captured_media(bytes: &[u8]) -> Result<CapturedMedia, SdkError> {
    let mut r = BinReader::new(bytes);
    let format_raw = r.read_u8()?;
    let format = MediaFormat::from_u8(format_raw)
        .ok_or_else(|| SdkError::Codec(format!("invalid MediaFormat {format_raw}")))?;
    let width = r.read_u32()?;
    let height = r.read_u32()?;
    let data = r.read_bytes()?.to_vec();
    let file_path = r.read_opt_string()?;
    let timestamp_ms = r.read_u64()?;
    r.finish()?;
    Ok(CapturedMedia {
        format,
        width,
        height,
        bytes: data,
        file_path,
        timestamp_ms,
    })
}

pub fn encode_preview_config(cfg: &CameraPreviewConfig) -> Result<Vec<u8>, SdkError> {
    validate_preview_config(cfg)?;
    let mut w = BinWriter::with_capacity(24);
    w.write_opt_str(cfg.device_id.as_deref())?;
    w.write_u32(cfg.width);
    w.write_u32(cfg.height);
    w.write_u16(cfg.target_fps);
    w.write_bool(cfg.mirror);
    Ok(w.into_bytes())
}

pub fn decode_preview_config(bytes: &[u8]) -> Result<CameraPreviewConfig, SdkError> {
    let mut r = BinReader::new(bytes);
    let device_id = r.read_opt_string()?;
    let width = r.read_u32()?;
    let height = r.read_u32()?;
    let target_fps = r.read_u16()?;
    let mirror = r.read_bool()?;
    r.finish()?;
    let cfg = CameraPreviewConfig {
        device_id,
        width,
        height,
        target_fps,
        mirror,
    };
    validate_preview_config(&cfg)?;
    Ok(cfg)
}

pub fn encode_session_id(session: CameraSessionId) -> Result<Vec<u8>, SdkError> {
    if session.0 == 0 {
        return Err(SdkError::InvalidArg("camera session id must be non-zero".into()));
    }
    let mut w = BinWriter::with_capacity(8);
    w.write_u64(session.0);
    Ok(w.into_bytes())
}

pub fn decode_session_id(bytes: &[u8]) -> Result<CameraSessionId, SdkError> {
    let mut r = BinReader::new(bytes);
    let id = r.read_u64()?;
    r.finish()?;
    if id == 0 {
        return Err(SdkError::InvalidArg("camera session id must be non-zero".into()));
    }
    Ok(CameraSessionId(id))
}

/// Typed ergonomic client for native camera devices, still capture, and live preview sessions.
#[derive(Debug, Clone, Copy)]
pub struct CameraClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> CameraClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn list_devices(&self) -> Result<Vec<CameraDevice>, SdkError> {
        let resp = self
            .provider
            .invoke_sync(SERVICE_CAMERA, METHOD_LIST_DEVICES, &[])?;
        decode_devices_response(&resp)
    }

    pub fn request_permission(&self) -> Result<PermissionStatus, SdkError> {
        let resp =
            self.provider
                .invoke_sync(SERVICE_CAMERA, METHOD_REQUEST_PERMISSION, &[])?;
        PermissionStatus::decode(&resp)
    }

    pub fn capture_photo(&self, options: &CameraCaptureOptions) -> Result<CapturedMedia, SdkError> {
        let req = encode_capture_options(options)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_CAMERA, METHOD_CAPTURE_PHOTO, &req)?;
        decode_captured_media(&resp)
    }

    pub fn start_preview(
        &self,
        config: &CameraPreviewConfig,
    ) -> Result<CameraSessionId, SdkError> {
        let req = encode_preview_config(config)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_CAMERA, METHOD_START_PREVIEW, &req)?;
        decode_session_id(&resp)
    }

    pub fn stop_preview(&self, session: CameraSessionId) -> Result<(), SdkError> {
        let req = encode_session_id(session)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_CAMERA, METHOD_STOP_PREVIEW, &req)?;
        if !resp.is_empty() {
            return Err(SdkError::Codec(
                "unexpected non-empty camera::stop_preview response".into(),
            ));
        }
        Ok(())
    }
}
