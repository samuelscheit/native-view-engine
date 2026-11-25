//! Unified opt-in NVE camera module (`packages/modules/camera`).
//!
//! Provides:
//! - Unified [`CameraFacing`] enum shared across all camera surfaces,
//! - Zero-JSON `NVNP` (`CameraPreviewProps`), `NVNC` (`CameraPreviewCommand`), and 40-byte
//!   `#[repr(C)]` `NVCF` (`CameraFrameDescriptor`) codecs (`feature = "spec"`),
//! - Optional `nve-interop` [`nve_interop::native_view::NativeViewSpec`] trait bindings
//!   (`feature = "interop"`),
//! - `SERVICE_CAMERA = 0x0023` host-service client, codecs, [`MemoryCamera`], and
//!   [`NativeCamera`] handlers (`feature = "host-service"`),
//! - Optional zero-JS-VM Nitro bridge bindings (`feature = "nitro"`).

pub mod types;

#[cfg(feature = "host-service")]
pub mod host_service;
#[cfg(feature = "interop")]
pub mod interop;
#[cfg(feature = "nitro")]
pub mod nitro;
#[cfg(feature = "spec")]
pub mod spec;
#[cfg(feature = "spec")]
pub mod wire;

pub use types::CameraFacing;

#[cfg(feature = "spec")]
pub use spec::{
    CameraFrameDescriptor, CameraFrameWireLayout, CameraPixelFormat, CameraPreview,
    CameraPreviewCommand, CameraPreviewProps, StabilizationMode, TorchMode,
    CAMERA_FRAME_DESCRIPTOR_SIZE, CAMERA_FRAME_MAGIC, CAMERA_FRAME_VERSION, CAMERA_PREVIEW_KIND,
    MAX_CAMERA_FPS,
};

#[cfg(feature = "spec")]
pub use wire::{CameraCodecError, NATIVE_CMD_MAGIC, NATIVE_PROPS_MAGIC};

#[cfg(feature = "host-service")]
pub use host_service::{
    CameraCaptureOptions, CameraClient, CameraDevice, CameraPreviewConfig, CameraSessionId,
    CapturedMedia, FlashMode, MediaFormat, MemoryCamera, NativeCamera, PermissionStatus,
    METHOD_CAPTURE_PHOTO, METHOD_LIST_DEVICES, METHOD_REQUEST_PERMISSION, METHOD_START_PREVIEW,
    METHOD_STOP_PREVIEW, SERVICE_CAMERA,
};
