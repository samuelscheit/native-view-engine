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
pub mod error;
pub mod events;
pub mod ffi_client;
pub mod provider;
pub mod registry;
#[cfg(target_os = "windows")]
pub mod win32_str;

pub use bridge::HostCallbackBridge;
pub use codec::{
    validate_url, BinReader, BinWriter, MAX_SEQ_ITEMS,
};
pub use error::ServiceError;
pub use events::{TypedAsyncTable, TypedEventBus, TypedEventSubscription, TypedHostEvent};
pub use ffi_client::{FfiHostServices, RequestTable, FFI_MAX_RESPONSE_BYTES, FFI_STACK_BUF_SIZE};
pub use provider::{
    pack_typed, resolve_typed, HostChannelEvent, HostEvent, HostPushEvent, HostServiceHandler,
    HostServiceProvider, InvokeResult, PermissionStatus, ResponseBuf,
};
pub use registry::{HostServiceRegistry, MemoryHostServices};
