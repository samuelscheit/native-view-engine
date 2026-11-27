//! Single-entry versioned C ABI (`nve_runtime_get_interface`) exporting [`NveRuntimeApi`].
//!
//! Every entry point runs through [`with_runtime`]: null-handle rejection, panic containment,
//! and `RuntimeError` → status / thread-local `last_error` mapping live in one place.

mod app_files;
mod hooks;

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use std::time::Duration;

use nve_abi::{
    NveBufferSlice, NveEventBatch, NveEventBatchRef, NveGetInterfaceFn, NveRuntimeApi,
    NveRuntimeConfig, NveRuntimeHandle, NveRuntimeMetrics, NveRuntimeSceneView, NveRuntimeStatus,
    NveRuntimeWakeFn, NveStrSlice, NVE_RUNTIME_ABI_VERSION, NVE_RUNTIME_SCHEMA_HASH,
};
use nve_engine::Viewport;
use nve_engine::app::format_panic_payload;

use crate::app::CAbiAppAdapter;
use crate::builder::{RuntimeBuilder, RuntimeError};
use crate::surface::SurfaceRuntime;

/// Maximum time `rt_destroy` waits for concurrent readers before refusing to UAF a leaked lock.
const DESTROY_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

thread_local! {
    static LAST_RUNTIME_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn set_last_error(msg: impl Into<String>) {
    let s = msg.into();
    let _ = LAST_RUNTIME_ERROR.try_with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            *slot = s;
        }
    });
}

fn clear_last_error() {
    let _ = LAST_RUNTIME_ERROR.try_with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            slot.clear();
        }
    });
}

#[inline]
fn runtime_catch_unwind<T>(op: &'static str, fallback: T, f: impl FnOnce() -> T) -> T {
    clear_last_error();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(val) => val,
        Err(payload) => {
            let detail = format_panic_payload(&*payload);
            set_last_error(format!("{op}: {detail}"));
            fallback
        }
    }
}

/// Host-owned opaque context pointer handed back to host callbacks (wake, text measurement).
struct SendPtr(*mut c_void);

impl SendPtr {
    #[inline]
    fn as_ptr(&self) -> *mut c_void {
        self.0
    }
}

// SAFETY: Host explicitly opts into receiving its `ctx` back from worker threads per ABI.
unsafe impl Send for SendPtr {}
// SAFETY: Host synchronizes access to its own opaque `ctx` pointer.
unsafe impl Sync for SendPtr {}

unsafe fn borrow_runtime<'a>(handle: NveRuntimeHandle) -> Option<&'a SurfaceRuntime> {
    if handle.is_null() {
        return None;
    }
    // SAFETY: Caller passes a live handle allocated by `rt_create` and not yet destroyed.
    Some(unsafe { &*(handle.cast::<SurfaceRuntime>()) })
}

/// Runs `f` against the runtime behind `handle`: null handles are `InvalidArg`, panics are
/// `Panic`, and errors set `last_error` and map to their [`NveRuntimeStatus`].
fn with_runtime(
    op: &'static str,
    handle: NveRuntimeHandle,
    f: impl FnOnce(&SurfaceRuntime) -> Result<(), RuntimeError>,
) -> i32 {
    runtime_catch_unwind(op, NveRuntimeStatus::Panic.as_i32(), || {
        // SAFETY: Caller passes a live `rt_create` handle or null (rejected here).
        let res = match unsafe { borrow_runtime(handle) } {
            Some(rt) => f(rt),
            None => Err(RuntimeError::InvalidArg("null NveRuntimeHandle")),
        };
        match res {
            Ok(()) => NveRuntimeStatus::Ok.as_i32(),
            Err(e) => {
                set_last_error(e.to_string());
                e.status().as_i32()
            }
        }
    })
}

/// Writes `value` through an optional (nullable) caller out pointer.
///
/// # Safety
/// `out` is null or valid for one write of `T`.
#[inline]
unsafe fn write_opt<T>(out: *mut T, value: T) {
    if !out.is_null() {
        // SAFETY: Non-null `out` is valid for writes per the caller contract.
        unsafe { out.write(value) };
    }
}

/// Validates a required out pointer up front, before any work runs (`InvalidArg` with `what`).
#[inline]
fn required<T>(out: *mut T, what: &'static str) -> Result<NonNull<T>, RuntimeError> {
    NonNull::new(out).ok_or(RuntimeError::InvalidArg(what))
}

/// Borrows a host byte slice (`InvalidArg` with `what` when structurally invalid).
///
/// # Safety
/// `slice` is readable for `slice.len` bytes for `'a` when structurally valid.
unsafe fn host_bytes<'a>(slice: NveBufferSlice, what: &'static str) -> Result<&'a [u8], RuntimeError> {
    // SAFETY: Forwarded caller contract.
    unsafe { slice.try_as_slice() }.ok_or(RuntimeError::InvalidArg(what))
}

unsafe extern "C" fn rt_create(config: *const NveRuntimeConfig) -> NveRuntimeHandle {
    runtime_catch_unwind("nve_runtime::create", std::ptr::null_mut(), || {
        let cfg = if config.is_null() {
            NveRuntimeConfig::default()
        } else {
            // SAFETY: Non-null `config` points to a readable `NveRuntimeConfig`.
            unsafe { *config }
        };
        let positive_or = |v: f32, fallback: f32| if v.is_finite() && v > 0.0 { v } else { fallback };
        let rt = RuntimeBuilder::new()
            .surface_id(cfg.surface_id.max(1))
            .viewport(Viewport {
                width: positive_or(cfg.width, 800.0),
                height: positive_or(cfg.height, 600.0),
                scale_factor: positive_or(cfg.scale_factor, 1.0),
            })
            .build();
        Box::into_raw(Box::new(rt)).cast::<c_void>()
    })
}

unsafe extern "C" fn rt_destroy(handle: NveRuntimeHandle) {
    if handle.is_null() {
        return;
    }
    runtime_catch_unwind("nve_runtime::destroy", (), || {
        // SAFETY: `handle` was allocated by `rt_create` via `Box::into_raw` and is freed once.
        let rt = unsafe { &*(handle.cast::<SurfaceRuntime>()) };
        if let Err(e) = rt.check_owner_thread() {
            set_last_error(e.to_string());
            return;
        }
        // Drain and clear any host wake callback first so workers stop before reader drain.
        rt.wake.clear_sink();
        if !rt.lock.try_acquire_write_timeout(DESTROY_WAIT_TIMEOUT) {
            set_last_error(
                "destroy timed out waiting for active scene_acquire reader; leaking runtime to avoid UAF",
            );
            return;
        }
        // SAFETY: Exclusive write lock acquired above on owner thread ensures no active readers.
        drop(unsafe { Box::from_raw(handle.cast::<SurfaceRuntime>()) });
    });
}

unsafe extern "C" fn rt_set_wake_callback(
    handle: NveRuntimeHandle,
    cb: Option<NveRuntimeWakeFn>,
    ctx: *mut c_void,
) -> i32 {
    with_runtime("nve_runtime::set_wake_callback", handle, |rt| {
        match cb {
            Some(func) => {
                let send_ctx = SendPtr(ctx);
                rt.set_wake_callback(Arc::new(move || {
                    // SAFETY: Host guarantees `func(ctx)` is valid until `clear_sink`/`destroy`
                    // returns; `WakeBridge` drains in-flight calls under its `RwLock`.
                    unsafe { func(send_ctx.as_ptr()) };
                }));
            }
            None => rt.clear_wake_callback(),
        }
        Ok(())
    })
}

unsafe extern "C" fn rt_set_viewport(
    handle: NveRuntimeHandle,
    width: f32,
    height: f32,
    scale_factor: f32,
) -> i32 {
    with_runtime("nve_runtime::set_viewport", handle, |rt| {
        rt.set_viewport_shared(Viewport {
            width,
            height,
            scale_factor,
        })
        .map(drop)
    })
}

unsafe extern "C" fn rt_attach_app_interface(
    handle: NveRuntimeHandle,
    get_iface: Option<NveGetInterfaceFn>,
) -> i32 {
    with_runtime("nve_runtime::attach_app_interface", handle, |rt| {
        rt.check_not_poisoned()?;
        rt.check_owner_thread()?;
        let Some(f) = get_iface else {
            return rt.detach_app_shared();
        };
        // Construct the adapter BEFORE resetting or replacing live surface state so a failing
        // `nve_get_interface` leaves any existing app and published view intact.
        // SAFETY: Host guarantees `f` is a valid NVE `nve_get_interface` symbol.
        let adapter = unsafe { CAbiAppAdapter::from_get_interface(f, rt.surface_id) }?;
        rt.attach_app_shared(Box::new(adapter))
    })
}

unsafe extern "C" fn rt_submit_nve2(handle: NveRuntimeHandle, frame: NveBufferSlice) -> i32 {
    with_runtime("nve_runtime::submit_nve2", handle, |rt| {
        // SAFETY: Caller guarantees `frame` is readable for `frame.len` bytes when valid.
        let bytes = unsafe {
            host_bytes(frame, "invalid NveBufferSlice (null pointer with len > 0 or len overflow)")
        }?;
        rt.submit_nve2_shared(bytes)
    })
}

unsafe extern "C" fn rt_tick(handle: NveRuntimeHandle, dt_s: f32, out_changed: *mut u32) -> i32 {
    with_runtime("nve_runtime::tick", handle, |rt| {
        let changed = rt.tick_shared(dt_s)?;
        // SAFETY: `out_changed` is null or a valid caller out slot.
        unsafe { write_opt(out_changed, u32::from(changed)) };
        Ok(())
    })
}

unsafe extern "C" fn rt_dispatch_events(
    handle: NveRuntimeHandle,
    batch: NveEventBatch,
    out_changed: *mut u32,
) -> i32 {
    with_runtime("nve_runtime::dispatch_events", handle, |rt| {
        // SAFETY: Caller guarantees `batch` points to valid `NveDirectEvent`s when structurally valid.
        let events = unsafe { NveEventBatchRef::try_from_raw(batch) }.ok_or(
            RuntimeError::InvalidArg("invalid NveEventBatch (null/misaligned pointer or len overflow)"),
        )?;
        let changed = rt.dispatch_events_shared(events)?;
        // SAFETY: `out_changed` is null or a valid caller out slot.
        unsafe { write_opt(out_changed, u32::from(changed)) };
        Ok(())
    })
}

unsafe extern "C" fn rt_hit_test(
    handle: NveRuntimeHandle,
    x: f32,
    y: f32,
    out_node_id: *mut u32,
) -> i32 {
    with_runtime("nve_runtime::hit_test", handle, |rt| {
        let out = required(out_node_id, "null out_node_id")?;
        let hit = rt.hit_test(x, y)?;
        // SAFETY: `out` is a non-null caller out slot.
        unsafe { out.write(hit.unwrap_or(0)) };
        Ok(())
    })
}

unsafe extern "C" fn rt_scroll_by(
    handle: NveRuntimeHandle,
    node_id: u32,
    dx: f32,
    dy: f32,
    out_x: *mut f32,
    out_y: *mut f32,
) -> i32 {
    with_runtime("nve_runtime::scroll_by", handle, |rt| {
        let (x, y) = rt.scroll_by_shared(node_id, dx, dy)?;
        // SAFETY: Both out pointers are null or valid caller out slots.
        unsafe {
            write_opt(out_x, x);
            write_opt(out_y, y);
        }
        Ok(())
    })
}

unsafe extern "C" fn rt_scene_acquire(
    handle: NveRuntimeHandle,
    out_view: *mut NveRuntimeSceneView,
) -> i32 {
    with_runtime("nve_runtime::scene_acquire", handle, |rt| {
        if out_view.is_null() {
            return Err(RuntimeError::InvalidArg("null out_view"));
        }
        let guard = rt.acquire_scene()?;
        let view = *guard.view();
        // The reader lock now belongs to the host until its matching `scene_release`.
        std::mem::forget(guard);
        // SAFETY: `out_view` checked non-null above.
        unsafe { out_view.write(view) };
        Ok(())
    })
}

unsafe extern "C" fn rt_scene_release(handle: NveRuntimeHandle) -> i32 {
    with_runtime("nve_runtime::scene_release", handle, |rt| {
        if rt.lock.release_read() {
            Ok(())
        } else {
            Err(RuntimeError::InvalidArg(
                "scene_release called without matching scene_acquire",
            ))
        }
    })
}

unsafe extern "C" fn rt_get_metrics(
    handle: NveRuntimeHandle,
    out_metrics: *mut NveRuntimeMetrics,
) -> i32 {
    with_runtime("nve_runtime::get_metrics", handle, |rt| {
        let out = required(out_metrics, "null out_metrics")?;
        let m = rt.metrics()?;
        // SAFETY: `out` is a non-null caller out slot.
        unsafe { out.write(m) };
        Ok(())
    })
}

unsafe extern "C" fn rt_pair_connect(
    handle: NveRuntimeHandle,
    uri: NveStrSlice,
    token: NveStrSlice,
) -> i32 {
    with_runtime("nve_runtime::pair_connect", handle, |rt| {
        // SAFETY: Caller guarantees both slices are readable for `len` bytes when `len > 0`
        // (`NveStrSlice::as_str` validates UTF-8).
        let uri = unsafe { uri.as_str() }
            .map_err(|_| RuntimeError::InvalidArg("invalid NveStrSlice for pairing URI"))?;
        // SAFETY: As above.
        let token = unsafe { token.as_str() }
            .map_err(|_| RuntimeError::InvalidArg("invalid NveStrSlice for pairing token"))?;
        rt.pair_connect_shared(uri, token)
    })
}

unsafe extern "C" fn rt_pair_disconnect(handle: NveRuntimeHandle) -> i32 {
    with_runtime("nve_runtime::pair_disconnect", handle, SurfaceRuntime::pair_disconnect_shared)
}

unsafe extern "C" fn rt_pair_is_connected(handle: NveRuntimeHandle) -> u32 {
    runtime_catch_unwind("nve_runtime::pair_is_connected", 0, || {
        // SAFETY: `handle` is a live `rt_create` handle or null.
        unsafe { borrow_runtime(handle) }.map_or(0, |rt| u32::from(rt.pair_is_connected()))
    })
}

unsafe extern "C" fn rt_last_error(out_buf: *mut u8, cap: usize) -> usize {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        LAST_RUNTIME_ERROR
            .try_with(|cell| {
                let Ok(s) = cell.try_borrow() else {
                    return 0;
                };
                let bytes = s.as_bytes();
                if !out_buf.is_null() && cap > 0 {
                    let mut n = bytes.len().min(cap.saturating_sub(1));
                    while n > 0 && !s.is_char_boundary(n) {
                        n -= 1;
                    }
                    // SAFETY: `out_buf` is valid for `cap` bytes; `n < cap` and on a UTF-8 boundary.
                    unsafe {
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, n);
                        *out_buf.add(n) = 0;
                    }
                }
                bytes.len()
            })
            .unwrap_or(0)
    }))
    .unwrap_or(0)
}

static RUNTIME_API: NveRuntimeApi = NveRuntimeApi {
    abi_version: NVE_RUNTIME_ABI_VERSION,
    struct_size: core::mem::size_of::<NveRuntimeApi>() as u32,
    schema_hash: NVE_RUNTIME_SCHEMA_HASH,
    create: rt_create,
    destroy: rt_destroy,
    set_wake_callback: rt_set_wake_callback,
    set_viewport: rt_set_viewport,
    set_text_measurer: hooks::rt_set_text_measurer,
    attach_app_interface: rt_attach_app_interface,
    load_app_module: app_files::rt_load_app_module,
    set_props: hooks::rt_set_props,
    drain_emitted_events: hooks::rt_drain_emitted_events,
    snapshot_state: hooks::rt_snapshot_state,
    restore_state: hooks::rt_restore_state,
    save_state_file: app_files::rt_save_state_file,
    restore_state_file: app_files::rt_restore_state_file,
    clear_state_file: app_files::rt_clear_state_file,
    apply_hot_patch: hooks::rt_apply_hot_patch,
    submit_nve2: rt_submit_nve2,
    tick: rt_tick,
    dispatch_events: rt_dispatch_events,
    hit_test: rt_hit_test,
    scroll_by: rt_scroll_by,
    scene_acquire: rt_scene_acquire,
    scene_release: rt_scene_release,
    resource_payload: hooks::rt_resource_payload,
    get_metrics: rt_get_metrics,
    pair_connect: rt_pair_connect,
    pair_disconnect: rt_pair_disconnect,
    pair_is_connected: rt_pair_is_connected,
    last_error: rt_last_error,
};

/// Single exported C ABI entry point for `libnve_runtime`.
///
/// Returns `NULL` when `abi_version != NVE_RUNTIME_ABI_VERSION`.
#[unsafe(no_mangle)]
pub extern "C" fn nve_runtime_get_interface(abi_version: u32) -> *const NveRuntimeApi {
    if abi_version == NVE_RUNTIME_ABI_VERSION {
        &RUNTIME_API
    } else {
        std::ptr::null()
    }
}
