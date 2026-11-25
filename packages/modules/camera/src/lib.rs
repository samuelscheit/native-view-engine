//! Optional NVE VisionCamera v5 adapter & typed `CameraPreview` spec (`packages/modules/camera`).
//!
//! Provides:
//! - Unified [`CameraFacing`] enum shared across camera adapters,
//! - Typed [`CameraPreview`] (`NativeViewSpec`), [`CameraPreviewProps`], [`CameraPreviewCommand`],
//!   and 40-byte `#[repr(C)]` [`CameraFrameDescriptor`] / [`CameraFrameWireLayout`] (`NVCF`)
//!   (`feature = "spec"`, reusing `nve-components::native_view`),
//! - Optional `nve-interop` re-exports (`feature = "interop"`),
//! - Optional Nitro C-ABI bridge bindings (`feature = "nitro"`).

pub mod types;

#[cfg(feature = "spec")]
pub mod frame;
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
