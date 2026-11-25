//! Unified opt-in NVE camera module (`packages/modules/camera`).
//!
//! Provides:
//! - Unified [`CameraFacing`] enum shared across all camera surfaces,
//! - Canonical zero-serialization [`CameraPreview`] (`NativeViewSpec`), [`CameraPreviewProps`],
//!   [`CameraPreviewCommand`], and zero-copy `#[repr(C)]` [`CameraFrameDescriptor`] /
//!   [`CameraFrameWireLayout`] (`NVCF`) (`feature = "spec"`, reusing `nve-components::native_view`),
//! - Optional `nve-interop` re-exports (`feature = "interop"`),
//! - `SERVICE_CAMERA = 0x0023` host-service client, codecs, [`MemoryCamera`], and
//!   [`NativeCamera`] handlers (`feature = "host-service"`),
//! - Optional zero-JS-VM Nitro bridge bindings (`feature = "nitro"`).

pub mod types;

#[cfg(feature = "spec")]
pub mod frame;
#[cfg(feature = "host-service")]
pub mod host_service;
#[cfg(feature = "interop")]
pub mod interop;
#[cfg(feature = "nitro")]
pub mod nitro;
#[cfg(feature = "spec")]
pub mod spec;

pub use types::CameraFacing;

#[cfg(feature = "spec")]
pub use nve_components::native_view::{
    NativeViewCodecError as CameraCodecError, NATIVE_CMD_MAGIC, NATIVE_PROPS_MAGIC,
};
#[cfg(feature = "spec")]
pub use spec::{
    CameraFrameDescriptor, CameraFrameProcessor, CameraFrameWireLayout, CameraPixelFormat,
    CameraPreview, CameraPreviewCommand, CameraPreviewProps, StabilizationMode, TorchMode,
    CAMERA_FRAME_DESCRIPTOR_SIZE, CAMERA_FRAME_MAGIC, CAMERA_FRAME_VERSION, CAMERA_PREVIEW_KIND,
    MAX_CAMERA_FPS,
};

#[cfg(feature = "host-service")]
pub use host_service::{
    CameraCaptureOptions, CameraClient, CameraDevice, CameraPreviewConfig, CameraSessionId,
    CapturedMedia, FlashMode, MediaFormat, MemoryCamera, NativeCamera, PermissionStatus,
    METHOD_CAPTURE_PHOTO, METHOD_LIST_DEVICES, METHOD_REQUEST_PERMISSION, METHOD_START_PREVIEW,
    METHOD_STOP_PREVIEW, SERVICE_CAMERA,
};
