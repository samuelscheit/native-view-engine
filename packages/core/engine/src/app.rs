//! Safe, DRY Rust application trait (`NveApp`) and `export_nve_app!` macro.
//!
//! Apps implement `NveApp` once with zero `unsafe`. Rust-native hosts (Web WASM, Headless tests,
//! static release builds) invoke `NveApp` directly with zero FFI overhead (monomorphized/inlined),
//! while `export_nve_app!(MyApp)` generates the modular `nve_get_interface(uint32_t)` C ABI for
//! C++ Fabric, iOS/macOS, Android/JNI, Flutter `dart:ffi`, SwiftUI, and Tier-2 `cdylib` hot-swap.

use std::any::Any;
use std::cell::RefCell;

use nve_abi::{
    FrameId, NveDirectEvent, NveEventBatchRef, NveGestureDisposition, NveHostCallbacks,
    NveIntrinsicSize, NvePropsRef, NveSizeConstraints, SurfaceId,
};

thread_local! {
    static LAST_FFI_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Formats a caught panic payload into a human-readable message.
#[cold]
#[inline(never)]
#[expect(
    clippy::disallowed_methods,
    reason = "FFI catch_unwind panic payload boundary requires std::any::Any downcast"
)]
pub fn format_panic_payload(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

/// Records a panic captured at an `extern "C"` FFI boundary for the current thread.
#[cold]
#[inline(never)]
#[doc(hidden)]
pub fn record_ffi_panic(fn_name: &'static str, payload: Box<dyn Any + Send>) {
    let detail = format_panic_payload(&*payload);
    let msg = format!("{fn_name}: {detail}");
    LAST_FFI_PANIC.with(|slot| {
        *slot.borrow_mut() = Some(msg);
    });
}

/// Returns a copy of the last panic message captured at an `extern "C"` FFI boundary on the
/// current thread, if any, without clearing it.
pub fn last_ffi_panic() -> Option<String> {
    LAST_FFI_PANIC.with(|slot| slot.borrow().clone())
}

/// Takes and clears the last panic message captured at an `extern "C"` FFI boundary on the
/// current thread, if any.
pub fn take_last_ffi_panic() -> Option<String> {
    LAST_FFI_PANIC.with(|slot| slot.borrow_mut().take())
}

/// Clears any recorded FFI panic on the current thread.
pub fn clear_last_ffi_panic() {
    LAST_FFI_PANIC.with(|slot| {
        let _ = slot.borrow_mut().take();
    });
}

/// Executes `f` inside a zero-cost `catch_unwind` boundary.
///
/// On normal non-panicking execution, this returns `f()` directly without touching thread-local
/// storage. On panic, it prevents unwinding across `extern "C"`, records the formatted message in
/// the current thread's FFI panic slot, and returns `fallback`.
#[inline(always)]
#[doc(hidden)]
pub fn ffi_catch_unwind<T, F>(fn_name: &'static str, fallback: T, f: F) -> T
where
    F: FnOnce() -> T,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(val) => val,
        Err(payload) => {
            record_ffi_panic(fn_name, payload);
            fallback
        }
    }
}

pub trait NveApp: Sized + 'static {
    fn create(surface_id: SurfaceId) -> Self;

    /// Renders pending mutations into a binary `nve_abi::CommandBuffer` slice valid until the next call.
    fn render(&mut self, frame_id: FrameId) -> &[u8];

    /// Processes a direct typed event batch (`NveEventBatchRef`); returns `true` if re-render is needed.
    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool;

    /// Intrinsic size measurement for embedded hosts (RN Yoga, Flutter RenderBox, SwiftUI, Compose).
    fn measure(&mut self, constraints: NveSizeConstraints) -> NveIntrinsicSize {
        let w = if constraints.max_w.is_finite() {
            constraints.max_w.max(constraints.min_w)
        } else {
            constraints.min_w
        };
        let h = if constraints.max_h.is_finite() {
            constraints.max_h.max(constraints.min_h)
        } else {
            constraints.min_h
        };
        NveIntrinsicSize {
            width: w,
            height: h,
        }
    }

    /// Synchronous hit-test / gesture disposition query across framework boundaries.
    fn hit_test(&mut self, _x: f32, _y: f32, _pointer_phase: u8) -> NveGestureDisposition {
        NveGestureDisposition::PassThrough
    }

    /// Applies host-supplied direct typed props (`NvePropsRef`); returns `true` if re-render is needed.
    fn set_props(&mut self, _props: NvePropsRef<'_>) -> bool {
        false
    }

    /// Drains component-emitted outbound events as a direct borrowed `NveDirectEvent` slice.
    fn drain_emitted_events(&mut self) -> &[NveDirectEvent] {
        &[]
    }

    /// Applies a Tier-1 hot-reload patch, decoded from the `NVP1` bytes the host passed to
    /// `NveDevApi::apply_hot_patch`.
    fn apply_hot_patch(&mut self, _patch: &crate::HotPatchMsg) -> bool {
        false
    }

    /// Serializes state for preservation across a Tier-2 `cdylib` hot-swap.
    fn snapshot_state(&mut self) -> &[u8] {
        &[]
    }

    /// Restores previously snapshotted state after a Tier-2 `cdylib` hot-swap.
    fn restore_state(&mut self, _state: &[u8]) {}

    /// Binds host-provided binary capability callbacks (`NVE_IFACE_HOST = 5`).
    /// Passing `None` unbinds any previously attached host callbacks.
    fn bind_host(&mut self, _callbacks: Option<NveHostCallbacks>) {}

    /// Dispatches a host-event payload buffer across `NVE_IFACE_HOST = 5`.
    /// Returns `true` if the surface should re-render.
    fn dispatch_host_events(&mut self, _batch: &[u8]) -> bool {
        false
    }
}

/// Generates the modular `nve_get_interface` C ABI (and optional custom symbol alias) from any
/// safe `NveApp` implementation with zero boilerplate `unsafe` in app code.
#[macro_export]
macro_rules! export_nve_app {
    ($app_ty:ty) => {
        $crate::export_nve_app!($app_ty, symbol = nve_get_interface);
    };
    ($app_ty:ty, symbol = $symbol:ident) => {
        const _: () = {
            use ::std::ffi::c_void;
            use $crate::abi::{
                FrameId, InterfaceId, NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi,
                NveEventBatch, NveEventBatchRef, NveGestureDisposition, NveHostApi,
                NveHostCallbacks, NveIntrinsicSize, NveLayoutApi, NveMapEntry, NvePropsRef,
                NveSizeConstraints, NveSurfaceApi, SurfaceId, NVE_IFACE_COMPONENT, NVE_IFACE_DEV,
                NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
            };
            use $crate::NveApp;

            unsafe extern "C" fn __nve_create(surface_id: SurfaceId) -> NveAppHandle {
                $crate::app::ffi_catch_unwind(
                    "NveSurfaceApi::create",
                    ::std::ptr::null_mut(),
                    || {
                        let canonical = SurfaceId::from_raw_or_null(surface_id.raw());
                        let boxed = Box::new(<$app_ty as NveApp>::create(canonical));
                        Box::into_raw(boxed) as NveAppHandle
                    },
                )
            }

            unsafe extern "C" fn __nve_destroy(handle: NveAppHandle) {
                if handle.is_null() {
                    return;
                }
                $crate::app::ffi_catch_unwind("NveSurfaceApi::destroy", (), || {
                    // SAFETY: `handle` was allocated by `__nve_create` via `Box::into_raw` and is destroyed at most once.
                    unsafe { drop(Box::from_raw(handle as *mut $app_ty)) };
                });
            }

            unsafe extern "C" fn __nve_render(
                handle: NveAppHandle,
                frame_id: FrameId,
            ) -> NveBufferSlice {
                if handle.is_null() {
                    return NveBufferSlice::EMPTY;
                }
                $crate::app::ffi_catch_unwind(
                    "NveSurfaceApi::render",
                    NveBufferSlice::EMPTY,
                    || {
                        // SAFETY: Non-null `handle` is an exclusive live `$app_ty` created by `__nve_create`.
                        let app = unsafe { &mut *(handle as *mut $app_ty) };
                        NveBufferSlice::from_slice(app.render(frame_id))
                    },
                )
            }

            unsafe extern "C" fn __nve_dispatch_events(
                handle: NveAppHandle,
                batch: NveEventBatch,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                $crate::app::ffi_catch_unwind("NveSurfaceApi::dispatch_events", 0, || {
                    // SAFETY: Non-null `handle` is an exclusive live `$app_ty` created by `__nve_create`.
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    // SAFETY: Host guarantees `batch` points to valid `NveDirectEvent`s for this call.
                    let events = unsafe { NveEventBatchRef::from_raw(batch) };
                    u32::from(app.dispatch_events(events))
                })
            }

            unsafe extern "C" fn __nve_measure(
                handle: NveAppHandle,
                constraints: NveSizeConstraints,
            ) -> NveIntrinsicSize {
                if handle.is_null() {
                    return NveIntrinsicSize {
                        width: 0.0,
                        height: 0.0,
                    };
                }
                $crate::app::ffi_catch_unwind(
                    "NveLayoutApi::measure",
                    NveIntrinsicSize {
                        width: 0.0,
                        height: 0.0,
                    },
                    || {
                        // SAFETY: Non-null `handle` is an exclusive live `$app_ty` created by `__nve_create`.
                        let app = unsafe { &mut *(handle as *mut $app_ty) };
                        app.measure(constraints)
                    },
                )
            }

            unsafe extern "C" fn __nve_hit_test(
                handle: NveAppHandle,
                x: f32,
                y: f32,
                pointer_phase: u8,
            ) -> u8 {
                if handle.is_null() {
                    return NveGestureDisposition::PassThrough as u8;
                }
                $crate::app::ffi_catch_unwind(
                    "NveLayoutApi::hit_test",
                    NveGestureDisposition::PassThrough as u8,
                    || {
                        // SAFETY: Non-null `handle` is an exclusive live `$app_ty` created by `__nve_create`.
                        let app = unsafe { &mut *(handle as *mut $app_ty) };
                        app.hit_test(x, y, pointer_phase) as u8
                    },
                )
            }

            unsafe extern "C" fn __nve_set_props(
                handle: NveAppHandle,
                entries: *const NveMapEntry,
                count: u32,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                $crate::app::ffi_catch_unwind("NveComponentApi::set_props", 0, || {
                    // SAFETY: Non-null `handle` is an exclusive live `$app_ty` created by `__nve_create`.
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    // SAFETY: Host guarantees `entries..entries+count` is readable for this call.
                    let props = unsafe { NvePropsRef::from_raw_parts(entries, count) };
                    u32::from(app.set_props(props))
                })
            }

            unsafe extern "C" fn __nve_drain_emitted_events(
                handle: NveAppHandle,
            ) -> NveEventBatch {
                if handle.is_null() {
                    return NveEventBatch::EMPTY;
                }
                $crate::app::ffi_catch_unwind(
                    "NveComponentApi::drain_emitted_events",
                    NveEventBatch::EMPTY,
                    || {
                        // SAFETY: Non-null `handle` is an exclusive live `$app_ty` created by `__nve_create`.
                        let app = unsafe { &mut *(handle as *mut $app_ty) };
                        NveEventBatch::from_slice(app.drain_emitted_events())
                    },
                )
            }

            unsafe extern "C" fn __nve_apply_hot_patch(
                handle: NveAppHandle,
                patch_nvp1: NveBufferSlice,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                $crate::app::ffi_catch_unwind("NveDevApi::apply_hot_patch", 0, || {
                    // SAFETY: Non-null `handle` is an exclusive live `$app_ty` created by `__nve_create`.
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    // SAFETY: Host guarantees `patch_nvp1` is readable for the duration of this call.
                    let raw = unsafe { patch_nvp1.as_slice() };
                    let Ok(msg) = $crate::HotPatchMsg::from_bytes(raw)
                    else {
                        return 0;
                    };
                    u32::from(app.apply_hot_patch(&msg))
                })
            }

            unsafe extern "C" fn __nve_snapshot_state(handle: NveAppHandle) -> NveBufferSlice {
                if handle.is_null() {
                    return NveBufferSlice::EMPTY;
                }
                $crate::app::ffi_catch_unwind(
                    "NveDevApi::snapshot_state",
                    NveBufferSlice::EMPTY,
                    || {
                        // SAFETY: Non-null `handle` is an exclusive live `$app_ty` created by `__nve_create`.
                        let app = unsafe { &mut *(handle as *mut $app_ty) };
                        NveBufferSlice::from_slice(app.snapshot_state())
                    },
                )
            }

            unsafe extern "C" fn __nve_restore_state(
                handle: NveAppHandle,
                state: NveBufferSlice,
            ) {
                if handle.is_null() {
                    return;
                }
                $crate::app::ffi_catch_unwind("NveDevApi::restore_state", (), || {
                    // SAFETY: Non-null `handle` is an exclusive live `$app_ty` created by `__nve_create`.
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    // SAFETY: Host guarantees `state` is readable for the duration of this call.
                    let bytes = unsafe { state.as_slice() };
                    app.restore_state(bytes);
                });
            }

            unsafe extern "C" fn __nve_bind_host(
                handle: NveAppHandle,
                callbacks: *const NveHostCallbacks,
            ) {
                if handle.is_null() {
                    return;
                }
                $crate::app::ffi_catch_unwind("NveHostApi::bind_host", (), || {
                    // SAFETY: Non-null `handle` is an exclusive live `$app_ty` created by `__nve_create`.
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    let bound = if callbacks.is_null() {
                        None
                    } else {
                        // SAFETY: Non-null `callbacks` points to a valid `NveHostCallbacks` struct.
                        Some(unsafe { *callbacks })
                    };
                    app.bind_host(bound);
                });
            }

            unsafe extern "C" fn __nve_dispatch_host_events(
                handle: NveAppHandle,
                batch: NveBufferSlice,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                $crate::app::ffi_catch_unwind("NveHostApi::dispatch_host_events", 0, || {
                    // SAFETY: Non-null `handle` is an exclusive live `$app_ty` created by `__nve_create`.
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    // SAFETY: Host guarantees `batch` is readable for the duration of this call.
                    let bytes = unsafe { batch.as_slice() };
                    u32::from(app.dispatch_host_events(bytes))
                })
            }

            static __NVE_SURFACE_API: NveSurfaceApi = NveSurfaceApi {
                create: __nve_create,
                destroy: __nve_destroy,
                render: __nve_render,
                dispatch_events: __nve_dispatch_events,
            };

            static __NVE_LAYOUT_API: NveLayoutApi = NveLayoutApi {
                measure: __nve_measure,
                hit_test: __nve_hit_test,
            };

            static __NVE_COMPONENT_API: NveComponentApi = NveComponentApi {
                set_props: __nve_set_props,
                drain_emitted_events: __nve_drain_emitted_events,
            };

            static __NVE_DEV_API: NveDevApi = NveDevApi {
                apply_hot_patch: __nve_apply_hot_patch,
                snapshot_state: __nve_snapshot_state,
                restore_state: __nve_restore_state,
            };

            static __NVE_HOST_API: NveHostApi = NveHostApi {
                bind_host: __nve_bind_host,
                dispatch_host_events: __nve_dispatch_host_events,
            };

            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $symbol(interface_id: InterfaceId) -> *const c_void {
                match interface_id {
                    NVE_IFACE_SURFACE => &__NVE_SURFACE_API as *const NveSurfaceApi as *const c_void,
                    NVE_IFACE_LAYOUT => &__NVE_LAYOUT_API as *const NveLayoutApi as *const c_void,
                    NVE_IFACE_COMPONENT => {
                        &__NVE_COMPONENT_API as *const NveComponentApi as *const c_void
                    }
                    NVE_IFACE_DEV => &__NVE_DEV_API as *const NveDevApi as *const c_void,
                    NVE_IFACE_HOST => &__NVE_HOST_API as *const NveHostApi as *const c_void,
                    _ => ::std::ptr::null(),
                }
            }
        };
    };
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
