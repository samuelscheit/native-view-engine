//! Zero-JSON `#[repr(C)]` C ABI matching `packages/modules/pair/include/nve_pair_abi.h`.

use std::mem::{offset_of, size_of};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use nve_interop::{DEFAULT_MAX_WIRE_FRAME_BYTES, DEFAULT_SYNC_WINDOW, PairingConfig};

use crate::{
    PairClientEngine, build_native_event_from_c, default_schema_or, parse_pair_uri, read_c_bytes,
    read_c_utf8,
};

pub const NVE_PAIR_OK: i32 = 0;
pub const NVE_PAIR_ERR_NULL_ARG: i32 = -1;
pub const NVE_PAIR_ERR_INVALID_UTF8: i32 = -2;
pub const NVE_PAIR_ERR_INVALID_URI: i32 = -3;
pub const NVE_PAIR_ERR_TRUNCATED: i32 = -4;
pub const NVE_PAIR_ERR_INVALID_MAGIC: i32 = -5;
pub const NVE_PAIR_ERR_UNSUPPORTED_VERSION: i32 = -6;
pub const NVE_PAIR_ERR_INVALID_KIND: i32 = -7;
pub const NVE_PAIR_ERR_CHECKSUM: i32 = -8;
pub const NVE_PAIR_ERR_BASE_SEQ_MISMATCH: i32 = -9;
pub const NVE_PAIR_ERR_AUTH_FAILED: i32 = -10;
pub const NVE_PAIR_ERR_SCHEMA_MISMATCH: i32 = -11;
pub const NVE_PAIR_ERR_SESSION_MISMATCH: i32 = -12;
pub const NVE_PAIR_ERR_NOT_PAIRED: i32 = -13;
pub const NVE_PAIR_ERR_BUFFER_TOO_SMALL: i32 = -14;
pub const NVE_PAIR_ERR_FRAME_TOO_LARGE: i32 = -15;
pub const NVE_PAIR_ERR_MALFORMED: i32 = -16;
pub const NVE_PAIR_ERR_HANDSHAKE_REJECTED: i32 = -17;
pub const NVE_PAIR_ERR_EMPTY_QUEUE: i32 = -18;
pub const NVE_PAIR_ERR_INTERNAL_PANIC: i32 = -99;

/// Opaque FFI handle wrapping [`PairClientEngine`].
pub struct NvePairClient {
    engine: PairClientEngine,
}

/// Zero-copy borrowed byte slice returned across the C ABI.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NvePairByteSlice {
    pub ptr: *const u8,
    pub len: usize,
}

impl NvePairByteSlice {
    pub const EMPTY: Self = Self {
        ptr: ptr::null(),
        len: 0,
    };
}

/// Configuration passed to [`nve_pair_client_create`].
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NvePairClientConfig {
    pub token_ptr: *const u8,
    pub token_len: usize,
    pub schema_version: u16,
    pub max_window: u16,
    pub max_frame_bytes: u32,
}

/// Flat zero-JSON input event matching `NvePairInputEvent` in `nve_pair_abi.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NvePairInputEvent {
    pub surface_id: u32,
    pub node_id: u32,
    pub timestamp_us: u64,
    pub kind: u8,
    pub reserved: [u8; 7],
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub text_ptr: *const u8,
    pub text_len: usize,
    pub data_ptr: *const u8,
    pub data_len: usize,
}

/// Summary populated by [`nve_pair_client_ingest_packet`].
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NvePairIngestOutcome {
    pub applied_frame_count: u32,
    pub latest_seq: u32,
    pub outbound_packet_count: u32,
    pub state: u32,
}

/// Fixed-layout runtime metrics snapshot matching `NvePairMetrics` in `nve_pair_abi.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NvePairMetrics {
    pub session_id: u32,
    pub last_applied_seq: u32,
    pub state: u32,
    pub reorder_buffer_len: u32,
    pub frames_received: u64,
    pub keyframes_received: u64,
    pub deltas_received: u64,
    pub events_sent: u64,
    pub wire_bytes_received: u64,
    pub raw_bytes_received: u64,
    pub wire_bytes_sent: u64,
    pub malformed_packets_dropped: u64,
    pub acks_sent: u64,
    pub nacks_sent: u64,
    pub keyframe_resyncs: u64,
    pub last_pong_timestamp_us: u64,
}

/// Caller-provided fixed buffers populated by [`nve_pair_parse_uri`].
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NvePairParsedUri {
    pub transport: u8,
    pub has_udp_port: u8,
    pub port: u16,
    pub udp_port: u16,
    pub schema_version: u16,
    pub width: f32,
    pub height: f32,
    pub host_buf: [u8; 256],
    pub host_len: usize,
    pub path_buf: [u8; 256],
    pub path_len: usize,
    pub token_buf: [u8; 256],
    pub token_len: usize,
}

impl Default for NvePairParsedUri {
    fn default() -> Self {
        Self {
            transport: 0,
            has_udp_port: 0,
            port: 0,
            udp_port: 0,
            schema_version: 0,
            width: 0.0,
            height: 0.0,
            host_buf: [0u8; 256],
            host_len: 0,
            path_buf: [0u8; 256],
            path_len: 0,
            token_buf: [0u8; 256],
            token_len: 0,
        }
    }
}

// Compile-time ABI layout checks locked against `include/nve_pair_abi.h`.
const _: () = {
    assert!(offset_of!(NvePairByteSlice, ptr) == 0);
    assert!(offset_of!(NvePairByteSlice, len) == size_of::<*const u8>());
    assert!(offset_of!(NvePairIngestOutcome, applied_frame_count) == 0);
    assert!(offset_of!(NvePairIngestOutcome, latest_seq) == 4);
    assert!(offset_of!(NvePairIngestOutcome, outbound_packet_count) == 8);
    assert!(offset_of!(NvePairIngestOutcome, state) == 12);
    assert!(size_of::<NvePairIngestOutcome>() == 16);

    assert!(offset_of!(NvePairMetrics, session_id) == 0);
    assert!(offset_of!(NvePairMetrics, last_applied_seq) == 4);
    assert!(offset_of!(NvePairMetrics, state) == 8);
    assert!(offset_of!(NvePairMetrics, reorder_buffer_len) == 12);
    assert!(offset_of!(NvePairMetrics, frames_received) == 16);
    assert!(offset_of!(NvePairMetrics, keyframes_received) == 24);
    assert!(offset_of!(NvePairMetrics, deltas_received) == 32);
    assert!(offset_of!(NvePairMetrics, events_sent) == 40);
    assert!(offset_of!(NvePairMetrics, wire_bytes_received) == 48);
    assert!(offset_of!(NvePairMetrics, raw_bytes_received) == 56);
    assert!(offset_of!(NvePairMetrics, wire_bytes_sent) == 64);
    assert!(offset_of!(NvePairMetrics, malformed_packets_dropped) == 72);
    assert!(offset_of!(NvePairMetrics, acks_sent) == 80);
    assert!(offset_of!(NvePairMetrics, nacks_sent) == 88);
    assert!(offset_of!(NvePairMetrics, keyframe_resyncs) == 96);
    assert!(offset_of!(NvePairMetrics, last_pong_timestamp_us) == 104);
    assert!(size_of::<NvePairMetrics>() == 112);
};

fn write_slice_to_out(
    src: &[u8],
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    if !out_len.is_null() {
        // SAFETY: Caller supplied writable `out_len`.
        unsafe { *out_len = src.len() };
    }
    if out_buf.is_null() || out_cap < src.len() {
        return NVE_PAIR_ERR_BUFFER_TOO_SMALL;
    }
    if !src.is_empty() {
        // SAFETY: Caller guarantees `out_buf` is valid for `out_cap >= src.len()` bytes.
        unsafe { ptr::copy_nonoverlapping(src.as_ptr(), out_buf, src.len()) };
    }
    NVE_PAIR_OK
}

fn copy_fixed_buf(src: &str, dst: &mut [u8; 256], out_len: &mut usize) -> Result<(), i32> {
    let bytes = src.as_bytes();
    if bytes.len() > dst.len() {
        return Err(NVE_PAIR_ERR_BUFFER_TOO_SMALL);
    }
    dst.fill(0);
    dst[..bytes.len()].copy_from_slice(bytes);
    *out_len = bytes.len();
    Ok(())
}

/// Parses a pairing URI into fixed caller buffers.
///
/// # Safety
/// `uri_ptr` (if `uri_len > 0`), `fallback_token_ptr` (if `fallback_token_len > 0`), and
/// `out_uri` must be valid pointers for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_pair_parse_uri(
    uri_ptr: *const u8,
    uri_len: usize,
    fallback_token_ptr: *const u8,
    fallback_token_len: usize,
    out_uri: *mut NvePairParsedUri,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if out_uri.is_null() {
            return NVE_PAIR_ERR_NULL_ARG;
        }
        let uri_str = match read_c_utf8(uri_ptr, uri_len) {
            Ok(s) => s,
            Err(code) => return code,
        };
        let fallback = if fallback_token_len == 0 {
            None
        } else {
            match read_c_utf8(fallback_token_ptr, fallback_token_len) {
                Ok(s) => Some(s),
                Err(code) => return code,
            }
        };
        let Ok(parsed) = parse_pair_uri(uri_str, fallback) else {
            return NVE_PAIR_ERR_INVALID_URI;
        };

        let mut out = NvePairParsedUri {
            transport: parsed.transport as u8,
            has_udp_port: u8::from(parsed.udp_port.is_some()),
            port: parsed.port,
            udp_port: parsed.udp_port.unwrap_or(0),
            schema_version: parsed.schema_version,
            width: parsed.width.unwrap_or(0.0),
            height: parsed.height.unwrap_or(0.0),
            ..NvePairParsedUri::default()
        };
        if let Err(code) = copy_fixed_buf(&parsed.host, &mut out.host_buf, &mut out.host_len) {
            return code;
        }
        if let Err(code) = copy_fixed_buf(&parsed.path, &mut out.path_buf, &mut out.path_len) {
            return code;
        }
        if let Err(code) = copy_fixed_buf(&parsed.token, &mut out.token_buf, &mut out.token_len) {
            return code;
        }
        // SAFETY: Checked non-null above.
        unsafe { *out_uri = out };
        NVE_PAIR_OK
    }))
    .unwrap_or(NVE_PAIR_ERR_INTERNAL_PANIC)
}

/// Creates a new `NvePairClient` from a `NvePairClientConfig`.
///
/// # Safety
/// `config` must be a valid pointer to a `NvePairClientConfig`, and `config.token_ptr`
/// must be valid for `config.token_len` bytes when `config.token_len > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_pair_client_create(
    config: *const NvePairClientConfig,
) -> *mut NvePairClient {
    catch_unwind(AssertUnwindSafe(|| {
        if config.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: Non-null pointer guaranteed by caller.
        let cfg = unsafe { &*config };
        let Ok(token) = read_c_utf8(cfg.token_ptr, cfg.token_len) else {
            return ptr::null_mut();
        };
        let schema = default_schema_or(cfg.schema_version);
        let max_window = if cfg.max_window == 0 {
            DEFAULT_SYNC_WINDOW
        } else {
            usize::from(cfg.max_window)
        };
        let max_frame_bytes = if cfg.max_frame_bytes == 0 {
            DEFAULT_MAX_WIRE_FRAME_BYTES
        } else {
            cfg.max_frame_bytes as usize
        };
        let pairing_cfg = PairingConfig::new(token)
            .with_schema_version(schema)
            .with_max_window(max_window)
            .with_max_frame_bytes(max_frame_bytes);
        Box::into_raw(Box::new(NvePairClient {
            engine: PairClientEngine::new(pairing_cfg),
        }))
    }))
    .unwrap_or(ptr::null_mut())
}

/// Creates a new `NvePairClient` directly from a pairing URI.
///
/// # Safety
/// `uri_ptr` (if `uri_len > 0`) and `fallback_token_ptr` (if `fallback_token_len > 0`)
/// must be valid pointers for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_pair_client_create_from_uri(
    uri_ptr: *const u8,
    uri_len: usize,
    fallback_token_ptr: *const u8,
    fallback_token_len: usize,
) -> *mut NvePairClient {
    catch_unwind(AssertUnwindSafe(|| {
        let Ok(uri_str) = read_c_utf8(uri_ptr, uri_len) else {
            return ptr::null_mut();
        };
        let fallback = if fallback_token_len == 0 {
            None
        } else {
            match read_c_utf8(fallback_token_ptr, fallback_token_len) {
                Ok(s) => Some(s),
                Err(_) => return ptr::null_mut(),
            }
        };
        let Ok(engine) = PairClientEngine::from_uri(uri_str, fallback) else {
            return ptr::null_mut();
        };
        Box::into_raw(Box::new(NvePairClient { engine }))
    }))
    .unwrap_or(ptr::null_mut())
}

/// Destroys a client created by `nve_pair_client_create*`. Safe to pass `NULL`.
///
/// # Safety
/// `client` must be either `NULL` or a pointer returned by `nve_pair_client_create*`
/// that has not already been destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_pair_client_destroy(client: *mut NvePairClient) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !client.is_null() {
            // SAFETY: Pointer came from `Box::into_raw` and is freed at most once.
            unsafe { drop(Box::from_raw(client)) };
        }
    }));
}

/// Begins a pairing handshake by writing an `NVT1` `Hello` envelope into `out_buf`.
///
/// # Safety
/// `client` must be valid; `out_buf` (if non-null) must be writable for `out_cap` bytes;
/// `out_len` (if non-null) must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_pair_client_begin_handshake(
    client: *mut NvePairClient,
    client_nonce: u64,
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() {
            return NVE_PAIR_ERR_NULL_ARG;
        }
        // SAFETY: Validated non-null client pointer.
        let client_ref = unsafe { &mut *client };
        let hello = client_ref.engine.begin_handshake(client_nonce);
        write_slice_to_out(&hello, out_buf, out_cap, out_len)
    }))
    .unwrap_or(NVE_PAIR_ERR_INTERNAL_PANIC)
}

/// Ingests one incoming `NVT1` envelope or standalone `NVD1` delta packet.
///
/// # Safety
/// `client` must be valid; `packet_ptr` must be valid for `packet_len` bytes when non-empty;
/// `out_outcome` may be `NULL` or a valid writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_pair_client_ingest_packet(
    client: *mut NvePairClient,
    packet_ptr: *const u8,
    packet_len: usize,
    out_outcome: *mut NvePairIngestOutcome,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() {
            return NVE_PAIR_ERR_NULL_ARG;
        }
        let packet = match read_c_bytes(packet_ptr, packet_len) {
            Ok(s) => s,
            Err(code) => return code,
        };
        // SAFETY: Validated non-null client pointer.
        let client_ref = unsafe { &mut *client };
        match client_ref.engine.ingest_packet(packet) {
            Ok(summary) => {
                if !out_outcome.is_null() {
                    // SAFETY: Caller supplied writable `out_outcome`.
                    unsafe {
                        *out_outcome = NvePairIngestOutcome {
                            applied_frame_count: summary.applied_frame_count,
                            latest_seq: summary.latest_seq,
                            outbound_packet_count: summary.outbound_packet_count,
                            state: summary.state as u32,
                        };
                    }
                }
                NVE_PAIR_OK
            }
            Err(code) => code,
        }
    }))
    .unwrap_or(NVE_PAIR_ERR_INTERNAL_PANIC)
}

/// Pops the next queued outbound packet (`Ack`/`Nack`/`Pong`/`KeyframeRequest`).
///
/// # Safety
/// `client` must be valid; `out_buf` (if non-null) must be writable for `out_cap` bytes;
/// `out_len` (if non-null) must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_pair_client_pop_outbound_packet(
    client: *mut NvePairClient,
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() {
            return NVE_PAIR_ERR_NULL_ARG;
        }
        // SAFETY: Validated non-null client pointer.
        let client_ref = unsafe { &mut *client };
        let Some(front) = client_ref.engine.peek_outbound_packet() else {
            if !out_len.is_null() {
                // SAFETY: Caller supplied writable `out_len`.
                unsafe { *out_len = 0 };
            }
            return NVE_PAIR_ERR_EMPTY_QUEUE;
        };
        let rc = write_slice_to_out(front, out_buf, out_cap, out_len);
        if rc == NVE_PAIR_OK {
            let _ = client_ref.engine.pop_outbound_packet();
        }
        rc
    }))
    .unwrap_or(NVE_PAIR_ERR_INTERNAL_PANIC)
}

/// Encodes one native input event into an `NVT1` `InputEvents = 11` envelope (`EventRing`).
///
/// # Safety
/// `client` and `event` must be valid pointers; `out_buf` (if non-null) must be writable for
/// `out_cap` bytes; `out_len` (if non-null) must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_pair_client_encode_input_event(
    client: *mut NvePairClient,
    event: *const NvePairInputEvent,
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() || event.is_null() {
            return NVE_PAIR_ERR_NULL_ARG;
        }
        // SAFETY: Validated non-null pointers.
        let client_ref = unsafe { &mut *client };
        let ev_ref = unsafe { &*event };
        let native_ev = match build_native_event_from_c(ev_ref) {
            Ok(ev) => ev,
            Err(code) => return code,
        };
        let encoded = match client_ref.engine.encode_input_event(&native_ev) {
            Ok(buf) => buf,
            Err(code) => return code,
        };
        write_slice_to_out(&encoded, out_buf, out_cap, out_len)
    }))
    .unwrap_or(NVE_PAIR_ERR_INTERNAL_PANIC)
}

/// Returns a zero-copy byte slice referencing the client's latest reconstructed `NVE1` frame.
///
/// # Safety
/// `client` may be `NULL` or a valid pointer. The returned pointer remains valid until the next
/// mutating call on `client` or `nve_pair_client_destroy`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_pair_client_current_frame(
    client: *const NvePairClient,
) -> NvePairByteSlice {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() {
            return NvePairByteSlice::EMPTY;
        }
        // SAFETY: Validated non-null client pointer.
        let frame = unsafe { &*client }.engine.current_frame();
        if frame.is_empty() {
            NvePairByteSlice::EMPTY
        } else {
            NvePairByteSlice {
                ptr: frame.as_ptr(),
                len: frame.len(),
            }
        }
    }))
    .unwrap_or(NvePairByteSlice::EMPTY)
}

/// Copies the current runtime metrics snapshot into `*out_metrics`.
///
/// # Safety
/// `client` and `out_metrics` must be valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_pair_client_metrics(
    client: *const NvePairClient,
    out_metrics: *mut NvePairMetrics,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() || out_metrics.is_null() {
            return NVE_PAIR_ERR_NULL_ARG;
        }
        // SAFETY: Validated non-null pointers.
        let snapshot = unsafe { &*client }.engine.metrics();
        unsafe { *out_metrics = snapshot };
        NVE_PAIR_OK
    }))
    .unwrap_or(NVE_PAIR_ERR_INTERNAL_PANIC)
}
