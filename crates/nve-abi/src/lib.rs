//! Versioned binary CommandBuffer, EventRing, and C ABI definitions shared across
//! Rust frontends, the C++20 React Native Fabric bridge, Skia, Web WASM, and Headless backends.

pub mod codec;
pub mod events;
pub mod ffi;

pub use codec::{
    ColorRgba, Command, CommandBufferBuilder, CommandBufferHeader, CommandDecoder, ComponentKind,
    DecodeError, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify, LayoutProps,
    RawPropValue, StyleProps, NVE_ABI_MAGIC, NVE_ABI_VERSION,
};
pub use events::{
    EventMask, EventPayload, NativeEvent, NativeEventKind, decode_events, encode_events,
};
pub use ffi::{
    NveAppHandle, NveAppVTable, NveBufferSlice, NveFrameStats, NveSurfaceConfig,
    NVE_APP_ABI_VERSION,
};

pub type SurfaceId = u32;
pub type NodeId = u32;
pub type FrameId = u64;

/// Reserved root node ID for every surface.
pub const ROOT_NODE_ID: NodeId = 1;
