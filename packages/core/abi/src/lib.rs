//! Zero-copy `#[repr(C)]` raw-memory ABI (`nve_get_interface`, `nve_runtime_get_interface`,
//! `NveSceneView`, `NveTableView`,
//! `NveValue`, `NvePropSlice`, `NveCommandSlice`, `NveEventBatch`), plus optional binary wire
//! codecs (`EventRing`, `NveBinValue`) gated behind `feature = "wire"`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

extern crate self as nve_abi;

pub mod appearance;
pub mod bitflags;
#[cfg(feature = "wire")]
pub mod events;
pub mod ffi;
pub mod hash;
pub mod host;
#[cfg(feature = "wire")]
pub mod hybrid;
mod layout_asserts;
pub mod num;
pub mod runtime;
pub mod schema;
pub mod style_types;
pub mod table;
pub mod value;

pub use appearance::{ColorScheme, ContrastLevel};
pub use nve_abi_macros::NveAbi;
pub use nve_hash::{const_id, xxh64, Key64, Xxh64Hasher};

#[cfg(feature = "wire")]
pub use events::{
    decode_events, direct_event_to_native, encode_events, native_events_to_direct,
    pack_key_modifiers, EventMask, EventPayload, NativeEvent,
};
pub use ffi::{
    DecodeError, NativeEventKind, NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi,
    NveEventPayloadTag, NveGestureDisposition, NveGetInterfaceFn, NveIntrinsicSize,
    slab_ptr, NveLayoutApi, NveSizeConstraints, NveSurfaceApi, NVE_CORE_ABI_SCHEMA_HASH,
    NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
pub use host::{
    NveHostApi, NveHostCallbacks, NveHostChannelPublishFn, NveHostInvokeFn, NveHostStatus,
    METHOD_WINDOW_REQUEST_FRAME, NVE_CHANNEL_FLAG_RETAIN, SERVICE_CHANNEL, SERVICE_CLIPBOARD,
    SERVICE_FILESYSTEM, SERVICE_HAPTICS, SERVICE_LINKING, SERVICE_NOTIFICATIONS,
    SERVICE_SECURE_STORE, SERVICE_SYSTEM_ENV, SERVICE_WINDOW,
};
pub use table::NVE_PRIMARY_SLOT_NONE;
pub use runtime::{
    NveLayoutRect, NveNodeRow, NveRuntimeApi, NveRuntimeConfig, NveRuntimeGetInterfaceFn,
    NveRuntimeHandle, NveRuntimeMeasureTextFn, NveRuntimeMetrics, NveSceneView,
    NveRuntimeTextStyle, NveScrollOffset, NVE_LINE_HEIGHT_AUTO, NVE_LINE_HEIGHT_PERCENT,
    NVE_LINE_HEIGHT_POINTS,
    NveRuntimeStatus, NveRuntimeWakeFn, RuntimeFnSignature, NVE_LAYOUT_RECT_SIZE,
    NVE_NODE_ROW_FLAG_LIVE, NVE_NODE_ROW_SIZE, NVE_RUNTIME_ABI_VERSION, NVE_RUNTIME_SCHEMA_HASH,
    RUNTIME_API_SLOTS, RUNTIME_CALLBACK_TYPEDEFS,
};
pub use style_types::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
#[cfg(feature = "wire")]
pub use value::{
    encode_bin_value, encode_bin_value_to_vec, BinArrayIter, BinArrayWriter, BinMapIter,
    BinMapWriter, BinValueView, BinWriter, NveValueTree, NVE_BIN_VALUE_MAX_DEPTH,
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
