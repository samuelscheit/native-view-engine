//! `NveRuntimeApi` vtable plus its single-source C signature descriptors
//! ([`RUNTIME_CALLBACK_TYPEDEFS`], [`RUNTIME_API_SLOTS`]) and their compile-time slot locks.

use core::ffi::c_void;
use core::mem::{offset_of, size_of};

use nve_hash::Xxh64Hasher;

use super::{
    NveRuntimeConfig, NveRuntimeHandle, NveRuntimeMeasureTextFn, NveRuntimeMetrics,
    NveSceneView, NveRuntimeWakeFn,
};
use crate::ffi::{NveBufferSlice, NveGetInterfaceFn};
use crate::value::{NveEventBatch, NveMapEntry, NveStrSlice};

/// C spelling of one runtime ABI function-pointer signature.
///
/// [`RUNTIME_CALLBACK_TYPEDEFS`] and [`RUNTIME_API_SLOTS`] are the single source for
/// [`NVE_RUNTIME_SCHEMA_HASH`] and the generated `nve_runtime_abi.generated.h`; the
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
        out_node_id: *mut u32,
    ) -> i32,
    pub scroll_by: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        node_id: u32,
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
            assert!(size_of::<NveRuntimeApi>() == 16 + i * size_of::<usize>());
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
);
