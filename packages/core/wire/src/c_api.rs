//! Thin C ABI (`include/nve_wire_abi.h`) wrapping `NveWireClient` so C++, Swift, and Dart bind
//! ONE canonical QUIC/WebTransport/WSS wire engine with zero protocol duplication.

use std::ffi::CStr;
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;

use nve_wire_core::{parse_wire_uri, NveWireFrame, NveWireKind};

use crate::{NveWireClient, NveWireClientConfig, NveWireTransportError};

pub const NVE_WIRE_OK: u32 = 0;
pub const NVE_WIRE_ERR_INVALID_ARG: u32 = 1;
pub const NVE_WIRE_ERR_PARSE: u32 = 2;
pub const NVE_WIRE_ERR_CONNECT: u32 = 3;
pub const NVE_WIRE_ERR_AUTH: u32 = 4;
pub const NVE_WIRE_ERR_IO: u32 = 5;
pub const NVE_WIRE_ERR_EMPTY: u32 = 6;
pub const NVE_WIRE_ERR_BUFFER_TOO_SMALL: u32 = 7;
pub const NVE_WIRE_ERR_PANIC: u32 = 255;

/// Opaque C handle owning a connected [`NveWireClient`].
pub struct NveWireClientHandle {
    inner: NveWireClient,
}

/// C-ABI telemetry snapshot matching `NveWireMetricsC` in `include/nve_wire_abi.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NveWireMetricsC {
    pub session_id: u32,
    pub transport_kind: u32,
    pub frames_received: u64,
    pub keyframes_received: u64,
    pub command_deltas_received: u64,
    pub node_patch_deltas_received: u64,
    pub events_sent: u64,
    pub wire_bytes_rx: u64,
    pub wire_bytes_tx: u64,
}

unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: Caller passes a valid NUL-terminated UTF-8 C string.
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Connects a native `nve-wire` client from a pairing URI (`nve://pair?...`, `quic://`, `wss://`).
///
/// # Safety
/// `uri_utf8` and `out_client` must be valid non-null pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_wire_client_connect_uri(
    uri_utf8: *const c_char,
    token_override_utf8: *const c_char,
    client_name_utf8: *const c_char,
    fallback_delay_ms: u32,
    out_client: *mut *mut NveWireClientHandle,
) -> u32 {
    catch_unwind(AssertUnwindSafe(|| {
        if uri_utf8.is_null() || out_client.is_null() {
            return NVE_WIRE_ERR_INVALID_ARG;
        }
        // SAFETY: Checked non-null above.
        let Some(uri_str) = (unsafe { cstr_to_str(uri_utf8) }) else {
            return NVE_WIRE_ERR_INVALID_ARG;
        };
        let Ok(uri) = parse_wire_uri(uri_str) else {
            return NVE_WIRE_ERR_PARSE;
        };
        // SAFETY: Nullable C strings handled by `cstr_to_str`.
        let token = unsafe { cstr_to_str(token_override_utf8) }
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| uri.token.clone());
        // SAFETY: Nullable C strings handled by `cstr_to_str`.
        let client_name = unsafe { cstr_to_str(client_name_utf8) }
            .filter(|s| !s.is_empty())
            .unwrap_or("nve-ffi-client")
            .to_owned();

        let mut cfg = NveWireClientConfig::new(token, client_name);
        cfg.cert_sha256 = uri.cert_sha256;
        if fallback_delay_ms > 0 {
            cfg.fallback_delay = Duration::from_millis(u64::from(fallback_delay_ms));
        }

        match NveWireClient::connect_uri_sync(&uri, &cfg) {
            Ok(client) => {
                let boxed = Box::new(NveWireClientHandle { inner: client });
                // SAFETY: Caller provided valid out pointer.
                unsafe {
                    *out_client = Box::into_raw(boxed);
                }
                NVE_WIRE_OK
            }
            Err(NveWireTransportError::Auth(_)) => NVE_WIRE_ERR_AUTH,
            Err(_) => NVE_WIRE_ERR_CONNECT,
        }
    }))
    .unwrap_or(NVE_WIRE_ERR_PANIC)
}

/// Polls one inbound `NveWireFrame` without blocking.
///
/// # Safety
/// `client` and output pointers must be valid; `out_payload_buf` must hold `payload_buf_cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_wire_client_poll_frame(
    client: *mut NveWireClientHandle,
    out_kind: *mut u8,
    out_sequence: *mut u32,
    out_payload_buf: *mut u8,
    payload_buf_cap: usize,
    out_payload_len: *mut usize,
) -> u32 {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() || out_kind.is_null() || out_sequence.is_null() || out_payload_len.is_null() {
            return NVE_WIRE_ERR_INVALID_ARG;
        }
        // SAFETY: Non-null handle owned by caller.
        let handle = unsafe { &*client };
        let Some(frame) = handle.inner.try_recv_frame() else {
            return NVE_WIRE_ERR_EMPTY;
        };
        // SAFETY: Checked non-null above.
        unsafe {
            *out_kind = frame.kind as u8;
            *out_sequence = frame.sequence;
            *out_payload_len = frame.payload.len();
        }
        if frame.payload.len() > payload_buf_cap {
            return NVE_WIRE_ERR_BUFFER_TOO_SMALL;
        }
        if !frame.payload.is_empty() {
            if out_payload_buf.is_null() {
                return NVE_WIRE_ERR_INVALID_ARG;
            }
            // SAFETY: Buffer has capacity >= `frame.payload.len()`.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    frame.payload.as_ptr(),
                    out_payload_buf,
                    frame.payload.len(),
                );
            }
        }
        NVE_WIRE_OK
    }))
    .unwrap_or(NVE_WIRE_ERR_PANIC)
}

/// Sends one raw `EventRing` (`NveEventBatch`) payload upstream.
///
/// # Safety
/// `client` must be valid; if `event_len > 0`, `event_bytes` must be valid for `event_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_wire_client_send_events(
    client: *mut NveWireClientHandle,
    event_bytes: *const u8,
    event_len: usize,
) -> u32 {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() || (event_len > 0 && event_bytes.is_null()) {
            return NVE_WIRE_ERR_INVALID_ARG;
        }
        // SAFETY: Valid handle & slice.
        let handle = unsafe { &*client };
        let slice = if event_len == 0 {
            &[]
        } else {
            // SAFETY: Non-null `event_bytes` checked above; caller guarantees validity for `event_len` bytes.
            unsafe { std::slice::from_raw_parts(event_bytes, event_len) }
        };
        match handle.inner.send_events(slice) {
            Ok(()) => NVE_WIRE_OK,
            Err(_) => NVE_WIRE_ERR_IO,
        }
    }))
    .unwrap_or(NVE_WIRE_ERR_PANIC)
}

/// Sends one unreliable datagram frame (RFC 9221 on QUIC/WebTransport, stream on WSS).
///
/// # Safety
/// `client` must be valid; if `payload_len > 0`, `payload` must be valid for `payload_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_wire_client_send_datagram(
    client: *mut NveWireClientHandle,
    kind: u8,
    payload: *const u8,
    payload_len: usize,
) -> u32 {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() || (payload_len > 0 && payload.is_null()) {
            return NVE_WIRE_ERR_INVALID_ARG;
        }
        let Ok(wire_kind) = NveWireKind::from_u8(kind) else {
            return NVE_WIRE_ERR_INVALID_ARG;
        };
        // SAFETY: Valid handle & slice.
        let handle = unsafe { &*client };
        let slice = if payload_len == 0 {
            &[]
        } else {
            // SAFETY: Non-null `payload` checked above; caller guarantees validity for `payload_len` bytes.
            unsafe { std::slice::from_raw_parts(payload, payload_len) }
        };
        let frame = NveWireFrame::new(wire_kind, 0, slice.to_vec());
        match handle.inner.send_datagram(&frame) {
            Ok(()) => NVE_WIRE_OK,
            Err(_) => NVE_WIRE_ERR_IO,
        }
    }))
    .unwrap_or(NVE_WIRE_ERR_PANIC)
}

/// Requests a fresh keyframe from the server.
///
/// # Safety
/// `client` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_wire_client_request_keyframe(client: *mut NveWireClientHandle) -> u32 {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() {
            return NVE_WIRE_ERR_INVALID_ARG;
        }
        // SAFETY: Non-null valid handle.
        let handle = unsafe { &*client };
        match handle.inner.request_keyframe() {
            Ok(()) => NVE_WIRE_OK,
            Err(_) => NVE_WIRE_ERR_IO,
        }
    }))
    .unwrap_or(NVE_WIRE_ERR_PANIC)
}

/// Copies current client telemetry into `*out_metrics`.
///
/// # Safety
/// `client` and `out_metrics` must be valid non-null pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_wire_client_metrics(
    client: *const NveWireClientHandle,
    out_metrics: *mut NveWireMetricsC,
) -> u32 {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() || out_metrics.is_null() {
            return NVE_WIRE_ERR_INVALID_ARG;
        }
        // SAFETY: Valid pointers.
        let handle = unsafe { &*client };
        let m = handle.inner.metrics();
        // SAFETY: Non-null `out_metrics` checked above; caller guarantees a valid writable pointer.
        unsafe {
            *out_metrics = NveWireMetricsC {
                session_id: m.session_id,
                transport_kind: handle.inner.transport_kind() as u32,
                frames_received: m.frames_received,
                keyframes_received: m.keyframes_received,
                command_deltas_received: m.command_deltas_received,
                node_patch_deltas_received: m.node_patch_deltas_received,
                events_sent: m.events_sent,
                wire_bytes_rx: m.wire_bytes_rx,
                wire_bytes_tx: m.wire_bytes_tx,
            };
        }
        NVE_WIRE_OK
    }))
    .unwrap_or(NVE_WIRE_ERR_PANIC)
}

/// Returns `true` if `client` is non-null and currently connected.
///
/// # Safety
/// `client` may be null or a valid pointer returned by `nve_wire_client_connect_uri`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_wire_client_is_connected(client: *const NveWireClientHandle) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() {
            return false;
        }
        // SAFETY: Non-null valid handle.
        unsafe { &*client }.inner.is_connected()
    }))
    .unwrap_or(false)
}

/// Disconnects and frees `client`. Safe to pass `NULL`.
///
/// # Safety
/// `client` must be null or a pointer previously returned by `nve_wire_client_connect_uri`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_wire_client_destroy(client: *mut NveWireClientHandle) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !client.is_null() {
            // SAFETY: Takes ownership back from `Box::into_raw`.
            let _ = unsafe { Box::from_raw(client) };
        }
    }));
}
