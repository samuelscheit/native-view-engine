//! Canonical binary `NveHostEventBatch` wire format delivered via `NveHostApi::dispatch_host_events`.
//!
//! Layout (little-endian, 8-byte aligned records):
//! - Batch header (8 bytes): `magic = "NVHB"`, `version = 1u16`, `count: u16`.
//! - `count` records, each starting at an 8-byte offset:
//!   - [`NveHostEventRecordHeader`] (32 bytes): `kind`, `flags`, `_pad`, `status`,
//!     `service_id`, `method_or_event_or_channel`, `request_id`, `len`, `_pad2`.
//!   - `len` payload bytes, followed by `(8 - (len % 8)) % 8` zero padding bytes.

use std::fmt;
use std::mem::{align_of, offset_of, size_of};

use nve_abi_macros::NveAbi;
use nve_codec::{ByteReader, NveCodec, NveDecode};

use super::{HostRequestId, HostServiceId, NveHostStatus};

/// 4-byte ASCII magic prefix identifying a canonical `NveHostEventBatch` buffer.
pub const NVE_HOST_BATCH_MAGIC: [u8; 4] = *b"NVHB";

/// Current canonical `NveHostEventBatch` wire format version.
pub const NVE_HOST_BATCH_VERSION: u16 = 1;

/// Record alignment in bytes inside an `NveHostEventBatch`.
pub const NVE_HOST_BATCH_ALIGN: usize = 8;

/// Exact byte length of [`NveHostEventBatchHeader`].
pub const NVE_HOST_BATCH_HEADER_LEN: usize = 8;

/// Exact byte length of [`NveHostEventRecordHeader`].
pub const NVE_HOST_RECORD_HEADER_LEN: usize = 32;

/// Record flag bit indicating retained semantics for [`NveHostEventKind::Channel`] records.
pub const NVE_HOST_RECORD_FLAG_RETAIN: u8 = 1 << 0;

/// Defensive maximum single record payload size (64 MiB).
pub const NVE_HOST_BATCH_MAX_PAYLOAD: usize = 64 * 1024 * 1024;

/// Maximum byte budget per drained `NveHostEventBatch` (4 MiB).
///
/// Prevents a single drain from allocating multi-gigabyte buffers (`u16::MAX` × 64 MiB) on
/// constrained targets such as `wasm32`; a single valid record exceeding this budget still drains
/// alone when the builder is empty so head-of-line stalling cannot occur.
pub const NVE_HOST_BATCH_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Exact encoded byte length of one record (32-byte header + 8-byte-aligned payload).
#[inline]
pub const fn host_batch_padded_record_len(payload_len: usize) -> usize {
    NVE_HOST_RECORD_HEADER_LEN + ((payload_len + (NVE_HOST_BATCH_ALIGN - 1)) & !(NVE_HOST_BATCH_ALIGN - 1))
}

/// Discriminant for records inside an [`NveHostEventBatchRef`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, NveAbi, NveCodec)]
#[nve_abi(c_prefix = "NVE_HOST_EVENT_KIND_")]
pub enum NveHostEventKind {
    /// Completion of an asynchronous `NVE_HOST_PENDING` request.
    Response = 1,
    /// Unsolicited host-to-app push event (e.g. theme change, incoming URL).
    Push = 2,
    /// Inbound `NveChannelBus` binary message.
    Channel = 3,
}

impl NveHostEventKind {
    #[inline]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[inline]
    pub fn try_from_u8(raw: u8) -> Option<Self> {
        let buf = [raw];
        let mut reader = ByteReader::new(&buf);
        <Self as NveDecode>::decode(&mut reader).ok()
    }
}

/// 8-byte header at offset 0 of every canonical `NveHostEventBatch` buffer.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, NveAbi)]
#[nve_abi(size = 8)]
pub struct NveHostEventBatchHeader {
    #[nve_abi(offset = 0, c_type = "uint8_t[4]")]
    pub magic: [u8; 4],
    #[nve_abi(offset = 4)]
    pub version: u16,
    #[nve_abi(offset = 6)]
    pub count: u16,
}

/// 32-byte 8-byte-aligned header preceding each record payload in an `NveHostEventBatch`.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, NveAbi)]
#[nve_abi(size = 32)]
pub struct NveHostEventRecordHeader {
    #[nve_abi(offset = 0)]
    pub kind: u8,
    #[nve_abi(offset = 1)]
    pub flags: u8,
    #[nve_abi(offset = 2)]
    pub _pad: u16,
    #[nve_abi(offset = 4)]
    pub status: i32,
    #[nve_abi(offset = 8, c_type = "uint32_t")]
    pub service_id: HostServiceId,
    #[nve_abi(offset = 12)]
    pub method_or_event_or_channel: u32,
    #[nve_abi(offset = 16, c_type = "uint64_t")]
    pub request_id: HostRequestId,
    #[nve_abi(offset = 24)]
    pub len: u32,
    #[nve_abi(offset = 28)]
    pub _pad2: u32,
}

const _: () = {
    assert!(size_of::<NveHostEventBatchHeader>() == NVE_HOST_BATCH_HEADER_LEN);
    assert!(offset_of!(NveHostEventBatchHeader, magic) == 0);
    assert!(offset_of!(NveHostEventBatchHeader, version) == 4);
    assert!(offset_of!(NveHostEventBatchHeader, count) == 6);

    assert!(size_of::<NveHostEventRecordHeader>() == NVE_HOST_RECORD_HEADER_LEN);
    assert!(align_of::<NveHostEventRecordHeader>() == NVE_HOST_BATCH_ALIGN);
    assert!(offset_of!(NveHostEventRecordHeader, kind) == 0);
    assert!(offset_of!(NveHostEventRecordHeader, flags) == 1);
    assert!(offset_of!(NveHostEventRecordHeader, _pad) == 2);
    assert!(offset_of!(NveHostEventRecordHeader, status) == 4);
    assert!(offset_of!(NveHostEventRecordHeader, service_id) == 8);
    assert!(offset_of!(NveHostEventRecordHeader, method_or_event_or_channel) == 12);
    assert!(offset_of!(NveHostEventRecordHeader, request_id) == 16);
    assert!(offset_of!(NveHostEventRecordHeader, len) == 24);
    assert!(offset_of!(NveHostEventRecordHeader, _pad2) == 28);
};

/// Validation or encoding error for [`NveHostEventBatchRef`] / [`NveHostEventBatchBuilder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NveHostBatchError {
    TooShort,
    BadMagic,
    UnsupportedVersion(u16),
    InvalidKind { index: u16, raw: u8 },
    TruncatedRecord { index: u16 },
    TrailingBytes(usize),
    CountOverflow,
    PayloadTooLarge(usize),
    OutputBufferTooSmall { required: usize, available: usize },
}

impl fmt::Display for NveHostBatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooShort => f.write_str("NveHostEventBatch shorter than 8-byte header"),
            Self::BadMagic => f.write_str("NveHostEventBatch invalid magic (expected NVHB)"),
            Self::UnsupportedVersion(v) => {
                write!(f, "NveHostEventBatch unsupported version {v}")
            }
            Self::InvalidKind { index, raw } => {
                write!(f, "NveHostEventBatch invalid record kind {raw} at index {index}")
            }
            Self::TruncatedRecord { index } => {
                write!(f, "NveHostEventBatch truncated record at index {index}")
            }
            Self::TrailingBytes(n) => {
                write!(f, "NveHostEventBatch has {n} trailing bytes after declared records")
            }
            Self::CountOverflow => f.write_str("NveHostEventBatch record count exceeds u16::MAX"),
            Self::PayloadTooLarge(n) => {
                write!(f, "NveHostEventBatch record payload {n} exceeds limit")
            }
            Self::OutputBufferTooSmall {
                required,
                available,
            } => write!(
                f,
                "NveHostEventBatch output buffer too small (required {required}, got {available})"
            ),
        }
    }
}

impl std::error::Error for NveHostBatchError {}

/// Zero-copy borrowed view of one validated record inside an [`NveHostEventBatchRef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NveHostEventRecord<'a> {
    pub kind: NveHostEventKind,
    pub flags: u8,
    pub status: NveHostStatus,
    pub service_id: HostServiceId,
    pub method_or_event_or_channel: u32,
    pub request_id: HostRequestId,
    pub payload: &'a [u8],
}

impl<'a> NveHostEventRecord<'a> {
    #[inline]
    pub const fn is_retained(&self) -> bool {
        (self.flags & NVE_HOST_RECORD_FLAG_RETAIN) != 0
    }
}

/// Computes the 8-byte-aligned byte length for a payload of `len` bytes.
#[inline]
pub const fn align8_payload_len(len: usize) -> Option<usize> {
    match len.checked_add(NVE_HOST_BATCH_ALIGN - 1) {
        Some(v) => Some(v & !(NVE_HOST_BATCH_ALIGN - 1)),
        None => None,
    }
}

/// Exact encoded byte size of a single-record `NveHostEventBatch` with payload length `len`.
#[inline]
pub const fn single_record_batch_len(payload_len: usize) -> Option<usize> {
    let Some(padded) = align8_payload_len(payload_len) else {
        return None;
    };
    (NVE_HOST_BATCH_HEADER_LEN + NVE_HOST_RECORD_HEADER_LEN).checked_add(padded)
}

#[inline]
fn read_u16_le(bytes: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([bytes[off], bytes[off + 1]])
}

#[inline]
fn read_u32_le(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

#[inline]
fn read_i32_le(bytes: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

#[inline]
fn read_u64_le(bytes: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        bytes[off],
        bytes[off + 1],
        bytes[off + 2],
        bytes[off + 3],
        bytes[off + 4],
        bytes[off + 5],
        bytes[off + 6],
        bytes[off + 7],
    ])
}

/// Validated zero-allocation view over a canonical `NveHostEventBatch` byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NveHostEventBatchRef<'a> {
    bytes: &'a [u8],
    count: u16,
}

impl<'a> NveHostEventBatchRef<'a> {
    /// Validates the batch header and every record boundary in a single zero-allocation pass.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, NveHostBatchError> {
        if bytes.len() < NVE_HOST_BATCH_HEADER_LEN {
            return Err(NveHostBatchError::TooShort);
        }
        if bytes[0..4] != NVE_HOST_BATCH_MAGIC {
            return Err(NveHostBatchError::BadMagic);
        }
        let version = read_u16_le(bytes, 4);
        if version != NVE_HOST_BATCH_VERSION {
            return Err(NveHostBatchError::UnsupportedVersion(version));
        }
        let count = read_u16_le(bytes, 6);
        let mut cursor = NVE_HOST_BATCH_HEADER_LEN;
        for index in 0..count {
            let Some(rec_end) = cursor.checked_add(NVE_HOST_RECORD_HEADER_LEN) else {
                return Err(NveHostBatchError::TruncatedRecord { index });
            };
            if rec_end > bytes.len() {
                return Err(NveHostBatchError::TruncatedRecord { index });
            }
            let raw_kind = bytes[cursor];
            if NveHostEventKind::try_from_u8(raw_kind).is_none() {
                return Err(NveHostBatchError::InvalidKind {
                    index,
                    raw: raw_kind,
                });
            }
            let len = read_u32_le(bytes, cursor + 24) as usize;
            if len > NVE_HOST_BATCH_MAX_PAYLOAD {
                return Err(NveHostBatchError::PayloadTooLarge(len));
            }
            let Some(padded) = align8_payload_len(len) else {
                return Err(NveHostBatchError::TruncatedRecord { index });
            };
            let Some(next) = rec_end.checked_add(padded) else {
                return Err(NveHostBatchError::TruncatedRecord { index });
            };
            if next > bytes.len() {
                return Err(NveHostBatchError::TruncatedRecord { index });
            }
            cursor = next;
        }
        if cursor != bytes.len() {
            return Err(NveHostBatchError::TrailingBytes(bytes.len() - cursor));
        }
        Ok(Self { bytes, count })
    }

    #[inline]
    pub const fn count(&self) -> u16 {
        self.count
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[inline]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    pub fn iter(&self) -> NveHostEventBatchIter<'a> {
        NveHostEventBatchIter {
            bytes: self.bytes,
            cursor: NVE_HOST_BATCH_HEADER_LEN,
            remaining: self.count,
        }
    }
}

impl<'a> IntoIterator for NveHostEventBatchRef<'a> {
    type Item = NveHostEventRecord<'a>;
    type IntoIter = NveHostEventBatchIter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Zero-allocation iterator over validated records in an [`NveHostEventBatchRef`].
#[derive(Debug, Clone)]
pub struct NveHostEventBatchIter<'a> {
    bytes: &'a [u8],
    cursor: usize,
    remaining: u16,
}

impl<'a> Iterator for NveHostEventBatchIter<'a> {
    type Item = NveHostEventRecord<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let base = self.cursor;
        let kind = NveHostEventKind::try_from_u8(self.bytes[base])?;
        let flags = self.bytes[base + 1];
        let status = NveHostStatus::from_i32(read_i32_le(self.bytes, base + 4));
        let service_id = HostServiceId(read_u32_le(self.bytes, base + 8));
        let method_or_event_or_channel = read_u32_le(self.bytes, base + 12);
        let request_id = HostRequestId(read_u64_le(self.bytes, base + 16));
        let len = read_u32_le(self.bytes, base + 24) as usize;
        let payload_start = base + NVE_HOST_RECORD_HEADER_LEN;
        let payload_end = payload_start + len;
        let padded = align8_payload_len(len)?;
        self.cursor = payload_start + padded;
        self.remaining -= 1;
        Some(NveHostEventRecord {
            kind,
            flags,
            status,
            service_id,
            method_or_event_or_channel,
            request_id,
            payload: &self.bytes[payload_start..payload_end],
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = usize::from(self.remaining);
        (rem, Some(rem))
    }
}

impl ExactSizeIterator for NveHostEventBatchIter<'_> {}

/// Reusable encoder producing canonical 8-byte-aligned `NveHostEventBatch` buffers.
#[derive(Debug, Clone)]
pub struct NveHostEventBatchBuilder {
    buf: Vec<u8>,
    count: u16,
}

impl Default for NveHostEventBatchBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl NveHostEventBatchBuilder {
    pub fn new() -> Self {
        Self::with_capacity(64)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let mut buf = Vec::with_capacity(capacity.max(NVE_HOST_BATCH_HEADER_LEN));
        Self::write_header(&mut buf);
        Self { buf, count: 0 }
    }

    #[inline]
    fn write_header(buf: &mut Vec<u8>) {
        buf.clear();
        buf.extend_from_slice(&NVE_HOST_BATCH_MAGIC);
        buf.extend_from_slice(&NVE_HOST_BATCH_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
    }

    /// Resets the builder for reuse while retaining its heap allocation.
    #[inline]
    pub fn clear(&mut self) {
        Self::write_header(&mut self.buf);
        self.count = 0;
    }

    #[inline]
    pub const fn len(&self) -> u16 {
        self.count
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Appends one record of any [`NveHostEventKind`].
    #[allow(clippy::too_many_arguments)]
    pub fn push_record(
        &mut self,
        kind: NveHostEventKind,
        flags: u8,
        status: NveHostStatus,
        service_id: u32,
        method_or_event_or_channel: u32,
        request_id: u64,
        payload: &[u8],
    ) -> Result<(), NveHostBatchError> {
        let next_count = self
            .count
            .checked_add(1)
            .ok_or(NveHostBatchError::CountOverflow)?;
        if payload.len() > NVE_HOST_BATCH_MAX_PAYLOAD {
            return Err(NveHostBatchError::PayloadTooLarge(payload.len()));
        }
        let len_u32 = u32::try_from(payload.len())
            .map_err(|_| NveHostBatchError::PayloadTooLarge(payload.len()))?;
        let padded = align8_payload_len(payload.len())
            .ok_or(NveHostBatchError::PayloadTooLarge(payload.len()))?;
        let pad_bytes = padded - payload.len();

        self.buf
            .reserve(NVE_HOST_RECORD_HEADER_LEN.saturating_add(padded));
        self.buf.push(kind.as_u8());
        self.buf.push(flags);
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&status.as_i32().to_le_bytes());
        self.buf.extend_from_slice(&service_id.to_le_bytes());
        self.buf
            .extend_from_slice(&method_or_event_or_channel.to_le_bytes());
        self.buf.extend_from_slice(&request_id.to_le_bytes());
        self.buf.extend_from_slice(&len_u32.to_le_bytes());
        self.buf.extend_from_slice(&0u32.to_le_bytes());
        self.buf.extend_from_slice(payload);
        if pad_bytes > 0 {
            const ZEROS: [u8; NVE_HOST_BATCH_ALIGN] = [0u8; NVE_HOST_BATCH_ALIGN];
            self.buf.extend_from_slice(&ZEROS[..pad_bytes]);
        }

        self.count = next_count;
        let count_le = self.count.to_le_bytes();
        self.buf[6] = count_le[0];
        self.buf[7] = count_le[1];
        Ok(())
    }

    pub fn push_response(
        &mut self,
        service_id: u32,
        method_id: u32,
        request_id: u64,
        status: NveHostStatus,
        payload: &[u8],
    ) -> Result<(), NveHostBatchError> {
        self.push_record(
            NveHostEventKind::Response,
            0,
            status,
            service_id,
            method_id,
            request_id,
            payload,
        )
    }

    pub fn push_push(
        &mut self,
        service_id: u32,
        event_id: u32,
        payload: &[u8],
    ) -> Result<(), NveHostBatchError> {
        self.push_record(
            NveHostEventKind::Push,
            0,
            NveHostStatus::Ok,
            service_id,
            event_id,
            0,
            payload,
        )
    }

    pub fn push_channel(
        &mut self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), NveHostBatchError> {
        let flags = if retain {
            NVE_HOST_RECORD_FLAG_RETAIN
        } else {
            0
        };
        self.push_record(
            NveHostEventKind::Channel,
            flags,
            NveHostStatus::Ok,
            super::SERVICE_CHANNEL,
            channel_id,
            0,
            payload,
        )
    }

    /// Returns the finalized canonical batch bytes without consuming the builder.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Consumes the builder and returns the finalized canonical batch buffer.
    #[inline]
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// Zero-allocation stack encoder for a single `Push` record into a caller-supplied buffer.
pub fn encode_single_push_into<'a>(
    out: &'a mut [u8],
    service_id: u32,
    event_id: u32,
    payload: &[u8],
) -> Result<&'a [u8], NveHostBatchError> {
    if payload.len() > NVE_HOST_BATCH_MAX_PAYLOAD {
        return Err(NveHostBatchError::PayloadTooLarge(payload.len()));
    }
    let required = single_record_batch_len(payload.len())
        .ok_or(NveHostBatchError::PayloadTooLarge(payload.len()))?;
    if out.len() < required {
        return Err(NveHostBatchError::OutputBufferTooSmall {
            required,
            available: out.len(),
        });
    }
    let len_u32 = u32::try_from(payload.len())
        .map_err(|_| NveHostBatchError::PayloadTooLarge(payload.len()))?;

    out[0..4].copy_from_slice(&NVE_HOST_BATCH_MAGIC);
    out[4..6].copy_from_slice(&NVE_HOST_BATCH_VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&1u16.to_le_bytes());

    let r = NVE_HOST_BATCH_HEADER_LEN;
    out[r] = NveHostEventKind::Push.as_u8();
    out[r + 1] = 0;
    out[r + 2..r + 4].copy_from_slice(&0u16.to_le_bytes());
    out[r + 4..r + 8].copy_from_slice(&NveHostStatus::Ok.as_i32().to_le_bytes());
    out[r + 8..r + 12].copy_from_slice(&service_id.to_le_bytes());
    out[r + 12..r + 16].copy_from_slice(&event_id.to_le_bytes());
    out[r + 16..r + 24].copy_from_slice(&0u64.to_le_bytes());
    out[r + 24..r + 28].copy_from_slice(&len_u32.to_le_bytes());
    out[r + 28..r + 32].copy_from_slice(&0u32.to_le_bytes());

    let p_start = r + NVE_HOST_RECORD_HEADER_LEN;
    let p_end = p_start + payload.len();
    out[p_start..p_end].copy_from_slice(payload);
    out[p_end..required].fill(0);
    Ok(&out[..required])
}
