//! Minimal binary `NVE_IFACE_HOST = 5` host-service transport, FFI client, binary codec,
//! and composable handler registry (`nve-host-services`).
//!
//! This crate intentionally has **zero** dependencies on any `packages/modules/*` capability
//! crate. Each standalone capability module implements [`HostServiceHandler`] for its own
//! `service_id` and registers cleanly onto [`HostServiceRegistry`] or [`MemoryHostServices`].

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

#[cfg(target_os = "macos")]
pub mod apple_objc;
pub mod bridge;
pub mod codec;
pub mod completion;
pub mod error;
pub mod events;
pub mod ffi_client;
pub mod memory;
pub mod provider;
pub mod reentrant_slab;
pub mod registry;
pub mod request_table;
#[cfg(target_os = "windows")]
pub mod win32_str;

pub use bridge::{
    dispatch_ffi_channel_publish, dispatch_ffi_invoke, dispatch_ffi_invoke_with_cache,
    HostCallbackBridge, OversizedRetryCache,
};
pub use codec::{validate_url, MAX_PAYLOAD_BYTES, MAX_SEQ_ITEMS};
pub use completion::{CompletionSink, HandlerReply, DEFAULT_COMPLETION_QUEUE_CAP};
pub use error::ServiceError;
pub use nve_codec::{ByteReader, ByteWriter, CodecError, NveCodec, NveDecode, NveEncode};
pub use events::{
    TypedAsyncTable, TypedEventBus, TypedEventSubscription, TypedHostEvent,
    DEFAULT_TYPED_EVENT_QUEUE_CAP,
};
pub use ffi_client::{
    BatchIngestStats, FfiHostServices, RequestTable, FFI_MAX_RESPONSE_BYTES, FFI_STACK_BUF_SIZE,
};
pub use memory::MemoryHostServices;
pub use provider::{
    next_host_request_id, pack_typed, resolve_typed, HostChannelEvent, HostEvent, HostPushEvent,
    HostServiceHandler, HostServiceProvider, InvokeResult, PermissionStatus, ResponseBuf,
};
pub use reentrant_slab::ReentrantSlotSlab;
pub use registry::HostServiceRegistry;
