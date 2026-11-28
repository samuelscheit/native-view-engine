//! `NveRuntimeApi` vtable and `NveRuntimeGetInterfaceFn` entry-point signature, derived directly
//! via `#[derive(NveAbi)]` and `abi_callbacks!` (single source of truth for C/Dart/Java/TS
//! bindings, compile-time slot locks, and [`super::NVE_RUNTIME_SCHEMA_HASH`]).

use core::ffi::c_void;

use nve_abi_macros::NveAbi;

use super::{
    NveRuntimeConfig, NveRuntimeHandle, NveRuntimeIntrinsicSizeInvalidatedFn,
    NveRuntimeMeasureTextFn, NveRuntimeMetrics, NveRuntimeWakeFn, NveSceneView, NveSurfaceMeasure,
};
use crate::event::NveEventBatch;
use crate::ffi::{NveBufferSlice, NveGetInterfaceFn, NveSizeConstraints};
use crate::host::{HostRequestId, HostServiceId, NveHostCallbacks};
use crate::id::NodeId;
use crate::value::{NveMapEntry, NveStrSlice};

/// Single exported C ABI vtable returned by `nve_runtime_get_interface(uint32_t abi_version)`.
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
#[nve_abi(vtable, slots_offset = 16)]
pub struct NveRuntimeApi {
    #[nve_abi(offset = 0)]
    pub abi_version: u32,
    #[nve_abi(offset = 4)]
    pub struct_size: u32,
    #[nve_abi(offset = 8)]
    pub schema_hash: u64,
    /// Creates a surface bound to the calling (owner) thread; NULL config = defaults.
    pub create: unsafe extern "C" fn(config: *const NveRuntimeConfig) -> NveRuntimeHandle,
    /// Owner thread; drains wake callbacks and readers, then frees.
    pub destroy: unsafe extern "C" fn(handle: NveRuntimeHandle),
    /// Installs (NULL clears) the wake callback.
    pub set_wake_callback: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        cb: Option<NveRuntimeWakeFn>,
        ctx: *mut c_void,
    ) -> i32,
    /// Owner thread; finite, > 0.
    pub set_viewport: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        width: f32,
        height: f32,
        scale_factor: f32,
    ) -> i32,
    /// Owner thread; installs a native text measurer (NULL restores the built-in one) and relayouts; `ctx` must outlive it.
    pub set_text_measurer: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        measure: Option<NveRuntimeMeasureTextFn>,
        ctx: *mut c_void,
    ) -> i32,
    /// Owner thread; attaches (NULL detaches) an `nve_get_interface` app.
    pub attach_app_interface: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        get_iface: Option<NveGetInterfaceFn>,
    ) -> i32,
    /// Owner thread; loads an app cdylib from a private staged copy (a rebuilt file always runs new code) and attaches it; the old module unloads after its app is destroyed. Nonzero carry_state restores the running app's NVE_IFACE_DEV state into the new app before it goes live. IO: cannot stage/open; NOT_FOUND: no nve_get_interface; UNSUPPORTED: no complete NVE_IFACE_SURFACE (or wasm32). A failure leaves the running app or pairing untouched.
    pub load_app_module: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        utf8_path: NveStrSlice,
        carry_state: u32,
    ) -> i32,
    /// Owner thread; forwards embedding-host props (NVE_IFACE_COMPONENT); UNSUPPORTED without it.
    pub set_props: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        entries: *const NveMapEntry,
        count: u32,
        out_changed: *mut u32,
    ) -> i32,
    /// Owner thread; drains app-emitted events, borrowed until the next owner-thread call.
    pub drain_emitted_events:
        unsafe extern "C" fn(handle: NveRuntimeHandle, out_events: *mut NveEventBatch) -> i32,
    /// Owner thread; app state snapshot (NVE_IFACE_DEV), borrowed until the next owner-thread call.
    pub snapshot_state:
        unsafe extern "C" fn(handle: NveRuntimeHandle, out_state: *mut NveBufferSlice) -> i32,
    /// Owner thread; restores a snapshot and schedules a re-render.
    pub restore_state: unsafe extern "C" fn(handle: NveRuntimeHandle, state: NveBufferSlice) -> i32,
    /// Owner thread; writes the app's state snapshot as a crash-atomic checksummed StateFile (an older-schema file is rotated to <file>.v<n>, a corrupt one quarantined); NOT_FOUND when the app has no state.
    pub save_state_file: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        utf8_path: NveStrSlice,
        schema_version: u32,
    ) -> i32,
    /// Owner thread; restores a save_state_file file into the app; NOT_FOUND when missing, corrupt (quarantined to <file>.corrupt), or older-schema (rotated to <file>.v<n>); IO for a newer schema.
    pub restore_state_file: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        utf8_path: NveStrSlice,
        schema_version: u32,
    ) -> i32,
    /// Owner thread; deletes a state file (cold start); OK when already absent.
    pub clear_state_file:
        unsafe extern "C" fn(handle: NveRuntimeHandle, utf8_path: NveStrSlice) -> i32,
    /// Owner thread; validates and forwards one NVP1 hot patch (NVE_IFACE_DEV).
    pub apply_hot_patch: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        patch_nvp1: NveBufferSlice,
        out_changed: *mut u32,
    ) -> i32,
    /// Owner thread; applies an NVE2 frame (no app attached).
    pub submit_nve2:
        unsafe extern "C" fn(handle: NveRuntimeHandle, frame: NveBufferSlice) -> i32,
    /// Owner thread; renders, lays out, publishes.
    pub tick:
        unsafe extern "C" fn(handle: NveRuntimeHandle, dt_s: f32, out_changed: *mut u32) -> i32,
    /// Owner thread.
    pub dispatch_events: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        batch: NveEventBatch,
        out_changed: *mut u32,
    ) -> i32,
    /// Owner thread; 0 = no hit.
    pub hit_test: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        x: f32,
        y: f32,
        out_node_id: *mut NodeId,
    ) -> i32,
    /// Owner thread.
    pub scroll_by: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        node_id: NodeId,
        dx: f32,
        dy: f32,
        out_x: *mut f32,
        out_y: *mut f32,
    ) -> i32,
    /// Any thread; BUSY while a write is in progress.
    pub scene_acquire:
        unsafe extern "C" fn(handle: NveRuntimeHandle, out_view: *mut NveSceneView) -> i32,
    /// Any thread.
    pub scene_release: unsafe extern "C" fn(handle: NveRuntimeHandle) -> i32,
    /// Any thread holding scene_acquire; canonical payload bytes + kind Key64 of a published ResourceId, borrowed until scene_release; NOT_FOUND when stale.
    pub resource_payload: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        resource_id: u32,
        out_kind: *mut u64,
        out_payload: *mut NveBufferSlice,
    ) -> i32,
    /// Any thread.
    pub get_metrics:
        unsafe extern "C" fn(handle: NveRuntimeHandle, out_metrics: *mut NveRuntimeMetrics) -> i32,
    /// Owner thread; replaces the local app or previous pairing (a rejected URI/token leaves it running).
    pub pair_connect: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        uri: NveStrSlice,
        token: NveStrSlice,
    ) -> i32,
    /// Owner thread.
    pub pair_disconnect: unsafe extern "C" fn(handle: NveRuntimeHandle) -> i32,
    /// Any thread.
    pub pair_is_connected: unsafe extern "C" fn(handle: NveRuntimeHandle) -> u32,
    /// Calling thread's last error (NUL-terminated); returns the full byte length.
    pub last_error: unsafe extern "C" fn(out_buf: *mut u8, cap: usize) -> usize,
    /// Owner thread; measures surface under constraints (`NVE_MEASURE_FLAG_DRY_RUN` never mutates committed viewport/rects or publishes; `NVE_MEASURE_FLAG_IDEAL` requests intrinsic unconstrained size).
    pub measure_surface: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        constraints: *const NveSizeConstraints,
        flags: u32,
        out_measure: *mut NveSurfaceMeasure,
    ) -> i32,
    /// Installs (NULL clears and drains) the intrinsic-size-invalidated callback; invoked outside internal locks on layout-affecting commits.
    pub set_intrinsic_size_invalidated_callback: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        cb: Option<NveRuntimeIntrinsicSizeInvalidatedFn>,
        ctx: *mut c_void,
    ) -> i32,
    /// Owner thread; writes the active scene as a crash-atomic checksummed Frame-0 NVE2 file; NOT_FOUND when empty, UNSUPPORTED on wasm32.
    pub save_frame0_file: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        utf8_path: NveStrSlice,
        app_build_id: u64,
        env_hash: u64,
    ) -> i32,
    /// Owner thread; validates and boots a Frame-0 NVE2 file into the replica scene; auto-removes corrupt/mismatched files and returns NOT_FOUND, UNSUPPORTED on wasm32.
    pub boot_frame0_file: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        utf8_path: NveStrSlice,
        app_build_id: u64,
        env_hash: u64,
    ) -> i32,
    /// Owner thread; validates and boots an in-memory Frame-0 NVE2 envelope into the replica scene (works on wasm32).
    pub boot_frame0_bytes: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        envelope: NveBufferSlice,
        app_build_id: u64,
        env_hash: u64,
    ) -> i32,
    /// Owner thread; installs (NULL clears and drains) host capability callbacks for NVE_IFACE_HOST = 5.
    pub set_host_callbacks: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        callbacks: *const NveHostCallbacks,
    ) -> i32,
    /// Any thread; completes an in-flight NVE_HOST_PENDING request in the runtime host router and triggers wake (stale/unknown ids are dropped).
    pub complete_host_request: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        request_id: HostRequestId,
        status: i32,
        payload: NveBufferSlice,
    ) -> i32,
    /// Any thread; enqueues an unsolicited host push event for delivery on the next tick and triggers wake.
    pub push_host_event: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        service_id: HostServiceId,
        event_id: u32,
        payload: NveBufferSlice,
    ) -> i32,
    /// Any thread; enqueues an inbound host channel message for delivery on the next tick and triggers wake.
    pub publish_host_channel: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        channel_id: u32,
        flags: u32,
        payload: NveBufferSlice,
    ) -> i32,
}

nve_abi_macros::abi_callbacks! {
    pub(super) const RUNTIME_ENTRY_CALLBACKS;

    /// Entry-point signature exported by `libnve_runtime` (`nve_runtime_get_interface`).
    pub type NveRuntimeGetInterfaceFn = unsafe extern "C" fn(abi_version: u32) -> *const NveRuntimeApi;
}
