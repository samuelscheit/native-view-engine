//! Framework- and runtime-agnostic binary byte transport (`ByteTransport`), error model
//! (`TransportError`), and zero-waste stack-first request encoding helpers (`StackWriter`,
//! `with_stack_writer`, `try_with_stack_writer`, and canonical float stack encoders).
//!
//! Lives in `nve-codec` (`#![forbid(unsafe_code)]`, zero runtime/engine/ABI dependencies) so
//! capability modules, RPC clients, and host adapters can share one clean transport abstraction
//! without coupling to `nve-abi` or `nve-host-services`.

use core::fmt;
use std::sync::Arc;

use crate::{
    canonical_f32_bits, canonical_f64_bits, ByteSink, ByteWriter, CodecError, SliceWriter,
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
///
/// Enforces single-reply delivery: if a buggy transport invokes `on_reply` more than once, returns
/// `Err(TransportError::Internal(_))`.
#[inline]
pub fn call_sync_decode<T, F>(
    transport: &(impl ByteTransport + ?Sized),
    service_id: u32,
    method_id: u32,
    req: &[u8],
    decode: F,
) -> Result<T, TransportError>
where
    F: FnOnce(&[u8]) -> Result<T, TransportError>,
{
    let mut decode = Some(decode);
    let mut decoded: Option<Result<T, TransportError>> = None;
    let mut duplicate = false;
    transport.call_sync(service_id, method_id, req, &mut |reply| {
        if let Some(f) = decode.take() {
            decoded = Some(f(reply));
        } else {
            duplicate = true;
        }
    })?;
    if duplicate {
        return Err(TransportError::Internal(
            "ByteTransport::call_sync invoked on_reply more than once".into(),
        ));
    }
    match decoded {
        Some(res) => res,
        None => match decode.take() {
            Some(f) => f(&[]),
            None => Err(TransportError::Internal(
                "ByteTransport::call_sync decoder missing".into(),
            )),
        },
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
    #[must_use]
    #[inline]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self::Stack(SliceWriter::new(buf))
    }

    #[must_use]
    #[inline]
    pub fn is_spilled(&self) -> bool {
        matches!(self, Self::Heap(_))
    }

    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Stack(sw) => sw.pos(),
            Self::Heap(bw) => bw.len(),
        }
    }

    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Stack(sw) => sw.written(),
            Self::Heap(bw) => bw.as_slice(),
        }
    }

    #[must_use]
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
        let min_cap = cur_len.saturating_add(additional);
        let desired_cap = min_cap
            .max(cur_len.saturating_mul(2))
            .max(64)
            .min(isize::MAX as usize);
        let mut vec = Vec::new();
        if vec.try_reserve(desired_cap).is_err() && vec.try_reserve_exact(min_cap).is_err() {
            return Err(CodecError::LenOverflow {
                len: min_cap,
                max: max_additional,
            });
        }
        vec.extend_from_slice(written);
        *self = Self::Heap(ByteWriter::from_vec(vec));
        match self {
            Self::Heap(bw) => Ok(bw),
            Self::Stack(_) => Err(CodecError::LenOverflow {
                len: min_cap,
                max: max_additional,
            }),
        }
    }

    crate::writer::delegate_slice_sink!();
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
///
/// Keep `N` small (`<= 256..512` bytes) to avoid bloating stack frames on deep or async call
/// paths; larger payloads spill cleanly to heap with amortized growth.
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
///
/// Keep `N` small (`<= 256..512` bytes) to keep caller stack frames compact.
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

/// Canonicalizes `v` (`-0.0` -> `+0.0`, NaN -> canonical quiet NaN) into 4 little-endian bytes.
#[must_use]
#[inline(always)]
pub const fn encode_f32_stack(v: f32) -> [u8; 4] {
    canonical_f32_bits(v).to_le_bytes()
}

/// Canonicalizes `v` (`-0.0` -> `+0.0`, NaN -> canonical quiet NaN) into 8 little-endian bytes.
#[must_use]
#[inline(always)]
pub const fn encode_f64_stack(v: f64) -> [u8; 8] {
    canonical_f64_bits(v).to_le_bytes()
}
