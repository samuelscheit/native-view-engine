//! Versioned binary CommandBuffer, EventRing, and modular `nve_get_interface` C ABI definitions
//! shared across Rust frontends, the C++20 Fabric bridge, Skia, Web WASM, Flutter, SwiftUI,
//! Jetpack Compose, and Headless backends.

pub mod codec;
pub mod events;
pub mod ffi;
pub mod scene;
pub mod value;

pub use codec::{
    ColorRgba, Command, CommandBufferBuilder, CommandBufferHeader, CommandDecoder, ComponentKind,
    DecodeError, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify, LayoutProps,
    RawPropValue, StyleProps, NVE_ABI_MAGIC, NVE_ABI_VERSION,
};
pub use events::{
    decode_events, encode_events, EventMask, EventPayload, NativeEvent, NativeEventKind,
};
pub use ffi::{
    NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi, NveFrameStats, NveGestureDisposition,
    NveGetInterfaceFn, NveIntrinsicSize, NveLayoutApi, NveSizeConstraints, NveSurfaceApi,
    NveSurfaceConfig, NVE_CORE_ABI_SCHEMA_HASH, NVE_IFACE_COMPONENT, NVE_IFACE_DEV,
    NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
pub use scene::{
    component_kind_to_scene_u8, NveRenderNode, NveSceneRef, NveSceneTable, NveSceneView,
    SceneError, SceneNodeInput, NVE_NODE_CLIP_BOUNDS, NVE_NODE_INTERACTIVE, NVE_NODE_KIND_CUSTOM,
    NVE_NODE_KIND_IMAGE, NVE_NODE_KIND_PRESSABLE, NVE_NODE_KIND_ROOT, NVE_NODE_KIND_SCROLL_VIEW,
    NVE_NODE_KIND_SKIA_CANVAS, NVE_NODE_KIND_TEXT, NVE_NODE_KIND_TEXT_INPUT, NVE_NODE_KIND_VIEW,
    NVE_NODE_LEAF_TEXT, NVE_RENDER_NODE_ALIGN, NVE_RENDER_NODE_SIZE, NVE_SCENE_SCHEMA_HASH,
};
pub use value::{
    encode_bin_value, encode_bin_value_to_vec, BinArrayIter, BinMapIter, BinValueView, NveArrayRef,
    NveDirectEvent, NveDirectEventRef, NveEventBatch, NveEventBatchRef, NveMapEntry, NveMapRef,
    NvePropsRef, NveStrSlice, NveValue, NveValueArray, NveValueData, NveValueMap, NveValueRef,
    NveValueTag, NVE_BIN_VALUE_MAX_DEPTH,
};

pub type SurfaceId = u32;
pub type NodeId = u32;
pub type FrameId = u64;

/// Reserved root node ID for every surface.
pub const ROOT_NODE_ID: NodeId = 1;
