//! Canonical native platform view types, zero-serialization in-process prop/command containers
//! (`NativeProps`, `NativeCommand`, `TypedNativeViewProps`), single-source cross-language
//! `NVNP`/`NVNC` wire codecs, and built-in `NativeViewSpec` contracts.

mod codec;
mod specs;
mod typed;
mod wire;

#[cfg(test)]
mod tests;

pub use codec::{
    NativeCommandDecoder, NativeCommandEncoder, NativeCommandSink, NativeCommandSource,
    NativePropDecoder, NativePropEncoder, NativePropSink, NativePropSource,
};
pub use specs::{
    BlurMaterial, BlurSurface, BlurSurfaceProps, DecodeNativeCommand, DecodeNativeProps,
    DecodedWebElementProps, EncodeNativeCommand, EncodeNativeProps, NativeViewSpec, NoCommand,
    VideoPlayer, VideoPlayerCommand, VideoPlayerProps, WebElement, WebElementProps, WebView,
    WebViewCommand, WebViewProps,
};
pub use typed::{
    NativeCommand, NativeCommandBuilder, NativeProp, NativePropValue, NativeProps,
    NativePropsBuilder, TypedNativeViewCommand, TypedNativeViewProps,
};
pub use wire::{
    NativeValueRef, NativeViewCodecError, MAX_NATIVE_CMD_ARGS, MAX_NATIVE_PROP_FIELDS,
    NATIVE_CMD_HEADER_SIZE, NATIVE_CMD_MAGIC, NATIVE_CMD_VERSION, NATIVE_PROPS_HEADER_SIZE,
    NATIVE_PROPS_MAGIC, NATIVE_PROPS_VERSION, NATIVE_VIEW_KIND_PROP_ALIAS,
    NATIVE_VIEW_KIND_PROP_KEY, NATIVE_VIEW_PROPS_PROP_ALIAS, NATIVE_VIEW_PROPS_PROP_KEY,
};
