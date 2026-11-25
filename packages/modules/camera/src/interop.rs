//! Canonical `nve-interop` convenience re-exports for [`CameraPreview`].
//!
//! Because [`CameraPreview`], [`CameraPreviewProps`], and [`CameraPreviewCommand`] directly
//! implement `nve_components::native_view::NativeViewSpec` (re-exported by `nve-interop`)
//! with the single canonical `NativeViewCodecError`, zero codec translation or double-encoding
//! occurs across the `nve-camera` / `nve-interop` boundary.

pub use nve_interop::native_view::{
    DecodeNativeCommand, DecodeNativeProps, EncodeNativeCommand, EncodeNativeProps,
    NativeViewCodecError, NativeViewHandle, NativeViewRegistry, NativeViewSpec,
};

pub use crate::spec::{
    CameraPreview, CameraPreviewCommand, CameraPreviewProps, CAMERA_PREVIEW_KIND,
};
