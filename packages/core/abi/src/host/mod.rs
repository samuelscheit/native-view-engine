//! Binary host-capability C ABI (`NVE_IFACE_HOST = 5`), reserved service IDs,
//! `NveHostStatus`, `NveHostCallbacks`, and `NveHostApi`.

use std::ffi::c_void;
use std::fmt;

use nve_abi_macros::NveAbi;

use crate::ffi::{NveAppHandle, NveBufferSlice};
use crate::InterfaceId;

pub mod batch;
#[cfg(test)]
mod tests;

pub use batch::{
    align8_payload_len, encode_single_push_into, host_batch_padded_record_len,
    single_record_batch_len, NveHostBatchError, NveHostEventBatchBuilder, NveHostEventBatchHeader,
    NveHostEventBatchIter, NveHostEventBatchRef, NveHostEventKind, NveHostEventRecord,
    NveHostEventRecordHeader, NVE_HOST_BATCH_ALIGN, NVE_HOST_BATCH_HEADER_LEN,
    NVE_HOST_BATCH_MAGIC, NVE_HOST_BATCH_MAX_BYTES, NVE_HOST_BATCH_MAX_PAYLOAD,
    NVE_HOST_BATCH_VERSION, NVE_HOST_RECORD_FLAG_RETAIN, NVE_HOST_RECORD_HEADER_LEN,
};

/// Modular interface ID for the optional host-capability & channel-bus interface (`const NveHostApi*`).
pub const NVE_IFACE_HOST: InterfaceId = InterfaceId(5);

/// Flag bit on `channel_publish` / `ChannelMessage` indicating retained state on `NveChannelBus`.
pub const NVE_CHANNEL_FLAG_RETAIN: u32 = 1 << 0;

/// Reserved built-in host service IDs (`0x0001..0x00FF`).
pub const SERVICE_CHANNEL: u32 = 0x0001;
pub const SERVICE_WINDOW: u32 = 0x0002;
/// `SERVICE_WINDOW` method requesting an asynchronous frame redraw from a background task or
/// `cdylib` app without mutating the 24-byte `NveHostCallbacks` C ABI struct.
pub const METHOD_WINDOW_REQUEST_FRAME: u32 = 1;
pub const SERVICE_CLIPBOARD: u32 = 0x0010;
pub const SERVICE_FILESYSTEM: u32 = 0x0011;
pub const SERVICE_SECURE_STORE: u32 = 0x0012;
pub const SERVICE_LINKING: u32 = 0x0013;
pub const SERVICE_SYSTEM_ENV: u32 = 0x0014;
pub const SERVICE_NOTIFICATIONS: u32 = 0x0015;
pub const SERVICE_HAPTICS: u32 = 0x0016;

/// Signed status codes returned by `NveHostCallbacks::invoke` and `channel_publish`.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, NveAbi)]
#[nve_abi(c_prefix = "NVE_HOST_")]
pub enum NveHostStatus {
    Ok = 0,
    Pending = 1,
    BufferTooSmall = -1,
    Unsupported = -2,
    PermissionDenied = -3,
    InvalidArg = -4,
    NotFound = -5,
    Cancelled = -6,
    IoError = -7,
    InternalError = -8,
}

impl NveHostStatus {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    pub const fn from_i32(code: i32) -> Self {
        match code {
            0 => Self::Ok,
            1 => Self::Pending,
            -1 => Self::BufferTooSmall,
            -2 => Self::Unsupported,
            -3 => Self::PermissionDenied,
            -4 => Self::InvalidArg,
            -5 => Self::NotFound,
            -6 => Self::Cancelled,
            -7 => Self::IoError,
            _ => Self::InternalError,
        }
    }

    pub const fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }

    pub const fn is_pending(self) -> bool {
        matches!(self, Self::Pending)
    }
}

impl fmt::Display for NveHostStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Ok => "ok",
            Self::Pending => "pending",
            Self::BufferTooSmall => "buffer_too_small",
            Self::Unsupported => "unsupported",
            Self::PermissionDenied => "permission_denied",
            Self::InvalidArg => "invalid_arg",
            Self::NotFound => "not_found",
            Self::Cancelled => "cancelled",
            Self::IoError => "io_error",
            Self::InternalError => "internal_error",
        };
        f.write_str(label)
    }
}

impl std::error::Error for NveHostStatus {}

/// Synchronous or asynchronous host capability invocation callback.
pub type NveHostInvokeFn = unsafe extern "C" fn(
    host_ctx: *mut c_void,
    service_id: u32,
    method_id: u32,
    request_id: u64,
    request: NveBufferSlice,
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32;

/// Publishes a binary message onto the host's `NveChannelBus`.
pub type NveHostChannelPublishFn = unsafe extern "C" fn(
    host_ctx: *mut c_void,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32;

/// Host-provided binary callback table bound via `NveHostApi::bind_host`.
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
#[nve_abi(size_64 = 24)]
pub struct NveHostCallbacks {
    #[nve_abi(offset64 = 0)]
    pub host_ctx: *mut c_void,
    #[nve_abi(offset64 = 8, c_type = "NveHostInvokeFn")]
    pub invoke: Option<NveHostInvokeFn>,
    #[nve_abi(offset64 = 16, c_type = "NveHostChannelPublishFn")]
    pub channel_publish: Option<NveHostChannelPublishFn>,
}

// SAFETY: `NveHostCallbacks` is a plain C function-pointer + opaque host context table;
// the host guarantees thread-safe invocation or single-threaded host-loop confinement.
unsafe impl Send for NveHostCallbacks {}
// SAFETY: `NveHostCallbacks` is a plain C function-pointer + opaque host context table;
// the host guarantees synchronized shared invocation across threads.
unsafe impl Sync for NveHostCallbacks {}

impl NveHostCallbacks {
    pub const EMPTY: Self = Self {
        host_ctx: std::ptr::null_mut(),
        invoke: None,
        channel_publish: None,
    };
}

/// Optional binary host-capability & channel-bus interface (`NVE_IFACE_HOST = 5`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveHostApi {
    pub bind_host:
        unsafe extern "C" fn(handle: NveAppHandle, callbacks: *const NveHostCallbacks),
    pub dispatch_host_events:
        unsafe extern "C" fn(handle: NveAppHandle, batch: NveBufferSlice) -> u32,
}
