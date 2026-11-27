//! Single-entry versioned C ABI (`nve_runtime_get_interface`) exporting [`NveRuntimeApi`].

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Arc;

use nve_abi::{
    NveBufferSlice, NveEventBatch, NveEventBatchRef, NveGetInterfaceFn, NveRuntimeApi,
    NveRuntimeConfig, NveRuntimeHandle, NveRuntimeMetrics, NveRuntimeSceneView, NveRuntimeStatus,
    NveRuntimeWakeFn, NveStrSlice, NVE_RUNTIME_ABI_VERSION, NVE_RUNTIME_SCHEMA_HASH,
};
use nve_engine::Viewport;
use nve_engine::app::ffi_catch_unwind;
use nve_store::NodeId;

use crate::app::CAbiAppAdapter;
use crate::builder::RuntimeBuilder;
use crate::surface::SurfaceRuntime;

thread_local! {
    static LAST_RUNTIME_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn set_last_error(msg: impl Into<String>) {
    let s = msg.into();
    LAST_RUNTIME_ERROR.with(|cell| {
        *cell.borrow_mut() = s;
    });
}

fn clear_last_error() {
    LAST_RUNTIME_ERROR.with(|cell| {
        cell.borrow_mut().clear();
    });
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
    ffi_catch_unwind("nve_runtime::create", std::ptr::null_mut(), || {
        clear_last_error();
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
    ffi_catch_unwind("nve_runtime::destroy", (), || {
        // SAFETY: `handle` was allocated by `rt_create` via `Box::into_raw` and is freed once.
        let rt = unsafe { &*(handle.cast::<SurfaceRuntime>()) };
        rt.lock.acquire_write_blocking();
        // SAFETY: Exclusive write lock acquired above ensures no reader holds references.
        let boxed = unsafe { Box::from_raw(handle.cast::<SurfaceRuntime>()) };
        drop(boxed);
    });
}

unsafe extern "C" fn rt_set_wake_callback(
    handle: NveRuntimeHandle,
    cb: Option<NveRuntimeWakeFn>,
    ctx: *mut c_void,
) -> i32 {
    ffi_catch_unwind(
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
                        // SAFETY: Host guarantees `func(ctx)` is valid until cleared or destroyed.
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
    ffi_catch_unwind(
        "nve_runtime::set_viewport",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            if !width.is_finite()
                || width <= 0.0
                || !height.is_finite()
                || height <= 0.0
                || !scale_factor.is_finite()
                || scale_factor <= 0.0
            {
                set_last_error("viewport dimensions and scale_factor must be finite and > 0");
                return NveRuntimeStatus::InvalidArg.as_i32();
            }
            let res = rt.with_write_shared(|inner| {
                if inner.state.scene.set_viewport(Viewport {
                    width,
                    height,
                    scale_factor,
                }) {
                    inner.state.needs_publish = true;
                }
                Ok(())
            });
            match res {
                Ok(()) => NveRuntimeStatus::Ok.as_i32(),
                Err(e) => {
                    set_last_error(e.to_string());
                    e.status().as_i32()
                }
            }
        },
    )
}

unsafe extern "C" fn rt_attach_app_interface(
    handle: NveRuntimeHandle,
    get_iface: Option<NveGetInterfaceFn>,
) -> i32 {
    ffi_catch_unwind(
        "nve_runtime::attach_app_interface",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            let res = rt.with_write_shared(|inner| {
                match get_iface {
                    Some(f) => {
                        // SAFETY: Host guarantees `f` is a valid NVE `nve_get_interface` symbol.
                        let Some(adapter) = (unsafe {
                            CAbiAppAdapter::from_get_interface(f, inner.state.surface_id)
                        }) else {
                            return Err(crate::RuntimeError::InvalidArg(
                                "app interface returned null surface API or app handle",
                            ));
                        };
                        inner.app = Some(Box::new(adapter));
                        inner.state.needs_render = true;
                        inner.state.needs_publish = true;
                    }
                    None => {
                        inner.app = None;
                    }
                }
                Ok(())
            });
            match res {
                Ok(()) => NveRuntimeStatus::Ok.as_i32(),
                Err(e) => {
                    set_last_error(e.to_string());
                    e.status().as_i32()
                }
            }
        },
    )
}

unsafe extern "C" fn rt_submit_nve2(handle: NveRuntimeHandle, frame: NveBufferSlice) -> i32 {
    ffi_catch_unwind(
        "nve_runtime::submit_nve2",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            // SAFETY: Caller guarantees `frame` is readable for `frame.len` bytes for this call.
            let bytes = unsafe { frame.as_slice() };
            match rt.with_write_shared(|inner| inner.submit_nve2(bytes)) {
                Ok(()) => NveRuntimeStatus::Ok.as_i32(),
                Err(e) => {
                    set_last_error(e.to_string());
                    e.status().as_i32()
                }
            }
        },
    )
}

unsafe extern "C" fn rt_tick(
    handle: NveRuntimeHandle,
    dt_s: f32,
    out_changed: *mut u32,
) -> i32 {
    ffi_catch_unwind("nve_runtime::tick", NveRuntimeStatus::Panic.as_i32(), || {
        // SAFETY: `handle` checked non-null inside `borrow_runtime`.
        let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
            return NveRuntimeStatus::InvalidArg.as_i32();
        };
        let dt = if dt_s.is_finite() && dt_s >= 0.0 {
            dt_s
        } else {
            0.0
        };
        let _ = rt.wake_bridge().take_pending();
        match rt.with_write_shared(|inner| inner.tick(dt)) {
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
    ffi_catch_unwind(
        "nve_runtime::dispatch_events",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            // SAFETY: Caller guarantees `batch` points to valid `NveDirectEvent`s for this call.
            let events = unsafe { NveEventBatchRef::from_raw(batch) };
            match rt.with_write_shared(|inner| {
                let changed = inner
                    .app
                    .as_mut()
                    .is_some_and(|app| app.dispatch_events(events));
                if changed {
                    inner.state.needs_render = true;
                }
                Ok(changed)
            }) {
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
    ffi_catch_unwind(
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
    ffi_catch_unwind(
        "nve_runtime::scroll_by",
        NveRuntimeStatus::Panic.as_i32(),
        || {
            // SAFETY: `handle` checked non-null inside `borrow_runtime`.
            let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
                return NveRuntimeStatus::InvalidArg.as_i32();
            };
            if !dx.is_finite() || !dy.is_finite() {
                set_last_error("scroll deltas must be finite");
                return NveRuntimeStatus::InvalidArg.as_i32();
            }
            match rt.with_write_shared(|inner| {
                let store = match inner.app.as_ref().and_then(|a| a.direct_store()) {
                    Some(s) => s,
                    None => &inner.replica_store,
                };
                let (x, y, _) = inner
                    .state
                    .scene
                    .apply_scroll_delta(store, NodeId(node_id), dx, dy);
                Ok((x, y))
            }) {
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
    ffi_catch_unwind(
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
    ffi_catch_unwind(
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
    ffi_catch_unwind(
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
    _uri: NveStrSlice,
    _token: NveStrSlice,
) -> i32 {
    if handle.is_null() {
        return NveRuntimeStatus::InvalidArg.as_i32();
    }
    set_last_error("nve-runtime built without wire pairing support");
    NveRuntimeStatus::Unsupported.as_i32()
}

unsafe extern "C" fn rt_pair_disconnect(handle: NveRuntimeHandle) -> i32 {
    if handle.is_null() {
        return NveRuntimeStatus::InvalidArg.as_i32();
    }
    NveRuntimeStatus::Unsupported.as_i32()
}

unsafe extern "C" fn rt_pair_is_connected(_handle: NveRuntimeHandle) -> u32 {
    0
}

unsafe extern "C" fn rt_last_error(out_buf: *mut u8, cap: usize) -> usize {
    LAST_RUNTIME_ERROR.with(|cell| {
        let s = cell.borrow();
        let bytes = s.as_bytes();
        if !out_buf.is_null() && cap > 0 {
            let n = bytes.len().min(cap.saturating_sub(1));
            // SAFETY: `out_buf` is valid for `cap` bytes; `n < cap`.
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, n);
                *out_buf.add(n) = 0;
            }
        }
        bytes.len()
    })
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
