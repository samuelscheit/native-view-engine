//! Zero-JSON host-capability C ABI (`NVE_IFACE_HOST = 5`), reserved service IDs,
//! `NveHostStatus`, `NveHostCallbacks`, `NveHostApi`, and the `NVH1` binary host-event
//! batch codec.

use std::ffi::c_void;
use std::fmt;

use crate::ffi::{NveAppHandle, NveBufferSlice};

/// Modular interface ID for the optional host-capability & channel-bus interface (`const NveHostApi*`).
pub const NVE_IFACE_HOST: u32 = 5;

/// Binary host-event batch magic bytes (`"NVH1"`).
pub const NVE_HOST_EVENT_MAGIC: [u8; 4] = *b"NVH1";

/// Binary host-event batch wire version.
pub const NVE_HOST_EVENT_VERSION: u16 = 1;

/// Flag bit on `channel_publish` / `ChannelMessage` indicating retained state on `NveChannelBus`.
pub const NVE_CHANNEL_FLAG_RETAIN: u32 = 1 << 0;

/// Reserved built-in host service IDs (`0x0001..0x00FF`).
pub const SERVICE_CHANNEL: u32 = 0x0001;
pub const SERVICE_WINDOW: u32 = 0x0002;
pub const SERVICE_CLIPBOARD: u32 = 0x0010;
pub const SERVICE_FILESYSTEM: u32 = 0x0011;
pub const SERVICE_SECURE_STORE: u32 = 0x0012;
pub const SERVICE_LINKING: u32 = 0x0013;
pub const SERVICE_SYSTEM_ENV: u32 = 0x0014;
pub const SERVICE_NOTIFICATIONS: u32 = 0x0015;
pub const SERVICE_HAPTICS: u32 = 0x0016;

const BATCH_HEADER_LEN: usize = 12;
const MIN_RECORD_WIRE_LEN: usize = 16;

/// Signed status codes returned by `NveHostCallbacks::invoke`, `channel_publish`, and `NVH1`
/// async response records.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
///
/// - Returns `NVE_HOST_OK (0)` after writing `*out_len` bytes into `out_buf[..*out_len]`.
/// - Returns `NVE_HOST_BUFFER_TOO_SMALL (-1)` and sets `*out_len = required_bytes` when `out_cap`
///   is insufficient; the caller reallocates and retries once.
/// - Returns `NVE_HOST_PENDING (1)` when the host will complete asynchronously via an `NVH1`
///   `AsyncResponse` record matching `request_id`.
/// - Returns any other negative `NveHostStatus` code on failure.
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

/// Publishes a zero-JSON binary message onto the host's `NveChannelBus`.
pub type NveHostChannelPublishFn = unsafe extern "C" fn(
    host_ctx: *mut c_void,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32;

/// Host-provided zero-JSON callback table bound via `NveHostApi::bind_host`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveHostCallbacks {
    pub host_ctx: *mut c_void,
    pub invoke: Option<NveHostInvokeFn>,
    pub channel_publish: Option<NveHostChannelPublishFn>,
}

// SAFETY: `NveHostCallbacks` is a plain C function-pointer + opaque host context table;
// the host guarantees thread-safe invocation or single-threaded host-loop confinement.
unsafe impl Send for NveHostCallbacks {}
unsafe impl Sync for NveHostCallbacks {}

impl NveHostCallbacks {
    pub const EMPTY: Self = Self {
        host_ctx: std::ptr::null_mut(),
        invoke: None,
        channel_publish: None,
    };
}

/// Optional zero-JSON host-capability & channel-bus interface (`NVE_IFACE_HOST = 5`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveHostApi {
    pub bind_host:
        unsafe extern "C" fn(handle: NveAppHandle, callbacks: *const NveHostCallbacks),
    pub dispatch_host_events:
        unsafe extern "C" fn(handle: NveAppHandle, batch: NveBufferSlice) -> u32,
}

/// Wire tag for each record inside an `NVH1` host-event batch.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NveHostEventKind {
    AsyncResponse = 1,
    HostPushEvent = 2,
    ChannelMessage = 3,
}

impl NveHostEventKind {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::AsyncResponse),
            2 => Some(Self::HostPushEvent),
            3 => Some(Self::ChannelMessage),
            _ => None,
        }
    }
}

/// Errors produced while decoding a binary `NVH1` host-event batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostBatchError {
    TruncatedHeader { actual: usize },
    InvalidMagic([u8; 4]),
    UnsupportedVersion(u16),
    RecordCountOverflow { count: u32, remaining: usize },
    TruncatedRecord { index: u32 },
    UnknownRecordKind { index: u32, kind: u8 },
    PayloadOutOfBounds { index: u32, declared: usize, remaining: usize },
    TrailingBytes { remaining: usize },
}

impl fmt::Display for HostBatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedHeader { actual } => {
                write!(f, "NVH1 header truncated: got {actual} bytes, need {BATCH_HEADER_LEN}")
            }
            Self::InvalidMagic(m) => write!(f, "invalid NVH1 magic: {m:?}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported NVH1 version: {v}"),
            Self::RecordCountOverflow { count, remaining } => {
                write!(f, "NVH1 record count {count} exceeds remaining {remaining} bytes")
            }
            Self::TruncatedRecord { index } => write!(f, "NVH1 record {index} header truncated"),
            Self::UnknownRecordKind { index, kind } => {
                write!(f, "NVH1 record {index} has unknown kind {kind}")
            }
            Self::PayloadOutOfBounds {
                index,
                declared,
                remaining,
            } => write!(
                f,
                "NVH1 record {index} payload length {declared} exceeds remaining {remaining} bytes"
            ),
            Self::TrailingBytes { remaining } => {
                write!(f, "NVH1 batch has {remaining} unexpected trailing bytes")
            }
        }
    }
}

impl std::error::Error for HostBatchError {}

/// Owned decoded `NVH1` host-event record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NveHostEventRecord {
    AsyncResponse {
        request_id: u64,
        status: NveHostStatus,
        payload: Vec<u8>,
    },
    HostPushEvent {
        service_id: u32,
        event_id: u32,
        payload: Vec<u8>,
    },
    ChannelMessage {
        channel_id: u32,
        flags: u32,
        payload: Vec<u8>,
    },
}

/// Zero-copy borrowed view of one `NVH1` record inside an encoded batch buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NveHostEventRecordRef<'a> {
    AsyncResponse {
        request_id: u64,
        status: NveHostStatus,
        payload: &'a [u8],
    },
    HostPushEvent {
        service_id: u32,
        event_id: u32,
        payload: &'a [u8],
    },
    ChannelMessage {
        channel_id: u32,
        flags: u32,
        payload: &'a [u8],
    },
}

impl<'a> NveHostEventRecordRef<'a> {
    pub fn to_owned_record(self) -> NveHostEventRecord {
        match self {
            Self::AsyncResponse {
                request_id,
                status,
                payload,
            } => NveHostEventRecord::AsyncResponse {
                request_id,
                status,
                payload: payload.to_vec(),
            },
            Self::HostPushEvent {
                service_id,
                event_id,
                payload,
            } => NveHostEventRecord::HostPushEvent {
                service_id,
                event_id,
                payload: payload.to_vec(),
            },
            Self::ChannelMessage {
                channel_id,
                flags,
                payload,
            } => NveHostEventRecord::ChannelMessage {
                channel_id,
                flags,
                payload: payload.to_vec(),
            },
        }
    }
}

/// Batched binary `NVH1` host-to-app event stream delivered via `NveHostApi::dispatch_host_events`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NveHostEventBatch {
    pub records: Vec<NveHostEventRecord>,
}

impl NveHostEventBatch {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    pub fn with_records(records: Vec<NveHostEventRecord>) -> Self {
        Self { records }
    }

    pub fn push(&mut self, record: NveHostEventRecord) {
        self.records.push(record);
    }

    pub fn push_async_response(
        &mut self,
        request_id: u64,
        status: NveHostStatus,
        payload: Vec<u8>,
    ) {
        self.records.push(NveHostEventRecord::AsyncResponse {
            request_id,
            status,
            payload,
        });
    }

    pub fn push_host_event(&mut self, service_id: u32, event_id: u32, payload: Vec<u8>) {
        self.records.push(NveHostEventRecord::HostPushEvent {
            service_id,
            event_id,
            payload,
        });
    }

    pub fn push_channel_message(&mut self, channel_id: u32, flags: u32, payload: Vec<u8>) {
        self.records.push(NveHostEventRecord::ChannelMessage {
            channel_id,
            flags,
            payload,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Encodes this batch into a fresh `NVH1` binary byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(BATCH_HEADER_LEN + self.records.len() * 24);
        self.encode_into(&mut out);
        out
    }

    /// Appends the `NVH1` binary encoding of this batch into `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        let count = u32::try_from(self.records.len()).expect("NVH1 record count exceeds u32::MAX");
        out.extend_from_slice(&NVE_HOST_EVENT_MAGIC);
        out.extend_from_slice(&NVE_HOST_EVENT_VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());

        for rec in &self.records {
            match rec {
                NveHostEventRecord::AsyncResponse {
                    request_id,
                    status,
                    payload,
                } => {
                    let plen =
                        u32::try_from(payload.len()).expect("NVH1 payload exceeds u32::MAX");
                    out.push(NveHostEventKind::AsyncResponse as u8);
                    out.extend_from_slice(&[0u8; 3]);
                    out.extend_from_slice(&request_id.to_le_bytes());
                    out.extend_from_slice(&status.as_i32().to_le_bytes());
                    out.extend_from_slice(&plen.to_le_bytes());
                    out.extend_from_slice(payload);
                }
                NveHostEventRecord::HostPushEvent {
                    service_id,
                    event_id,
                    payload,
                } => {
                    let plen =
                        u32::try_from(payload.len()).expect("NVH1 payload exceeds u32::MAX");
                    out.push(NveHostEventKind::HostPushEvent as u8);
                    out.extend_from_slice(&[0u8; 3]);
                    out.extend_from_slice(&service_id.to_le_bytes());
                    out.extend_from_slice(&event_id.to_le_bytes());
                    out.extend_from_slice(&plen.to_le_bytes());
                    out.extend_from_slice(payload);
                }
                NveHostEventRecord::ChannelMessage {
                    channel_id,
                    flags,
                    payload,
                } => {
                    let plen =
                        u32::try_from(payload.len()).expect("NVH1 payload exceeds u32::MAX");
                    out.push(NveHostEventKind::ChannelMessage as u8);
                    out.extend_from_slice(&[0u8; 3]);
                    out.extend_from_slice(&channel_id.to_le_bytes());
                    out.extend_from_slice(&flags.to_le_bytes());
                    out.extend_from_slice(&plen.to_le_bytes());
                    out.extend_from_slice(payload);
                }
            }
        }
    }

    /// Validates and decodes an `NVH1` byte slice into zero-copy borrowed records.
    pub fn decode_refs(bytes: &[u8]) -> Result<Vec<NveHostEventRecordRef<'_>>, HostBatchError> {
        if bytes.len() < BATCH_HEADER_LEN {
            return Err(HostBatchError::TruncatedHeader {
                actual: bytes.len(),
            });
        }
        let magic: [u8; 4] = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if magic != NVE_HOST_EVENT_MAGIC {
            return Err(HostBatchError::InvalidMagic(magic));
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != NVE_HOST_EVENT_VERSION {
            return Err(HostBatchError::UnsupportedVersion(version));
        }
        let record_count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let remaining_after_hdr = bytes.len() - BATCH_HEADER_LEN;
        if (record_count as usize) > remaining_after_hdr / MIN_RECORD_WIRE_LEN {
            return Err(HostBatchError::RecordCountOverflow {
                count: record_count,
                remaining: remaining_after_hdr,
            });
        }

        let mut records = Vec::with_capacity(record_count as usize);
        let mut cursor = BATCH_HEADER_LEN;

        for index in 0..record_count {
            let Some(&kind_raw) = bytes.get(cursor) else {
                return Err(HostBatchError::TruncatedRecord { index });
            };
            let kind = NveHostEventKind::from_u8(kind_raw).ok_or(
                HostBatchError::UnknownRecordKind {
                    index,
                    kind: kind_raw,
                },
            )?;
            let Some(after_tag) = cursor.checked_add(4) else {
                return Err(HostBatchError::TruncatedRecord { index });
            };
            cursor = after_tag;

            match kind {
                NveHostEventKind::AsyncResponse => {
                    let Some(hdr_end) = cursor.checked_add(16) else {
                        return Err(HostBatchError::TruncatedRecord { index });
                    };
                    if hdr_end > bytes.len() {
                        return Err(HostBatchError::TruncatedRecord { index });
                    }
                    let request_id = u64::from_le_bytes(
                        bytes[cursor..cursor + 8].try_into().unwrap(),
                    );
                    let status_raw = i32::from_le_bytes(
                        bytes[cursor + 8..cursor + 12].try_into().unwrap(),
                    );
                    let payload_len = u32::from_le_bytes(
                        bytes[cursor + 12..cursor + 16].try_into().unwrap(),
                    ) as usize;
                    cursor = hdr_end;
                    let remaining = bytes.len() - cursor;
                    if payload_len > remaining {
                        return Err(HostBatchError::PayloadOutOfBounds {
                            index,
                            declared: payload_len,
                            remaining,
                        });
                    }
                    let payload = &bytes[cursor..cursor + payload_len];
                    cursor += payload_len;
                    records.push(NveHostEventRecordRef::AsyncResponse {
                        request_id,
                        status: NveHostStatus::from_i32(status_raw),
                        payload,
                    });
                }
                NveHostEventKind::HostPushEvent => {
                    let Some(hdr_end) = cursor.checked_add(12) else {
                        return Err(HostBatchError::TruncatedRecord { index });
                    };
                    if hdr_end > bytes.len() {
                        return Err(HostBatchError::TruncatedRecord { index });
                    }
                    let service_id = u32::from_le_bytes(
                        bytes[cursor..cursor + 4].try_into().unwrap(),
                    );
                    let event_id = u32::from_le_bytes(
                        bytes[cursor + 4..cursor + 8].try_into().unwrap(),
                    );
                    let payload_len = u32::from_le_bytes(
                        bytes[cursor + 8..cursor + 12].try_into().unwrap(),
                    ) as usize;
                    cursor = hdr_end;
                    let remaining = bytes.len() - cursor;
                    if payload_len > remaining {
                        return Err(HostBatchError::PayloadOutOfBounds {
                            index,
                            declared: payload_len,
                            remaining,
                        });
                    }
                    let payload = &bytes[cursor..cursor + payload_len];
                    cursor += payload_len;
                    records.push(NveHostEventRecordRef::HostPushEvent {
                        service_id,
                        event_id,
                        payload,
                    });
                }
                NveHostEventKind::ChannelMessage => {
                    let Some(hdr_end) = cursor.checked_add(12) else {
                        return Err(HostBatchError::TruncatedRecord { index });
                    };
                    if hdr_end > bytes.len() {
                        return Err(HostBatchError::TruncatedRecord { index });
                    }
                    let channel_id = u32::from_le_bytes(
                        bytes[cursor..cursor + 4].try_into().unwrap(),
                    );
                    let flags = u32::from_le_bytes(
                        bytes[cursor + 4..cursor + 8].try_into().unwrap(),
                    );
                    let payload_len = u32::from_le_bytes(
                        bytes[cursor + 8..cursor + 12].try_into().unwrap(),
                    ) as usize;
                    cursor = hdr_end;
                    let remaining = bytes.len() - cursor;
                    if payload_len > remaining {
                        return Err(HostBatchError::PayloadOutOfBounds {
                            index,
                            declared: payload_len,
                            remaining,
                        });
                    }
                    let payload = &bytes[cursor..cursor + payload_len];
                    cursor += payload_len;
                    records.push(NveHostEventRecordRef::ChannelMessage {
                        channel_id,
                        flags,
                        payload,
                    });
                }
            }
        }

        if cursor != bytes.len() {
            return Err(HostBatchError::TrailingBytes {
                remaining: bytes.len() - cursor,
            });
        }

        Ok(records)
    }

    /// Validates and decodes an `NVH1` byte slice into an owned [`NveHostEventBatch`].
    pub fn decode(bytes: &[u8]) -> Result<Self, HostBatchError> {
        let refs = Self::decode_refs(bytes)?;
        Ok(Self {
            records: refs.into_iter().map(NveHostEventRecordRef::to_owned_record).collect(),
        })
    }
}


#[cfg(test)]
mod tests;
