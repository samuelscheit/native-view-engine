//! `NveRuntimeApi` vtable plus its single-source C signature descriptors
//! ([`RUNTIME_CALLBACK_TYPEDEFS`], [`RUNTIME_API_SLOTS`]) and their compile-time slot locks.

use core::ffi::c_void;
use core::mem::{align_of, offset_of, size_of};

use nve_hash::Xxh64Hasher;

use super::{
    NveRuntimeConfig, NveRuntimeHandle, NveRuntimeIntrinsicSizeInvalidatedFn,
    NveRuntimeMeasureTextFn, NveRuntimeMetrics, NveRuntimeWakeFn, NveSceneView, NveSurfaceMeasure,
};
use crate::event::NveEventBatch;
use crate::ffi::{NveBufferSlice, NveGetInterfaceFn, NveSizeConstraints};
use crate::host::{HostRequestId, HostServiceId, NveHostCallbacks};
use crate::id::NodeId;
use crate::value::{NveMapEntry, NveStrSlice};

/// C spelling of one runtime ABI function-pointer signature.
///
/// [`RUNTIME_CALLBACK_TYPEDEFS`] and [`RUNTIME_API_SLOTS`] are the single source for
/// [`NVE_RUNTIME_SCHEMA_HASH`](super::NVE_RUNTIME_SCHEMA_HASH) and the generated `nve_runtime_abi.generated.h`; the
/// `lock_runtime_api_slots!` const block below proves they match [`NveRuntimeApi`] slot for slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeFnSignature {
    pub name: &'static str,
    /// C return type.
    pub ret: &'static str,
    /// C parameter list (types and names).
    pub params: &'static str,
    /// One-line contract emitted into the generated header (not hashed).
    pub doc: &'static str,
}

const fn sig(
    name: &'static str,
    ret: &'static str,
    params: &'static str,
    doc: &'static str,
) -> RuntimeFnSignature {
    RuntimeFnSignature {
        name,
        ret,
        params,
        doc,
    }
}

/// Host callback typedefs of the runtime ABI.
pub const RUNTIME_CALLBACK_TYPEDEFS: &[RuntimeFnSignature] = &[
    sig("NveRuntimeWakeFn", "void", "void* ctx", "Wake callback; may fire on any thread."),
    sig(
        "NveRuntimeMeasureTextFn",
        "void",
        "void* ctx, NveStrSlice text, const NveRuntimeTextStyle* style, float max_width, float* out_width, float* out_height",
        "Native text measurer; owner thread, during layout; measures `text` with the node's resolved typography (`style`, borrowed for the call); `max_width` is INFINITY when unconstrained.",
    ),
    sig(
        "NveRuntimeIntrinsicSizeInvalidatedFn",
        "void",
        "void* ctx",
        "Intrinsic-size-invalidated callback; owner thread, invoked outside internal locks after layout-affecting commits.",
    ),
];

/// Every function-pointer slot of [`NveRuntimeApi`], in vtable order.
pub const RUNTIME_API_SLOTS: &[RuntimeFnSignature] = &[
    sig("create", "NveRuntimeHandle", "const NveRuntimeConfig* config", "Creates a surface bound to the calling (owner) thread; NULL config = defaults."),
    sig("destroy", "void", "NveRuntimeHandle handle", "Owner thread; drains wake callbacks and readers, then frees."),
    sig("set_wake_callback", "int32_t", "NveRuntimeHandle handle, NveRuntimeWakeFn cb, void* ctx", "Installs (NULL clears) the wake callback."),
    sig("set_viewport", "int32_t", "NveRuntimeHandle handle, float width, float height, float scale_factor", "Owner thread; finite, > 0."),
    sig("set_text_measurer", "int32_t", "NveRuntimeHandle handle, NveRuntimeMeasureTextFn measure, void* ctx", "Owner thread; installs a native text measurer (NULL restores the built-in one) and relayouts; `ctx` must outlive it."),
    sig("attach_app_interface", "int32_t", "NveRuntimeHandle handle, NveGetInterfaceFn get_iface", "Owner thread; attaches (NULL detaches) an `nve_get_interface` app."),
    sig("load_app_module", "int32_t", "NveRuntimeHandle handle, NveStrSlice utf8_path, uint32_t carry_state", "Owner thread; loads an app cdylib from a private staged copy (a rebuilt file always runs new code) and attaches it; the old module unloads after its app is destroyed. Nonzero carry_state restores the running app's NVE_IFACE_DEV state into the new app before it goes live. IO: cannot stage/open; NOT_FOUND: no nve_get_interface; UNSUPPORTED: no complete NVE_IFACE_SURFACE (or wasm32). A failure leaves the running app or pairing untouched."),
    sig("set_props", "int32_t", "NveRuntimeHandle handle, const struct NveMapEntry* entries, uint32_t count, uint32_t* out_changed", "Owner thread; forwards embedding-host props (NVE_IFACE_COMPONENT); UNSUPPORTED without it."),
    sig("drain_emitted_events", "int32_t", "NveRuntimeHandle handle, NveEventBatch* out_events", "Owner thread; drains app-emitted events, borrowed until the next owner-thread call."),
    sig("snapshot_state", "int32_t", "NveRuntimeHandle handle, NveBufferSlice* out_state", "Owner thread; app state snapshot (NVE_IFACE_DEV), borrowed until the next owner-thread call."),
    sig("restore_state", "int32_t", "NveRuntimeHandle handle, NveBufferSlice state", "Owner thread; restores a snapshot and schedules a re-render."),
    sig("save_state_file", "int32_t", "NveRuntimeHandle handle, NveStrSlice utf8_path, uint32_t schema_version", "Owner thread; writes the app's state snapshot as a crash-atomic checksummed StateFile (an older-schema file is rotated to <file>.v<n>, a corrupt one quarantined); NOT_FOUND when the app has no state."),
    sig("restore_state_file", "int32_t", "NveRuntimeHandle handle, NveStrSlice utf8_path, uint32_t schema_version", "Owner thread; restores a save_state_file file into the app; NOT_FOUND when missing, corrupt (quarantined to <file>.corrupt), or older-schema (rotated to <file>.v<n>); IO for a newer schema."),
    sig("clear_state_file", "int32_t", "NveRuntimeHandle handle, NveStrSlice utf8_path", "Owner thread; deletes a state file (cold start); OK when already absent."),
    sig("apply_hot_patch", "int32_t", "NveRuntimeHandle handle, NveBufferSlice patch_nvp1, uint32_t* out_changed", "Owner thread; validates and forwards one NVP1 hot patch (NVE_IFACE_DEV)."),
    sig("submit_nve2", "int32_t", "NveRuntimeHandle handle, NveBufferSlice frame", "Owner thread; applies an NVE2 frame (no app attached)."),
    sig("tick", "int32_t", "NveRuntimeHandle handle, float dt_s, uint32_t* out_changed", "Owner thread; renders, lays out, publishes."),
    sig("dispatch_events", "int32_t", "NveRuntimeHandle handle, NveEventBatch batch, uint32_t* out_changed", "Owner thread."),
    sig("hit_test", "int32_t", "NveRuntimeHandle handle, float x, float y, uint32_t* out_node_id", "Owner thread; 0 = no hit."),
    sig("scroll_by", "int32_t", "NveRuntimeHandle handle, uint32_t node_id, float dx, float dy, float* out_x, float* out_y", "Owner thread."),
    sig("scene_acquire", "int32_t", "NveRuntimeHandle handle, NveSceneView* out_view", "Any thread; BUSY while a write is in progress."),
    sig("scene_release", "int32_t", "NveRuntimeHandle handle", "Any thread."),
    sig("resource_payload", "int32_t", "NveRuntimeHandle handle, uint32_t resource_id, uint64_t* out_kind, NveBufferSlice* out_payload", "Any thread holding scene_acquire; canonical payload bytes + kind Key64 of a published ResourceId, borrowed until scene_release; NOT_FOUND when stale."),
    sig("get_metrics", "int32_t", "NveRuntimeHandle handle, NveRuntimeMetrics* out_metrics", "Any thread."),
    sig("pair_connect", "int32_t", "NveRuntimeHandle handle, NveStrSlice uri, NveStrSlice token", "Owner thread; replaces the local app or previous pairing (a rejected URI/token leaves it running)."),
    sig("pair_disconnect", "int32_t", "NveRuntimeHandle handle", "Owner thread."),
    sig("pair_is_connected", "uint32_t", "NveRuntimeHandle handle", "Any thread."),
    sig("last_error", "size_t", "uint8_t* out_buf, size_t cap", "Calling thread's last error (NUL-terminated); returns the full byte length."),
    sig("measure_surface", "int32_t", "NveRuntimeHandle handle, const NveSizeConstraints* constraints, uint32_t flags, NveSurfaceMeasure* out_measure", "Owner thread; measures surface under constraints (`NVE_MEASURE_FLAG_DRY_RUN` never mutates committed viewport/rects or publishes; `NVE_MEASURE_FLAG_IDEAL` requests intrinsic unconstrained size)."),
    sig("set_intrinsic_size_invalidated_callback", "int32_t", "NveRuntimeHandle handle, NveRuntimeIntrinsicSizeInvalidatedFn cb, void* ctx", "Installs (NULL clears and drains) the intrinsic-size-invalidated callback; invoked outside internal locks on layout-affecting commits."),
    sig("save_frame0_file", "int32_t", "NveRuntimeHandle handle, NveStrSlice utf8_path, uint64_t app_build_id, uint64_t env_hash", "Owner thread; writes the active scene as a crash-atomic checksummed Frame-0 NVE2 file; NOT_FOUND when empty, UNSUPPORTED on wasm32."),
    sig("boot_frame0_file", "int32_t", "NveRuntimeHandle handle, NveStrSlice utf8_path, uint64_t app_build_id, uint64_t env_hash", "Owner thread; validates and boots a Frame-0 NVE2 file into the replica scene; auto-removes corrupt/mismatched files and returns NOT_FOUND, UNSUPPORTED on wasm32."),
    sig("boot_frame0_bytes", "int32_t", "NveRuntimeHandle handle, NveBufferSlice envelope, uint64_t app_build_id, uint64_t env_hash", "Owner thread; validates and boots an in-memory Frame-0 NVE2 envelope into the replica scene (works on wasm32)."),
    sig("set_host_callbacks", "int32_t", "NveRuntimeHandle handle, const NveHostCallbacks* callbacks", "Owner thread; installs (NULL clears and drains) host capability callbacks for NVE_IFACE_HOST = 5."),
    sig("complete_host_request", "int32_t", "NveRuntimeHandle handle, uint64_t request_id, int32_t status, NveBufferSlice payload", "Any thread; completes an in-flight NVE_HOST_PENDING request in the runtime host router and triggers wake (stale/unknown ids are dropped)."),
    sig("push_host_event", "int32_t", "NveRuntimeHandle handle, uint32_t service_id, uint32_t event_id, NveBufferSlice payload", "Any thread; enqueues an unsolicited host push event for delivery on the next tick and triggers wake."),
    sig("publish_host_channel", "int32_t", "NveRuntimeHandle handle, uint32_t channel_id, uint32_t flags, NveBufferSlice payload", "Any thread; enqueues an inbound host channel message for delivery on the next tick and triggers wake."),
];

/// Single exported C ABI vtable returned by `nve_runtime_get_interface(uint32_t abi_version)`.
///
/// Slot contracts live in [`RUNTIME_API_SLOTS`] (emitted as header docs).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveRuntimeApi {
    pub abi_version: u32,
    pub struct_size: u32,
    pub schema_hash: u64,
    pub create: unsafe extern "C" fn(config: *const NveRuntimeConfig) -> NveRuntimeHandle,
    pub destroy: unsafe extern "C" fn(handle: NveRuntimeHandle),
    pub set_wake_callback: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        cb: Option<NveRuntimeWakeFn>,
        ctx: *mut c_void,
    ) -> i32,
    pub set_viewport: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        width: f32,
        height: f32,
        scale_factor: f32,
    ) -> i32,
    pub set_text_measurer: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        measure: Option<NveRuntimeMeasureTextFn>,
        ctx: *mut c_void,
    ) -> i32,
    pub attach_app_interface: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        get_iface: Option<NveGetInterfaceFn>,
    ) -> i32,
    pub load_app_module: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        utf8_path: NveStrSlice,
        carry_state: u32,
    ) -> i32,
    pub set_props: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        entries: *const NveMapEntry,
        count: u32,
        out_changed: *mut u32,
    ) -> i32,
    pub drain_emitted_events:
        unsafe extern "C" fn(handle: NveRuntimeHandle, out_events: *mut NveEventBatch) -> i32,
    pub snapshot_state:
        unsafe extern "C" fn(handle: NveRuntimeHandle, out_state: *mut NveBufferSlice) -> i32,
    pub restore_state: unsafe extern "C" fn(handle: NveRuntimeHandle, state: NveBufferSlice) -> i32,
    pub save_state_file: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        utf8_path: NveStrSlice,
        schema_version: u32,
    ) -> i32,
    pub restore_state_file: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        utf8_path: NveStrSlice,
        schema_version: u32,
    ) -> i32,
    pub clear_state_file:
        unsafe extern "C" fn(handle: NveRuntimeHandle, utf8_path: NveStrSlice) -> i32,
    pub apply_hot_patch: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        patch_nvp1: NveBufferSlice,
        out_changed: *mut u32,
    ) -> i32,
    pub submit_nve2:
        unsafe extern "C" fn(handle: NveRuntimeHandle, frame: NveBufferSlice) -> i32,
    pub tick:
        unsafe extern "C" fn(handle: NveRuntimeHandle, dt_s: f32, out_changed: *mut u32) -> i32,
    pub dispatch_events: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        batch: NveEventBatch,
        out_changed: *mut u32,
    ) -> i32,
    pub hit_test: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        x: f32,
        y: f32,
        out_node_id: *mut NodeId,
    ) -> i32,
    pub scroll_by: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        node_id: NodeId,
        dx: f32,
        dy: f32,
        out_x: *mut f32,
        out_y: *mut f32,
    ) -> i32,
    pub scene_acquire:
        unsafe extern "C" fn(handle: NveRuntimeHandle, out_view: *mut NveSceneView) -> i32,
    pub scene_release: unsafe extern "C" fn(handle: NveRuntimeHandle) -> i32,
    pub resource_payload: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        resource_id: u32,
        out_kind: *mut u64,
        out_payload: *mut NveBufferSlice,
    ) -> i32,
    pub get_metrics:
        unsafe extern "C" fn(handle: NveRuntimeHandle, out_metrics: *mut NveRuntimeMetrics) -> i32,
    pub pair_connect: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        uri: NveStrSlice,
        token: NveStrSlice,
    ) -> i32,
    pub pair_disconnect: unsafe extern "C" fn(handle: NveRuntimeHandle) -> i32,
    pub pair_is_connected: unsafe extern "C" fn(handle: NveRuntimeHandle) -> u32,
    pub last_error: unsafe extern "C" fn(out_buf: *mut u8, cap: usize) -> usize,
    pub measure_surface: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        constraints: *const NveSizeConstraints,
        flags: u32,
        out_measure: *mut NveSurfaceMeasure,
    ) -> i32,
    pub set_intrinsic_size_invalidated_callback: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        cb: Option<NveRuntimeIntrinsicSizeInvalidatedFn>,
        ctx: *mut c_void,
    ) -> i32,
    pub save_frame0_file: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        utf8_path: NveStrSlice,
        app_build_id: u64,
        env_hash: u64,
    ) -> i32,
    pub boot_frame0_file: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        utf8_path: NveStrSlice,
        app_build_id: u64,
        env_hash: u64,
    ) -> i32,
    pub boot_frame0_bytes: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        envelope: NveBufferSlice,
        app_build_id: u64,
        env_hash: u64,
    ) -> i32,
    pub set_host_callbacks: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        callbacks: *const NveHostCallbacks,
    ) -> i32,
    pub complete_host_request: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        request_id: HostRequestId,
        status: i32,
        payload: NveBufferSlice,
    ) -> i32,
    pub push_host_event: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        service_id: HostServiceId,
        event_id: u32,
        payload: NveBufferSlice,
    ) -> i32,
    pub publish_host_channel: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        channel_id: u32,
        flags: u32,
        payload: NveBufferSlice,
    ) -> i32,
}

/// Entry-point signature exported by `libnve_runtime` (`nve_runtime_get_interface`).
pub type NveRuntimeGetInterfaceFn = unsafe extern "C" fn(abi_version: u32) -> *const NveRuntimeApi;

pub(super) const fn hash_rt_signatures(h: &mut Xxh64Hasher, tag: &[u8], sigs: &[RuntimeFnSignature]) {
    h.update(tag);
    let mut i = 0;
    while i < sigs.len() {
        h.update(b"|fn#");
        h.update_u64(i as u64);
        h.update(b":");
        h.update(sigs[i].name.as_bytes());
        h.update(b"=");
        h.update(sigs[i].ret.as_bytes());
        h.update(b"(");
        h.update(sigs[i].params.as_bytes());
        h.update(b")");
        i += 1;
    }
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Proves [`RUNTIME_API_SLOTS`] names every [`NveRuntimeApi`] slot in order at pointer stride.
macro_rules! lock_runtime_api_slots {
    ($($slot:ident),* $(,)?) => {
        const _: () = {
            assert!(offset_of!(NveRuntimeApi, abi_version) == 0);
            assert!(offset_of!(NveRuntimeApi, struct_size) == 4);
            assert!(offset_of!(NveRuntimeApi, schema_hash) == 8);
            let mut i = 0;
            $(
                assert!(str_eq(RUNTIME_API_SLOTS[i].name, stringify!($slot)));
                assert!(offset_of!(NveRuntimeApi, $slot) == 16 + i * size_of::<usize>());
                i += 1;
            )*
            assert!(i == RUNTIME_API_SLOTS.len());
            let raw_size = 16 + i * size_of::<usize>();
            let align = align_of::<NveRuntimeApi>();
            assert!(size_of::<NveRuntimeApi>() == (raw_size + align - 1) & !(align - 1));
        };
    };
}

lock_runtime_api_slots!(
    create,
    destroy,
    set_wake_callback,
    set_viewport,
    set_text_measurer,
    attach_app_interface,
    load_app_module,
    set_props,
    drain_emitted_events,
    snapshot_state,
    restore_state,
    save_state_file,
    restore_state_file,
    clear_state_file,
    apply_hot_patch,
    submit_nve2,
    tick,
    dispatch_events,
    hit_test,
    scroll_by,
    scene_acquire,
    scene_release,
    resource_payload,
    get_metrics,
    pair_connect,
    pair_disconnect,
    pair_is_connected,
    last_error,
    measure_surface,
    set_intrinsic_size_invalidated_callback,
    save_frame0_file,
    boot_frame0_file,
    boot_frame0_bytes,
    set_host_callbacks,
    complete_host_request,
    push_host_event,
    publish_host_channel,
);

const _: () = {
    const P: usize = size_of::<usize>();
    assert!(align_of::<NveRuntimeApi>() == align_of::<u64>());
    assert!(offset_of!(NveRuntimeApi, abi_version) == 0);
    assert!(offset_of!(NveRuntimeApi, struct_size) == 4);
    assert!(offset_of!(NveRuntimeApi, schema_hash) == 8);
    assert!(offset_of!(NveRuntimeApi, create) == 16);
    assert!(offset_of!(NveRuntimeApi, destroy) == 16 + P);
    assert!(offset_of!(NveRuntimeApi, set_wake_callback) == 16 + 2 * P);
    assert!(offset_of!(NveRuntimeApi, set_viewport) == 16 + 3 * P);
    assert!(offset_of!(NveRuntimeApi, set_text_measurer) == 16 + 4 * P);
    assert!(offset_of!(NveRuntimeApi, attach_app_interface) == 16 + 5 * P);
    assert!(offset_of!(NveRuntimeApi, load_app_module) == 16 + 6 * P);
    assert!(offset_of!(NveRuntimeApi, set_props) == 16 + 7 * P);
    assert!(offset_of!(NveRuntimeApi, drain_emitted_events) == 16 + 8 * P);
    assert!(offset_of!(NveRuntimeApi, snapshot_state) == 16 + 9 * P);
    assert!(offset_of!(NveRuntimeApi, restore_state) == 16 + 10 * P);
    assert!(offset_of!(NveRuntimeApi, save_state_file) == 16 + 11 * P);
    assert!(offset_of!(NveRuntimeApi, restore_state_file) == 16 + 12 * P);
    assert!(offset_of!(NveRuntimeApi, clear_state_file) == 16 + 13 * P);
    assert!(offset_of!(NveRuntimeApi, apply_hot_patch) == 16 + 14 * P);
    assert!(offset_of!(NveRuntimeApi, submit_nve2) == 16 + 15 * P);
    assert!(offset_of!(NveRuntimeApi, tick) == 16 + 16 * P);
    assert!(offset_of!(NveRuntimeApi, dispatch_events) == 16 + 17 * P);
    assert!(offset_of!(NveRuntimeApi, hit_test) == 16 + 18 * P);
    assert!(offset_of!(NveRuntimeApi, scroll_by) == 16 + 19 * P);
    assert!(offset_of!(NveRuntimeApi, scene_acquire) == 16 + 20 * P);
    assert!(offset_of!(NveRuntimeApi, scene_release) == 16 + 21 * P);
    assert!(offset_of!(NveRuntimeApi, resource_payload) == 16 + 22 * P);
    assert!(offset_of!(NveRuntimeApi, get_metrics) == 16 + 23 * P);
    assert!(offset_of!(NveRuntimeApi, pair_connect) == 16 + 24 * P);
    assert!(offset_of!(NveRuntimeApi, pair_disconnect) == 16 + 25 * P);
    assert!(offset_of!(NveRuntimeApi, pair_is_connected) == 16 + 26 * P);
    assert!(offset_of!(NveRuntimeApi, last_error) == 16 + 27 * P);
    assert!(offset_of!(NveRuntimeApi, measure_surface) == 16 + 28 * P);
    assert!(offset_of!(NveRuntimeApi, set_intrinsic_size_invalidated_callback) == 16 + 29 * P);
    assert!(offset_of!(NveRuntimeApi, save_frame0_file) == 16 + 30 * P);
    assert!(offset_of!(NveRuntimeApi, boot_frame0_file) == 16 + 31 * P);
    assert!(offset_of!(NveRuntimeApi, boot_frame0_bytes) == 16 + 32 * P);
    assert!(offset_of!(NveRuntimeApi, set_host_callbacks) == 16 + 33 * P);
    assert!(offset_of!(NveRuntimeApi, complete_host_request) == 16 + 34 * P);
    assert!(offset_of!(NveRuntimeApi, push_host_event) == 16 + 35 * P);
    assert!(offset_of!(NveRuntimeApi, publish_host_channel) == 16 + 36 * P);
};
