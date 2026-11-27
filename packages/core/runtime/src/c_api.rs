//! Single-entry versioned C ABI (`nve_runtime_get_interface`) exporting [`NveRuntimeApi`].

use std::cell::RefCell;
use std::ffi::c_void;
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

mod component_slots;
use component_slots::{
    rt_apply_hot_patch, rt_drain_emitted_events, rt_measure, rt_resource_bytes, rt_restore_state,
    rt_set_props, rt_set_text_measurer, rt_snapshot_state,
};

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

#[inline]
fn status_from_result(res: Result<(), RuntimeError>) -> i32 {
    match res {
        Ok(()) => NveRuntimeStatus::Ok.as_i32(),
        Err(e) => {
            set_last_error(e.to_string());
            e.status().as_i32()
        }
    }
}

struct SendPtr(*mut c_void);

impl SendPtr {
    #[inline]
    fn as_ptr(&self) -> *mut c_void {
        self.0
    }
}

// SAFETY: Host explicitly opts into receiving `NveRuntimeWakeFn(ctx)` from worker threads per ABI.
unsafe impl Send for SendPtr {}
// SAFETY: Host synchronizes access to its own opaque wake `ctx` pointer.
unsafe impl Sync for SendPtr {}

unsafe fn borrow_runtime<'a>(handle: NveRuntimeHandle) -> Option<&'a SurfaceRuntime> {
    if handle.is_null() {
        set_last_error("null NveRuntimeHandle");
        return None;
    }
    // SAFETY: Caller passes a live handle allocated by `rt_create` and not yet destroyed.
    Some(unsafe { &*(handle.cast::<SurfaceRuntime>()) })
}

unsafe extern "C" fn rt_create(config: *const NveRuntimeConfig) -> NveRuntimeHandle {
    runtime_catch_unwind("nve_runtime::create", std::ptr::null_mut(), || {
        let cfg = if config.is_null() {
            NveRuntimeConfig::default()
        } else {
            // SAFETY: Non-null `config` points to a readable `NveRuntimeConfig`.
            unsafe { *config }
        };
        let width = if cfg.width.is_finite() && cfg.width > 0.0 {
            cfg.width
        } else {
            800.0
        };
        let height = if cfg.height.is_finite() && cfg.height > 0.0 {
            cfg.height
        } else {
            600.0
        };
        let scale_factor = if cfg.scale_factor.is_finite() && cfg.scale_factor > 0.0 {
            cfg.scale_factor
        } else {
            1.0
        };
        let rt = RuntimeBuilder::new()
            .surface_id(cfg.surface_id.max(1))
            .viewport(Viewport {
                width,
                height,
                scale_factor,
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
        let boxed = unsafe { Box::from_raw(handle.cast::<SurfaceRuntime>()) };
        drop(boxed);
    });
}

unsafe extern "C" fn rt_set_wake_callback(
    handle: NveRuntimeHandle,
    cb: Option<NveRuntimeWakeFn>,
    ctx: *mut c_void,
) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::set_wake_callback",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
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
            NveRuntimeStatus::Ok.as_i32()
        },
    )
}

unsafe extern "C" fn rt_set_viewport(
    handle: NveRuntimeHandle,
    width: f32,
    height: f32,
    scale_factor: f32,
) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::set_viewport",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            status_from_result(
                rt.set_viewport_shared(Viewport {
                    width,
                    height,
                    scale_factor,
                })
                .map(|_| ()),
            )
        },
    )
}

unsafe extern "C" fn rt_attach_app_interface(
    handle: NveRuntimeHandle,
    get_iface: Option<NveGetInterfaceFn>,
) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::attach_app_interface",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            if let Err(e) = rt.check_not_poisoned().and_then(|()| rt.check_owner_thread()) {
                set_last_error(e.to_string());
                return e.status().as_i32();
            }
            match get_iface {
                Some(f) => {
                    // Construct the adapter BEFORE resetting or replacing live surface state so a
                    // failing `nve_get_interface` leaves any existing app and published view intact.
                    // SAFETY: Host guarantees `f` is a valid NVE `nve_get_interface` symbol.
                    let Some(adapter) = (unsafe {
                        CAbiAppAdapter::from_get_interface(f, rt.surface_id)
                    }) else {
                        set_last_error("app interface returned null surface API or app handle");
                        return NveRuntimeStatus::InvalidArg.as_i32();
                    };
                    status_from_result(rt.attach_app_shared(Box::new(adapter)))
                }
                None => status_from_result(rt.detach_app_shared()),
            }
        },
    )
}

unsafe extern "C" fn rt_submit_nve2(handle: NveRuntimeHandle, frame: NveBufferSlice) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::submit_nve2",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            // SAFETY: Caller guarantees `frame` is readable for `frame.len` bytes when valid.
            let Some(bytes) = (unsafe { frame.try_as_slice() }) else {
                set_last_error("invalid NveBufferSlice (null pointer with len > 0 or len overflow)");
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            status_from_result(rt.submit_nve2_shared(bytes))
        },
    )
}

unsafe extern "C" fn rt_tick(
    handle: NveRuntimeHandle,
    dt_s: f32,
    out_changed: *mut u32,
) -> i32 {
    runtime_catch_unwind("nve_runtime::tick", NveRuntimeStatus::Panic.as_i32(), || {
        // SAFETY: `handle` checked non-null inside `borrow_runtime`.
        let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
            return NveRuntimeStatus::InvalidArg.as_i32();
        };
        match rt.tick_shared(dt_s) {
            Ok(changed) => {
                if !out_changed.is_null() {
                    // SAFETY: Caller provided a non-null output slot.
                    unsafe { *out_changed = u32::from(changed) };
                }
                NveRuntimeStatus::Ok.as_i32()
            }
            Err(e) => {
                set_last_error(e.to_string());
                e.status().as_i32()
            }
        }
    })
}

unsafe extern "C" fn rt_dispatch_events(
    handle: NveRuntimeHandle,
    batch: NveEventBatch,
    out_changed: *mut u32,
) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::dispatch_events",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            // SAFETY: Caller guarantees `batch` points to valid `NveDirectEvent`s when structurally valid.
            let Some(events) = (unsafe { NveEventBatchRef::try_from_raw(batch) }) else {
                set_last_error("invalid NveEventBatch (null/misaligned pointer or len overflow)");
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            match rt.dispatch_events_shared(events) {
                Ok(changed) => {
                    if !out_changed.is_null() {
                        // SAFETY: Caller provided a non-null output slot.
                        unsafe { *out_changed = u32::from(changed) };
                    }
                    NveRuntimeStatus::Ok.as_i32()
                }
                Err(e) => {
                    set_last_error(e.to_string());
                    e.status().as_i32()
                }
            }
        },
    )
}

unsafe extern "C" fn rt_hit_test(
    handle: NveRuntimeHandle,
    x: f32,
    y: f32,
    out_node_id: *mut u32,
) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::hit_test",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            if out_node_id.is_null() {
                set_last_error("null out_node_id");
                return NveRuntimeStatus::InvalidArg.as_i32();
            }
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            match rt.hit_test(x, y) {
                Ok(hit) => {
                    // SAFETY: `out_node_id` checked non-null above.
                    unsafe { *out_node_id = hit.unwrap_or(0) };
                    NveRuntimeStatus::Ok.as_i32()
                }
                Err(e) => {
                    set_last_error(e.to_string());
                    e.status().as_i32()
                }
            }
        },
    )
}

unsafe extern "C" fn rt_scroll_by(
    handle: NveRuntimeHandle,
    node_id: u32,
    dx: f32,
    dy: f32,
    out_x: *mut f32,
    out_y: *mut f32,
) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::scroll_by",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            match rt.scroll_by_shared(node_id, dx, dy) {
                Ok((x, y)) => {
                    if !out_x.is_null() {
                        // SAFETY: Non-null caller out pointer.
                        unsafe { *out_x = x };
                    }
                    if !out_y.is_null() {
                        // SAFETY: Non-null caller out pointer.
                        unsafe { *out_y = y };
                    }
                    NveRuntimeStatus::Ok.as_i32()
                }
                Err(e) => {
                    set_last_error(e.to_string());
                    e.status().as_i32()
                }
            }
        },
    )
}

unsafe extern "C" fn rt_scene_acquire(
    handle: NveRuntimeHandle,
    out_view: *mut NveRuntimeSceneView,
) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::scene_acquire",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            if out_view.is_null() {
                set_last_error("null out_view");
                return NveRuntimeStatus::InvalidArg.as_i32();
            }
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            if let Err(e) = rt.check_not_poisoned() {
                set_last_error(e.to_string());
                return e.status().as_i32();
            }
            if !rt.lock.try_acquire_read() {
                set_last_error("scene write in progress");
                return NveRuntimeStatus::Busy.as_i32();
            }
            // SAFETY: Shared reader lock acquired above; released by matching `rt_scene_release`.
            let view = unsafe { &*rt.inner.get() }.state.published_view;
            // SAFETY: `out_view` checked non-null above.
            unsafe { *out_view = view };
            NveRuntimeStatus::Ok.as_i32()
        },
    )
}

unsafe extern "C" fn rt_scene_release(handle: NveRuntimeHandle) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::scene_release",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            if rt.lock.release_read() {
                NveRuntimeStatus::Ok.as_i32()
            } else {
                set_last_error("scene_release called without matching scene_acquire");
                NveRuntimeStatus::InvalidArg.as_i32()
            }
        },
    )
}

unsafe extern "C" fn rt_get_metrics(
    handle: NveRuntimeHandle,
    out_metrics: *mut NveRuntimeMetrics,
) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::get_metrics",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            if out_metrics.is_null() {
                set_last_error("null out_metrics");
                return NveRuntimeStatus::InvalidArg.as_i32();
            }
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            match rt.metrics() {
                Ok(m) => {
                    // SAFETY: `out_metrics` checked non-null above.
                    unsafe { *out_metrics = m };
                    NveRuntimeStatus::Ok.as_i32()
                }
                Err(e) => {
                    set_last_error(e.to_string());
                    e.status().as_i32()
                }
            }
        },
    )
}

unsafe extern "C" fn rt_pair_connect(
    handle: NveRuntimeHandle,
    uri: NveStrSlice,
    token: NveStrSlice,
) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::pair_connect",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            // SAFETY: Caller guarantees `uri.ptr` points to readable memory for `uri.len` bytes
            // when `uri.len > 0` (`NveStrSlice::as_str` validates UTF-8).
            let Ok(uri_str) = (unsafe { uri.as_str() }) else {
                set_last_error("invalid NveStrSlice for pairing URI");
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            // SAFETY: Caller guarantees `token.ptr` points to readable memory for `token.len`
            // bytes when `token.len > 0` (`NveStrSlice::as_str` validates UTF-8).
            let Ok(token_str) = (unsafe { token.as_str() }) else {
                set_last_error("invalid NveStrSlice for pairing token");
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            status_from_result(rt.pair_connect_shared(uri_str, token_str))
        },
    )
}

unsafe extern "C" fn rt_pair_disconnect(handle: NveRuntimeHandle) -> i32 {
    runtime_catch_unwind(
        "nve_runtime::pair_disconnect",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            status_from_result(rt.pair_disconnect_shared())
        },
    )
}

unsafe extern "C" fn rt_pair_is_connected(handle: NveRuntimeHandle) -> u32 {
    runtime_catch_unwind("nve_runtime::pair_is_connected", 0, || {
        // SAFETY: `handle` checked non-null inside `borrow_runtime`.
        let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
            return 0;
        };
        u32::from(rt.pair_is_connected())
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
    attach_app_interface: rt_attach_app_interface,
    submit_nve2: rt_submit_nve2,
    tick: rt_tick,
    dispatch_events: rt_dispatch_events,
    hit_test: rt_hit_test,
    scroll_by: rt_scroll_by,
    scene_acquire: rt_scene_acquire,
    scene_release: rt_scene_release,
    get_metrics: rt_get_metrics,
    pair_connect: rt_pair_connect,
    pair_disconnect: rt_pair_disconnect,
    pair_is_connected: rt_pair_is_connected,
    set_props: rt_set_props,
    drain_emitted_events: rt_drain_emitted_events,
    measure: rt_measure,
    apply_hot_patch: rt_apply_hot_patch,
    snapshot_state: rt_snapshot_state,
    restore_state: rt_restore_state,
    set_text_measurer: rt_set_text_measurer,
    resource_bytes: rt_resource_bytes,
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
