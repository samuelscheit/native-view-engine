//! A C module that leaves a slot of a returned `NveXxxApi` NULL must never reach Rust as a
//! non-nullable fn pointer: a partial required `NVE_IFACE_SURFACE` is rejected at attach, and a
//! partial optional `NVE_IFACE_DEV` reads as absent (`snapshot_state` -> `NotFound`/`Unsupported`).

use std::ffi::c_void;
use std::ptr;

use nve_abi::{
    FrameId, InterfaceId, NveBufferSlice, NveEventBatch, NveRuntimeApi, NveRuntimeConfig,
    NveRuntimeStatus, SurfaceId, NVE_IFACE_DEV, NVE_IFACE_SURFACE, NVE_RUNTIME_ABI_VERSION,
};
use nve_runtime::nve_runtime_get_interface;

/// Layout twin of `NveSurfaceApi` / `NveDevApi` whose slots may be NULL (as a C module may write).
#[repr(C)]
struct RawVtable<const N: usize>([*const c_void; N]);
// SAFETY: Immutable table of function addresses.
unsafe impl<const N: usize> Sync for RawVtable<N> {}

unsafe extern "C" fn create(_surface: SurfaceId) -> *mut c_void {
    Box::into_raw(Box::new(0u8)).cast()
}
unsafe extern "C" fn destroy(h: *mut c_void) {
    // SAFETY: `h` came from `create`.
    drop(unsafe { Box::from_raw(h.cast::<u8>()) });
}
unsafe extern "C" fn render(_h: *mut c_void, _frame: FrameId) -> NveBufferSlice {
    NveBufferSlice::EMPTY
}
unsafe extern "C" fn dispatch(_h: *mut c_void, _batch: NveEventBatch) -> u32 {
    0
}
unsafe extern "C" fn snapshot(_h: *mut c_void) -> NveBufferSlice {
    NveBufferSlice::from_slice(b"state")
}
unsafe extern "C" fn restore(_h: *mut c_void, _state: NveBufferSlice) {}

static FULL_SURFACE: RawVtable<4> = RawVtable([
    create as *const c_void,
    destroy as *const c_void,
    render as *const c_void,
    dispatch as *const c_void,
]);
static PARTIAL_SURFACE: RawVtable<4> = RawVtable([
    create as *const c_void,
    destroy as *const c_void,
    render as *const c_void,
    ptr::null(),
]);
/// `apply_hot_patch` left NULL (a C app without hot patching).
static PARTIAL_DEV: RawVtable<3> =
    RawVtable([ptr::null(), snapshot as *const c_void, restore as *const c_void]);

unsafe extern "C" fn partial_surface_module(id: InterfaceId) -> *const c_void {
    if id == NVE_IFACE_SURFACE { (&raw const PARTIAL_SURFACE).cast() } else { ptr::null() }
}
unsafe extern "C" fn partial_dev_module(id: InterfaceId) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => (&raw const FULL_SURFACE).cast(),
        NVE_IFACE_DEV => (&raw const PARTIAL_DEV).cast(),
        _ => ptr::null(),
    }
}

fn api() -> NveRuntimeApi {
    // SAFETY: Static vtable returned for the matching ABI version.
    unsafe { *nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION) }
}

#[test]
fn partial_vtables_are_rejected_instead_of_transmuted() {
    let api = api();
    let cfg = NveRuntimeConfig { surface_id: 1, width: 100.0, height: 100.0, scale_factor: 1.0, flags: 0, _reserved: 0 };
    // SAFETY: Valid config; the handle is destroyed below on this thread.
    let h = unsafe { (api.create)(&cfg) };
    assert!(!h.is_null());
    // SAFETY: Live handle on its owner thread; the modules follow the C ABI except for NULL slots.
    unsafe {
        assert_ne!((api.attach_app_interface)(h, Some(partial_surface_module)), NveRuntimeStatus::Ok.as_i32());
        assert_eq!((api.attach_app_interface)(h, Some(partial_dev_module)), NveRuntimeStatus::Ok.as_i32());
        let mut state = NveBufferSlice::EMPTY;
        let rc = (api.snapshot_state)(h, &mut state);
        // The dev interface reads as absent: no state, never a call through a NULL slot.
        assert!(rc != NveRuntimeStatus::Ok.as_i32() || state.len == 0, "rc={rc} len={}", state.len);
        (api.destroy)(h);
    }
}
