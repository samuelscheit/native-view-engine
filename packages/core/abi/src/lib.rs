//! Zero-copy `#[repr(C)]` raw-memory ABI (`nve_get_interface`, `NveSceneView`, `NveValue`,
//! `NvePropSlice`, `NveCommandSlice`, `NveEventBatch`), plus optional binary wire codecs
//! (`CommandBuffer`, `EventRing`, `NveBinValue`) gated behind `feature = "wire"`.

extern crate self as nve_abi;

pub mod bitflags;
#[cfg(feature = "wire")]
pub mod codec;
#[cfg(feature = "wire")]
pub mod events;
pub mod ffi;
pub mod hash;
pub mod host;
mod layout_asserts;
pub mod num;
pub mod scene;
pub mod schema;
pub mod value;

pub use nve_abi_macros::NveAbi;

#[cfg(feature = "wire")]
pub use codec::{
    ChildIdIter, ColorRgba, Command, CommandBufferBuilder, CommandBufferHeader, CommandDecoder,
    CommandRef, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify, LayoutProps,
    RawPropValue, RawPropValueRef, StyleProps, NVE_ABI_MAGIC, NVE_ABI_VERSION,
};
#[cfg(feature = "wire")]
pub use events::{
    decode_events, direct_event_to_native, encode_events, native_events_to_direct,
    pack_key_modifiers, EventMask, EventPayload, NativeEvent,
};
pub use ffi::{
    ComponentKind, DecodeError, NativeEventKind, NveAppHandle, NveBufferSlice, NveComponentApi,
    NveDevApi, NveEventPayloadTag, NveFrameStats, NveGestureDisposition, NveGetInterfaceFn,
    NveIntrinsicSize, NveLayoutApi, NveSizeConstraints, NveSurfaceApi, NveSurfaceConfig, Opcode,
    NVE_CORE_ABI_SCHEMA_HASH, NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_HOST,
    NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
pub use host::{
    NveHostApi, NveHostCallbacks, NveHostChannelPublishFn, NveHostInvokeFn, NveHostStatus,
    METHOD_WINDOW_REQUEST_FRAME, NVE_CHANNEL_FLAG_RETAIN, SERVICE_CHANNEL, SERVICE_CLIPBOARD,
    SERVICE_FILESYSTEM, SERVICE_HAPTICS, SERVICE_LINKING, SERVICE_NOTIFICATIONS,
    SERVICE_SECURE_STORE, SERVICE_SYSTEM_ENV, SERVICE_WINDOW,
};
pub use scene::{
    component_kind_to_scene_u8, NveNodeMeta, NveRenderNode, NveSceneRef, NveSceneTable,
    NveSceneView, SceneError, SceneNodeInput, NVE_META_FLAG_ACCESSIBILITY_HIDDEN,
    NVE_META_FLAG_DISABLED, NVE_META_FLAG_DRAW_LIST_CORRUPT, NVE_META_FLAG_HAS_DRAW_LIST,
    NVE_META_FLAG_MULTILINE, NVE_META_FLAG_SECURE_TEXT_ENTRY, NVE_META_RESIZE_MODE_MASK,
    NVE_META_RESIZE_MODE_SHIFT, NVE_NODE_CLIP_BOUNDS, NVE_NODE_INTERACTIVE, NVE_NODE_KIND_CUSTOM,
    NVE_NODE_KIND_IMAGE, NVE_NODE_KIND_PRESSABLE, NVE_NODE_KIND_ROOT, NVE_NODE_KIND_SCROLL_VIEW,
    NVE_NODE_KIND_SKIA_CANVAS, NVE_NODE_KIND_TEXT, NVE_NODE_KIND_TEXT_INPUT, NVE_NODE_KIND_VIEW,
    NVE_NODE_LEAF_TEXT, NVE_NODE_META_ALIGN, NVE_NODE_META_SIZE, NVE_RENDER_NODE_ALIGN,
    NVE_RENDER_NODE_SIZE, NVE_SCENE_SCHEMA_HASH,
};
#[cfg(feature = "wire")]
pub use value::{
    encode_bin_value, encode_bin_value_to_vec, BinArrayIter, BinMapIter, BinValueView,
    NVE_BIN_VALUE_MAX_DEPTH,
};
pub use value::{
    NveArrayRef, NveCommandSlice, NveCommandSliceRef, NveDirectEvent, NveDirectEventRef,
    NveEventBatch, NveEventBatchRef, NveMapEntry, NveMapRef, NvePropEntry, NvePropSlice,
    NvePropSliceRef, NvePropsRef, NveStrSlice, NveValue, NveValueArray, NveValueData, NveValueMap,
    NveValueRef, NveValueTag, NVE_EVENT_FLAG_BUTTON_MASK, NVE_EVENT_FLAG_CAPTURE,
    NVE_EVENT_FLAG_COMPOSING, NVE_KEY_MOD_ALT, NVE_KEY_MOD_CTRL, NVE_KEY_MOD_META,
    NVE_KEY_MOD_REPEAT, NVE_KEY_MOD_SHIFT,
};

pub type SurfaceId = u32;
pub type NodeId = u32;
pub type FrameId = u64;

/// Reserved root node ID for every surface.
pub const ROOT_NODE_ID: NodeId = 1;
