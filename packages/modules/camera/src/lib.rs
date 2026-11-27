//! Optional NVE camera module (`packages/modules/camera`).
//!
//! Pure-spec root crate with zero dependency on `nve-components`, `nve-interop`, or any backend:
//! - [`table::CameraPreviewTable`] (`#[derive(NveTable)]`, `"nve.camera.preview"`),
//! - [`CameraFacing`] enum shared across camera adapters,
//! - 40-byte `#[repr(C)]` [`CameraFrameDescriptor`] / [`CameraFrameWireLayout`] (`NVCF`),
//! - Optional Nitro C-ABI bridge bindings (`feature = "nitro"`).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod frame;
#[cfg(feature = "nitro")]
pub mod nitro;
pub mod table;
pub mod types;

pub use frame::{
    CameraCodecError, CameraFrameDescriptor, CameraFrameProcessor, CameraFrameWireLayout,
    CameraPixelFormat, CAMERA_FRAME_DESCRIPTOR_SIZE, CAMERA_FRAME_MAGIC, CAMERA_FRAME_VERSION,
};
pub use table::{CameraPreviewTable, CameraStabilization, CameraTorch};
pub use types::CameraFacing;

pub const CAMERA_PREVIEW_KIND: &str = "nve.camera.preview";
pub const MAX_CAMERA_FPS: u16 = 240;
