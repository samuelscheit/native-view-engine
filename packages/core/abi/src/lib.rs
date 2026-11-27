//! Zero-copy `#[repr(C)]` raw-memory ABI (`nve_get_interface`, `NveSceneView`, `NveTableView`,
//! `NveValue`, `NvePropSlice`, `NveCommandSlice`, `NveEventBatch`), plus optional binary wire
//! codecs (`EventRing`, `NveBinValue`) gated behind `feature = "wire"`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

extern crate self as nve_abi;

pub mod appearance;
pub mod bitflags;
pub mod event;
pub mod ffi;
pub mod hash;
pub mod host;
pub mod id;
mod layout_asserts;
pub mod num;
pub mod scene;
pub mod schema;
pub mod style_types;
pub mod table;
pub mod value;

pub use appearance::{ColorScheme, ContrastLevel};
pub use event::{
    EventBatchBuf, EventEnum, EventSet, EventSetIter, EventValidationError, InteractionEvent,
    LayoutEvent, NveEvent, NveEventBatch, RawEventBytes, ScrollEvent, TextInputEvent,
    ValidatedEvent, ValidatedEvents, ValidatedEventsIter, NVE_EVENT_ALIGN,
    NVE_EVENT_INLINE_KEY_CAP, NVE_EVENT_SIZE, NVE_EVENT_WIRE_HEADER_SIZE, NVE_EVENT_WIRE_MAGIC,
    NVE_EVENT_WIRE_VERSION, NVE_TABLE_CUSTOM, NVE_TABLE_INTERACTION, NVE_TABLE_LAYOUT,
    NVE_TABLE_SCROLL, NVE_TABLE_TEXT_INPUT,
};
pub use nve_abi_macros::{EventEnum, NveAbi};
pub use nve_hash::{const_id, xxh64, Key64, Xxh64Hasher};

pub use ffi::{
    DecodeError, NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi, NveFrameStats,
    NveGestureDisposition, NveGetInterfaceFn, NveIntrinsicSize, NveLayoutApi, NveSizeConstraints,
    NveSurfaceApi, NveSurfaceConfig, NVE_CORE_ABI_SCHEMA_HASH, NVE_IFACE_COMPONENT, NVE_IFACE_DEV,
    NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
pub use host::{
    NveHostApi, NveHostCallbacks, NveHostChannelPublishFn, NveHostInvokeFn, NveHostStatus,
    METHOD_WINDOW_REQUEST_FRAME, NVE_CHANNEL_FLAG_RETAIN, SERVICE_CHANNEL, SERVICE_CLIPBOARD,
    SERVICE_FILESYSTEM, SERVICE_HAPTICS, SERVICE_LINKING, SERVICE_NOTIFICATIONS,
    SERVICE_SECURE_STORE, SERVICE_SYSTEM_ENV, SERVICE_WINDOW,
};
pub use scene::{
    NveNodeMeta, NveRenderNode, NveSceneRef, NveSceneTable, NveSceneView, SceneError,
    SceneNodeInput, NVE_META_FLAG_ACCESSIBILITY_HIDDEN, NVE_META_FLAG_DISABLED,
    NVE_META_FLAG_MULTILINE, NVE_META_FLAG_SECURE_TEXT_ENTRY, NVE_META_RESIZE_MODE_MASK,
    NVE_META_RESIZE_MODE_SHIFT, NVE_NODE_CLIP_BOUNDS, NVE_NODE_INTERACTIVE, NVE_NODE_LEAF_TEXT,
    NVE_NODE_META_ALIGN, NVE_NODE_META_SIZE, NVE_PRIMARY_SLOT_NONE, NVE_RENDER_NODE_ALIGN,
    NVE_RENDER_NODE_SIZE, NVE_SCENE_SCHEMA_HASH,
};
pub use style_types::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
#[cfg(feature = "wire")]
pub use value::{
    encode_bin_value, encode_bin_value_to_vec, BinArrayIter, BinMapIter, BinValueView,
    NVE_BIN_VALUE_MAX_DEPTH,
};
pub use value::{
    NveArrayRef, NveCommandSlice, NveCommandSliceRef, NveMapEntry, NveMapRef, NvePropEntry,
    NvePropSlice, NvePropSliceRef, NvePropsRef, NveStrSlice, NveValue, NveValueArray, NveValueData,
    NveValueMap, NveValueRef, NveValueTag,
};

pub use id::{
    ArenaOffset, ByteLen, DurationMs, DurationUs, EventFlags, EventId, FrameId, InterfaceId,
    NodeId, NodeIndex, PrimarySlot, PropFieldId, ShortLen, SurfaceId, TimestampMs, TimestampUs,
    ZIndex, ROOT_NODE_ID,
};
