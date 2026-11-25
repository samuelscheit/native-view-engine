//! Safe, DRY Rust application trait (`NveApp`) and `export_nve_app!` macro.
//!
//! Apps implement `NveApp` once with zero `unsafe`. Rust-native hosts (Web WASM, Headless tests,
//! static release builds) invoke `NveApp` directly with zero FFI overhead (monomorphized/inlined),
//! while `export_nve_app!(MyApp)` generates the modular `nve_get_interface(uint32_t)` C ABI for
//! C++ Fabric, iOS/macOS, Android/JNI, Flutter `dart:ffi`, SwiftUI, and Tier-2 `cdylib` hot-swap.

use nve_abi::{
    FrameId, NveGestureDisposition, NveIntrinsicSize, NveSizeConstraints, SurfaceId,
};

pub trait NveApp: Sized + 'static {
    fn create(surface_id: SurfaceId) -> Self;

    /// Renders pending mutations into a binary `nve_abi::CommandBuffer` slice valid until the next call.
    fn render(&mut self, frame_id: FrameId) -> &[u8];

    /// Processes a binary `EventRing` batch (`nve_abi::decode_events`); returns `true` if re-render is needed.
    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool;

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

    /// Applies host-supplied JSON props (for embedded components in RN/Flutter/SwiftUI/Compose).
    fn set_props(&mut self, _props_json: &[u8]) -> bool {
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
                NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi, NveIntrinsicSize,
                NveLayoutApi, NveSizeConstraints, NveSurfaceApi, NVE_IFACE_COMPONENT,
                NVE_IFACE_DEV, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
            };
            use $crate::NveApp;

            unsafe extern "C" fn __nve_create(surface_id: u32) -> NveAppHandle {
                let boxed = Box::new(<$app_ty as NveApp>::create(surface_id));
                Box::into_raw(boxed) as NveAppHandle
            }

            unsafe extern "C" fn __nve_destroy(handle: NveAppHandle) {
                if !handle.is_null() {
                    unsafe { drop(Box::from_raw(handle as *mut $app_ty)) };
                }
            }

            unsafe extern "C" fn __nve_render(
                handle: NveAppHandle,
                frame_id: u64,
            ) -> NveBufferSlice {
                if handle.is_null() {
                    return NveBufferSlice::EMPTY;
                }
                let app = unsafe { &mut *(handle as *mut $app_ty) };
                NveBufferSlice::from_slice(app.render(frame_id))
            }

            unsafe extern "C" fn __nve_dispatch_events(
                handle: NveAppHandle,
                events: NveBufferSlice,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                let app = unsafe { &mut *(handle as *mut $app_ty) };
                let bytes = unsafe { events.as_slice() };
                u32::from(app.dispatch_events(bytes))
            }

            unsafe extern "C" fn __nve_measure(
                handle: NveAppHandle,
                constraints: NveSizeConstraints,
            ) -> NveIntrinsicSize {
                if handle.is_null() {
                    return NveIntrinsicSize::default();
                }
                let app = unsafe { &mut *(handle as *mut $app_ty) };
                app.measure(constraints)
            }

            unsafe extern "C" fn __nve_hit_test(
                handle: NveAppHandle,
                x: f32,
                y: f32,
                pointer_phase: u8,
            ) -> u8 {
                if handle.is_null() {
                    return 2; // PassThrough
                }
                let app = unsafe { &mut *(handle as *mut $app_ty) };
                app.hit_test(x, y, pointer_phase) as u8
            }

            unsafe extern "C" fn __nve_set_props(
                handle: NveAppHandle,
                props: NveBufferSlice,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                let app = unsafe { &mut *(handle as *mut $app_ty) };
                let bytes = unsafe { props.as_slice() };
                u32::from(app.set_props(bytes))
            }

            unsafe extern "C" fn __nve_drain_emitted_events(
                handle: NveAppHandle,
            ) -> NveBufferSlice {
                if handle.is_null() {
                    return NveBufferSlice::EMPTY;
                }
                let app = unsafe { &mut *(handle as *mut $app_ty) };
                NveBufferSlice::from_slice(app.drain_emitted_events())
            }

            unsafe extern "C" fn __nve_apply_hot_patch(
                handle: NveAppHandle,
                patch_json: NveBufferSlice,
            ) -> u32 {
                if handle.is_null() {
                    return 0;
                }
                let app = unsafe { &mut *(handle as *mut $app_ty) };
                let Ok(s) = ::std::str::from_utf8(unsafe { patch_json.as_slice() }) else {
                    return 0;
                };
                u32::from(app.apply_hot_patch(s))
            }

            unsafe extern "C" fn __nve_snapshot_state(handle: NveAppHandle) -> NveBufferSlice {
                if handle.is_null() {
                    return NveBufferSlice::EMPTY;
                }
                let app = unsafe { &mut *(handle as *mut $app_ty) };
                NveBufferSlice::from_slice(app.snapshot_state())
            }

            unsafe extern "C" fn __nve_restore_state(
                handle: NveAppHandle,
                state: NveBufferSlice,
            ) {
                if handle.is_null() {
                    return;
                }
                let app = unsafe { &mut *(handle as *mut $app_ty) };
                app.restore_state(unsafe { state.as_slice() });
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

            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $symbol(interface_id: u32) -> *const c_void {
                match interface_id {
                    NVE_IFACE_SURFACE => &__NVE_SURFACE_API as *const NveSurfaceApi as *const c_void,
                    NVE_IFACE_LAYOUT => &__NVE_LAYOUT_API as *const NveLayoutApi as *const c_void,
                    NVE_IFACE_COMPONENT => {
                        &__NVE_COMPONENT_API as *const NveComponentApi as *const c_void
                    }
                    NVE_IFACE_DEV => &__NVE_DEV_API as *const NveDevApi as *const c_void,
                    _ => ::std::ptr::null(),
                }
            }
        };
    };
}
