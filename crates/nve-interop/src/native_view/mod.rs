//! First-Class Native Platform Primitive Escape Hatches (`NativeViewSpec`, zero-JSON binary
//! prop/command codecs, `NativeViewHandle`, and wildcard-aware `NativeViewRegistry`).

mod camera;
mod codec;
mod handle;
mod specs;
mod wire;

pub use camera::{
    CameraFrameDescriptor, CameraPixelFormat, CameraPreview, CameraPreviewCommand,
    CameraPreviewFacing, CameraPreviewProps, StabilizationMode, TorchMode,
    CAMERA_FRAME_DESCRIPTOR_SIZE, CAMERA_FRAME_MAGIC, CAMERA_FRAME_VERSION, CAMERA_PREVIEW_KIND,
    MAX_CAMERA_FPS,
};
pub use codec::{
    NativeCommandDecoder, NativeCommandEncoder, NativePropDecoder, NativePropEncoder,
};
pub use handle::{
    DecodeNativeCommand, DecodeNativeProps, EncodeNativeCommand, EncodeNativeProps,
    NativeViewHandle, NativeViewHandleBinder, NativeViewRegistry, NativeViewSpec, NoCommand,
    QueuedNativeCommand,
};
pub use specs::{
    BlurMaterial, BlurSurface, BlurSurfaceProps, DecodedWebElementProps, VideoPlayer,
    VideoPlayerCommand, VideoPlayerProps, WebElement, WebElementProps, WebView, WebViewCommand,
    WebViewProps,
};
pub use wire::{
    NativeValueRef, NativeViewCodecError, MAX_NATIVE_CMD_ARGS, MAX_NATIVE_PROP_FIELDS,
    NATIVE_CMD_HEADER_SIZE, NATIVE_CMD_MAGIC, NATIVE_CMD_VERSION, NATIVE_PROPS_HEADER_SIZE,
    NATIVE_PROPS_MAGIC, NATIVE_PROPS_VERSION, NATIVE_VIEW_KIND_PROP_KEY,
    NATIVE_VIEW_PROPS_PROP_KEY,
};
