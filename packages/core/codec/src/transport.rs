//! Framework- and runtime-agnostic binary byte transport (`ByteTransport`), error model
//! (`TransportError`), and zero-waste stack-first request encoding helpers (`StackWriter`,
//! `with_stack_writer`, and POD stack encoders).
//!
//! Lives in `nve-codec` (`#![forbid(unsafe_code)]`, zero runtime/engine/ABI dependencies) so
//! capability modules, RPC clients, and host adapters can share one clean transport abstraction
//! without coupling to `nve-abi` or `nve-host-services`.

use core::fmt;
use std::sync::Arc;

use crate::{
    canonical_f32_bits, canonical_f64_bits, try_write_fixed, ByteSink, ByteWriter, CodecError,
    SliceWriter,
};

/// Transport-layer error decoupled from any specific FFI status table or runtime crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// Service or method is not supported on the active transport or platform.
    Unsupported,
    /// Caller lacks permission for the requested capability or operation.
    PermissionDenied,
    /// Caller supplied invalid arguments or a malformed payload.
    Invalid(String),
    /// Requested resource, handle, or key was not found.
    NotFound,
    /// Operation was cancelled (user dismissal, surface teardown, or hot reload).
    Cancelled,
    /// Underlying I/O or link-layer failure.
    Io(String),
    /// Payload exceeded the configured or negotiated buffer capacity.
    BufferOverflow { needed: usize, capacity: usize },
    /// Binary codec serialization or deserialization failed.
    Codec(CodecError),
    /// Transport-specific non-zero status code (for raw foreign/C-ABI errors).
    Status(i32),
    /// Internal transport or peer failure.
    Internal(String),
}

impl From<CodecError> for TransportError {
    #[inline]
    fn from(err: CodecError) -> Self {
        Self::Codec(err)
    }
}

impl From<std::io::Error> for TransportError {
    #[inline]
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("transport operation unsupported"),
            Self::PermissionDenied => f.write_str("transport permission denied"),
            Self::Invalid(msg) => write!(f, "invalid transport argument: {msg}"),
            Self::NotFound => f.write_str("transport resource not found"),
            Self::Cancelled => f.write_str("transport operation cancelled"),
            Self::Io(msg) => write!(f, "transport I/O error: {msg}"),
            Self::BufferOverflow { needed, capacity } => {
                write!(
                    f,
                    "transport buffer overflow: needed {needed}B, capacity {capacity}B"
                )
            }
            Self::Codec(err) => write!(f, "transport codec error: {err}"),
            Self::Status(code) => write!(f, "transport status error ({code})"),
            Self::Internal(msg) => write!(f, "internal transport error: {msg}"),
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(err) => Some(err),
            _ => None,
        }
    }
}

/// Callback invoked once when an asynchronous [`ByteTransport::call_async`] completes.
pub type TransportDoneFn = Box<dyn FnOnce(Result<&[u8], TransportError>) + Send>;

/// Pluggable binary request/reply transport with zero-copy synchronous reply delivery and
/// callback-driven asynchronous completion.
///
/// Implementations may bridge in-process registries, direct C-ABI callback sinks, IPC sockets,
/// WebWorkers, or deterministic unit-test harnesses without coupling callers to any concrete host.
pub trait ByteTransport: Send + Sync {
    /// Synchronously invokes `(service_id, method_id)` with `req`, delivering reply bytes to
    /// `on_reply` while the transport's stack or borrowed reply buffer is still live (enabling
    /// zero-allocation reply decoding on the fast path).
    fn call_sync(
        &self,
        service_id: u32,
        method_id: u32,
        req: &[u8],
        on_reply: &mut dyn FnMut(&[u8]),
    ) -> Result<(), TransportError>;

    /// Initiates an asynchronous invocation of `(service_id, method_id)` with `req`, delivering
    /// the borrowed reply slice or [`TransportError`] to `on_done` exactly once on completion.
    fn call_async(
        &self,
        service_id: u32,
        method_id: u32,
        req: &[u8],
        on_done: TransportDoneFn,
    );
}

impl<T: ByteTransport + ?Sized> ByteTransport for &T {
    #[inline]
    fn call_sync(
        &self,
        service_id: u32,
        method_id: u32,
        req: &[u8],
        on_reply: &mut dyn FnMut(&[u8]),
    ) -> Result<(), TransportError> {
        (**self).call_sync(service_id, method_id, req, on_reply)
    }

    #[inline]
    fn call_async(&self, service_id: u32, method_id: u32, req: &[u8], on_done: TransportDoneFn) {
        (**self).call_async(service_id, method_id, req, on_done);
    }
}

impl<T: ByteTransport + ?Sized> ByteTransport for Box<T> {
    #[inline]
    fn call_sync(
        &self,
        service_id: u32,
        method_id: u32,
        req: &[u8],
        on_reply: &mut dyn FnMut(&[u8]),
    ) -> Result<(), TransportError> {
        (**self).call_sync(service_id, method_id, req, on_reply)
    }

    #[inline]
    fn call_async(&self, service_id: u32, method_id: u32, req: &[u8], on_done: TransportDoneFn) {
        (**self).call_async(service_id, method_id, req, on_done);
    }
}

impl<T: ByteTransport + ?Sized> ByteTransport for Arc<T> {
    #[inline]
    fn call_sync(
        &self,
        service_id: u32,
        method_id: u32,
        req: &[u8],
        on_reply: &mut dyn FnMut(&[u8]),
    ) -> Result<(), TransportError> {
        (**self).call_sync(service_id, method_id, req, on_reply)
    }

    #[inline]
    fn call_async(&self, service_id: u32, method_id: u32, req: &[u8], on_done: TransportDoneFn) {
        (**self).call_async(service_id, method_id, req, on_done);
    }
}

/// Zero-allocation helper calling [`ByteTransport::call_sync`] and decoding the borrowed reply
/// slice in place without copying into an intermediate `Vec<u8>`.
#[inline]
pub fn call_sync_decode<T, F>(
    transport: &(impl ByteTransport + ?Sized),
    service_id: u32,
    method_id: u32,
    req: &[u8],
    mut decode: F,
) -> Result<T, TransportError>
where
    F: FnMut(&[u8]) -> Result<T, TransportError>,
{
    let mut decoded: Option<Result<T, TransportError>> = None;
    transport.call_sync(service_id, method_id, req, &mut |reply| {
        decoded = Some(decode(reply));
    })?;
    match decoded {
        Some(res) => res,
        None => decode(&[]),
    }
}

/// Stack-first binary writer backed by [`SliceWriter`] on the fast path, spilling seamlessly into
/// a heap [`ByteWriter`] (`Vec<u8>`) only when the encoded payload exceeds the stack buffer.
#[derive(Debug)]
pub enum StackWriter<'a> {
    Stack(SliceWriter<'a>),
    Heap(ByteWriter),
}

impl<'a> StackWriter<'a> {
    #[inline]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self::Stack(SliceWriter::new(buf))
    }

    #[inline]
    pub fn is_spilled(&self) -> bool {
        matches!(self, Self::Heap(_))
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Stack(sw) => sw.pos(),
            Self::Heap(bw) => bw.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Stack(sw) => sw.written(),
            Self::Heap(bw) => bw.as_slice(),
        }
    }

    #[inline]
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            Self::Stack(sw) => sw.written().to_vec(),
            Self::Heap(bw) => bw.into_bytes(),
        }
    }

    #[cold]
    #[inline(never)]
    fn spill_with_additional(&mut self, additional: usize) -> Result<&mut ByteWriter, CodecError> {
        let written = match self {
            Self::Stack(sw) => sw.written(),
            Self::Heap(bw) => return Ok(bw),
        };
        let cur_len = written.len();
        let max_additional = (isize::MAX as usize).saturating_sub(cur_len);
        if additional > max_additional {
            return Err(CodecError::LenOverflow {
                len: additional,
                max: max_additional,
            });
        }
        let target_cap = cur_len.saturating_add(additional);
        let mut vec = Vec::new();
        if vec.try_reserve(target_cap).is_err() {
            return Err(CodecError::LenOverflow {
                len: target_cap,
                max: max_additional,
            });
        }
        vec.extend_from_slice(written);
        *self = Self::Heap(ByteWriter::from_vec(vec));
        match self {
            Self::Heap(bw) => Ok(bw),
            Self::Stack(_) => Err(CodecError::LenOverflow {
                len: target_cap,
                max: max_additional,
            }),
        }
    }

    #[inline]
    pub fn write_raw(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        ByteSink::put_raw(self, bytes)
    }

    #[inline]
    pub fn write_zeroes(&mut self, count: usize) -> Result<(), CodecError> {
        ByteSink::put_zeroes(self, count)
    }

    #[inline]
    pub fn write_u8(&mut self, v: u8) -> Result<(), CodecError> {
        ByteSink::put_u8(self, v)
    }

    #[inline]
    pub fn write_i8(&mut self, v: i8) -> Result<(), CodecError> {
        ByteSink::put_i8(self, v)
    }

    #[inline]
    pub fn write_bool(&mut self, v: bool) -> Result<(), CodecError> {
        ByteSink::put_bool(self, v)
    }

    #[inline]
    pub fn write_u16(&mut self, v: u16) -> Result<(), CodecError> {
        ByteSink::put_u16(self, v)
    }

    #[inline]
    pub fn write_i16(&mut self, v: i16) -> Result<(), CodecError> {
        ByteSink::put_i16(self, v)
    }

    #[inline]
    pub fn write_u32(&mut self, v: u32) -> Result<(), CodecError> {
        ByteSink::put_u32(self, v)
    }

    #[inline]
    pub fn write_i32(&mut self, v: i32) -> Result<(), CodecError> {
        ByteSink::put_i32(self, v)
    }

    #[inline]
    pub fn write_u64(&mut self, v: u64) -> Result<(), CodecError> {
        ByteSink::put_u64(self, v)
    }

    #[inline]
    pub fn write_i64(&mut self, v: i64) -> Result<(), CodecError> {
        ByteSink::put_i64(self, v)
    }

    #[inline]
    pub fn write_f32(&mut self, v: f32) -> Result<(), CodecError> {
        ByteSink::put_f32(self, v)
    }

    #[inline]
    pub fn write_f32_raw(&mut self, v: f32) -> Result<(), CodecError> {
        ByteSink::put_f32_raw(self, v)
    }

    #[inline]
    pub fn write_f64(&mut self, v: f64) -> Result<(), CodecError> {
        ByteSink::put_f64(self, v)
    }

    #[inline]
    pub fn write_f64_raw(&mut self, v: f64) -> Result<(), CodecError> {
        ByteSink::put_f64_raw(self, v)
    }

    #[inline]
    pub fn write_len(&mut self, len: usize) -> Result<(), CodecError> {
        ByteSink::put_len(self, len)
    }

    #[inline]
    pub fn write_len_max(&mut self, len: usize, max: usize) -> Result<(), CodecError> {
        ByteSink::put_len_max(self, len, max)
    }

    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        ByteSink::put_bytes(self, bytes)
    }

    #[inline]
    pub fn write_bytes_max(&mut self, bytes: &[u8], max: usize) -> Result<(), CodecError> {
        ByteSink::put_bytes_max(self, bytes, max)
    }

    #[inline]
    pub fn write_bytes_u16(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        ByteSink::put_bytes_u16(self, bytes)
    }

    #[inline]
    pub fn write_bytes_u8(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        ByteSink::put_bytes_u8(self, bytes)
    }

    #[inline]
    pub fn write_str(&mut self, s: &str) -> Result<(), CodecError> {
        ByteSink::put_str(self, s)
    }

    #[inline]
    pub fn write_str_max(&mut self, s: &str, max: usize) -> Result<(), CodecError> {
        ByteSink::put_str_max(self, s, max)
    }

    #[inline]
    pub fn write_str_u16(&mut self, s: &str) -> Result<(), CodecError> {
        ByteSink::put_str_u16(self, s)
    }

    #[inline]
    pub fn write_str_u8(&mut self, s: &str) -> Result<(), CodecError> {
        ByteSink::put_str_u8(self, s)
    }

    #[inline]
    pub fn write_opt<T>(
        &mut self,
        opt: Option<T>,
        write_some: impl FnOnce(&mut Self, T) -> Result<(), CodecError>,
    ) -> Result<(), CodecError> {
        ByteSink::put_opt(self, opt, write_some)
    }
}

impl ByteSink for StackWriter<'_> {
    #[inline]
    fn put_raw(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        match self {
            Self::Stack(sw) => {
                if bytes.len() <= sw.remaining() {
                    sw.put_raw(bytes)
                } else {
                    let bw = self.spill_with_additional(bytes.len())?;
                    bw.write_raw(bytes);
                    Ok(())
                }
            }
            Self::Heap(bw) => {
                bw.write_raw(bytes);
                Ok(())
            }
        }
    }

    #[inline]
    fn put_zeroes(&mut self, count: usize) -> Result<(), CodecError> {
        match self {
            Self::Stack(sw) => {
                if count <= sw.remaining() {
                    sw.put_zeroes(count)
                } else {
                    let bw = self.spill_with_additional(count)?;
                    bw.write_zeroes(count)
                }
            }
            Self::Heap(bw) => bw.write_zeroes(count),
        }
    }
}

/// Encodes a multi-field binary payload into a stack `[u8; N]` buffer via [`SliceWriter`] with
/// **0 heap allocations** on the fast path, spilling seamlessly to `Vec<u8>` only on overflow,
/// and passes the encoded slice to `use_bytes`.
#[inline]
pub fn with_stack_writer<const N: usize, R>(
    encode: impl FnOnce(&mut StackWriter<'_>) -> Result<(), CodecError>,
    use_bytes: impl FnOnce(&[u8]) -> R,
) -> Result<R, CodecError> {
    let mut stack = [0u8; N];
    let mut writer = StackWriter::new(&mut stack);
    encode(&mut writer)?;
    Ok(use_bytes(writer.as_slice()))
}

/// Fallible companion to [`with_stack_writer`] flattening `CodecError` into a caller error type
/// `E` (such as [`TransportError`]) so callers can use a single `?`.
#[inline]
pub fn try_with_stack_writer<const N: usize, R, E: From<CodecError>>(
    encode: impl FnOnce(&mut StackWriter<'_>) -> Result<(), CodecError>,
    use_bytes: impl FnOnce(&[u8]) -> Result<R, E>,
) -> Result<R, E> {
    let mut stack = [0u8; N];
    let mut writer = StackWriter::new(&mut stack);
    encode(&mut writer)?;
    use_bytes(writer.as_slice())
}

/// Fallible zero-allocation fixed-size stack POD encoder (`N` bytes exact).
#[inline]
pub fn encode_pod_stack<const N: usize>(
    write: impl FnOnce(&mut SliceWriter<'_>) -> Result<(), CodecError>,
) -> Result<[u8; N], CodecError> {
    try_write_fixed(write)
}

#[inline(always)]
pub const fn encode_u8_stack(v: u8) -> [u8; 1] {
    [v]
}

#[inline(always)]
pub const fn encode_i8_stack(v: i8) -> [u8; 1] {
    [v as u8]
}

#[inline(always)]
pub const fn encode_bool_stack(v: bool) -> [u8; 1] {
    [v as u8]
}

#[inline(always)]
pub const fn encode_u16_stack(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}

#[inline(always)]
pub const fn encode_i16_stack(v: i16) -> [u8; 2] {
    v.to_le_bytes()
}

#[inline(always)]
pub const fn encode_u32_stack(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

#[inline(always)]
pub const fn encode_i32_stack(v: i32) -> [u8; 4] {
    v.to_le_bytes()
}

#[inline(always)]
pub const fn encode_u64_stack(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

#[inline(always)]
pub const fn encode_i64_stack(v: i64) -> [u8; 8] {
    v.to_le_bytes()
}

#[inline(always)]
pub const fn encode_f32_stack(v: f32) -> [u8; 4] {
    canonical_f32_bits(v).to_le_bytes()
}

#[inline(always)]
pub const fn encode_f64_stack(v: f64) -> [u8; 8] {
    canonical_f64_bits(v).to_le_bytes()
}
