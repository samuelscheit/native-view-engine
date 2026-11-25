//! Unified opt-in NVE camera module (`packages/modules/camera`).
//!
//! Provides zero-JSON `NVNP` (`CameraPreviewProps`), `NVNC` (`CameraPreviewCommand`), and
//! 40-byte `#[repr(C)]` `NVCF` (`CameraFrameDescriptor`) binary codecs alongside optional
//! zero-JS-VM Nitro bridge bindings (`feature = "nitro"`).

#[cfg(feature = "spec")]
pub mod spec;
#[cfg(feature = "spec")]
pub mod wire;

#[cfg(feature = "nitro")]
pub mod nitro;

#[cfg(feature = "spec")]
pub use spec::{
    CAMERA_FRAME_DESCRIPTOR_SIZE, CAMERA_FRAME_MAGIC, CAMERA_FRAME_VERSION, CAMERA_PREVIEW_KIND,
    CameraFrameDescriptor, CameraFrameWireLayout, CameraPixelFormat, CameraPreview,
    CameraPreviewCommand, CameraPreviewFacing, CameraPreviewProps, MAX_CAMERA_FPS,
    StabilizationMode, TorchMode,
};

#[cfg(feature = "spec")]
pub use wire::{CameraCodecError, NATIVE_CMD_MAGIC, NATIVE_PROPS_MAGIC};
