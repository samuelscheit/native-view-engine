//! Zero-copy `#[repr(C)]` raw-memory ABI (`nve_get_interface`, `nve_runtime_get_interface`,
//! `NveSceneView`, `NveTableView`,
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
#[cfg(feature = "wire")]
pub mod hybrid;
pub mod id;
mod layout_asserts;
pub mod num;
pub mod runtime;
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
    DecodeError, NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi,
    NveGestureDisposition, NveGetInterfaceFn, NveIntrinsicSize, slab_ptr, NveLayoutApi,
    NveSizeConstraints, NveSurfaceApi, NVE_CORE_ABI_SCHEMA_HASH, NVE_IFACE_COMPONENT,
    NVE_IFACE_DEV, NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
pub use host::{
    align8_payload_len, encode_single_push_into, host_batch_padded_record_len,
    single_record_batch_len, HostRequestId, HostServiceId, NveHostApi, NveHostBatchError,
    NveHostCallbacks,
    NveHostChannelPublishFn, NveHostEventBatchBuilder, NveHostEventBatchHeader,
    NveHostEventBatchIter, NveHostEventBatchRef, NveHostEventKind, NveHostEventRecord,
    NveHostEventRecordHeader, NveHostInvokeFn, NveHostStatus, METHOD_WINDOW_REQUEST_FRAME,
    NVE_CHANNEL_FLAG_RETAIN, NVE_HOST_BATCH_ALIGN, NVE_HOST_BATCH_HEADER_LEN, NVE_HOST_BATCH_MAGIC,
    NVE_HOST_BATCH_MAX_BYTES, NVE_HOST_BATCH_MAX_PAYLOAD, NVE_HOST_BATCH_VERSION,
    NVE_HOST_RECORD_FLAG_RETAIN, NVE_HOST_RECORD_HEADER_LEN, SERVICE_CHANNEL, SERVICE_CLIPBOARD,
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
    NveArrayRef, NveCommandSlice, NveCommandSliceRef, NveMapEntry, NveMapRef, NvePropEntry,
    NvePropSlice, NvePropSliceRef, NvePropsRef, NveStrSlice, NveValue, NveValueArray, NveValueData,
    NveValueMap, NveValueRef, NveValueTag,
};

pub use id::{
    ArenaOffset, ByteLen, DurationMs, DurationUs, EventFlags, EventId, FrameId, InterfaceId,
    NodeId, NodeIndex, PrimarySlot, PropFieldId, ShortLen, SurfaceId, TimestampMs, TimestampUs,
    ZIndex, ROOT_NODE_ID,
};
