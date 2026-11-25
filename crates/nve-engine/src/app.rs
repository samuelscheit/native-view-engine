//! Safe, DRY Rust application trait (`NveApp`) and `export_nve_app!` macro.
//!
//! Apps implement `NveApp` once with zero `unsafe`. Rust-native hosts (Web WASM, Headless tests,
//! static release builds) invoke `NveApp` directly with zero FFI overhead (monomorphized/inlined),
//! while `export_nve_app!(MyApp)` generates the modular `nve_get_interface(uint32_t)` C ABI for
//! C++ Fabric, iOS/macOS, Android/JNI, Flutter `dart:ffi`, SwiftUI, and Tier-2 `cdylib` hot-swap.

use std::any::Any;
use std::cell::RefCell;

use nve_abi::{
    encode_bin_value_to_vec, encode_events, EventPayload, FrameId, NativeEvent, NativeEventKind,
    NveEventBatchRef, NveGestureDisposition, NveHostCallbacks, NveIntrinsicSize, NvePropsRef,
    NveSizeConstraints, SurfaceId,
};

thread_local! {
    static LAST_FFI_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Formats a caught panic payload into a human-readable message.
#[cold]
#[inline(never)]
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

    /// Processes a binary `EventRing` batch (`nve_abi::decode_events`); returns `true` if re-render is needed.
    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool;

    /// Processes a zero-copy direct FFI event batch (`NveEventBatchRef`) with zero deserialization.
    ///
    /// Default implementation bridges to `dispatch_events` via binary `EventRing` encoding so
    /// existing `NveApp` implementations continue working; override for true zero-allocation
    /// in-process direct event handling.
    fn dispatch_events_direct(&mut self, events: NveEventBatchRef<'_>) -> bool {
        if events.is_empty() {
            return false;
        }
        let mut owned = Vec::with_capacity(events.len());
        for ev in events.iter() {
            let Some(kind) = ev.kind() else {
                continue;
            };
            let payload = match ev.payload_tag() {
                0 => EventPayload::Empty,
                1 => EventPayload::Point {
                    x: ev.x(),
                    y: ev.y(),
                },
                2 => EventPayload::Scroll {
                    x: ev.x(),
                    y: ev.y(),
                    content_width: ev.width(),
                    content_height: ev.height(),
                },
                3 => EventPayload::Text(ev.text().to_owned()),
                4 => EventPayload::Layout {
                    x: ev.x(),
                    y: ev.y(),
                    width: ev.width(),
                    height: ev.height(),
                },
                5 => EventPayload::Custom {
                    name: ev.name().to_owned(),
                    data: encode_bin_value_to_vec(ev.value()).unwrap_or_default(),
                },
                _ => continue,
            };
            owned.push(NativeEvent {
                surface_id: ev.surface_id(),
                node_id: ev.node_id(),
                timestamp_us: ev.timestamp_us(),
                kind: if ev.payload_tag() == 5 {
                    NativeEventKind::Custom
                } else {
                    kind
                },
                payload,
            });
        }
        if owned.is_empty() {
            return false;
        }
        let encoded = encode_events(&owned);
        self.dispatch_events(&encoded)
    }

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

    /// Applies host-supplied encoded props (for embedded components in RN/Flutter/SwiftUI/Compose).
    fn set_props(&mut self, _props_bytes: &[u8]) -> bool {
        false
    }

    /// Applies zero-copy structured props (`NvePropsRef`) directly with zero serialization.
    fn set_props_direct(&mut self, _props: NvePropsRef<'_>) -> bool {
        false
    }

    /// Drains component-emitted outbound events as an `EventRing` byte slice.
    fn drain_emitted_events(&mut self) -> &[u8] {
        &[]
    }

    /// Applies a Tier-1 hot-reload JSON patch (`HotPatchMsg`).
    fn apply_hot_patch(&mut self, _patch_json: &str) -> bool {
        false
    }

    /// Serializes state for preservation across a Tier-2 `cdylib` hot-swap.
    fn snapshot_state(&mut self) -> &[u8] {
        &[]
    }

    /// Restores previously snapshotted state after a Tier-2 `cdylib` hot-swap.
    fn restore_state(&mut self, _state: &[u8]) {}

    /// Binds host-provided zero-JSON capability callbacks (`NVE_IFACE_HOST = 5`).
    /// Passing `None` unbinds any previously attached host callbacks.
    fn bind_host(&mut self, _callbacks: Option<NveHostCallbacks>) {}

    /// Dispatches a binary `NVH1` host-event batch (async completions, push events, channel messages).
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
                NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi, NveEventBatch,
                NveEventBatchRef, NveGestureDisposition, NveHostApi, NveHostCallbacks,
                NveIntrinsicSize, NveLayoutApi, NveMapEntry, NvePropsRef, NveSizeConstraints,
                NveSurfaceApi, NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_HOST,
                NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
            };
            use $crate::NveApp;

            unsafe extern "C" fn __nve_create(surface_id: u32) -> NveAppHandle {
                $crate::app::ffi_catch_unwind(
                    "NveSurfaceApi::create",
                    ::std::ptr::null_mut(),
                    || {
                        let boxed = Box::new(<$app_ty as NveApp>::create(surface_id));
                        Box::into_raw(boxed) as NveAppHandle
                    },
                )
            }

            unsafe extern "C" fn __nve_destroy(handle: NveAppHandle) {
                if handle.is_null() {
                    return;
                }
                $crate::app::ffi_catch_unwind("NveSurfaceApi::destroy", (), || {
                    unsafe { drop(Box::from_raw(handle as *mut $app_ty)) };
                });
            }

            unsafe extern "C" fn __nve_render(
                handle: NveAppHandle,
                frame_id: u64,
            ) -> NveBufferSlice {
                if handle.is_null() {
                    return NveBufferSlice::EMPTY;
                }
                $crate::app::ffi_catch_unwind(
                    "NveSurfaceApi::render",
                    NveBufferSlice::EMPTY,
                    || {
                        let app = unsafe { &mut *(handle as *mut $app_ty) };
                        NveBufferSlice::from_slice(app.render(frame_id))
                    },
                )
            }

            unsafe extern "C" fn __nve_dispatch_events(
                handle: NveAppHandle,
                events: NveBufferSlice,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                $crate::app::ffi_catch_unwind("NveSurfaceApi::dispatch_events", 0, || {
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    let bytes = unsafe { events.as_slice() };
                    u32::from(app.dispatch_events(bytes))
                })
            }

            unsafe extern "C" fn __nve_dispatch_events_direct(
                handle: NveAppHandle,
                batch: NveEventBatch,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                $crate::app::ffi_catch_unwind("NveSurfaceApi::dispatch_events_direct", 0, || {
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    let events = unsafe { NveEventBatchRef::from_raw(batch) };
                    u32::from(app.dispatch_events_direct(events))
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
                        let app = unsafe { &mut *(handle as *mut $app_ty) };
                        app.hit_test(x, y, pointer_phase) as u8
                    },
                )
            }

            unsafe extern "C" fn __nve_set_props(
                handle: NveAppHandle,
                props: NveBufferSlice,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                $crate::app::ffi_catch_unwind("NveComponentApi::set_props", 0, || {
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    let bytes = unsafe { props.as_slice() };
                    u32::from(app.set_props(bytes))
                })
            }

            unsafe extern "C" fn __nve_set_props_direct(
                handle: NveAppHandle,
                entries: *const NveMapEntry,
                count: u32,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                $crate::app::ffi_catch_unwind("NveComponentApi::set_props_direct", 0, || {
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    let props = unsafe { NvePropsRef::from_raw_parts(entries, count) };
                    u32::from(app.set_props_direct(props))
                })
            }

            unsafe extern "C" fn __nve_drain_emitted_events(
                handle: NveAppHandle,
            ) -> NveBufferSlice {
                if handle.is_null() {
                    return NveBufferSlice::EMPTY;
                }
                $crate::app::ffi_catch_unwind(
                    "NveComponentApi::drain_emitted_events",
                    NveBufferSlice::EMPTY,
                    || {
                        let app = unsafe { &mut *(handle as *mut $app_ty) };
                        NveBufferSlice::from_slice(app.drain_emitted_events())
                    },
                )
            }

            unsafe extern "C" fn __nve_apply_hot_patch(
                handle: NveAppHandle,
                patch_json: NveBufferSlice,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                $crate::app::ffi_catch_unwind("NveDevApi::apply_hot_patch", 0, || {
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    let Ok(s) = ::std::str::from_utf8(unsafe { patch_json.as_slice() }) else {
                        return 0;
                    };
                    u32::from(app.apply_hot_patch(s))
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
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    app.restore_state(unsafe { state.as_slice() });
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
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    let bound = if callbacks.is_null() {
                        None
                    } else {
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
                    let app = unsafe { &mut *(handle as *mut $app_ty) };
                    let bytes = unsafe { batch.as_slice() };
                    u32::from(app.dispatch_host_events(bytes))
                })
            }

            static __NVE_SURFACE_API: NveSurfaceApi = NveSurfaceApi {
                create: __nve_create,
                destroy: __nve_destroy,
                render: __nve_render,
                dispatch_events: __nve_dispatch_events,
                dispatch_events_direct: Some(__nve_dispatch_events_direct),
            };

            static __NVE_LAYOUT_API: NveLayoutApi = NveLayoutApi {
                measure: __nve_measure,
                hit_test: __nve_hit_test,
            };

            static __NVE_COMPONENT_API: NveComponentApi = NveComponentApi {
                set_props: __nve_set_props,
                drain_emitted_events: __nve_drain_emitted_events,
                set_props_direct: Some(__nve_set_props_direct),
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
            pub unsafe extern "C" fn $symbol(interface_id: u32) -> *const c_void {
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
mod tests {
    use std::cell::Cell;
    use std::ffi::c_void;

    use nve_abi::{
        NveBufferSlice, NveComponentApi, NveDevApi, NveDirectEvent, NveEventBatch,
        NveGestureDisposition, NveHostApi, NveHostCallbacks, NveIntrinsicSize, NveLayoutApi,
        NveMapEntry, NveSizeConstraints, NveSurfaceApi, NveValue, NVE_IFACE_COMPONENT,
        NVE_IFACE_DEV, NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
    };

    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum PanicTarget {
        None,
        Create,
        Render,
        DispatchEvents,
        DispatchEventsDirect,
        Measure,
        HitTest,
        SetProps,
        SetPropsDirect,
        DrainEmittedEvents,
        ApplyHotPatch,
        SnapshotState,
        RestoreState,
        BindHost,
        DispatchHostEvents,
        Destroy,
        NonStringPayload,
    }

    thread_local! {
        static PANIC_TARGET: Cell<PanicTarget> = const { Cell::new(PanicTarget::None) };
    }

    fn set_panic_target(target: PanicTarget) {
        PANIC_TARGET.with(|cell| cell.set(target));
    }

    fn current_panic_target() -> PanicTarget {
        PANIC_TARGET.with(Cell::get)
    }

    struct PanickingTestApp {
        surface_id: SurfaceId,
        frame_buf: Vec<u8>,
        emitted_buf: Vec<u8>,
        state_buf: Vec<u8>,
    }

    impl Drop for PanickingTestApp {
        fn drop(&mut self) {
            if current_panic_target() == PanicTarget::Destroy {
                panic!("boom in destroy for surface {}", self.surface_id);
            }
        }
    }

    impl NveApp for PanickingTestApp {
        fn create(surface_id: SurfaceId) -> Self {
            match current_panic_target() {
                PanicTarget::Create => panic!("boom in create"),
                PanicTarget::NonStringPayload => std::panic::panic_any(404u32),
                _ => Self {
                    surface_id,
                    frame_buf: vec![0xAA, 0xBB],
                    emitted_buf: vec![0xCC],
                    state_buf: vec![1, 2, 3],
                },
            }
        }

        fn render(&mut self, frame_id: FrameId) -> &[u8] {
            if current_panic_target() == PanicTarget::Render {
                panic!("boom in render frame {frame_id}");
            }
            &self.frame_buf
        }

        fn dispatch_events(&mut self, _encoded_events: &[u8]) -> bool {
            if current_panic_target() == PanicTarget::DispatchEvents {
                panic!("boom in dispatch_events");
            }
            true
        }

        fn dispatch_events_direct(&mut self, _events: NveEventBatchRef<'_>) -> bool {
            if current_panic_target() == PanicTarget::DispatchEventsDirect {
                panic!("boom in dispatch_events_direct");
            }
            true
        }

        fn measure(&mut self, _constraints: NveSizeConstraints) -> NveIntrinsicSize {
            if current_panic_target() == PanicTarget::Measure {
                panic!("boom in measure");
            }
            NveIntrinsicSize {
                width: 123.0,
                height: 45.0,
            }
        }

        fn hit_test(&mut self, _x: f32, _y: f32, _pointer_phase: u8) -> NveGestureDisposition {
            if current_panic_target() == PanicTarget::HitTest {
                panic!("boom in hit_test");
            }
            NveGestureDisposition::Claim
        }

        fn set_props(&mut self, _props_bytes: &[u8]) -> bool {
            if current_panic_target() == PanicTarget::SetProps {
                panic!("boom in set_props");
            }
            true
        }

        fn set_props_direct(&mut self, _props: NvePropsRef<'_>) -> bool {
            if current_panic_target() == PanicTarget::SetPropsDirect {
                panic!("boom in set_props_direct");
            }
            true
        }

        fn drain_emitted_events(&mut self) -> &[u8] {
            if current_panic_target() == PanicTarget::DrainEmittedEvents {
                panic!("boom in drain_emitted_events");
            }
            &self.emitted_buf
        }

        fn apply_hot_patch(&mut self, _patch_json: &str) -> bool {
            if current_panic_target() == PanicTarget::ApplyHotPatch {
                panic!("boom in apply_hot_patch");
            }
            true
        }

        fn snapshot_state(&mut self) -> &[u8] {
            if current_panic_target() == PanicTarget::SnapshotState {
                panic!("boom in snapshot_state");
            }
            &self.state_buf
        }

        fn restore_state(&mut self, state: &[u8]) {
            if current_panic_target() == PanicTarget::RestoreState {
                panic!("boom in restore_state");
            }
            self.state_buf = state.to_vec();
        }

        fn bind_host(&mut self, _callbacks: Option<NveHostCallbacks>) {
            if current_panic_target() == PanicTarget::BindHost {
                panic!("boom in bind_host");
            }
        }

        fn dispatch_host_events(&mut self, _batch: &[u8]) -> bool {
            if current_panic_target() == PanicTarget::DispatchHostEvents {
                panic!("boom in dispatch_host_events");
            }
            true
        }
    }

    crate::export_nve_app!(PanickingTestApp, symbol = test_panicking_app_get_interface);

    unsafe extern "C" {
        fn test_panicking_app_get_interface(interface_id: u32) -> *const c_void;
    }

    #[test]
    fn export_nve_app_catches_panics_across_all_13_shims_and_null_handles() {
        clear_last_ffi_panic();
        set_panic_target(PanicTarget::None);

        let surface = unsafe {
            &*(test_panicking_app_get_interface(NVE_IFACE_SURFACE) as *const NveSurfaceApi)
        };
        let layout = unsafe {
            &*(test_panicking_app_get_interface(NVE_IFACE_LAYOUT) as *const NveLayoutApi)
        };
        let component = unsafe {
            &*(test_panicking_app_get_interface(NVE_IFACE_COMPONENT) as *const NveComponentApi)
        };
        let dev =
            unsafe { &*(test_panicking_app_get_interface(NVE_IFACE_DEV) as *const NveDevApi) };
        let host =
            unsafe { &*(test_panicking_app_get_interface(NVE_IFACE_HOST) as *const NveHostApi) };
        let dispatch_direct = surface
            .dispatch_events_direct
            .expect("dispatch_events_direct exported");
        let set_props_direct = component
            .set_props_direct
            .expect("set_props_direct exported");

        // 0. Null handle guards across all 14 handle-taking shims (no dereference, no panic).
        let null_h = std::ptr::null_mut();
        unsafe {
            assert_eq!((surface.render)(null_h, 1).len, 0);
            assert_eq!((surface.dispatch_events)(null_h, NveBufferSlice::EMPTY), 0);
            assert_eq!(dispatch_direct(null_h, NveEventBatch::EMPTY), 0);
            assert_eq!(
                (layout.measure)(null_h, NveSizeConstraints::loose(100.0, 100.0)),
                NveIntrinsicSize {
                    width: 0.0,
                    height: 0.0
                }
            );
            assert_eq!(
                (layout.hit_test)(null_h, 0.0, 0.0, 0),
                NveGestureDisposition::PassThrough as u8
            );
            assert_eq!((component.set_props)(null_h, NveBufferSlice::EMPTY), 0);
            assert_eq!(set_props_direct(null_h, std::ptr::null(), 0), 0);
            assert_eq!((component.drain_emitted_events)(null_h).len, 0);
            assert_eq!((dev.apply_hot_patch)(null_h, NveBufferSlice::EMPTY), 0);
            assert_eq!((dev.snapshot_state)(null_h).len, 0);
            (dev.restore_state)(null_h, NveBufferSlice::EMPTY);
            (host.bind_host)(null_h, std::ptr::null());
            assert_eq!((host.dispatch_host_events)(null_h, NveBufferSlice::EMPTY), 0);
            (surface.destroy)(null_h);
        }
        assert_eq!(last_ffi_panic(), None);

        // 1. Panicking `create` returns null and records message; non-string payloads also work.
        set_panic_target(PanicTarget::Create);
        let h_fail = unsafe { (surface.create)(7) };
        assert!(h_fail.is_null());
        let peeked = last_ffi_panic().expect("create panic recorded");
        assert!(peeked.contains("NveSurfaceApi::create"));
        assert!(peeked.contains("boom in create"));
        assert_eq!(take_last_ffi_panic(), Some(peeked));
        assert_eq!(last_ffi_panic(), None);

        set_panic_target(PanicTarget::NonStringPayload);
        let h_non_str = unsafe { (surface.create)(8) };
        assert!(h_non_str.is_null());
        let non_str_msg = take_last_ffi_panic().expect("non-string panic recorded");
        assert!(non_str_msg.contains("non-string panic payload"));

        // 2. Normal create + verify happy path does not record any panic.
        set_panic_target(PanicTarget::None);
        let handle = unsafe { (surface.create)(42) };
        assert!(!handle.is_null());
        assert_eq!(unsafe { (surface.render)(handle, 1).as_slice() }, &[0xAA, 0xBB]);
        assert_eq!(
            unsafe { (layout.measure)(handle, NveSizeConstraints::loose(300.0, 200.0)) },
            NveIntrinsicSize {
                width: 123.0,
                height: 45.0
            }
        );
        assert_eq!(
            unsafe { (layout.hit_test)(handle, 1.0, 2.0, 0) },
            NveGestureDisposition::Claim as u8
        );
        assert_eq!(last_ffi_panic(), None);

        // 3. Verify every remaining shim survives panics and returns its exact sentinel.
        set_panic_target(PanicTarget::Render);
        let r = unsafe { (surface.render)(handle, 99) };
        assert!(r.ptr.is_null() && r.len == 0);
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveSurfaceApi::render") && msg.contains("boom in render frame 99"));

        set_panic_target(PanicTarget::DispatchEvents);
        assert_eq!(
            unsafe { (surface.dispatch_events)(handle, NveBufferSlice::from_slice(b"ev")) },
            0
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveSurfaceApi::dispatch_events") && msg.contains("boom in dispatch_events"));

        set_panic_target(PanicTarget::DispatchEventsDirect);
        let direct_ev = [NveDirectEvent::press(42, 2, 100, 1.0, 2.0)];
        assert_eq!(
            unsafe { dispatch_direct(handle, NveEventBatch::from_slice(&direct_ev)) },
            0
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveSurfaceApi::dispatch_events_direct")
                && msg.contains("boom in dispatch_events_direct")
        );

        set_panic_target(PanicTarget::Measure);
        assert_eq!(
            unsafe { (layout.measure)(handle, NveSizeConstraints::loose(100.0, 100.0)) },
            NveIntrinsicSize {
                width: 0.0,
                height: 0.0
            }
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveLayoutApi::measure") && msg.contains("boom in measure"));

        set_panic_target(PanicTarget::HitTest);
        assert_eq!(
            unsafe { (layout.hit_test)(handle, 5.0, 6.0, 1) },
            NveGestureDisposition::PassThrough as u8
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveLayoutApi::hit_test") && msg.contains("boom in hit_test"));

        set_panic_target(PanicTarget::SetProps);
        assert_eq!(
            unsafe { (component.set_props)(handle, NveBufferSlice::from_slice(b"props")) },
            0
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveComponentApi::set_props") && msg.contains("boom in set_props"));

        set_panic_target(PanicTarget::SetPropsDirect);
        let entries = [NveMapEntry::new("k", NveValue::i64(1))];
        assert_eq!(
            unsafe { set_props_direct(handle, entries.as_ptr(), entries.len() as u32) },
            0
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveComponentApi::set_props_direct")
                && msg.contains("boom in set_props_direct")
        );

        set_panic_target(PanicTarget::DrainEmittedEvents);
        let drained = unsafe { (component.drain_emitted_events)(handle) };
        assert!(drained.ptr.is_null() && drained.len == 0);
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveComponentApi::drain_emitted_events")
                && msg.contains("boom in drain_emitted_events")
        );

        set_panic_target(PanicTarget::ApplyHotPatch);
        assert_eq!(
            unsafe { (dev.apply_hot_patch)(handle, NveBufferSlice::from_slice(b"{}")) },
            0
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveDevApi::apply_hot_patch") && msg.contains("boom in apply_hot_patch")
        );

        set_panic_target(PanicTarget::SnapshotState);
        let snap = unsafe { (dev.snapshot_state)(handle) };
        assert!(snap.ptr.is_null() && snap.len == 0);
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveDevApi::snapshot_state") && msg.contains("boom in snapshot_state")
        );

        set_panic_target(PanicTarget::RestoreState);
        unsafe { (dev.restore_state)(handle, NveBufferSlice::from_slice(&[9, 8, 7])) };
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveDevApi::restore_state") && msg.contains("boom in restore_state"));

        set_panic_target(PanicTarget::BindHost);
        let empty_cb = NveHostCallbacks::EMPTY;
        unsafe { (host.bind_host)(handle, &empty_cb) };
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveHostApi::bind_host") && msg.contains("boom in bind_host"));

        set_panic_target(PanicTarget::DispatchHostEvents);
        assert_eq!(
            unsafe { (host.dispatch_host_events)(handle, NveBufferSlice::from_slice(b"NVH1")) },
            0
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveHostApi::dispatch_host_events")
                && msg.contains("boom in dispatch_host_events")
        );

        // 4. Panicking `destroy` (Drop) survives without aborting and records the message.
        set_panic_target(PanicTarget::Destroy);
        unsafe { (surface.destroy)(handle) };
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveSurfaceApi::destroy") && msg.contains("boom in destroy for surface 42")
        );
        set_panic_target(PanicTarget::None);
        assert_eq!(last_ffi_panic(), None);
    }
}
