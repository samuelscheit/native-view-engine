//! First-Class Native Platform Primitive Escape Hatches (`NativeViewSpec`, zero-serialization
//! in-process prop/command containers, single-source `NVNP`/`NVNC` FFI codecs, `NativeViewHandle`,
//! and wildcard-aware `NativeViewRegistry`).

mod handle;

pub use handle::{
    NativeViewHandle, NativeViewHandleBinder, NativeViewRegistry, QueuedNativeCommand,
};
pub use nve_components::native_view::{
    BlurMaterial, BlurSurface, BlurSurfaceProps, DecodeNativeCommand, DecodeNativeProps,
    DecodedWebElementProps, EncodeNativeCommand, EncodeNativeProps, NativeCommand,
    NativeCommandBuilder, NativeCommandDecoder, NativeCommandEncoder, NativeCommandSink,
    NativeCommandSource, NativeProp, NativePropDecoder, NativePropEncoder, NativePropSink,
    NativePropSource, NativePropValue, NativeProps, NativePropsBuilder, NativeValueRef,
    NativeViewCodecError, NativeViewSpec, NoCommand, TypedNativeViewCommand, TypedNativeViewProps,
    VideoPlayer, VideoPlayerCommand, VideoPlayerProps, WebElement, WebElementProps, WebView,
    WebViewCommand, WebViewProps, MAX_NATIVE_CMD_ARGS, MAX_NATIVE_PROP_FIELDS,
    NATIVE_CMD_HEADER_SIZE, NATIVE_CMD_MAGIC, NATIVE_CMD_VERSION, NATIVE_PROPS_HEADER_SIZE,
    NATIVE_PROPS_MAGIC, NATIVE_PROPS_VERSION, NATIVE_VIEW_KIND_PROP_ALIAS,
    NATIVE_VIEW_KIND_PROP_KEY, NATIVE_VIEW_PROPS_PROP_ALIAS, NATIVE_VIEW_PROPS_PROP_KEY,
};
